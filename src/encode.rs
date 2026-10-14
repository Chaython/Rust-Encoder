use crate::{media::MediaInfo, model::{EncodeOptions, Preset}};
use crossbeam_channel::Sender;
use std::{
    collections::VecDeque, ffi::OsString, fs::{self, File, OpenOptions}, io::{BufRead, BufReader, Write},
    path::{Path, PathBuf}, process::{Command, Stdio},
    sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex},
    thread, time::Duration,
};

#[derive(Debug)]
pub enum Event {
    Probed { id: u64, result: Result<MediaInfo, String> },
    PreviewReady { id: u64, result: Result<crate::media::Thumbnail, String> },
    Started { id: u64, output: PathBuf },
    Progress { id: u64, elapsed_sec: f64, speed: String },
    Completed { id: u64, output: PathBuf },
    Failed { id: u64, message: String },
    Cancelled { id: u64 },
}

pub fn validate_options(options: &EncodeOptions) -> Result<(), String> {
    if options.quality > 51 { return Err("Quality (CRF) must be between 0 and 51".into()); }
    if options.target_width != 0 && (options.target_width < 64 || options.target_width > 16384) {
        return Err("Target width must be 0 (original) or 64–16384".into());
    }
    if options.audio_bitrate_kbps < 32 || options.audio_bitrate_kbps > 512 {
        return Err("Audio bitrate must be 32–512 kb/s".into());
    }
    if !options.start_seconds.is_finite() || options.start_seconds < 0.0
        || !options.end_seconds.is_finite() || options.end_seconds < 0.0 {
        return Err("Trim positions must be finite and nonnegative".into());
    }
    if options.end_seconds != 0.0 && options.end_seconds <= options.start_seconds {
        return Err("Trim end must be greater than start (or 0 to disable)".into());
    }
    Ok(())
}

pub fn expected_duration(info: &MediaInfo, settings: &EncodeOptions) -> Option<f64> {
    let input_end = info.duration_sec.unwrap_or(f64::INFINITY);
    let desired_end = if settings.end_seconds == 0.0 { input_end } else { settings.end_seconds.min(input_end) };
    let duration = desired_end - settings.start_seconds;
    duration.is_finite().then_some(duration).filter(|v| *v > 0.0)
}

