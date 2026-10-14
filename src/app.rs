use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use rust_encoder::{
    encode::{self, Event},
    media::{self, MediaInfo},
    model::{self, ColorSignal, EncodeOptions, HardwareEncoder, Preset, Settings},
    player::{self, PlaybackSession},
    preview_stream::{self, NativeVideoPreview},
    audio_preview::NativeAudioPreview,
    dependencies::{self, State as DependencyState},
    hardware::{self, Capabilities},
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

#[derive(Debug)]
enum JobState {
    Analyzing,
    Queued,
    Running,
    Cancelling,
    Completed,
    Failed(String),
    Cancelled,
}

impl JobState {
    fn label(&self) -> &'static str {
        match self {
            Self::Analyzing => "Inspecting",
            Self::Queued => "Ready",
            Self::Running => "Encoding",
            Self::Cancelling => "Cancelling…",
            Self::Completed => "Completed",
            Self::Failed(_) => "Failed",
            Self::Cancelled => "Cancelled",
        }
    }
    fn is_pending(&self) -> bool {
        matches!(self, Self::Analyzing | Self::Queued)
    }
}

struct Job {
    id: u64,
    input: PathBuf,
    output: Option<PathBuf>,
    media: Option<MediaInfo>,
    options: EncodeOptions,
    status: JobState,
    elapsed_sec: f64,
    speed: String,
    pass: u8,
}

struct Running {
    id: u64,
    cancelled: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
}

pub struct EncoderApp {
    settings: Settings,
    jobs: Vec<Job>,
    sender: Sender<Event>,
    receiver: Receiver<Event>,
    dependency_sender: Sender<DependencyState>,
    dependency_receiver: Receiver<DependencyState>,
    dependency_state: DependencyState,
    hardware_sender: Sender<Result<Capabilities, String>>,
    hardware_receiver: Receiver<Result<Capabilities, String>>,
    hardware_scan: Option<Capabilities>,
    hardware_error: Option<String>,
    hardware_scanning: bool,
    pending_input_paths: Vec<PathBuf>,
    next_id: u64,
    selected_job: Option<u64>,
    queue_filter: String,
    compact_queue: bool,
    preview_for: Option<(u64, u64)>,
    preview_loading: bool,
    preview_playing: bool,
    av_player: Option<PlaybackSession>,
    native_video: Option<NativeVideoPreview>,
    native_audio: Option<NativeAudioPreview>,
    native_paused_job: Option<u64>,
    native_error: Option<String>,
    native_audio_error: Option<String>,
    native_held_frame: Option<preview_stream::StreamFrame>,
    native_volume: f32,
    preview_last_tick: Instant,
    waveform_for: Option<u64>,
    waveform_pixels: Option<media::Thumbnail>,
    waveform_texture: Option<egui::TextureHandle>,
    waveform_error: Option<String>,
    preview_owner: Option<u64>,
    preview_seconds: f64,
    preview_requested_seconds: f64,
    preview_texture: Option<egui::TextureHandle>,
    preview_pixels: Option<media::Thumbnail>,
    preview_error: Option<String>,
    current: Option<Running>,
    process_queue: bool,
    notice: String,
    dirty_since: Option<Instant>,
}

impl EncoderApp {
    pub fn new() -> Self {
        let (sender, receiver) = crossbeam_channel::unbounded();
        let (dependency_sender, dependency_receiver) = crossbeam_channel::unbounded();
        let (hardware_sender, hardware_receiver) = crossbeam_channel::unbounded();
        let settings = model::load_settings();
        // Unit tests construct the GUI without a running desktop session.
        // Never trigger external network installers merely by constructing a test app.
        if !cfg!(test) {
            dependencies::start_check(
                settings.ffmpeg_path.clone(),
                settings.ffprobe_path.clone(),
                dependency_sender.clone()
            );
        }
        Self {
            settings,
            jobs: Vec::new(),
            sender,
            receiver,
            dependency_sender,
            dependency_receiver,
            dependency_state: DependencyState::Checking,
            hardware_sender,
            hardware_receiver,
            hardware_scan: None,
            hardware_error: None,
            hardware_scanning: false,
            pending_input_paths: Vec::new(),
            next_id: 1,
            selected_job: None,
            queue_filter: String::new(),
            compact_queue: true,
            preview_for: None,
            preview_loading: false,
            preview_playing: false,
            av_player: None,
            native_video: None,
            native_audio: None,
            native_paused_job: None,
            native_error: None,
            native_audio_error: None,
            native_held_frame: None,
            native_volume: 0.8,
            preview_last_tick: Instant::now(),
            waveform_for: None,
            waveform_pixels: None,
            waveform_texture: None,
            waveform_error: None,
            preview_owner: None,
            preview_seconds: 0.0,
            preview_requested_seconds: 0.0,
            preview_texture: None,
            preview_pixels: None,
            preview_error: None,
            current: None,
            process_queue: false,
            notice: String::new(),
            dirty_since: None,
        }
    }

    fn add_files(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        if !matches!(self.dependency_state, DependencyState::Ready { .. }) {
            self.pending_input_paths.extend(paths.into_iter().filter(|p| p.is_file()));
            self.notice = "Files queued until FFmpeg and FFprobe are available.".into();
            return;
        }
        let probe_tool = model::resolve_tool("ffprobe", &self.settings.ffprobe_path);
        for path in paths {
            if !path.is_file() { continue; }
            let id = self.next_id;
            self.next_id += 1;
            self.jobs.push(Job {
                id,
                input: path.clone(),
                output: None,
                media: None,
                options: self.settings.encode_options(),
                status: JobState::Analyzing,
                elapsed_sec: 0.0,
                speed: String::new(),
            pass: 1,
            });
            self.selected_job.get_or_insert(id);
            media::probe_async(id, probe_tool.clone(), path, self.sender.clone());
        }
    }

    /// Keep queued jobs in sync with the selected function after importing files.
    /// Pending jobs follow the editor; running and completed jobs remain immutable.
    fn update_pending_options(&mut self) {
        let options = self.settings.encode_options();
        for job in &mut self.jobs {
            if job.status.is_pending() {
                job.options = options.clone();
            }
        }
    }

    fn mark_settings_changed(&mut self) {
        self.dirty_since = Some(Instant::now());
        self.update_pending_options();
    }

    fn save_when_idle(&mut self, ctx: &egui::Context) {
        if let Some(since) = self.dirty_since {
            if since.elapsed() >= Duration::from_millis(400) {
                if let Err(error) = model::save_settings(&self.settings) {
                    self.notice = format!("Unable to save settings: {error}");
                }
                self.dirty_since = None;
            } else {
                ctx.request_repaint_after(Duration::from_millis(420));
            }
        }
    }

    fn poll_dependencies(&mut self) {
        while let Ok(state) = self.dependency_receiver.try_recv() {
            if let DependencyState::Ready { installed, .. } = &state {
                // Invalid saved override paths must not shadow the newly installed tools.
                if !dependencies::usable(&model::resolve_tool("ffmpeg", &self.settings.ffmpeg_path)) {
                    self.settings.ffmpeg_path.clear();
                    self.mark_settings_changed();
                }
                if !dependencies::usable(&model::resolve_tool("ffprobe", &self.settings.ffprobe_path)) {
                    self.settings.ffprobe_path.clear();
                    self.mark_settings_changed();
                }
                if *installed {
                    self.notice = "FFmpeg and FFprobe installed in the Rust Encoder data directory.".into();
                }
            }
            let ready = matches!(state, DependencyState::Ready { .. });
            self.dependency_state = state;
            if ready && !self.hardware_scanning && self.hardware_scan.is_none() {
                self.start_hardware_scan();
            }
            if ready && !self.pending_input_paths.is_empty() {
                let paths = std::mem::take(&mut self.pending_input_paths);
                self.add_files(paths);
            }
        }
    }

    fn start_hardware_scan(&mut self) {
        self.hardware_scanning = true;
        self.hardware_error = None;
        self.hardware_scan = None;
        hardware::start_scan(
            model::resolve_tool("ffmpeg", &self.settings.ffmpeg_path),
            self.hardware_sender.clone(),
        );
    }

    fn poll_hardware_scan(&mut self) {
        while let Ok(result) = self.hardware_receiver.try_recv() {
            self.hardware_scanning = false;
            match result {
                Ok(report) => {
                    if report.ffmpeg != model::resolve_tool("ffmpeg", &self.settings.ffmpeg_path) {
                        continue; // an older scan finished after executable change
                    }
                    if !report.supports(self.settings.hardware_encoder, self.settings.preset)
                        && self.settings.hardware_encoder != HardwareEncoder::Software
                    {
                        self.settings.hardware_encoder = HardwareEncoder::Software;
                        self.settings.two_pass = false;
                        self.mark_settings_changed();
                        self.notice = "Previously selected hardware encoder is unavailable. Using software.".into();
                    }
                    self.hardware_scan = Some(report);
                }
                Err(error) => self.hardware_error = Some(error),
            }
        }
    }

    fn retry_dependencies(&mut self) {
        self.dependency_state = DependencyState::Checking;
        dependencies::start_check(
            self.settings.ffmpeg_path.clone(),
            self.settings.ffprobe_path.clone(),
            self.dependency_sender.clone(),
        );
    }

