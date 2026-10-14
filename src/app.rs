use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use rust_encoder::{
    encode::{self, Event},
    media::{self, MediaInfo},
    model::{self, EncodeOptions, HardwareEncoder, Preset, Settings},
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
    next_id: u64,
    selected_job: Option<u64>,
    preview_for: Option<(u64, u64)>,
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
        Self {
            settings: model::load_settings(),
            jobs: Vec::new(),
            sender,
            receiver,
            next_id: 1,
            selected_job: None,
            preview_for: None,
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
                        match result {
                            Ok(preview) => self.preview_pixels = Some(preview),
                            Err(message) => self.preview_error = Some(message),
                        }
                    }
                }
                Event::Started { id, output } => {
                    if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
                        job.output = Some(output);
                    }
                }
                Event::Progress { id, elapsed_sec, speed } => {
                    if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
                        job.elapsed_sec = elapsed_sec;
                        job.speed = speed;
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
        ui.small("Drop video or audio files here. Pending files use the function and settings shown on the right.");
        ui.separator();

        let mut remove_id = None;
        let mut retry_id = None;
        // Fill the available column but allow scrolling instead of clipping long queues.
        egui::ScrollArea::vertical()
            .id_salt("file-queue")
            .max_height(list_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for job in &self.jobs {
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
                                        .map(|d| (job.elapsed_sec / d).clamp(0.0, 1.0) as f32)
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
                }
                if self.jobs.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(24.0);
                        ui.label("No files added");
                        ui.small("Choose files or drag and drop media to start.");
                    });
                }
            });
        if let Some(id) = remove_id {
            self.jobs.retain(|j| j.id != id);
            if self.selected_job == Some(id) {
                self.selected_job = self.jobs.first().map(|j| j.id);
            }
        }
        if let Some(id) = retry_id { self.retry_failed(id); }
    }

    fn ensure_preview(&mut self) {
        if self.preview_owner != self.selected_job {
            self.preview_owner = self.selected_job;
            self.preview_seconds = 0.0;
            self.preview_requested_seconds = 0.0;
            self.preview_for = None;
        }
        let Some(id) = self.selected_job else { return; };
        let seek_ms = (self.preview_requested_seconds.max(0.0) * 1000.0).round() as u64;
        if self.preview_for == Some((id, seek_ms)) { return; }
        self.preview_for = Some((id, seek_ms));
        self.preview_texture = None;
        self.preview_pixels = None;
        self.preview_error = None;
        let Some(job) = self.jobs.iter().find(|j| j.id == id) else { return; };
        let Some(info) = &job.media else {
            if matches!(job.status, JobState::Analyzing) {
                self.preview_for = None;
            } else {
                self.preview_error = Some("No preview: media inspection did not succeed.".into());
            }
            return;
        };
        if info.video_codec.is_none() {
            self.preview_error = Some("Audio-only media: no video frame to preview.".into());
            return;
        }
        media::thumbnail_async(
            id, seek_ms,
            model::resolve_tool("ffmpeg", &self.settings.ffmpeg_path),
            job.input.clone(), self.sender.clone(),
        );
    }

    fn preview_ui(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.heading("Media preview");
        if let Some(preview) = self.preview_pixels.take() {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [preview.width, preview.height], &preview.rgba,
            );
            self.preview_texture = Some(ui.ctx().load_texture(
                "selected-media-thumbnail", image, egui::TextureOptions::LINEAR,
            ));
        }
        if let Some(texture) = &self.preview_texture {
            let size = texture.size_vec2();
            let factor = (ui.available_width() / size.x).min(200.0 / size.y).min(1.0);
            ui.vertical_centered(|ui| {
                ui.image((texture.id(), size * factor.max(0.1)));
            });
        } else if let Some(error) = &self.preview_error {
            ui.small(error);
        } else if self.selected_job.is_some() {
            ui.small("Generating a still-frame preview…");
        } else {
            ui.small("Select a file to inspect its video frame and metadata.");
        }
        if let Some(job) = self.jobs.iter().find(|j| Some(j.id) == self.selected_job)
            && let Some(info) = &job.media
        {
            ui.small(info.summary());
            if let Some(duration) = info.duration_sec && info.video_codec.is_some() {
                ui.horizontal_wrapped(|ui| {
                    ui.add(egui::Slider::new(&mut self.preview_seconds, 0.0..=duration)
                        .text("Seek (s)").fixed_decimals(2));
                    if ui.button("Preview frame").clicked() {
                        self.preview_requested_seconds = self.preview_seconds;
                        self.preview_for = None;
                        ui.ctx().request_repaint();
                    }
                });
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
        ui.small("Still-frame seeking only. Playback, waveforms and timeline editing remain unported.");
    }

    fn options_ui(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        ui.heading("Choose function");
        let previous_preset = self.settings.preset;
        egui::ComboBox::from_id_salt("preset")
            .selected_text(self.settings.preset.label())
            .width(ui.available_width().max(190.0))
            .show_ui(ui, |ui| {
                for preset in Preset::ALL {
                    changed |= ui.selectable_value(&mut self.settings.preset, preset, preset.label()).changed();
                }
            });
        if previous_preset != self.settings.preset {
            if !matches!(self.settings.preset, Preset::H264 | Preset::H265) {
                self.settings.hardware_encoder = HardwareEncoder::Software;
            }
            if !self.settings.preset.video() {
                self.settings.target_width = 0;
                self.settings.frame_rate = 0;
                self.settings.crop_width = 0; self.settings.crop_height = 0;
                self.settings.crop_x = 0; self.settings.crop_y = 0;
                self.settings.rotate_degrees = 0;
                self.settings.flip_horizontal = false; self.settings.flip_vertical = false;
            }
        }
        ui.small("Function changes update queued jobs; active and completed jobs are unchanged.");
        ui.add_space(6.0);

        egui::CollapsingHeader::new("Image / codec")
            .default_open(true)
            .show(ui, |ui| {
                if self.settings.preset.video() {
                    if !matches!(self.settings.preset, Preset::ProRes | Preset::Dnxhr) {
                        ui.label("Quality (lower CRF = higher quality)");
                        changed |= ui.add(egui::Slider::new(&mut self.settings.quality, 0..=51)).changed();
                    } else {
                        ui.small("This codec uses a fixed profile rather than CRF.");
                    }
                    ui.label("Resize width (0 = retain source size)");
                    changed |= ui.add(egui::DragValue::new(&mut self.settings.target_width)
                        .range(0..=16384).speed(2)).changed();
                } else if matches!(self.settings.preset, Preset::RemuxMkv | Preset::RemuxMp4 | Preset::VideoOnly) {
                    ui.small("Lossless stream copy. Video scaling and filters are unavailable.");
                } else {
                    ui.small("Audio-only output; video streams will be excluded.");
                }
            });

        egui::CollapsingHeader::new("Audio settings")
            .default_open(true)
            .show(ui, |ui| {
                if matches!(self.settings.preset, Preset::H264 | Preset::H265 | Preset::Av1 | Preset::Mp3 | Preset::Aac | Preset::Opus) {
                    ui.label("Audio bitrate (kb/s)");
                    changed |= ui.add(egui::Slider::new(&mut self.settings.audio_bitrate_kbps, 32..=512)).changed();
                }
                if self.settings.preset.video() || self.settings.preset.audio_only() {
                    changed |= ui.checkbox(&mut self.settings.normalize_audio, "Normalize loudness (one-pass)").changed();
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
                } else {
                    ui.small("Non-encoding operations preserve frames/streams or export directly; filters are disabled.");
                    if ui.button("Reset video filters").clicked() {
                        self.settings.frame_rate = 0; self.settings.crop_width = 0; self.settings.crop_height = 0;
                        self.settings.crop_x = 0; self.settings.crop_y = 0;
                        self.settings.rotate_degrees = 0; self.settings.flip_horizontal = false;
                        self.settings.flip_vertical = false; self.settings.target_width = 0; changed = true;
                    }
                }
            });

        egui::CollapsingHeader::new("Stream mapping / subtitles")
            .default_open(false)
            .show(ui, |ui| {
                let selected_info = self.jobs.iter()
                    .find(|j| Some(j.id) == self.selected_job).and_then(|j| j.media.as_ref());
                if self.settings.preset.video() || matches!(self.settings.preset,
                    Preset::VideoOnly | Preset::FrameMd5 | Preset::PngStill | Preset::JpegStill) {
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
                    egui::ComboBox::from_id_salt("hardware-encoder")
                        .selected_text(self.settings.hardware_encoder.label())
                        .show_ui(ui, |ui| {
                            for encoder in HardwareEncoder::ALL {
                                changed |= ui.selectable_value(&mut self.settings.hardware_encoder,
                                    encoder, encoder.label()).changed();
                            }
                        });
                    ui.small("Requires hardware/driver support and an FFmpeg build exposing the chosen encoder. Software is the reliable default.");
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

impl eframe::App for EncoderApp {
    // eframe 0.36 uses `App::ui` for the native root viewport.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_events();
        let dropped_files: Vec<PathBuf> = ui.ctx().input(|input| input.raw.dropped_files.iter()
            .map(|file| file.path().to_path_buf()).collect());
        if !dropped_files.is_empty() { self.add_files(dropped_files); }
        ui.ctx().set_visuals(if self.settings.dark_theme { egui::Visuals::dark() }
            else { egui::Visuals::light() });

        // Top/center/footer are flexibly sized. The main queue and settings
        // scroll independently in a wide window and stack on narrow windows.
        self.header_ui(ui);
        let width = ui.available_width();
        let height = ui.available_height();
        if width >= 940.0 {
            ui.columns(2, |columns| {
                self.files_ui(&mut columns[0], (height - 445.0).clamp(140.0, 320.0));
                self.ensure_preview();
                self.preview_ui(&mut columns[0]);
                egui::ScrollArea::vertical().id_salt("setting-scroll")
                    .auto_shrink([false, false])
                    .max_height((height - 125.0).max(220.0))
                    .show(&mut columns[1], |ui| self.options_ui(ui));
            });
        } else {
            egui::ScrollArea::vertical().id_salt("compact-main")
                .auto_shrink([false, false])
                .max_height((height - 105.0).max(220.0))
                .show(ui, |ui| {
                    // A bounded queue prevents the settings from being pushed
                    // indefinitely below the fold on smaller windows.
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), 235.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| self.files_ui(ui, 155.0),
                    );
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
            || (self.preview_for.is_some() && self.preview_texture.is_none() && self.preview_error.is_none())
        {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
    }
}

impl Drop for EncoderApp {
    fn drop(&mut self) {
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