/// FFmpeg args are kept as OS strings and never joined into a shell command.
/// The output argument must be a unique temporary path, not a user file.
pub fn build_args(input: &Path, temp_output: &Path, o: &EncodeOptions) -> Result<Vec<OsString>, String> {
    validate_options(o)?;
    if !input.is_file() { return Err("Input does not exist".into()); }
    if input == temp_output { return Err("Input and output must differ".into()); }

    let mut args: Vec<OsString> = ["-nostdin", "-hide_banner", "-loglevel", "error", "-nostats",
        "-stats_period", "0.25", "-progress", "pipe:1", "-n"]
        .iter().map(|v| OsString::from(*v)).collect();
    macro_rules! arg { ($value:expr) => { args.push(OsString::from(($value).to_string())) }; }
    arg!("-i"); args.push(input.as_os_str().to_owned());
    if o.start_seconds > 0.0 {
        arg!("-ss"); arg!(&format!("{:.3}", o.start_seconds));
    }
    if o.end_seconds > 0.0 {
        arg!("-t"); arg!(&format!("{:.3}", o.end_seconds - o.start_seconds));
    }
    match o.preset {
        Preset::RemuxMkv => {
            for a in ["-map", "0", "-c", "copy", "-map_metadata", "0", "-map_chapters", "0"] { arg!(a); }
        }
        preset if preset.audio_only() => {
            for a in ["-map", "0:a:0", "-vn", "-sn", "-dn"] { arg!(a); }
            match preset {
                Preset::Wav => { arg!("-c:a"); arg!("pcm_s24le"); }
                Preset::Flac => { arg!("-c:a"); arg!("flac"); }
                Preset::Mp3 => { arg!("-c:a"); arg!("libmp3lame"); }
                Preset::Aac => { arg!("-c:a"); arg!("aac"); }
                _ => unreachable!(),
            }
            if matches!(preset, Preset::Mp3 | Preset::Aac) {
                arg!("-b:a"); arg!(&format!("{}k", o.audio_bitrate_kbps));
            }
            if o.normalize_audio { arg!("-af"); arg!("loudnorm=I=-16:TP=-1.5:LRA=11"); }
        }
        preset => {
            // First video stream, all audio streams; subtitles are not silently discarded by
            // attempting unsupported stream copies. Subtitle conversion is a future feature.
            for a in ["-map", "0:v:0", "-map", "0:a?"] { arg!(a); }
            match preset {
                Preset::H264 => { arg!("-c:v"); arg!("libx264"); arg!("-crf"); arg!(&o.quality.to_string()); arg!("-preset"); arg!("medium"); arg!("-pix_fmt"); arg!("yuv420p"); }
                Preset::H265 => { arg!("-c:v"); arg!("libx265"); arg!("-crf"); arg!(&o.quality.to_string()); arg!("-preset"); arg!("medium"); arg!("-pix_fmt"); arg!("yuv420p"); }
                Preset::Av1 => { arg!("-c:v"); arg!("libsvtav1"); arg!("-crf"); arg!(&o.quality.to_string()); arg!("-preset"); arg!("6"); }
                Preset::Vp9 => { arg!("-c:v"); arg!("libvpx-vp9"); arg!("-b:v"); arg!("0"); arg!("-crf"); arg!(&o.quality.to_string()); arg!("-deadline"); arg!("good"); }
                Preset::ProRes => { arg!("-c:v"); arg!("prores_ks"); arg!("-profile:v"); arg!("3"); arg!("-pix_fmt"); arg!("yuv422p10le"); }
                Preset::Dnxhr => { arg!("-c:v"); arg!("dnxhd"); arg!("-profile:v"); arg!("dnxhr_hq"); arg!("-pix_fmt"); arg!("yuv422p"); }
                _ => unreachable!(),
            }
            if o.target_width > 0 {
                arg!("-vf"); arg!(&format!("scale={}:-2:flags=lanczos", o.target_width));
            }
            if preset == Preset::Vp9 { arg!("-c:a"); arg!("libopus"); }
            else if matches!(preset, Preset::ProRes | Preset::Dnxhr) { arg!("-c:a"); arg!("pcm_s16le"); }
            else { arg!("-c:a"); arg!("aac"); arg!("-b:a"); arg!(&format!("{}k", o.audio_bitrate_kbps)); }
            if o.normalize_audio { arg!("-af"); arg!("loudnorm=I=-16:TP=-1.5:LRA=11"); }
            if matches!(preset, Preset::H264 | Preset::H265) { arg!("-movflags"); arg!("+faststart"); }
        }
    }
    args.push(temp_output.as_os_str().to_owned());
    Ok(args)
}

fn tag(preset: Preset) -> &'static str {
    match preset {
        Preset::H264 => "h264", Preset::H265 => "h265", Preset::Av1 => "av1",
        Preset::Vp9 => "vp9", Preset::ProRes => "prores", Preset::Dnxhr => "dnxhr",
        Preset::RemuxMkv => "rewrap", Preset::Wav => "wav", Preset::Flac => "flac",
        Preset::Mp3 => "mp3", Preset::Aac => "aac",
    }
}

/// Collision-resistant relative names. Never overwrite an existing output.
pub fn output_path(input: &Path, o: &EncodeOptions) -> Result<PathBuf, String> {
    let parent = o.output_dir.as_deref().or_else(|| input.parent()).ok_or("No output directory")?;
    if !parent.is_dir() { return Err(format!("Output directory does not exist: {}", parent.display())); }
    let stem = input.file_stem().and_then(|v| v.to_str()).unwrap_or("media");
    let root = format!("{stem}_{}", tag(o.preset));
    for index in 0..100_000 {
        let suffix = if index == 0 { String::new() } else { format!("_{index}") };
        let output = parent.join(format!("{root}{suffix}.{}", o.preset.extension()));
        if output != input && !output.exists() { return Ok(output); }
    }
    Err("Too many output name collisions".into())
}