    fn dependencies_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            match &self.dependency_state {
                DependencyState::Checking => { ui.spinner(); ui.label("Checking FFmpeg tools…"); }
                DependencyState::Downloading(message) => {
                    ui.spinner();
                    ui.label(message);
                }
                DependencyState::Extracting => {
                    ui.spinner();
                    ui.label("Verifying and installing FFmpeg/FFprobe into user data…");
                }
                DependencyState::Ready { installed, directory } => {
                    let status = if *installed { "FFmpeg and FFprobe installed" }
                        else { "FFmpeg and FFprobe available" };
                    ui.label(status);
                    if let Some(dir) = directory {
                        ui.small(dir.display().to_string());
                    }
                }
                DependencyState::Failed(message) => {
                    ui.colored_label(egui::Color32::LIGHT_RED, message);
                }
            }
            if matches!(self.dependency_state, DependencyState::Failed(_) | DependencyState::Ready { .. })
                && ui.button("Recheck / install tools").clicked() {
                    self.retry_dependencies();
            }
        });
    }

    fn poll_events(&mut self) {
        while let Ok(event) = self.receiver.try_recv() {
            match event {
                Event::Probed { id, result } => {
                    if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id)
                        && matches!(job.status, JobState::Analyzing)
                    {
                        match result {
                            Ok(media) => { job.media = Some(media); job.status = JobState::Queued; }
                            Err(error) => job.status = JobState::Failed(error),
                        }
                    }
                }
                Event::PreviewReady { id, seek_ms, result } => {
                    if self.preview_for == Some((id, seek_ms)) {
                        self.preview_loading = false;
                        let requested_ms = (self.preview_requested_seconds.max(0.0) * 1000.0).round() as u64;
                        if self.selected_job == Some(id) && requested_ms == seek_ms {
                            match result {
                                Ok(preview) => {
                                    self.preview_pixels = Some(preview);
                                    self.preview_error = None;
                                }
                                Err(message) => {
                                    self.preview_texture = None;
                                    self.preview_error = Some(message);
                                }
                            }
                        } else {
                            // A new seek superseded the request while FFmpeg was decoding.
                            // Let ensure_preview send the latest request next frame.
                            self.preview_for = None;
                        }
                    }
                }
                Event::WaveformReady { id, result } => {
                    if self.waveform_for == Some(id) && self.selected_job == Some(id) {
                        match result {
                            Ok(waveform) => self.waveform_pixels = Some(waveform),
                            Err(message) => self.waveform_error = Some(message),
                        }
                    }
                }
                Event::Started { id, output } => {
                    if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
                        job.output = Some(output);
                    }
                }
                Event::Progress { id, elapsed_sec, speed, pass } => {
                    if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
                        job.elapsed_sec = elapsed_sec;
                        job.speed = speed;
                        job.pass = pass;
                    }
                }
                Event::Completed { id, output } => {
                    if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
                        job.output = Some(output);
                        job.status = JobState::Completed;
                    }
                    self.release_running(id);
                }
                Event::Failed { id, message } => {
                    if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
                        job.status = JobState::Failed(message);
                    }
                    self.release_running(id);
                }
                Event::Cancelled { id } => {
                    if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
                        job.status = JobState::Cancelled;
                    }
                    self.release_running(id);
                }
            }
        }
    }

    fn release_running(&mut self, id: u64) {
        if self.current.as_ref().is_some_and(|r| r.id == id) {
            self.current = None;
        }
    }

    fn start_next(&mut self) {
        if !matches!(self.dependency_state, DependencyState::Ready { .. }) { return; }
        if !self.process_queue || self.current.is_some() { return; }
        if let Some(job) = self.jobs.iter_mut().find(|j| matches!(j.status, JobState::Queued)) {
            let flag = Arc::new(AtomicBool::new(false));
            let stopped = Arc::new(AtomicBool::new(false));
            job.status = JobState::Running;
            self.current = Some(Running {
                id: job.id,
                cancelled: flag.clone(),
                stopped: stopped.clone(),
            });
            encode::start_job(job.id, job.input.clone(), job.options.clone(), self.sender.clone(), flag, stopped);
        } else if !self.jobs.iter().any(|j| matches!(j.status, JobState::Analyzing)) {
            self.process_queue = false;
        }
    }

    fn cancel_running(&mut self) {
        if let Some(running) = &self.current {
            running.cancelled.store(true, Ordering::SeqCst);
            if let Some(job) = self.jobs.iter_mut().find(|j| j.id == running.id) {
                job.status = JobState::Cancelling;
            }
        }
    }

    fn retry_failed(&mut self, id: u64) {
        if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id)
            && matches!(job.status, JobState::Failed(_))
        {
            job.media = None;
            job.output = None;
            job.elapsed_sec = 0.0;
            job.speed.clear();
            job.options = self.settings.encode_options();
            job.status = JobState::Analyzing;
            let probe_tool = model::resolve_tool("ffprobe", &self.settings.ffprobe_path);
            media::probe_async(id, probe_tool, job.input.clone(), self.sender.clone());
        }
    }

    fn header_ui(&mut self, ui: &mut egui::Ui) {
        self.dependencies_ui(ui);
        ui.horizontal_wrapped(|ui| {
            ui.heading("Rust Encoder");
            ui.label("Media encoder");
            ui.separator();
            if ui.button("Save profile…").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Rust Encoder profile", &["json"])
                    .set_file_name("encoder-profile.json").save_file()
            {
                match serde_json::to_vec_pretty(&self.settings)
                    .map_err(|e| e.to_string())
                    .and_then(|data| std::fs::write(path, data).map_err(|e| e.to_string()))
                {
                    Ok(()) => self.notice = "Profile saved.".into(),
                    Err(e) => self.notice = format!("Unable to save profile: {e}"),
                }
            }
            if ui.button("Load profile…").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Rust Encoder profile", &["json"]).pick_file()
            {
                match std::fs::read(path).map_err(|e| e.to_string())
                    .and_then(|bytes| serde_json::from_slice::<Settings>(&bytes).map_err(|e| e.to_string()))
                {
                    Ok(settings) => { self.settings = settings; self.mark_settings_changed(); self.notice = "Profile loaded.".into(); }
                    Err(e) => self.notice = format!("Unable to load profile: {e}"),
                }
            }
            ui.separator();
            if ui.checkbox(&mut self.settings.dark_theme, "Dark theme").changed() {
                self.mark_settings_changed();
            }
            if ui.button("Reset layout").on_hover_text("Restore default pane widths").clicked() {
                self.settings.workspace_left_fraction = 0.21;
                self.settings.workspace_right_fraction = 0.29;
                self.mark_settings_changed();
            }
        });
        ui.separator();
    }

    fn files_ui(&mut self, ui: &mut egui::Ui, list_height: f32) {
        ui.horizontal_wrapped(|ui| {
            ui.heading(format!("Choose files ({})", self.jobs.len()));
            if ui.button("Add files…").clicked()
                && let Some(files) = rfd::FileDialog::new().pick_files()
            {
                self.add_files(files);
            }
            if ui.button("Clear finished").clicked() {
                self.jobs.retain(|j| !matches!(j.status, JobState::Completed | JobState::Cancelled | JobState::Failed(_)));
                if !self.jobs.iter().any(|j| Some(j.id) == self.selected_job) {
                    self.selected_job = self.jobs.first().map(|j| j.id);
                }
            }
        });
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.queue_filter)
                .hint_text("Filter files…").desired_width((ui.available_width()-105.0).max(80.0)));
            ui.checkbox(&mut self.compact_queue, "Compact");
        });
        ui.small("Drop files here. Settings apply to pending jobs.");
        ui.separator();

        let mut remove_id = None;
        let mut retry_id = None;
        let visible = matching_jobs(&self.jobs, self.queue_filter.trim());
        if self.jobs.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(20.0);
                ui.label("No files added");
                ui.small("Choose files or drop media here.");
            });
        } else if visible.is_empty() {
            ui.small("No files match the filter.");
        } else if self.compact_queue {
            // A fixed one-line height lets egui skip allocation/painting for all
            // off-screen jobs. The older layout constructed widgets for every file.
            let row_height = ui.spacing().interact_size.y.max(22.0);
            egui::ScrollArea::vertical()
                .id_salt("file-queue-compact")
                .max_height(list_height)
                .auto_shrink([false, false])
                .show_rows(ui, row_height, visible.len(), |ui, rows| {
                    for row in rows {
                        let job = &self.jobs[visible[row]];
                        ui.push_id(job.id, |ui| {
                            ui.horizontal(|ui| {
                                let filename = job.input.file_name()
                                    .map(|x| x.to_string_lossy()).unwrap_or_default();
                                let selected = self.selected_job == Some(job.id);
                                let name = format!("{} · {}", filename, job.status.label());
                                if ui.selectable_label(selected, name)
                                    .on_hover_text(job.input.display().to_string()).clicked() {
                                    self.selected_job = Some(job.id);
                                }
                                if matches!(job.status, JobState::Running | JobState::Cancelling) {
                                    let fraction = job.media.as_ref()
                                        .and_then(|m| encode::expected_duration(m, &job.options))
                                        .map(|d| if job.options.two_pass {
                                        (((job.pass as f64 - 1.0) + job.elapsed_sec / d) / 2.0).clamp(0.0, 1.0) as f32
                                    } else { (job.elapsed_sec / d).clamp(0.0, 1.0) as f32 })
                                        .unwrap_or(0.0);
                                    ui.add(egui::ProgressBar::new(fraction).desired_width(56.0));
                                }
                                if matches!(job.status, JobState::Failed(_))
                                    && ui.small_button("↻").on_hover_text("Retry").clicked() {
                                    retry_id = Some(job.id);
                                }
                                if !matches!(job.status, JobState::Running | JobState::Cancelling | JobState::Analyzing)
                                    && ui.small_button("×").on_hover_text("Remove").clicked() {
                                    remove_id = Some(job.id);
                                }
                            });
                        });
                    }
                });
        } else {
            // Variable-height detailed cards use normal scrolling; fixed-height
            // virtualization would overlap progress bars and error details.
            egui::ScrollArea::vertical()
                .id_salt("file-queue-detailed")
                .max_height(list_height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for index in visible {
                        let job = &self.jobs[index];
                        ui.push_id(job.id, |ui| {
                    ui.group(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            let filename = job.input.file_name()
                                .map(|v| v.to_string_lossy()).unwrap_or_default();
                            if ui.selectable_label(self.selected_job == Some(job.id), filename.as_ref())
                                .on_hover_text(job.input.display().to_string()).clicked()
                            {
                                self.selected_job = Some(job.id);
                            }
                            ui.small(job.status.label());
                            if matches!(job.status, JobState::Failed(_)) && ui.small_button("Retry").clicked() {
                                retry_id = Some(job.id);
                            }
                            if !matches!(job.status, JobState::Running | JobState::Cancelling | JobState::Analyzing)
                                && ui.small_button("Remove").clicked()
                            {
                                remove_id = Some(job.id);
                            }
                        });
                        ui.small(format!("{} · {}", job.options.preset.label(), job.input.display()));
                        if let Some(media) = &job.media { ui.small(media.summary()); }
                        if matches!(job.status, JobState::Running | JobState::Cancelling | JobState::Completed) {
                            let fraction = if matches!(job.status, JobState::Completed) { 1.0 }
                                else {
                                    job.media.as_ref()
                                        .and_then(|m| encode::expected_duration(m, &job.options))
                                        .map(|d| if job.options.two_pass {
                                            (((job.pass as f64 - 1.0) + job.elapsed_sec / d) / 2.0).clamp(0.0, 1.0) as f32
                                        } else { (job.elapsed_sec / d).clamp(0.0, 1.0) as f32 })
                                        .unwrap_or(0.0)
                                };
                            let label = if job.speed.is_empty() { "Processing".to_owned() }
                                else { format!("{} · {:.1}s encoded", job.speed, job.elapsed_sec) };
                            ui.add(egui::ProgressBar::new(fraction).show_percentage().text(label));
                        }
                        if let Some(output) = &job.output {
                            ui.small(format!("Output: {}", output.display()));
                        }
                        if let JobState::Failed(error) = &job.status {
                            ui.collapsing("Error details", |ui| {
                                ui.colored_label(egui::Color32::LIGHT_RED, error);
                            });
                        }
                    });
                            ui.add_space(4.0);
                        });
                    }
                });
        }
        if let Some(id) = remove_id {
            self.jobs.retain(|j| j.id != id);
            if self.selected_job == Some(id) {
                self.selected_job = self.jobs.first().map(|j| j.id);
            }
        }
        if let Some(id) = retry_id { self.retry_failed(id); }
    }

    /// Keep at most one preview decode in flight per selected file. Rapid scrubs
    /// supersede pending requests instead of starting an FFmpeg process per pixel.
    fn ensure_preview(&mut self) {
        if self.preview_owner != self.selected_job {
            self.preview_owner = self.selected_job;
            self.preview_seconds = 0.0;
            self.preview_requested_seconds = 0.0;
            self.preview_for = None;
            self.preview_loading = false;
            self.preview_playing = false;
            self.av_player = None;
            self.native_video = None;
            self.native_audio = None;
            self.native_paused_job = None;
            self.native_held_frame = None;
            self.native_error = None;
            self.native_audio_error = None;
            self.preview_pixels = None;
            self.preview_texture = None;
            self.preview_error = None;
            self.waveform_for = None;
            self.waveform_pixels = None;
            self.waveform_texture = None;
            self.waveform_error = None;
        }
        let Some(id) = self.selected_job else { return; };
        let Some(job) = self.jobs.iter().find(|j| j.id == id) else { return; };
        let Some(info) = &job.media else {
            if !matches!(job.status, JobState::Analyzing) {
                self.preview_error = Some("Media inspection did not succeed.".into());
            }
            return;
        };
        if self.waveform_for != Some(id) {
            self.waveform_for = Some(id);
            if !info.audio_tracks.is_empty() {
                media::waveform_async(id,
                    model::resolve_tool("ffmpeg", &self.settings.ffmpeg_path),
                    job.input.clone(), self.sender.clone());
            }
        }
        if info.video_codec.is_none() || self.native_video.is_some() || self.native_audio.is_some() { return; }
        let seek_ms = (self.preview_requested_seconds.max(0.0) * 1000.0).round() as u64;
        if self.preview_loading || self.preview_for == Some((id, seek_ms)) { return; }
        self.preview_for = Some((id, seek_ms));
        self.preview_loading = true;
        self.preview_error = None;
        media::thumbnail_async(
            id, seek_ms,
            model::resolve_tool("ffmpeg", &self.settings.ffmpeg_path),
            job.input.clone(), self.sender.clone(),
        );
    }

    fn stop_native_video(&mut self) {
        self.native_audio = None;
        self.native_held_frame = None;
        self.native_video = None;
        self.native_paused_job = None;
        self.preview_for = None;
        self.preview_loading = false;
        self.preview_requested_seconds = self.preview_seconds;
    }

    /// Pause at the output audio clock, preserving the playhead for resume.
    /// We restart FFmpeg on resume rather than freezing an OS decoder process.
    fn pause_native_video(&mut self) {
        let id = self.native_video.as_ref().map(|stream| stream.job_id)
            .or_else(|| self.native_audio.as_ref().map(|audio| audio.job_id));
        if let Some(audio) = self.native_audio.as_ref() {
            let timestamp = audio.playhead();
            if timestamp.is_finite() {
                self.preview_seconds = timestamp.max(0.0);
            }
        }
        self.stop_native_video();
        self.native_paused_job = id;
    }

    fn launch_native_video(&mut self, job_id: u64, input: PathBuf,
                           position: f64, stop: Option<f64>) {
        self.stop_native_video();
        self.av_player = None;
        self.preview_playing = false;
        self.native_error = None;
        self.native_audio_error = None;
        let ffmpeg = model::resolve_tool("ffmpeg", &self.settings.ffmpeg_path);
        let info = self.jobs.iter().find(|job| job.id == job_id)
            .and_then(|job| job.media.as_ref());
        let has_video = info.is_some_and(|media| media.video_codec.is_some());
        let audio_tracks = info.map(|media| media.audio_tracks.len()).unwrap_or(0);
        if has_video {
            match NativeVideoPreview::launch(job_id, &ffmpeg, &input, position, stop) {
                Ok(session) => self.native_video = Some(session),
                Err(error) => self.native_error = Some(error),
            }
        }
        if audio_tracks > 0 && self.settings.audio_stream != -2 {
            let index = self.settings.audio_stream.max(0) as usize;
            if index < audio_tracks {
                match NativeAudioPreview::launch(job_id, &ffmpeg, &input, position, stop, index) {
                    Ok(audio) => {
                        audio.set_volume(self.native_volume);
                        self.native_audio = Some(audio);
                    }
                    Err(error) => self.native_audio_error = Some(error),
                }
            } else {
                self.native_audio_error = Some(format!(
                    "Audio track {index} does not exist in the selected media."
                ));
            }
        }
        if self.native_video.is_some() || self.native_audio.is_some() {
            self.notice.clear();
        } else if self.native_error.is_none() && self.native_audio_error.is_none() {
            self.native_error = Some("The selected media has no playable video or audio tracks.".into());
        }
    }

    fn update_native_video(&mut self, ctx: &egui::Context) {
        if self.native_video.as_ref().is_some_and(|stream| self.selected_job != Some(stream.job_id))
            || self.native_audio.as_ref().is_some_and(|audio| self.selected_job != Some(audio.job_id)) {
            self.stop_native_video();
        }
        if let Some(audio) = self.native_audio.as_ref() {
            if let Some(Err(error)) = audio.decoder_result() {
                self.native_audio_error = Some(error);
            }
            if audio.finished_playback() {
                self.native_audio = None;
            }
        }
        let clock = self.native_audio.as_ref().map(NativeAudioPreview::playhead);
        if self.native_video.is_none() && let Some(time) = clock {
            // Audio-only files and audio tails continue advancing the playhead.
            self.preview_seconds = time;
            self.preview_requested_seconds = time;
            ctx.request_repaint();
        }
        let mut ended = None;
        if let Some(stream) = self.native_video.as_ref() {
            // Hold the FIRST early frame instead of overwriting it with newer
            // early frames at every UI tick. That previously caused starvation.
            let frame = preview_stream::frame_at_clock(
                stream.frames(), &mut self.native_held_frame, clock.map(|t| t + 0.065)
            );
            if let Some(frame) = frame {
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [preview_stream::WIDTH, preview_stream::HEIGHT], &frame.rgba
                );
                if let Some(texture) = self.preview_texture.as_mut() {
                    texture.set(image, egui::TextureOptions::LINEAR);
                } else {
                    self.preview_texture = Some(ctx.load_texture(
                        "native-stream-frame", image, egui::TextureOptions::LINEAR
                    ));
                }
                self.preview_seconds = frame.position;
                self.preview_requested_seconds = frame.position;
                ctx.request_repaint();
            }
            ended = stream.try_finished();
        }
        if let Some(result) = ended {
            // Video can finish before audio, e.g. a music outro, an audio track
            // longer than the picture or encoder priming/tail samples.
            self.native_video = None;
            self.native_held_frame = None;
            if let Err(error) = result { self.native_error = Some(error); }
            if self.native_audio.is_none() {
                self.preview_for = None;
                self.preview_loading = false;
                self.preview_requested_seconds = self.preview_seconds;
            }
        }
    }

    fn refresh_av_player(&mut self) {
        let mut finished = false;
        if let Some(player) = self.av_player.as_mut() {
            if self.selected_job != Some(player.job_id)
                || !self.jobs.iter().any(|job| job.id == player.job_id) {
                finished = true;
            } else {
                match player.try_wait() {
                    Ok(Some(status)) => {
                        if !status.success() {
                            self.notice = format!("FFplay closed ({status}). Verify FFplay and media compatibility.");
                        }
                        finished = true;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        self.notice = format!("Cannot check FFplay process: {error}");
                        finished = true;
                    }
                }
            }
        }
        if finished { self.av_player = None; }
    }

    fn launch_av_preview(&mut self, job_id: u64, input: PathBuf,
                         position: f64, stop: Option<f64>) {
        self.av_player = None;
        self.stop_native_video();
        self.preview_playing = false;
        let binary = player::resolve_ffplay(&self.settings);
        match PlaybackSession::launch(job_id, &binary, &input, position, stop) {
            Ok(session) => {
                self.av_player = Some(session);
                self.notice.clear();
            }
            Err(error) => self.notice = error,
        }
    }

    fn tick_playback(&mut self) {
        if !self.preview_playing { return; }
        let now = Instant::now();
        let elapsed = now.duration_since(self.preview_last_tick).as_secs_f64().min(0.4);
        self.preview_last_tick = now;
        let duration = self.jobs.iter().find(|j| Some(j.id) == self.selected_job)
            .and_then(|j| j.media.as_ref())
            .and_then(|m| m.duration_sec).unwrap_or(0.0);
        if duration <= 0.0 { self.preview_playing = false; return; }
        let (start, end) = preview_bounds(duration, self.settings.start_seconds, self.settings.end_seconds);
        if self.preview_seconds < start { self.preview_seconds = start; }
        self.preview_seconds = (self.preview_seconds + elapsed).min(end);
        // Decoding one accurate still per 250 ms is bounded-cost and UI-safe;
        // this is video-only sampled playback, not synchronized A/V playback.
        let requested_bucket = (self.preview_seconds * 4.0).floor() / 4.0;
        if requested_bucket > self.preview_requested_seconds || self.preview_seconds == end {
            self.preview_requested_seconds = self.preview_seconds;
        }
        if self.preview_seconds >= end { self.preview_playing = false; }
    }

    fn preview_ui(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.heading("Media preview / trim timeline");
        if let Some(preview) = self.preview_pixels.take() {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [preview.width, preview.height], &preview.rgba,
            );
            self.preview_texture = Some(ui.ctx().load_texture(
                "selected-media-thumbnail", image, egui::TextureOptions::LINEAR,
            ));
        }
        if let Some(waveform) = self.waveform_pixels.take() {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [waveform.width, waveform.height], &waveform.rgba,
            );
            self.waveform_texture = Some(ui.ctx().load_texture(
                "selected-audio-waveform", image, egui::TextureOptions::LINEAR,
            ));
        }
        if let Some(texture) = &self.preview_texture {
            let size = texture.size_vec2();
            let factor = (ui.available_width() / size.x).min(190.0 / size.y).min(1.0);
            ui.vertical_centered(|ui| {
                ui.image((texture.id(), size * factor.max(0.1)));
            });
        } else if let Some(error) = &self.preview_error {
            ui.small(error);
        } else if self.selected_job.is_some() {
            if self.jobs.iter().find(|j| Some(j.id) == self.selected_job)
                .and_then(|j| j.media.as_ref()).is_some_and(|m| m.video_codec.is_none()) {
                ui.small("Audio-only media: waveform and timeline below.");
            } else {
                ui.small("Waiting for a video preview…");
            }
        } else {
            ui.small("Select a file to inspect its video frame and metadata.");
        }
        let selected = self.jobs.iter().find(|j| Some(j.id) == self.selected_job)
            .and_then(|j| j.media.as_ref()).cloned();
        if let Some(info) = selected {
            ui.small(info.summary());
            // Use FFplay's native synchronized A/V renderer in a companion window.
            // The egui timeline remains a low-cost, silent sampled preview.
            ui.horizontal_wrapped(|ui| {
                let running = self.av_player.as_ref().is_some_and(|p| p.job_id == self.selected_job.unwrap_or(0));
                let title = if running { "▶ Restart A/V at playhead" }
                    else { "▶ Play A/V (FFplay window)" };
                if ui.button(title).clicked()
                    && let Some(job) = self.jobs.iter().find(|j| Some(j.id) == self.selected_job)
                {
                    let end = info.duration_sec.map(|duration| {
                        preview_bounds(duration, self.settings.start_seconds, self.settings.end_seconds)
                    });
                    let position = end.map(|(start, finish)| {
                        if self.preview_seconds < start || self.preview_seconds >= finish {
                            start
                        } else {
                            self.preview_seconds
                        }
                    }).unwrap_or(self.preview_seconds.max(0.0));
                    let stop = end.map(|(_, finish)| finish);
                    self.launch_av_preview(job.id, job.input.clone(), position, stop);
                }
                if ui.add_enabled(running, egui::Button::new("■ Stop A/V")).clicked() {
                    self.av_player = None;
                }
            });
            ui.small("FFplay opens a separate synchronized audio/video player. Space pauses there, arrows seek; close it or Stop A/V to exit.");
            if let Some(error) = &self.native_error {
                ui.colored_label(egui::Color32::LIGHT_RED, format!("Embedded preview: {error}"));
            }
            ui.horizontal_wrapped(|ui| {
                let live = self.native_video.as_ref().is_some_and(|p| Some(p.job_id) == self.selected_job)
                    || self.native_audio.as_ref().is_some_and(|a| Some(a.job_id) == self.selected_job);
                let paused = self.native_paused_job == self.selected_job;
                let playable = info.video_codec.is_some()
                    || (!info.audio_tracks.is_empty() && self.settings.audio_stream != -2);
                let play_label = if live { "▶ Restart embedded" }
                    else if paused { "▶ Resume embedded" }
                    else { "▶ Embedded playback (experimental)" };
                if ui.add_enabled(playable, egui::Button::new(play_label)).clicked()
                    && let Some(job) = self.jobs.iter().find(|j| Some(j.id) == self.selected_job)
                {
                    let (position, until) = info.duration_sec.map(|duration| {
                        let (start, end) = preview_bounds(duration, self.settings.start_seconds, self.settings.end_seconds);
                        let position = if self.preview_seconds >= start && self.preview_seconds < end {
                            self.preview_seconds
                        } else { start };
                        (position, Some(end))
                    }).unwrap_or((self.preview_seconds.max(0.0), None));
                    self.launch_native_video(job.id, job.input.clone(), position, until);
                }
                if ui.add_enabled(live, egui::Button::new("⏸ Pause embedded")).clicked() {
                    self.pause_native_video();
                }
                if ui.add_enabled(live || paused, egui::Button::new("■ Stop embedded")).clicked() {
                    self.stop_native_video();
                }
            });
            if let Some(error) = &self.native_audio_error {
                ui.colored_label(egui::Color32::YELLOW, format!("Native audio: {error}"));
                ui.small("FFplay remains available as an audio/video fallback.");
            }
            if let Some(audio) = self.native_audio.as_ref() {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Playback volume");
                    if ui.add(egui::Slider::new(&mut self.native_volume, 0.0..=1.0)).changed() {
                        audio.set_volume(self.native_volume);
                    }
                    if audio.underruns() > 4800 {
                        ui.small("Audio output may be underrunning.");
                    }
                });
            }
            ui.small("Embedded playback uses an approximate audio-master clock. Pause/Resume restarts decoding at the preserved playhead; FFplay remains the fallback.");
            if let Some(texture) = &self.waveform_texture {
                let size = egui::vec2(ui.available_width().max(100.0), 64.0);
                ui.image((texture.id(), size));
                ui.small("Waveform: first audio stream (whole file)");
            } else if let Some(error) = &self.waveform_error {
                ui.small(format!("Waveform: {error}"));
            } else if !info.audio_tracks.is_empty() {
                ui.small("Generating audio waveform…");
            }
            if let Some(duration) = info.duration_sec {
                let (start, end) = preview_bounds(duration, self.settings.start_seconds, self.settings.end_seconds);
                let width = ui.available_width().max(100.0);
                let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 24.0),
                    egui::Sense::click_and_drag());
                let painter = ui.painter();
                painter.rect_filled(rect, 4.0, ui.visuals().faint_bg_color);
                let x_at = |seconds: f64| rect.left() + (seconds / duration).clamp(0.0, 1.0) as f32 * rect.width();
                if end > start {
                    painter.rect_filled(
                        egui::Rect::from_min_max(egui::pos2(x_at(start), rect.top()),
                            egui::pos2(x_at(end), rect.bottom())),
                        2.0, egui::Color32::from_rgb(40, 105, 125));
                }
                for (timestamp, color) in [
                    (start, egui::Color32::LIGHT_GREEN),
                    (end, egui::Color32::LIGHT_RED),
                    (self.preview_seconds, egui::Color32::YELLOW),
                ] {
                    let x = x_at(timestamp);
                    painter.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                        egui::Stroke::new(2.0, color));
                }
                if (response.clicked() || response.dragged())
                    && let Some(pos) = response.interact_pointer_pos()
                {
                    self.preview_seconds = (duration *
                        ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64)
                        .clamp(0.0, duration);
                    self.preview_requested_seconds = self.preview_seconds;
                    self.preview_playing = false;
                    self.stop_native_video();
                    ui.ctx().request_repaint();
                }
                let frame_step = 1.0 / info.video_frame_rate.unwrap_or(30.0);
                let mut seek_changed = false;
                ui.horizontal_wrapped(|ui| {
                    if ui.add_enabled(info.video_codec.is_some(), egui::Button::new(
                        if self.preview_playing { "⏸ Pause" } else { "▶ Preview" }
                    )).clicked() {
                        if !self.preview_playing && self.preview_seconds >= end {
                            self.preview_seconds = start;
                            self.preview_requested_seconds = start;
                        }
                        self.stop_native_video();
                        self.preview_playing = !self.preview_playing;
                        self.preview_last_tick = Instant::now();
                    }
                    if ui.add_enabled(info.video_codec.is_some(), egui::Button::new("◀ Frame")).clicked() {
                        self.preview_seconds = (self.preview_seconds - frame_step).max(0.0);
                        seek_changed = true;
                    }
                    if ui.add_enabled(info.video_codec.is_some(), egui::Button::new("Frame ▶")).clicked() {
                        self.preview_seconds = (self.preview_seconds + frame_step).min(duration);
                        seek_changed = true;
                    }
                    ui.label(format!("{:.2}s / {:.2}s", self.preview_seconds, duration));
                });
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Mark In").clicked() {
                        self.settings.start_seconds = self.preview_seconds;
                        if self.settings.end_seconds > 0.0
                            && self.settings.end_seconds <= self.settings.start_seconds {
                            self.settings.end_seconds = 0.0;
                        }
                        self.mark_settings_changed();
                    }
                    if ui.button("Mark Out").clicked() {
                        if self.preview_seconds > self.settings.start_seconds {
                            self.settings.end_seconds = self.preview_seconds;
                            self.mark_settings_changed();
                        } else {
                            self.notice = "Out point must be after the In point".into();
                        }
                    }
                    if ui.button("Clear In/Out").clicked() {
                        self.settings.start_seconds = 0.0;
                        self.settings.end_seconds = 0.0;
                        self.mark_settings_changed();
                    }
                    ui.small(format!("Selection: {:.2}–{:.2}s", start, end));
                });
                if seek_changed {
                    self.stop_native_video();
                    self.preview_requested_seconds = self.preview_seconds;
                    self.preview_playing = false;
                }
                if self.preview_playing { ui.ctx().request_repaint_after(Duration::from_millis(50)); }
            }
            ui.collapsing("Detailed media streams", |ui| {
                for (kind, tracks) in [
                    ("Video", &info.video_tracks),
                    ("Audio", &info.audio_tracks),
                    ("Subtitles", &info.subtitle_tracks),
                ] {
                    ui.small(format!("{kind}: {}", tracks.len()));
                    for (n, track) in tracks.iter().enumerate() { ui.small(track.description(n)); }
                }
            });
        }
        ui.small("Experimental embedded A/V supports pause/resume, audio-only media and independently finishing audio tails. FFplay is the fallback; source-PTS synchronization, multi-cut EDL and audio-track editing remain unported.");
    }

    fn function_ui(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        ui.heading("Choose function");
        let previous_preset = self.settings.preset;
        egui::ComboBox::from_id_salt("preset")
            .selected_text(self.settings.preset.label())
            .width(ui.available_width().max(190.0))
            .show_ui(ui, |ui| {
                for category in ["Video codecs", "Stream copy", "Audio", "Images & subtitles", "Analysis"] {
                    ui.label(egui::RichText::new(category).strong());
                    for preset in Preset::ALL {
                        let group = if preset.video() { "Video codecs" }
                            else if preset.audio_only() { "Audio" }
                            else if preset.report() || preset == Preset::FrameMd5 { "Analysis" }
                            else if matches!(preset, Preset::RemuxMkv | Preset::RemuxMp4 | Preset::VideoOnly) { "Stream copy" }
                            else { "Images & subtitles" };
                        if group == category {
                            changed |= ui.selectable_value(&mut self.settings.preset, preset, preset.label()).changed();
                        }
                    }
                    ui.separator();
                }
            });
        if previous_preset != self.settings.preset {
            if !matches!(self.settings.preset, Preset::H264 | Preset::H265) {
                self.settings.hardware_encoder = HardwareEncoder::Software;
            }
            if !(self.settings.preset.video() || self.settings.preset.audio_only()) {
                self.settings.audio_channels = 0;
                self.settings.audio_sample_rate = 0;
                self.settings.audio_gain_db = 0.0;
            }
            if !self.settings.preset.video() {
                self.settings.target_width = 0;
                self.settings.frame_rate = 0;
                self.settings.crop_width = 0; self.settings.crop_height = 0;
                self.settings.crop_x = 0; self.settings.crop_y = 0;
                self.settings.rotate_degrees = 0;
                self.settings.flip_horizontal = false; self.settings.flip_vertical = false;
                self.settings.deinterlace = false; self.settings.denoise = false;
                self.settings.sharpen = false;
                self.settings.brightness = 0.0; self.settings.contrast = 1.0;
                self.settings.saturation = 1.0; self.settings.gamma = 1.0;
            }
        }
        ui.small("Function changes update queued jobs; active and completed jobs are unchanged.");
        ui.add_space(6.0);
        if changed { self.mark_settings_changed(); }
    }

    fn options_ui(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        ui.heading("Encoding settings");


        egui::CollapsingHeader::new("Image / codec")
            .default_open(true)
            .show(ui, |ui| {
                if self.settings.preset.video() {
                    if matches!(self.settings.preset, Preset::ProRes | Preset::Dnxhr) {
                        ui.small("Professional editing codec: profile controls quality.");
                    } else {
                        ui.label("Bitrate adjustment");
                        ui.horizontal_wrapped(|ui| {
                            changed |= ui.selectable_value(&mut self.settings.use_video_bitrate,
                                false, "Constant quality (CRF)").changed();
                            changed |= ui.selectable_value(&mut self.settings.use_video_bitrate,
                                true, "Target bitrate").changed();
                        });
                        if self.settings.use_video_bitrate {
                            let allowed = matches!(self.settings.preset, Preset::H264 | Preset::H265 | Preset::Vp9)
                                && self.settings.hardware_encoder == HardwareEncoder::Software;
                            changed |= ui.add_enabled(allowed,
                                egui::Checkbox::new(&mut self.settings.two_pass,
                                    "Two-pass bitrate encoding (slower, more precise)"
                                )).changed();
                            if !allowed && self.settings.two_pass {
                                self.settings.two_pass = false;
                                changed = true;
                            }
                            if !allowed { ui.small("Two-pass needs software H.264, H.265 or VP9."); }
                            ui.label("Target video bitrate (kb/s)");
                            changed |= ui.add(egui::DragValue::new(&mut self.settings.video_bitrate_kbps)
                                .range(100..=100_000).speed(50.0)).changed();
                            ui.label("Maximum bitrate (kb/s; 0 = uncapped)");
                            changed |= ui.add(egui::DragValue::new(&mut self.settings.max_video_bitrate_kbps)
                                .range(0..=200_000).speed(50.0)).changed();
                            if self.settings.max_video_bitrate_kbps > 0
                                && self.settings.max_video_bitrate_kbps < self.settings.video_bitrate_kbps {
                                ui.colored_label(egui::Color32::LIGHT_RED,
                                    "Peak bitrate must be at least the target bitrate.");
                            }
                            if let Some(duration) = self.jobs.iter().find(|j| Some(j.id) == self.selected_job)
                                .and_then(|j| j.media.as_ref()).and_then(|m| m.duration_sec)
                                && duration > 0.0
                            {
                                let mb = duration * (self.settings.video_bitrate_kbps as f64
                                    + self.settings.audio_bitrate_kbps as f64) / 8000.0;
                                ui.small(format!("Estimated output size: ~{mb:.1} MB for selected source"));
                            }
                        } else {
                            if self.settings.two_pass {
                                self.settings.two_pass = false;
                                changed = true;
                            }
                            ui.label("Constant quality (lower CRF = higher quality)");
                            changed |= ui.add(egui::Slider::new(&mut self.settings.quality, 0..=51)).changed();
                        }
                    }
                    if matches!(self.settings.preset, Preset::H264 | Preset::H265 | Preset::Av1 | Preset::Vp9) {
                        ui.separator();
                        ui.label("Output precision / pixel depth");
                        egui::ComboBox::from_id_salt("output-depth")
                            .selected_text(match self.settings.output_bit_depth {
                                8 => "8-bit 4:2:0", 10 => "10-bit 4:2:0", _ => "Encoder default"
                            })
                            .show_ui(ui, |ui| {
                                changed |= ui.selectable_value(&mut self.settings.output_bit_depth,
                                    0, "Encoder default").changed();
                                changed |= ui.selectable_value(&mut self.settings.output_bit_depth,
                                    8, "8-bit 4:2:0").changed();
                                if matches!(self.settings.preset, Preset::H265 | Preset::Av1)
                                    && self.settings.hardware_encoder == HardwareEncoder::Software {
                                    changed |= ui.selectable_value(&mut self.settings.output_bit_depth,
                                        10, "10-bit 4:2:0 (Main10)").changed();
                                }
                            });
                        ui.label("Color metadata / signal");
                        egui::ComboBox::from_id_salt("output-color-signal")
                            .selected_text(self.settings.color_signal.label())
                            .show_ui(ui, |ui| {
                                for (signal, name) in [
                                    (ColorSignal::Automatic, "Automatic / source defaults"),
                                    (ColorSignal::Bt709, "Rec.709 SDR (tags only)"),
                                    (ColorSignal::Bt2020Pq, "BT.2020 PQ / HDR10 (tags only)"),
                                    (ColorSignal::Bt2020Hlg, "BT.2020 HLG (tags only)"),
                                ] {
                                    let allowed = !matches!(signal, ColorSignal::Bt2020Pq | ColorSignal::Bt2020Hlg)
                                        || (self.settings.output_bit_depth == 10
                                            && matches!(self.settings.preset, Preset::H265 | Preset::Av1)
                                            && self.settings.hardware_encoder == HardwareEncoder::Software);
                                    changed |= ui.add_enabled(allowed,
                                        egui::Button::selectable(self.settings.color_signal == signal, name)
                                    ).clicked().then(|| self.settings.color_signal = signal).is_some();
                                }
                            });
                        ui.small("Metadata is NOT a color transform. PQ/HLG tags are ONLY for footage already graded in that transfer; SDR footage will not become HDR. No tonemapping.");
                    } else {
                        ui.small("ProRes and DNxHR use their codec-specific fixed pixel formats.");
                    }
                    ui.label("Resize width (0 = retain source size)");
                    changed |= ui.add(egui::DragValue::new(&mut self.settings.target_width)
                        .range(0..=16384).speed(2)).changed();
                } else if self.settings.preset.report() {
                    ui.small("Analysis reads the media and writes FFmpeg diagnostic findings to a text report.");
                } else if matches!(self.settings.preset, Preset::RemuxMkv | Preset::RemuxMp4 | Preset::VideoOnly) {
                    ui.small("Lossless stream copy. Video scaling and filters are unavailable.");
                } else {
                    ui.small("Audio-only output; video streams will be excluded.");
                }
            });

        egui::CollapsingHeader::new("Encoding speed / performance")
            .default_open(true)
            .show(ui, |ui| {
                if self.settings.preset.video() {
                    ui.label("Compression speed (software H.264 / H.265 / AV1 / VP9)");
                    egui::ComboBox::from_id_salt("encoder-speed")
                        .selected_text(match self.settings.encoder_speed {
                            0 => "Fastest", 2 => "Quality / smaller files", _ => "Balanced"
                        })
                        .show_ui(ui, |ui| {
                            for (speed, label) in [(0, "Fast"), (1, "Balanced"), (2, "Quality")] {
                                changed |= ui.selectable_value(
                                    &mut self.settings.encoder_speed, speed, label).changed();
                            }
                        });
                    ui.small("Fast spends less CPU time; Quality compresses better but takes longer. Hardware acceleration has separate driver limits.");
                    ui.label("Encoding threads (0 = automatic)");
                    changed |= ui.add(egui::DragValue::new(&mut self.settings.encoder_threads)
                        .range(0..=32)).changed();
                    ui.label("Keyframe interval / GOP (0 = codec default)");
                    changed |= ui.add(egui::DragValue::new(&mut self.settings.keyframe_interval)
                        .range(0..=1000)).changed();
                    if matches!(self.settings.preset, Preset::H264 | Preset::H265) {
                        changed |= ui.checkbox(&mut self.settings.faststart,
                            "Fast start (optimize MP4 for streaming)").changed();
                    }
                } else {
                    ui.small("Only applies when video is re-encoded.");
                }
            });

        egui::CollapsingHeader::new("Audio settings")
            .default_open(true)
            .show(ui, |ui| {
                if matches!(self.settings.preset, Preset::H264 | Preset::H265 | Preset::Av1 | Preset::Mp3 | Preset::Aac | Preset::Opus) {
                    ui.label("Audio bitrate (kb/s)");
                    changed |= ui.add(egui::Slider::new(&mut self.settings.audio_bitrate_kbps, 32..=512)).changed();
                }
                if self.settings.preset.video() || self.settings.preset.audio_only()
                    || matches!(self.settings.preset, Preset::SilenceDetect | Preset::LoudnessReport) {
                    changed |= ui.checkbox(&mut self.settings.normalize_audio, "Normalize loudness (one-pass)").changed();
                    if self.settings.preset.video() || self.settings.preset.audio_only() {
                        ui.separator();
                        ui.label("Output channels");
                        egui::ComboBox::from_id_salt("audio-channels")
                            .selected_text(match self.settings.audio_channels { 1 => "Mono", 2 => "Stereo", _ => "Source" })
                            .show_ui(ui, |ui| {
                                for (channels, label) in [(0, "Source"), (1, "Mono"), (2, "Stereo")] {
                                    changed |= ui.selectable_value(&mut self.settings.audio_channels, channels, label).changed();
                                }
                            });
                        ui.label("Output sample rate");
                        egui::ComboBox::from_id_salt("audio-rate")
                            .selected_text(if self.settings.audio_sample_rate == 0 { "Source".to_owned() }
                                else { format!("{} Hz", self.settings.audio_sample_rate) })
                            .show_ui(ui, |ui| {
                                for sample_rate in [0, 22050, 32000, 44100, 48000, 96000, 192000] {
                                    let label = if sample_rate == 0 { "Source".to_owned() } else { format!("{sample_rate} Hz") };
                                    changed |= ui.selectable_value(&mut self.settings.audio_sample_rate, sample_rate, label).changed();
                                }
                            });
                        ui.label("Audio gain (dB, before normalization)");
                        changed |= ui.add(egui::Slider::new(&mut self.settings.audio_gain_db, -30.0..=30.0)
                            .step_by(0.5)).changed();
                    }
                } else {
                    ui.small("Audio is copied without processing.");
                }
            });

        egui::CollapsingHeader::new("Video filters / image tools")
            .default_open(false)
            .show(ui, |ui| {
                if self.settings.preset.video() {
                    ui.label("Output frame rate (0 = source)");
                    changed |= ui.add(egui::DragValue::new(&mut self.settings.frame_rate)
                        .range(0..=240)).changed();
                    ui.label("Crop (width and height 0 = off)");
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Width");
                        changed |= ui.add(egui::DragValue::new(&mut self.settings.crop_width)
                            .range(0..=16384).speed(2)).changed();
                        ui.label("Height");
                        changed |= ui.add(egui::DragValue::new(&mut self.settings.crop_height)
                            .range(0..=16384).speed(2)).changed();
                    });
                    if self.settings.crop_width > 0 || self.settings.crop_height > 0 {
                        ui.horizontal_wrapped(|ui| {
                            ui.label("X");
                            changed |= ui.add(egui::DragValue::new(&mut self.settings.crop_x)
                                .range(0..=16384)).changed();
                            ui.label("Y");
                            changed |= ui.add(egui::DragValue::new(&mut self.settings.crop_y)
                                .range(0..=16384)).changed();
                        });
                    }
                    ui.label("Rotate");
                    egui::ComboBox::from_id_salt("rotation")
                        .selected_text(format!("{}°", self.settings.rotate_degrees))
                        .show_ui(ui, |ui| {
                            for degrees in [0, 90, 180, 270] {
                                changed |= ui.selectable_value(&mut self.settings.rotate_degrees,
                                    degrees, format!("{degrees}°")).changed();
                            }
                        });
                    changed |= ui.checkbox(&mut self.settings.flip_horizontal, "Flip horizontally").changed();
                    changed |= ui.checkbox(&mut self.settings.flip_vertical, "Flip vertically").changed();
                    ui.separator();
                    ui.label("Restoration / color adjustment");
                    changed |= ui.checkbox(&mut self.settings.deinterlace, "Deinterlace (YADIF)").changed();
                    changed |= ui.checkbox(&mut self.settings.denoise, "Temporal/spatial denoise (HQDN3D)").changed();
                    changed |= ui.checkbox(&mut self.settings.sharpen, "Sharpen after scaling").changed();
                    ui.label("Brightness");
                    changed |= ui.add(egui::Slider::new(&mut self.settings.brightness, -1.0..=1.0)).changed();
                    ui.label("Contrast");
                    changed |= ui.add(egui::Slider::new(&mut self.settings.contrast, 0.0..=2.0)).changed();
                    ui.label("Saturation");
                    changed |= ui.add(egui::Slider::new(&mut self.settings.saturation, 0.0..=3.0)).changed();
                    ui.label("Gamma");
                    changed |= ui.add(egui::Slider::new(&mut self.settings.gamma, 0.1..=4.0)).changed();
                    if ui.button("Reset color and restoration").clicked() {
                        self.settings.deinterlace = false; self.settings.denoise = false;
                        self.settings.sharpen = false;
                        self.settings.brightness = 0.0; self.settings.contrast = 1.0;
                        self.settings.saturation = 1.0; self.settings.gamma = 1.0;
                        changed = true;
                    }
                } else {
                    ui.small("Non-encoding operations preserve frames/streams or export directly; filters are disabled.");
                    if ui.button("Reset video filters").clicked() {
                        self.settings.frame_rate = 0; self.settings.crop_width = 0; self.settings.crop_height = 0;
                        self.settings.crop_x = 0; self.settings.crop_y = 0;
                        self.settings.rotate_degrees = 0; self.settings.flip_horizontal = false;
                        self.settings.flip_vertical = false; self.settings.target_width = 0;
                        self.settings.deinterlace = false; self.settings.denoise = false;
                        self.settings.sharpen = false;
                        self.settings.brightness = 0.0; self.settings.contrast = 1.0;
                        self.settings.saturation = 1.0; self.settings.gamma = 1.0;
                        changed = true;
                    }
                }
            });

        egui::CollapsingHeader::new("Stream mapping / subtitles")
            .default_open(false)
            .show(ui, |ui| {
                let selected_info = self.jobs.iter()
                    .find(|j| Some(j.id) == self.selected_job).and_then(|j| j.media.as_ref());
                if self.settings.preset.video() || matches!(self.settings.preset,
                    Preset::VideoOnly | Preset::FrameMd5 | Preset::PngStill | Preset::JpegStill |
                    Preset::BlackDetect | Preset::FreezeDetect) {
                    ui.label("Video track (0 = first)");
                    changed |= ui.add(egui::DragValue::new(&mut self.settings.video_stream)
                        .range(0..=32)).changed();
                }
                if self.settings.preset.video() || self.settings.preset.audio_only() {
                    ui.label("Audio track selection");
                    egui::ComboBox::from_id_salt("audio-stream")
                        .selected_text(match self.settings.audio_stream {
                            -2 => "No audio".to_owned(),
                            -1 => "All audio tracks (first for audio-only)".to_owned(),
                            n => format!("Audio #{n}"),
                        })
                        .show_ui(ui, |ui| {
                            if self.settings.preset.video() {
                                changed |= ui.selectable_value(&mut self.settings.audio_stream, -2, "No audio").changed();
                            }
                            changed |= ui.selectable_value(&mut self.settings.audio_stream, -1,
                                "All tracks (first for audio-only)").changed();
                            let count = selected_info.map(|m| m.audio_tracks.len()).unwrap_or(8).min(32);
                            for n in 0..count {
                                let name = selected_info.map(|m| m.audio_tracks[n].description(n))
                                    .unwrap_or_else(|| format!("Audio #{n}"));
                                changed |= ui.selectable_value(&mut self.settings.audio_stream, n as i32, name).changed();
                            }
                        });
                }
                if self.settings.preset.video() || matches!(self.settings.preset,
                    Preset::ExtractSrt | Preset::ExtractAss) {
                    ui.label("Subtitle track");
                    egui::ComboBox::from_id_salt("subtitle-stream")
                        .selected_text(if self.settings.subtitle_stream == -1 { "None (first for extraction)".into() }
                            else { format!("Subtitle #{}", self.settings.subtitle_stream) })
                        .show_ui(ui, |ui| {
                            changed |= ui.selectable_value(&mut self.settings.subtitle_stream, -1,
                                "No subtitles (first for extraction)").changed();
                            let count = selected_info.map(|m| m.subtitle_tracks.len()).unwrap_or(8).min(32);
                            for n in 0..count {
                                let name = selected_info.map(|m| m.subtitle_tracks[n].description(n))
                                    .unwrap_or_else(|| format!("Subtitle #{n}"));
                                changed |= ui.selectable_value(&mut self.settings.subtitle_stream, n as i32, name).changed();
                            }
                        });
                    ui.small("Subtitle muxing is supported for compatible containers. Subtitle burn-in is not yet available.");
                }
                if matches!(self.settings.preset, Preset::RemuxMkv) {
                    ui.small("MKV rewrap copies every stream, chapter and attachment.");
                }
            });

        egui::CollapsingHeader::new("Hardware encoder")
            .default_open(false)
            .show(ui, |ui| {
                if matches!(self.settings.preset, Preset::H264 | Preset::H265) {
                    ui.horizontal(|ui| {
                        if self.hardware_scanning {
                            ui.spinner(); ui.small("Testing hardware encoders…");
                        } else if ui.button("Rescan hardware").clicked() {
                            self.start_hardware_scan();
                        }
                    });
                    if let Some(error) = &self.hardware_error {
                        ui.colored_label(egui::Color32::LIGHT_RED, error);
                    }
                    egui::ComboBox::from_id_salt("hardware-encoder")
                        .selected_text(self.settings.hardware_encoder.label())
                        .show_ui(ui, |ui| {
                            for encoder in HardwareEncoder::ALL {
                                let supported = encoder == HardwareEncoder::Software
                                    || self.hardware_scan.as_ref().is_some_and(|report|
                                        report.supports(encoder, self.settings.preset));
                                changed |= ui.add_enabled(supported,
                                    egui::Button::selectable(
                                        self.settings.hardware_encoder == encoder,
                                        encoder.label(),
                                    )).clicked().then(|| {
                                        self.settings.hardware_encoder = encoder;
                                        if encoder != HardwareEncoder::Software {
                                            self.settings.two_pass = false;
                                        }
                                    }).is_some();
                            }
                        });
                    ui.small("Hardware choices are enabled only after FFmpeg completes a real one-frame encode with the installed driver. Scan runs in the background.");
                } else { ui.small("Hardware selection currently supports H.264 and H.265 functions."); }
            });

        egui::CollapsingHeader::new("Cut / trim")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Start (seconds)");
                    changed |= ui.add(egui::DragValue::new(&mut self.settings.start_seconds)
                        .range(0.0..=864000.0).speed(0.25)).changed();
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label("End (seconds, 0 = full file)");
                    changed |= ui.add(egui::DragValue::new(&mut self.settings.end_seconds)
                        .range(0.0..=864000.0).speed(0.25)).changed();
                });
                if self.settings.end_seconds > 0.0
                    && self.settings.end_seconds <= self.settings.start_seconds
                {
                    ui.colored_label(egui::Color32::LIGHT_RED, "End must be greater than start.");
                }
                if matches!(self.settings.preset, Preset::RemuxMkv | Preset::RemuxMp4 | Preset::VideoOnly) {
                    ui.small("Stream-copy trims are keyframe-inexact.");
                }
            });

        egui::CollapsingHeader::new("Output destination")
            .default_open(true)
            .show(ui, |ui| {
                ui.label("Output folder (blank = beside original)");
                changed |= ui.add(egui::TextEdit::singleline(&mut self.settings.output_dir)
                    .desired_width(ui.available_width())).changed();
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Browse…").clicked()
                        && let Some(folder) = rfd::FileDialog::new().pick_folder()
                    {
                        self.settings.output_dir = folder.to_string_lossy().to_string();
                        changed = true;
                    }
                    if ui.button("Same as source").clicked() {
                        self.settings.output_dir.clear();
                        changed = true;
                    }
                });
                ui.small("An existing output file is not overwritten.");
            });

        egui::CollapsingHeader::new("External tools")
            .default_open(false)
            .show(ui, |ui| {
                ui.small("Use a full path or leave blank to discover executables beside this app, then in PATH.");
                ui.label("FFmpeg executable");
                changed |= ui.add(egui::TextEdit::singleline(&mut self.settings.ffmpeg_path)
                    .desired_width(ui.available_width())).changed();
                ui.label("FFprobe executable");
                changed |= ui.add(egui::TextEdit::singleline(&mut self.settings.ffprobe_path)
                    .desired_width(ui.available_width())).changed();
                ui.label("FFplay executable (optional for synchronized A/V playback)");
                changed |= ui.add(egui::TextEdit::singleline(&mut self.settings.ffplay_path)
                    .desired_width(ui.available_width())).changed();
                ui.small("An empty FFplay path checks beside the configured FFmpeg, beside this app, then PATH.");
            });

        ui.separator();
        ui.small("Rust-native UI · FFmpeg engine. Some specialist functions and realtime editing are still in development.");
        if changed {
            self.mark_settings_changed();
        }
    }

    fn footer_ui(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            let pending = self.jobs.iter().any(|j| j.status.is_pending());
            if ui.add_enabled(pending && !self.process_queue, egui::Button::new(
                if self.current.is_some() { "Resume after current" } else { "▶ Start function" }
            )).clicked() {
                self.process_queue = true;
            }
            if self.process_queue && ui.button("Pause after current").clicked() {
                self.process_queue = false;
            }
            if let Some(running) = &self.current {
                let cancelling = self.jobs.iter().any(|j| j.id == running.id && matches!(j.status, JobState::Cancelling));
                if ui.add_enabled(!cancelling, egui::Button::new("Cancel current")).clicked() {
                    self.cancel_running();
                }
            }
            let complete = self.jobs.iter().filter(|j| matches!(j.status, JobState::Completed)).count();
            ui.label(format!("{complete}/{} completed", self.jobs.len()));
        });
        if let Some(running) = &self.current
            && let Some(job) = self.jobs.iter().find(|j| j.id == running.id)
        {
            ui.horizontal_wrapped(|ui| {
                ui.small(format!("Progress status: {}", job.status.label()));
                if !job.speed.is_empty() {
                    ui.small(format!("{} · {:.1}s", job.speed, job.elapsed_sec));
                }
            });
        } else {
            ui.small(if self.process_queue { "Preparing queue…" } else { "No encoding running" });
        }
        if !self.notice.is_empty() {
            ui.colored_label(egui::Color32::LIGHT_RED, &self.notice);
        }
    }
}

/// Filter only the indices, not entire job clones; empty query avoids filename
/// conversion. Virtual scrolling only builds UI widgets for visible indices.
fn matching_jobs(jobs: &[Job], filter: &str) -> Vec<usize> {
    if filter.is_empty() { return (0..jobs.len()).collect(); }
    let filter = filter.to_lowercase();
    jobs.iter().enumerate().filter_map(|(index, job)| {
        job.input.to_string_lossy().to_lowercase().contains(filter.as_str()).then_some(index)
    }).collect()
}

/// Fractions are persisted so the three-pane layout scales with the monitor.
/// Clamp corrupt/legacy values and reserve a useful video preview minimum.
fn pane_widths(total: f32, left_fraction: f32, right_fraction: f32) -> (f32, f32, f32) {
    // Reserve both 12-point drag targets plus egui's horizontal widget spacing.
    let usable = (total - 64.0).max(900.0);
    let left_fraction = if left_fraction.is_finite() { left_fraction } else { 0.21 };
    let right_fraction = if right_fraction.is_finite() { right_fraction } else { 0.29 };
    let left = (usable * left_fraction).clamp(220.0, (usable - 675.0).min(470.0));
    let right = (usable * right_fraction).clamp(325.0, (usable - left - 350.0).min(550.0));
    let center = usable - left - right;
    (left, center, right)
}