fn temporary_output(output: &Path, id: u64) -> Result<PathBuf, String> {
    let parent = output.parent().ok_or("Missing output parent")?;
    let stem = output.file_stem().and_then(|v| v.to_str()).unwrap_or("media");
    let ext = output.extension().and_then(|v| v.to_str()).unwrap_or("mkv");
    for n in 0..100_000 {
        let path = parent.join(format!(".{stem}.rust-encoder-{id}-{n}.partial.{ext}"));
        if !path.exists() { return Ok(path); }
    }
    Err("Cannot find unused temporary output path".into())
}

/// Atomic no-clobber hard link where supported; otherwise exclusive create + copy.
fn finalize_output(temp: &Path, output: &Path) -> std::io::Result<()> {
    match fs::hard_link(temp, output) {
        Ok(()) => { let _ = fs::remove_file(temp); Ok(()) }
        Err(e) if output.exists() => Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, format!("Output already exists: {e}"))),
        Err(_) => {
            let mut destination = OpenOptions::new().create_new(true).write(true).open(output)?;
            let result = (|| {
                let mut source = File::open(temp)?;
                std::io::copy(&mut source, &mut destination)?;
                destination.flush()?;
                destination.sync_all()
            })();
            drop(destination);
            if let Err(e) = result { let _ = fs::remove_file(output); return Err(e); }
            fs::remove_file(temp)?;
            Ok(())
        }
    }
}

#[derive(Default)]
struct ProgressFields { elapsed_sec: f64, speed: String }

fn parse_progress_line(line: &str, fields: &mut ProgressFields) -> bool {
    let Some((key, value)) = line.trim().split_once('=') else { return false };
    match key {
        "out_time_us" | "out_time_ms" => {
            if let Ok(v) = value.parse::<i64>() { fields.elapsed_sec = (v.max(0) as f64) / 1_000_000.0; }
        }
        "out_time" => {
            if let Some(seconds) = parse_timestamp(value) { fields.elapsed_sec = seconds; }
        }
        "speed" => fields.speed = value.to_owned(),
        "progress" => return true,
        _ => {}
    }
    false
}

fn parse_timestamp(value: &str) -> Option<f64> {
    let segments: Vec<_> = value.split(':').collect();
    if segments.len() != 3 { return None; }
    let hours = segments[0].parse::<u64>().ok()?;
    let minutes = segments[1].parse::<u64>().ok()?;
    let seconds = segments[2].parse::<f64>().ok()?;
    (minutes < 60 && seconds.is_finite() && seconds >= 0.0 && seconds < 60.0)
        .then_some(hours as f64 * 3600.0 + minutes as f64 * 60.0 + seconds)
}

pub fn start_job(id: u64, input: PathBuf, options: EncodeOptions, tx: Sender<Event>, cancelled: Arc<AtomicBool>, stopped: Arc<AtomicBool>) {
    thread::spawn(move || {
        let result = run_job(id, &input, &options, &tx, &cancelled);
        if let Err(message) = result {
            let _ = tx.send(if cancelled.load(Ordering::SeqCst) { Event::Cancelled { id } } else { Event::Failed { id, message } });
        }
        stopped.store(true, Ordering::SeqCst);
    });
}