fn pane_drag_handle(ui: &mut egui::Ui, height: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(12.0, height), egui::Sense::drag());
    let tint = if response.dragged() || response.hovered() {
        ui.visuals().widgets.active.bg_fill
    } else {
        ui.visuals().widgets.noninteractive.bg_stroke.color
    };
    let track = egui::Rect::from_center_size(rect.center(), egui::vec2(3.0, height.min(68.0)));
    ui.painter().rect_filled(track, 2.0, tint);
    response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
}

/// Establish an always valid playback/selection region even with legacy settings.
fn preview_bounds(duration: f64, start: f64, end: f64) -> (f64, f64) {
    if !duration.is_finite() || duration <= 0.0 { return (0.0, 0.0); }
    let start = start.clamp(0.0, duration);
    let end = if end > start { end.min(duration) } else { duration };
    if end <= start { (0.0, duration) } else { (start, end) }
}

impl eframe::App for EncoderApp {
    // eframe 0.36 uses `App::ui` for the native root viewport.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_dependencies();
        self.poll_hardware_scan();
        self.poll_events();
        self.refresh_av_player();
        self.update_native_video(ui.ctx());
        let dropped_files: Vec<PathBuf> = ui.ctx().input(|input| input.raw.dropped_files.iter()
            .map(|file| file.path().to_path_buf()).collect());
        if !dropped_files.is_empty() { self.add_files(dropped_files); }
        ui.ctx().set_visuals(if self.settings.dark_theme { egui::Visuals::dark() }
            else { egui::Visuals::light() });

        self.tick_playback();
        // Shutter-inspired wide workspace: queue/function | preview | settings.
        self.header_ui(ui);
        let width = ui.available_width();
        let height = ui.available_height();
        let body_height = (height - 108.0).max(210.0);
        if width >= 1250.0 {
            let (left, center, right) = pane_widths(
                width, self.settings.workspace_left_fraction, self.settings.workspace_right_fraction
            );
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(egui::vec2(left, body_height),
                    egui::Layout::top_down(egui::Align::Min), |ui| {
                        self.files_ui(ui, (body_height - 245.0).max(130.0));
                        ui.separator();
                        self.function_ui(ui);
                    });
                let drag_left = pane_drag_handle(ui, body_height);
                if drag_left.dragged() {
                    let delta = ui.ctx().input(|input| input.pointer.delta().x);
                    if delta != 0.0 {
                        let new_left = (left + delta).clamp(220.0, width - 64.0 - right - 350.0);
                        self.settings.workspace_left_fraction = new_left / (width - 64.0);
                        self.mark_settings_changed();
                    }
                }
                ui.allocate_ui_with_layout(egui::vec2(center, body_height),
                    egui::Layout::top_down(egui::Align::Min), |ui| {
                        egui::ScrollArea::vertical().id_salt("preview-pane")
                            .auto_shrink([false, false]).max_height(body_height)
                            .show(ui, |ui| {
                                self.ensure_preview();
                                self.preview_ui(ui);
                            });
                    });
                let drag_right = pane_drag_handle(ui, body_height);
                if drag_right.dragged() {
                    let delta = ui.ctx().input(|input| input.pointer.delta().x);
                    if delta != 0.0 {
                        let new_right = (right - delta).clamp(325.0, width - 64.0 - left - 350.0);
                        self.settings.workspace_right_fraction = new_right / (width - 64.0);
                        self.mark_settings_changed();
                    }
                }
                ui.allocate_ui_with_layout(egui::vec2(right, body_height),
                    egui::Layout::top_down(egui::Align::Min), |ui| {
                        egui::ScrollArea::vertical().id_salt("settings-pane")
                            .auto_shrink([false, false]).max_height(body_height)
                            .show(ui, |ui| self.options_ui(ui));
                    });
            });
        } else if width >= 870.0 {
            ui.columns(2, |columns| {
                egui::ScrollArea::vertical().id_salt("queue-and-timeline-scroll")
                    .auto_shrink([false, false]).max_height(body_height)
                    .show(&mut columns[0], |ui| {
                        self.files_ui(ui, (body_height - 430.0).clamp(135.0, 265.0));
                        self.function_ui(ui);
                        ui.separator();
                        self.ensure_preview();
                        self.preview_ui(ui);
                    });
                egui::ScrollArea::vertical().id_salt("setting-scroll")
                    .auto_shrink([false, false]).max_height(body_height)
                    .show(&mut columns[1], |ui| self.options_ui(ui));
            });
        } else {
            egui::ScrollArea::vertical().id_salt("compact-main")
                .auto_shrink([false, false]).max_height(body_height)
                .show(ui, |ui| {
                    self.files_ui(ui, 190.0);
                    self.function_ui(ui);
                    ui.separator();
                    self.ensure_preview();
                    self.preview_ui(ui);
                    ui.separator();
                    self.options_ui(ui);
                });
        }
        self.start_next();
        self.footer_ui(ui);
        self.save_when_idle(ui.ctx());
        if self.current.is_some()
            || self.jobs.iter().any(|j| matches!(j.status, JobState::Analyzing))
            || self.preview_loading || self.preview_playing || self.av_player.is_some()
            || self.native_video.is_some() || self.native_audio.is_some()
            || self.hardware_scanning
            || matches!(self.dependency_state, DependencyState::Checking | DependencyState::Downloading(_) | DependencyState::Extracting)
        {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
    }
}