fn run_job(id: u64, input: &Path, options: &EncodeOptions, tx: &Sender<Event>, cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::SeqCst) { return Err("Cancelled".into()); }
    let output = output_path(input, options)?;
    let temp = temporary_output(&output, id)?;
    let args = build_args(input, &temp, options)?;
    let mut child = Command::new(&options.ffmpeg)
        .args(&args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|e| format!("Cannot launch {}: {e}", options.ffmpeg.display()))?;
    let _ = tx.send(Event::Started { id, output: output.clone() });

    let (progress_tx, progress_rx) = crossbeam_channel::unbounded::<String>();
    let stdout = child.stdout.take().ok_or("Cannot capture FFmpeg progress")?;
    let reader_handle = thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(line) => { if progress_tx.send(line).is_err() { break; } }
                Err(_) => break,
            }
        }
    });
    let stderr = child.stderr.take().ok_or("Cannot capture FFmpeg diagnostics")?;
    let errors = Arc::new(Mutex::new(VecDeque::<String>::with_capacity(64)));
    let error_sink = Arc::clone(&errors);
    let stderr_handle = thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            match line {
                Ok(line) => {
                    if let Ok(mut tail) = error_sink.lock() {
                        if tail.len() == 64 { tail.pop_front(); }
                        tail.push_back(line.chars().take(4096).collect());
                    }
                }
                Err(_) => break,
            }
        }
    });
    let mut progress = ProgressFields::default();
    let (status, was_cancelled) = loop {
        if cancelled.load(Ordering::SeqCst) {
            let _ = child.kill();
            let status = child.wait().map_err(|e| format!("Cannot reap cancelled FFmpeg: {e}"))?;
            break (status, true);
        }
        if let Ok(line) = progress_rx.recv_timeout(Duration::from_millis(125)) {
            if parse_progress_line(&line, &mut progress) {
                let _ = tx.send(Event::Progress { id, elapsed_sec: progress.elapsed_sec, speed: progress.speed.clone() });
            }
        }
        match child.try_wait() {
            Ok(Some(status)) => break (status, false),
            Ok(None) => {}
            Err(e) => { let _ = child.kill(); let _ = child.wait(); return Err(format!("Cannot wait for FFmpeg: {e}")); }
        }
    };
    let _ = reader_handle.join();
    let _ = stderr_handle.join();
    if was_cancelled || cancelled.load(Ordering::SeqCst) {
        let _ = fs::remove_file(&temp);
        return Err("Cancelled".into());
    }
    if !status.success() {
        let stderr = errors.lock().ok().map(|lines| lines.iter().cloned().collect::<Vec<_>>().join("\n"))
            .unwrap_or_else(|| "Could not read FFmpeg diagnostics".into());
        let _ = fs::remove_file(&temp);
        return Err(format!("FFmpeg exited with {status}: {}", if stderr.is_empty() { "No details; inspect codecs and input" } else { &stderr }));
    }
    if cancelled.load(Ordering::SeqCst) { let _ = fs::remove_file(&temp); return Err("Cancelled".into()); }
    finalize_output(&temp, &output).map_err(|e| format!("Could not finalize output {}: {e}; intermediate file: {}", output.display(), temp.display()))?;
    let _ = tx.send(Event::Completed { id, output });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options(preset: Preset) -> EncodeOptions {
        EncodeOptions { preset, quality: 23, target_width: 0, audio_bitrate_kbps: 192,
            start_seconds: 0.0, end_seconds: 0.0, normalize_audio: false,
            output_dir: None, ffmpeg: PathBuf::from("ffmpeg") }
    }
    #[test]
    fn parse_progress() {
        let mut p = ProgressFields::default();
        assert!(!parse_progress_line("out_time_us=2500000", &mut p));
        assert!(!parse_progress_line("speed=3.2x", &mut p));
        assert!(parse_progress_line("progress=continue", &mut p));
        assert_eq!(p.elapsed_sec, 2.5);
        assert_eq!(p.speed, "3.2x");
        assert_eq!(parse_timestamp("00:01:30.250"), Some(90.25));
    }
    #[test]
    fn invalid_trim_rejected() {
        let mut o = options(Preset::H264);
        o.start_seconds = 20.0;
        o.end_seconds = 12.0;
        assert!(validate_options(&o).is_err());
    }
    #[test]
    fn output_names_do_not_overwrite_input() {
        let tmp = std::env::temp_dir();
        let mut o = options(Preset::H264);
        o.output_dir = Some(tmp.clone());
        let path = output_path(&tmp.join("sample.mp4"), &o).unwrap();
        assert_ne!(path, tmp.join("sample.mp4"));
        assert_eq!(path.extension().and_then(|x| x.to_str()), Some("mp4"));
    }
    #[test]
    fn args_do_not_require_shell_interpolation() {
        let tmp = std::env::temp_dir().join(format!("encoder-test-{}-{}.mp4", std::process::id(), "special &;$'"));
        std::fs::write(&tmp, b"not video").unwrap();
        let out = tmp.with_extension("mkv");
        let a = build_args(&tmp, &out, &options(Preset::RemuxMkv)).unwrap();
        assert!(a.iter().any(|v| v.as_os_str() == tmp.as_os_str()));
        assert!(!a.iter().any(|v| v.to_string_lossy() == "sh"));
        std::fs::remove_file(tmp).unwrap();
    }
}