impl Drop for EncoderApp {
    fn drop(&mut self) {
        // Drop the managed FFplay child before releasing encoder state.
        self.av_player = None;
        self.stop_native_video();
        if self.dirty_since.is_some() {
            let _ = model::save_settings(&self.settings);
        }
        if let Some(current) = &self.current {
            current.cancelled.store(true, Ordering::SeqCst);
            let deadline = Instant::now() + Duration::from_millis(1200);
            while !current.stopped.load(Ordering::SeqCst) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_job(id: u64, status: JobState, preset: Preset) -> Job {
        let mut settings = Settings::default();
        settings.preset = preset;
        Job {
            id,
            input: PathBuf::from(format!("example-{id}.mp4")),
            output: None,
            media: None,
            options: settings.encode_options(),
            status,
            elapsed_sec: 0.0,
            speed: String::new(),
            pass: 1,
        }
    }

    #[test]
    fn changing_function_updates_only_pending_jobs() {
        let mut app = EncoderApp::new();
        app.settings = Settings::default();
        app.jobs = vec![
            dummy_job(1, JobState::Analyzing, Preset::H264),
            dummy_job(2, JobState::Queued, Preset::H264),
            dummy_job(3, JobState::Running, Preset::H264),
            dummy_job(4, JobState::Completed, Preset::H264),
        ];
        app.settings.preset = Preset::Flac;
        app.update_pending_options();
        assert_eq!(app.jobs[0].options.preset, Preset::Flac);
        assert_eq!(app.jobs[1].options.preset, Preset::Flac);
        assert_eq!(app.jobs[2].options.preset, Preset::H264);
        assert_eq!(app.jobs[3].options.preset, Preset::H264);
    }

    #[test]
    fn pane_widths_respect_minimum_preview_and_corrupt_settings() {
        for total in [1250.0, 1600.0, 2560.0] {
            for (a, b) in [(0.21, 0.29), (0.9, 0.95), (-10.0, 4.0), (f32::NAN, f32::INFINITY)] {
                let (left, center, right) = pane_widths(total, a, b);
                assert!(left >= 220.0 && right >= 325.0 && center >= 350.0);
                assert!((left + center + right - (total - 64.0)).abs() < 0.01);
            }
        }
    }

    #[test]
    fn large_queue_search_preserves_original_indices() {
        let jobs: Vec<Job> = (0..10_000)
            .map(|id| dummy_job(id, JobState::Queued, Preset::H264)).collect();
        let all = matching_jobs(&jobs, "");
        assert_eq!(all.len(), 10_000);
        let searched = matching_jobs(&jobs, "EXAMPLE-9999.MP4");
        assert_eq!(searched, vec![9999]);
    }

    #[test]
    fn gui_panels_can_render_without_a_native_window() {
        egui::__run_test_ui(|ui| {
            let mut app = EncoderApp::new();
            app.settings = Settings::default();
            app.header_ui(ui);
            app.files_ui(ui, 160.0);
            app.ensure_preview();
            app.preview_ui(ui);
            app.options_ui(ui);
            app.footer_ui(ui);
        });
    }

    #[test]
    fn preview_ranges_handle_invalid_legacy_options() {
        assert_eq!(preview_bounds(10.0, 2.0, 7.0), (2.0, 7.0));
        assert_eq!(preview_bounds(10.0, 5.0, 0.0), (5.0, 10.0));
        assert_eq!(preview_bounds(10.0, 11.0, 0.0), (0.0, 10.0));
        assert_eq!(preview_bounds(0.0, 2.0, 7.0), (0.0, 0.0));
    }

    #[test]
    fn removed_job_probe_event_is_safe() {
        let mut app = EncoderApp::new();
        app.sender.send(Event::Probed {
            id: 312,
            result: Ok(MediaInfo::default()),
        }).unwrap();
        app.poll_events();
        assert!(app.jobs.is_empty());
    }
}
