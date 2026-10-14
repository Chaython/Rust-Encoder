use crate::{media::MediaInfo, model::{ColorSignal, EncodeOptions, HardwareEncoder, Preset}};
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
    PreviewReady { id: u64, seek_ms: u64, result: Result<crate::media::Thumbnail, String> },
    WaveformReady { id: u64, result: Result<crate::media::Thumbnail, String> },
    Started { id: u64, output: PathBuf },
    Progress { id: u64, elapsed_sec: f64, speed: String, pass: u8 },
    Completed { id: u64, output: PathBuf },
    Failed { id: u64, message: String },
    Cancelled { id: u64 },
}

pub fn validate_options(options: &EncodeOptions) -> Result<(), String> {
    if options.quality > 51 { return Err("Quality (CRF) must be between 0 and 51".into()); }
    if !matches!(options.output_bit_depth, 0 | 8 | 10) {
        return Err("Output bit depth must be Auto, 8-bit or 10-bit".into());
    }
    if options.output_bit_depth > 0 && !matches!(options.preset,
        Preset::H264 | Preset::H265 | Preset::Av1 | Preset::Vp9) {
        return Err("Explicit bit depth is only available for H.264, H.265, AV1 and VP9".into());
    }
    if options.output_bit_depth == 10 && (
        !matches!(options.preset, Preset::H265 | Preset::Av1)
        || options.hardware_encoder != HardwareEncoder::Software
    ) {
        return Err("10-bit output currently requires software HEVC or AV1 encoding".into());
    }
    if options.color_signal != ColorSignal::Automatic {
        if !matches!(options.preset, Preset::H264 | Preset::H265 | Preset::Av1 | Preset::Vp9) {
            return Err("Color signaling requires a video encoding preset".into());
        }
        if matches!(options.color_signal, ColorSignal::Bt2020Pq | ColorSignal::Bt2020Hlg)
            && (options.output_bit_depth != 10
                || !matches!(options.preset, Preset::H265 | Preset::Av1)
                || options.hardware_encoder != HardwareEncoder::Software)
        {
            return Err("HDR PQ/HLG tags require software H.265 or AV1 with 10-bit output; signaling does not convert SDR to HDR".into());
        }
    }
    if options.encoder_speed > 2 { return Err("Encoder speed must be 0–2".into()); }
    if options.two_pass {
        if !options.use_video_bitrate {
            return Err("Two-pass requires target bitrate, not constant quality (CRF)".into());
        }
        if options.hardware_encoder != HardwareEncoder::Software
            || !matches!(options.preset, Preset::H264 | Preset::H265 | Preset::Vp9) {
            return Err("Two-pass requires software H.264, H.265 or VP9".into());
        }
    }
    if options.encoder_threads > 32 { return Err("Encoder threads must be 0–32".into()); }
    if options.keyframe_interval > 1000 { return Err("Keyframe interval must be 0–1000".into()); }
    if options.use_video_bitrate && options.preset.video() {
        if !(100..=100_000).contains(&options.video_bitrate_kbps) {
            return Err("Target bitrate must be 100–100000 kb/s".into());
        }
        if options.max_video_bitrate_kbps != 0 && (options.max_video_bitrate_kbps < options.video_bitrate_kbps
            || options.max_video_bitrate_kbps > 200_000) {
            return Err("Peak bitrate must be at least target and ≤ 200000 kb/s".into());
        }
    }
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
    if !options.brightness.is_finite() || !(-1.0..=1.0).contains(&options.brightness)
        || !options.contrast.is_finite() || !(0.0..=2.0).contains(&options.contrast)
        || !options.saturation.is_finite() || !(0.0..=3.0).contains(&options.saturation)
        || !options.gamma.is_finite() || !(0.1..=4.0).contains(&options.gamma) {
        return Err("Color settings out of range or non-finite".into());
    }
    if options.audio_channels > 2 { return Err("Audio channels: choose source (0), mono (1) or stereo (2)".into()); }
    if options.audio_sample_rate != 0 && !(8_000..=192_000).contains(&options.audio_sample_rate) {
        return Err("Sample rate must be 0 (source) or 8000–192000 Hz".into());
    }
    if !options.audio_gain_db.is_finite() || !(-30.0..=30.0).contains(&options.audio_gain_db) {
        return Err("Audio gain must be finite and between -30 and +30 dB".into());
    }
    let audio_processing = options.audio_channels != 0 || options.audio_sample_rate != 0
        || options.audio_gain_db != 0.0;
    if audio_processing && !(options.preset.video() || options.preset.audio_only()) {
        return Err("Audio processing requires an encoding preset, not stream copy or analysis".into());
    }
    if audio_processing && options.audio_stream == -2 {
        return Err("Audio processing requires an enabled audio stream".into());
    }
    if options.frame_rate > 240 {
        return Err("Frame rate must be 0 (original) or 1–240".into());
    }
    if options.crop_width != 0 || options.crop_height != 0 {
        if options.crop_width < 2 || options.crop_height < 2 || options.crop_width > 16384 || options.crop_height > 16384 {
            return Err("Crop width and height must both be 2–16384".into());
        }
    }
    if !matches!(options.rotate_degrees, 0 | 90 | 180 | 270) {
        return Err("Rotation must be 0, 90, 180 or 270 degrees".into());
    }
    if options.audio_stream < -2 || options.subtitle_stream < -1 {
        return Err("Invalid audio or subtitle stream index".into());
    }
    if options.hardware_encoder != HardwareEncoder::Software
        && options.hardware_encoder.codec(options.preset).is_none() {
        return Err("This hardware encoder supports H.264/H.265 only. Switch to CPU/software for other functions".into());
    }
    if !options.preset.video() && (
        options.frame_rate != 0 || options.crop_width != 0 || options.crop_height != 0 ||
        options.rotate_degrees != 0 || options.flip_horizontal || options.flip_vertical || options.target_width != 0
        || options.deinterlace || options.denoise || options.sharpen
        || options.brightness != 0.0 || options.contrast != 1.0
        || options.saturation != 1.0 || options.gamma != 1.0
    ) {
        return Err("Video filters require a video-encoding function. Disable filters for copying, extraction or audio functions".into());
    }
    Ok(())
}

pub fn expected_duration(info: &MediaInfo, settings: &EncodeOptions) -> Option<f64> {
    let input_end = info.duration_sec.unwrap_or(f64::INFINITY);
    let desired_end = if settings.end_seconds == 0.0 { input_end } else { settings.end_seconds.min(input_end) };
    let duration = desired_end - settings.start_seconds;
    duration.is_finite().then_some(duration).filter(|v| *v > 0.0)
}

fn audio_filters(o: &EncodeOptions) -> Vec<String> {
    let mut filters = Vec::new();
    if o.audio_gain_db != 0.0 { filters.push(format!("volume={:.2}dB", o.audio_gain_db)); }
    if o.normalize_audio { filters.push("loudnorm=I=-16:TP=-1.5:LRA=11".into()); }
    filters
}

/// FFmpeg args are kept as OS strings and never joined into a shell command.
/// The output argument must be a unique temporary path, not a user file.
pub fn build_args(input: &Path, temp_output: &Path, o: &EncodeOptions) -> Result<Vec<OsString>, String> {
    validate_options(o)?;
    if !input.is_file() { return Err("Input does not exist".into()); }
    if input == temp_output { return Err("Input and output must differ".into()); }

    let loglevel = if o.preset.report() { "info" } else { "error" };
    let mut args: Vec<OsString> = ["-nostdin", "-hide_banner", "-loglevel", loglevel, "-nostats",
        "-stats_period", "0.25", "-progress", "pipe:1", "-n"]
        .iter().map(|v| OsString::from(*v)).collect();
    macro_rules! arg { ($value:expr) => { args.push(OsString::from(($value).to_string())) }; }
    arg!("-i"); args.push(input.as_os_str().to_owned());
    // Always pass FFmpeg options as arguments, never through a command shell.
    if o.start_seconds > 0.0 {
        arg!("-ss"); arg!(&format!("{:.3}", o.start_seconds));
    }
    if o.end_seconds > 0.0 {
        arg!("-t"); arg!(&format!("{:.3}", o.end_seconds - o.start_seconds));
    }
    match o.preset {
        Preset::BlackDetect | Preset::FreezeDetect => {
            arg!("-map"); arg!(&format!("0:v:{}", o.video_stream));
            arg!("-vf"); arg!(if o.preset == Preset::BlackDetect {
                "blackdetect=d=0.5:pix_th=0.10"
            } else { "freezedetect=n=-60dB:d=0.5" });
            for a in ["-an", "-sn", "-f", "null"] { arg!(a); }
        }
        Preset::SilenceDetect | Preset::LoudnessReport => {
            arg!("-map"); arg!(&format!("0:a:{}", o.audio_stream.max(0)));
            arg!("-af"); arg!(if o.preset == Preset::SilenceDetect {
                "silencedetect=n=-50dB:d=0.5"
            } else { "ebur128=peak=true" });
            for a in ["-vn", "-sn", "-f", "null"] { arg!(a); }
        }
        Preset::RemuxMkv => {
            // Preserve all streams (including subtitles, chapters, attachments and metadata).
            for a in ["-map", "0", "-c", "copy", "-map_metadata", "0", "-map_chapters", "0"] { arg!(a); }
        }
        Preset::RemuxMp4 => {
            // MP4 cannot contain every data/subtitle codec; copy A/V streams only.
            for a in ["-map", "0:v?", "-map", "0:a?", "-c", "copy", "-map_metadata", "0", "-map_chapters", "0", "-movflags", "+faststart"] { arg!(a); }
        }
        Preset::VideoOnly => {
            arg!("-map"); arg!(&format!("0:v:{}", o.video_stream));
            for a in ["-c:v", "copy", "-an", "-sn", "-dn"] { arg!(a); }
        }
        Preset::ExtractSrt | Preset::ExtractAss => {
            let stream = o.subtitle_stream.max(0);
            arg!("-map"); arg!(&format!("0:s:{stream}"));
            arg!("-c:s"); arg!(if o.preset == Preset::ExtractSrt { "srt" } else { "ass" });
            arg!("-f"); arg!(if o.preset == Preset::ExtractSrt { "srt" } else { "ass" });
        }
        Preset::PngStill | Preset::JpegStill => {
            arg!("-map"); arg!(&format!("0:v:{}", o.video_stream));
            arg!("-frames:v"); arg!("1");
            arg!("-an"); arg!("-sn");
            arg!("-c:v"); arg!(if o.preset == Preset::PngStill { "png" } else { "mjpeg" });
            if o.preset == Preset::JpegStill { arg!("-q:v"); arg!("2"); }
        }
        Preset::FrameMd5 => {
            arg!("-map"); arg!(&format!("0:v:{}", o.video_stream));
            arg!("-f"); arg!("framemd5");
        }
        preset if preset.audio_only() => {
            if o.audio_stream == -2 { return Err("An audio stream is required for audio extraction".into()); }
            arg!("-map"); arg!(&format!("0:a:{}", o.audio_stream.max(0)));
            for a in ["-vn", "-sn", "-dn"] { arg!(a); }
            match preset {
                Preset::Wav => { arg!("-c:a"); arg!("pcm_s24le"); }
                Preset::Flac => { arg!("-c:a"); arg!("flac"); }
                Preset::Mp3 => { arg!("-c:a"); arg!("libmp3lame"); }
                Preset::Aac => { arg!("-c:a"); arg!("aac"); }
                Preset::Opus => { arg!("-c:a"); arg!("libopus"); }
                _ => unreachable!(),
            }
            if matches!(preset, Preset::Mp3 | Preset::Aac | Preset::Opus) {
                arg!("-b:a"); arg!(&format!("{}k", o.audio_bitrate_kbps));
            }
            let filters = audio_filters(o);
            if !filters.is_empty() { arg!("-af"); arg!(&filters.join(",")); }
            if o.audio_channels != 0 { arg!("-ac"); arg!(&o.audio_channels.to_string()); }
            if o.audio_sample_rate != 0 { arg!("-ar"); arg!(&o.audio_sample_rate.to_string()); }
        }
        preset => {
            arg!("-map"); arg!(&format!("0:v:{}", o.video_stream));
            match o.audio_stream {
                -2 => {},
                -1 => { arg!("-map"); arg!("0:a?"); },
                index => { arg!("-map"); arg!(&format!("0:a:{index}")); }
            }
            if o.subtitle_stream >= 0 {
                arg!("-map"); arg!(&format!("0:s:{}", o.subtitle_stream));
                arg!("-c:s"); arg!(if preset == Preset::Vp9 { "webvtt" }
                    else if matches!(preset, Preset::H264 | Preset::H265 | Preset::ProRes | Preset::Dnxhr) { "mov_text" }
                    else { "srt" });
            }
            let bitrate = o.use_video_bitrate;
            match preset {
                Preset::H264 | Preset::H265 if o.hardware_encoder != HardwareEncoder::Software => {
                    let codec = o.hardware_encoder.codec(preset).ok_or("Unsupported hardware codec")?;
                    arg!("-c:v"); arg!(codec);
                    if bitrate {
                        if o.hardware_encoder == HardwareEncoder::Amf { arg!("-rc"); arg!("vbr_peak"); }
                        arg!("-b:v"); arg!(&format!("{}k", o.video_bitrate_kbps));
                    } else {
                        match o.hardware_encoder {
                            HardwareEncoder::Nvenc => {
                                arg!("-rc"); arg!("vbr"); arg!("-cq"); arg!(&o.quality.to_string());
                                arg!("-b:v"); arg!("0");
                            }
                            HardwareEncoder::Qsv => { arg!("-global_quality"); arg!(&o.quality.to_string()); }
                            HardwareEncoder::Amf => {
                                arg!("-rc"); arg!("cqp");
                                arg!("-qp_i"); arg!(&o.quality.to_string());
                                arg!("-qp_p"); arg!(&o.quality.to_string());
                            }
                            HardwareEncoder::VideoToolbox => { arg!("-b:v"); arg!("6M"); }
                            HardwareEncoder::Software => unreachable!(),
                        }
                    }
                }
                Preset::H264 | Preset::H265 => {
                    arg!("-c:v"); arg!(if preset == Preset::H264 { "libx264" } else { "libx265" });
                    arg!("-preset"); arg!(["fast", "medium", "slow"][o.encoder_speed as usize]);
                    if bitrate { arg!("-b:v"); arg!(&format!("{}k", o.video_bitrate_kbps)); }
                    else { arg!("-crf"); arg!(&o.quality.to_string()); }
                }
                Preset::Av1 => {
                    arg!("-c:v"); arg!("libsvtav1");
                    arg!("-preset"); arg!(["10", "6", "4"][o.encoder_speed as usize]);
                    if bitrate { arg!("-b:v"); arg!(&format!("{}k", o.video_bitrate_kbps)); }
                    else { arg!("-crf"); arg!(&o.quality.to_string()); }
                }
                Preset::Vp9 => {
                    arg!("-c:v"); arg!("libvpx-vp9");
                    arg!("-deadline"); arg!("good");
                    arg!("-cpu-used"); arg!(["4", "2", "1"][o.encoder_speed as usize]);
                    if bitrate { arg!("-b:v"); arg!(&format!("{}k", o.video_bitrate_kbps)); }
                    else {
                        arg!("-b:v"); arg!("0"); arg!("-crf"); arg!(&o.quality.to_string());
                    }
                }
                Preset::ProRes => { arg!("-c:v"); arg!("prores_ks"); arg!("-profile:v"); arg!("3"); arg!("-pix_fmt"); arg!("yuv422p10le"); }
                Preset::Dnxhr => { arg!("-c:v"); arg!("dnxhd"); arg!("-profile:v"); arg!("dnxhr_hq"); arg!("-pix_fmt"); arg!("yuv422p"); }
                _ => unreachable!(),
            }
            if bitrate && o.max_video_bitrate_kbps > 0 {
                arg!("-maxrate"); arg!(&format!("{}k", o.max_video_bitrate_kbps));
                arg!("-bufsize"); arg!(&format!("{}k", o.max_video_bitrate_kbps * 2));
            }
            if o.encoder_threads > 0 { arg!("-threads:v"); arg!(&o.encoder_threads.to_string()); }
            if o.keyframe_interval > 0 { arg!("-g"); arg!(&o.keyframe_interval.to_string()); }
            // Video encoders can otherwise silently fall back to 8-bit output.
            // Professional ProRes already uses a fixed 10-bit 4:2:2 format.
            if o.output_bit_depth == 10 {
                arg!("-pix_fmt"); arg!("yuv420p10le");
            } else if o.output_bit_depth == 8 || matches!(preset, Preset::H264 | Preset::H265) {
                arg!("-pix_fmt"); arg!("yuv420p");
            }
            if let Some((primaries, trc, matrix)) = o.color_signal.ffmpeg_values() {
                arg!("-color_primaries"); arg!(primaries);
                arg!("-color_trc"); arg!(trc);
                arg!("-colorspace"); arg!(matrix);
                if preset == Preset::H265
                    && o.hardware_encoder == HardwareEncoder::Software {
                    // libx265 requires matching bitstream VUI as well as FFmpeg
                    // stream fields. Otherwise FFprobe may report unknown tags.
                    let (cp, ct, cm) = match o.color_signal {
                        ColorSignal::Bt709 => (1, 1, 1),
                        ColorSignal::Bt2020Pq => (9, 16, 9),
                        ColorSignal::Bt2020Hlg => (9, 18, 9),
                        ColorSignal::Automatic => unreachable!(),
                    };
                    arg!("-x265-params");
                    arg!(&format!("colorprim={cp}:transfer={ct}:colormatrix={cm}"));
                }
            }
            let mut video_filters: Vec<String> = Vec::new();
            if o.deinterlace { video_filters.push("yadif=mode=send_frame:parity=auto:deint=all".into()); }
            if o.denoise { video_filters.push("hqdn3d=1.5:1.5:6:6".into()); }
            if o.crop_width > 0 {
                video_filters.push(format!("crop={}:{}:{}:{}", o.crop_width, o.crop_height, o.crop_x, o.crop_y));
            }
            match o.rotate_degrees {
                90 => video_filters.push("transpose=clock".into()),
                180 => video_filters.push("hflip,vflip".into()),
                270 => video_filters.push("transpose=cclock".into()),
                _ => {}
            }
            if o.flip_horizontal { video_filters.push("hflip".into()); }
            if o.flip_vertical { video_filters.push("vflip".into()); }
            if o.brightness != 0.0 || o.contrast != 1.0 || o.saturation != 1.0 || o.gamma != 1.0 {
                video_filters.push(format!("eq=brightness={:.3}:contrast={:.3}:saturation={:.3}:gamma={:.3}",
                    o.brightness, o.contrast, o.saturation, o.gamma));
            }
            if o.target_width > 0 { video_filters.push(format!("scale={}:-2:flags=lanczos", o.target_width)); }
            if o.sharpen { video_filters.push("unsharp=5:5:1.0:5:5:0.0".into()); }
            if o.frame_rate > 0 { video_filters.push(format!("fps={}", o.frame_rate)); }
            if !video_filters.is_empty() { arg!("-vf"); arg!(&video_filters.join(",")); }
            if o.audio_stream != -2 {
                if preset == Preset::Vp9 { arg!("-c:a"); arg!("libopus"); }
                else if matches!(preset, Preset::ProRes | Preset::Dnxhr) { arg!("-c:a"); arg!("pcm_s16le"); }
                else { arg!("-c:a"); arg!("aac"); arg!("-b:a"); arg!(&format!("{}k", o.audio_bitrate_kbps)); }
                let filters = audio_filters(o);
                if !filters.is_empty() { arg!("-af"); arg!(&filters.join(",")); }
                if o.audio_channels != 0 { arg!("-ac"); arg!(&o.audio_channels.to_string()); }
                if o.audio_sample_rate != 0 { arg!("-ar"); arg!(&o.audio_sample_rate.to_string()); }
            }
            if matches!(preset, Preset::H264 | Preset::H265) && o.faststart {
                arg!("-movflags"); arg!("+faststart");
            }
        }
    }
    if o.preset.report() {
        // Null output forces full decoding/filter traversal; diagnostic lines are
        // collected from stderr into the output TXT after successful execution.
        arg!("-");
    } else {
        args.push(temp_output.as_os_str().to_owned());
    }
    Ok(args)
}

fn tag(preset: Preset) -> &'static str {
    match preset {
        Preset::H264 => "h264", Preset::H265 => "h265", Preset::Av1 => "av1",
        Preset::Vp9 => "vp9", Preset::ProRes => "prores", Preset::Dnxhr => "dnxhr",
        Preset::RemuxMkv => "rewrap", Preset::RemuxMp4 => "rewrap_mp4",
        Preset::VideoOnly => "video", Preset::PngStill => "still",
        Preset::JpegStill => "still", Preset::BlackDetect => "black_report",
        Preset::FreezeDetect => "freeze_report", Preset::SilenceDetect => "silence_report",
        Preset::LoudnessReport => "loudness_report", Preset::ExtractSrt => "subtitles",
        Preset::ExtractAss => "subtitles", Preset::FrameMd5 => "checksums",
        Preset::Wav => "wav", Preset::Flac => "flac",
        Preset::Mp3 => "mp3", Preset::Aac => "aac", Preset::Opus => "opus",
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

/// Create an isolated statistics directory with an exclusive mkdir, not a
/// reusable global passlog. Drop removes files even if a pass fails or is
/// cancelled; the final output remains untouched until both passes succeed.
struct PassLogDir(PathBuf);

impl PassLogDir {
    fn create(output: &Path, id: u64) -> Result<Self, String> {
        let parent = output.parent().ok_or("No output directory for pass statistics")?;
        for n in 0..1000 {
            let candidate = parent.join(format!(
                ".rust-encoder-pass-{}-{}-{n}", std::process::id(), id
            ));
            match fs::create_dir(&candidate) {
                Ok(()) => return Ok(Self(candidate)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {},
                Err(error) => return Err(format!("Cannot create pass directory: {error}")),
            }
        }
        Err("Cannot find an unused two-pass statistics directory".into())
    }

    fn prefix(&self) -> PathBuf { self.0.join("encode") }
}

impl Drop for PassLogDir {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

pub fn build_two_pass_args(
    input: &Path, temp_output: &Path, options: &EncodeOptions,
    pass: u8, passlog: &Path,
) -> Result<Vec<OsString>, String> {
    if !options.two_pass || !matches!(pass, 1 | 2) {
        return Err("Two-pass arguments require pass 1 or 2 and two-pass mode".into());
    }
    // These passes use a private working directory for x265's stats. Resolve
    // input/output paths first, so a relative user path still reaches its file.
    let absolute_input = fs::canonicalize(input)
        .map_err(|error| format!("Cannot resolve two-pass input: {error}"))?;
    let absolute_output = if temp_output.is_absolute() {
        temp_output.to_owned()
    } else {
        std::env::current_dir().map_err(|error| error.to_string())?.join(temp_output)
    };
    let mut args = build_args(&absolute_input, &absolute_output, options)?;
    args.pop();
    if options.preset == Preset::H265 {
        // Generic FFmpeg -pass can exit successfully while x265 writes EMPTY
        // stats. Use x265-native pass/stat settings in the private cwd.
        let suffix = format!("pass={pass}:stats=encode.stats");
        if let Some(index) = args.iter().position(|item| item == "-x265-params") {
            args[index + 1].push(":");
            args[index + 1].push(&suffix);
        } else {
            args.push(OsString::from("-x265-params"));
            args.push(OsString::from(suffix));
        }
    } else {
        args.extend([OsString::from("-pass"), OsString::from(pass.to_string()),
            OsString::from("-passlogfile"), passlog.as_os_str().to_owned()]);
    }
    if pass == 1 {
        // Pass 1 decodes/filters video with the selected encoder, but MUST NOT
        // waste resources encoding audio or writing a regular media file.
        args.extend(["-an", "-sn", "-dn", "-f", "null", "-"]
            .into_iter().map(OsString::from));
    } else {
        args.push(absolute_output.as_os_str().to_owned());
    }
    Ok(args)
}

/// One FFmpeg pass. Both stdout progress and stderr diagnostics are drained
/// concurrently, so noisy encoder logs cannot deadlock on a full pipe.
fn run_ffmpeg_command(
    id: u64, ffmpeg: &Path, args: &[OsString], tx: &Sender<Event>,
    cancelled: &AtomicBool, report_lines: usize, pass: u8, two_pass: bool,
    working_directory: Option<&Path>,
) -> Result<Vec<String>, String> {
    let executable = if ffmpeg.is_file() {
        fs::canonicalize(ffmpeg).unwrap_or_else(|_| ffmpeg.to_owned())
    } else { ffmpeg.to_owned() };
    let mut command = Command::new(&executable);
    command.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(directory) = working_directory { command.current_dir(directory); }
    let mut child = command.spawn().map_err(|error|
        format!("Cannot launch {}: {error}", executable.display()))?;
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
    let sink = Arc::clone(&errors);
    let stderr_handle = thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            match line {
                Ok(line) => {
                    if let Ok(mut tail) = sink.lock() {
                        if tail.len() == report_lines { tail.pop_front(); }
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
        if let Ok(line) = progress_rx.recv_timeout(Duration::from_millis(125))
            && parse_progress_line(&line, &mut progress) {
            let speed = if two_pass {
                format!("Pass {pass}/2 · {}", progress.speed)
            } else { progress.speed.clone() };
            let _ = tx.send(Event::Progress {
                id, elapsed_sec: progress.elapsed_sec, speed, pass
            });
        }
        match child.try_wait() {
            Ok(Some(status)) => break (status, false),
            Ok(None) => {},
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("Cannot wait for FFmpeg: {error}"));
            }
        }
    };
    let _ = reader_handle.join();
    let _ = stderr_handle.join();
    if was_cancelled || cancelled.load(Ordering::SeqCst) { return Err("Cancelled".into()); }
    let diagnostics = errors.lock().ok()
        .map(|lines| lines.iter().cloned().collect::<Vec<_>>())
        .unwrap_or_else(|| vec!["Could not read FFmpeg diagnostics".into()]);
    if !status.success() {
        return Err(format!("FFmpeg pass {pass} exited with {status}: {}",
            if diagnostics.is_empty() {
                "No details; inspect codecs and input".to_owned()
            } else { diagnostics.join("\n") }));
    }
    Ok(diagnostics)
}

fn run_job(
    id: u64, input: &Path, options: &EncodeOptions, tx: &Sender<Event>,
    cancelled: &AtomicBool,
) -> Result<(), String> {
    if cancelled.load(Ordering::SeqCst) { return Err("Cancelled".into()); }
    let output = output_path(input, options)?;
    let temp = temporary_output(&output, id)?;
    // Always validate the command BEFORE broadcasting Started.
    let args = build_args(input, &temp, options)?;
    let _ = tx.send(Event::Started { id, output: output.clone() });
    let report_lines = if options.preset.report() { 4096 } else { 64 };
    let execute = (|| -> Result<Vec<String>, String> {
        if options.two_pass {
            let stats = PassLogDir::create(&temp, id)?;
            let prefix = stats.prefix();
            let first = build_two_pass_args(input, &temp, options, 1, &prefix)?;
            let second = build_two_pass_args(input, &temp, options, 2, &prefix)?;
            run_ffmpeg_command(id, &options.ffmpeg, &first, tx,
                cancelled, report_lines, 1, true, Some(&stats.0))?;
            if cancelled.load(Ordering::SeqCst) { return Err("Cancelled".into()); }
            if options.preset == Preset::H265 {
                let stats_path = stats.0.join("encode.stats");
                let stats_size = fs::metadata(&stats_path)
                    .map_err(|error| format!("x265 did not write pass-one statistics: {error}"))?
                    .len();
                if stats_size == 0 {
                    return Err("x265 pass-one statistics were empty; refusing invalid two-pass output".into());
                }
            }
            run_ffmpeg_command(id, &options.ffmpeg, &second, tx,
                cancelled, report_lines, 2, true, Some(&stats.0))
        } else {
            run_ffmpeg_command(id, &options.ffmpeg, &args, tx,
                cancelled, report_lines, 1, false, None)
        }
    })();
    let diagnostics = match execute {
        Ok(lines) => lines,
        Err(error) => {
            let _ = fs::remove_file(&temp);
            return Err(error);
        }
    };
    if cancelled.load(Ordering::SeqCst) {
        let _ = fs::remove_file(&temp);
        return Err("Cancelled".into());
    }
    if options.preset.report() {
        fs::write(&temp, format!("Rust Encoder analysis: {:?}\nInput: {}\n\n{}\n",
            options.preset, input.display(), diagnostics.join("\n")))
            .map_err(|error| format!("Could not save media analysis report: {error}"))?;
    }
    finalize_output(&temp, &output)
        .map_err(|error| format!("Could not finalize output {}: {error}; intermediate file: {}",
            output.display(), temp.display()))?;
    let _ = tx.send(Event::Completed { id, output });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options(preset: Preset) -> EncodeOptions {
        EncodeOptions { preset, quality: 23,
            output_bit_depth: 0, color_signal: ColorSignal::Automatic,
            use_video_bitrate: false, two_pass: false, video_bitrate_kbps: 4000,
            max_video_bitrate_kbps: 0, encoder_speed: 1, encoder_threads: 0,
            keyframe_interval: 0, faststart: true,
            target_width: 0, audio_bitrate_kbps: 192,
            start_seconds: 0.0, end_seconds: 0.0, normalize_audio: false,
            hardware_encoder: HardwareEncoder::Software,
            frame_rate: 0, crop_width: 0, crop_height: 0, crop_x: 0, crop_y: 0,
            rotate_degrees: 0, flip_horizontal: false, flip_vertical: false,
            audio_stream: -1, subtitle_stream: -1, video_stream: 0,
            deinterlace: false, denoise: false, sharpen: false,
            brightness: 0.0, contrast: 1.0, saturation: 1.0, gamma: 1.0,
            audio_channels: 0, audio_sample_rate: 0, audio_gain_db: 0.0,
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
    #[test]
    fn all_presets_create_non_shell_commands() {
        let input = std::env::temp_dir().join(format!("rust-encoder-fixture-{}.mkv", std::process::id()));
        std::fs::write(&input, b"test").unwrap();
        for preset in Preset::ALL {
            let o = options(preset);
            let args = build_args(&input, &std::env::temp_dir().join(format!("rust-encoder-test-output-{preset:?}.{}", preset.extension())), &o).unwrap();
            assert!(args.iter().any(|a| a.as_os_str() == input.as_os_str()), "{preset:?}");
            assert!(!args.is_empty(), "{preset:?}");
        }
        std::fs::remove_file(input).unwrap();
    }
    #[test]
    fn filter_chain_tracks_selected_streams() {
        let input = std::env::temp_dir().join(format!("rust-encoder-filter-{}.mkv", std::process::id()));
        std::fs::write(&input, b"test").unwrap();
        let mut o = options(Preset::H264);
        o.crop_width = 640; o.crop_height = 360; o.rotate_degrees = 90;
        o.frame_rate = 30; o.audio_stream = -2; o.subtitle_stream = 0;
        o.video_stream = 1;
        let args = build_args(&input, &input.with_extension("mp4"), &o).unwrap();
        let text = args.iter().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>();
        assert!(text.contains(&"0:v:1".to_owned()));
        assert!(text.contains(&"0:s:0".to_owned()));
        assert!(text.iter().any(|x| x.contains("crop=640:360:0:0,transpose=clock,fps=30")));
        assert!(!text.iter().any(|x| x == "-c:a"));
        std::fs::remove_file(input).unwrap();
    }
    #[test]
    fn advanced_filters_generate_single_video_and_audio_chains() {
        let path = std::env::temp_dir().join(format!("rust-encoder-adv-{}.mkv", std::process::id()));
        std::fs::write(&path, b"test").unwrap();
        let mut o = options(Preset::H264);
        o.deinterlace = true; o.denoise = true; o.sharpen = true;
        o.brightness = 0.1; o.contrast = 1.2; o.saturation = 1.3; o.gamma = 0.9;
        o.audio_channels = 2; o.audio_sample_rate = 48_000; o.audio_gain_db = -3.0;
        o.normalize_audio = true;
        let a: Vec<String> = build_args(&path, &path.with_extension("mp4"), &o).unwrap()
            .iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(a.iter().filter(|a| *a == "-vf").count(), 1);
        assert_eq!(a.iter().filter(|a| *a == "-af").count(), 1);
        assert!(a.iter().any(|a| a.contains("yadif=") && a.contains("hqdn3d=")
            && a.contains("eq=") && a.contains("unsharp=")));
        assert!(a.iter().any(|a| a == "volume=-3.00dB,loudnorm=I=-16:TP=-1.5:LRA=11"));
        assert!(a.windows(2).any(|a| a[0] == "-ac" && a[1] == "2"));
        assert!(a.windows(2).any(|a| a[0] == "-ar" && a[1] == "48000"));
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn incompatible_processing_fails_closed() {
        let mut o = options(Preset::RemuxMkv);
        o.audio_gain_db = 1.0;
        assert!(validate_options(&o).is_err());
        let mut o = options(Preset::H264);
        o.gamma = f32::NAN;
        assert!(validate_options(&o).is_err());
        o.gamma = 1.0; o.audio_stream = -2; o.audio_channels = 1;
        assert!(validate_options(&o).is_err());
        let mut o = options(Preset::RemuxMp4);
        o.deinterlace = true;
        assert!(validate_options(&o).is_err());
    }
    #[test]
    fn bitrate_speed_and_gop_controls_generate_valid_ffmpeg_options() {
        let input = std::env::temp_dir().join(format!("encoder-bitrate-{}.mkv", std::process::id()));
        std::fs::write(&input, b"fixture").unwrap();
        let mut o = options(Preset::H264);
        o.use_video_bitrate = true; o.video_bitrate_kbps = 2400;
        o.max_video_bitrate_kbps = 4000; o.encoder_speed = 0;
        o.encoder_threads = 4; o.keyframe_interval = 60; o.faststart = false;
        let args: Vec<String> = build_args(&input, &input.with_extension("mp4"), &o).unwrap()
            .into_iter().map(|x| x.to_string_lossy().into_owned()).collect();
        for (key, val) in [("-b:v", "2400k"), ("-maxrate", "4000k"),
            ("-bufsize", "8000k"), ("-preset", "fast"),
            ("-threads:v", "4"), ("-g", "60")] {
            assert!(args.windows(2).any(|a| a[0] == key && a[1] == val), "{key}");
        }
        assert!(!args.iter().any(|v| v == "-crf" || v == "-movflags"));
        o.max_video_bitrate_kbps = 100;
        assert!(validate_options(&o).is_err());
        o.max_video_bitrate_kbps = 0; o.video_bitrate_kbps = 0;
        assert!(validate_options(&o).is_err());
        std::fs::remove_file(input).unwrap();
    }

    #[test]
    fn two_pass_commands_are_safe_and_isolated() {
        let input = std::env::temp_dir().join(format!("two-pass-{}.mkv", std::process::id()));
        std::fs::write(&input, b"fixture").unwrap();
        let mut o = options(Preset::H264);
        o.use_video_bitrate = true;
        o.two_pass = true;
        let out = input.with_extension("mp4");
        let dir = PassLogDir::create(&out, 42).unwrap();
        let first = build_two_pass_args(&input, &out, &o, 1, &dir.prefix()).unwrap();
        let second = build_two_pass_args(&input, &out, &o, 2, &dir.prefix()).unwrap();
        assert_eq!(first.last().unwrap(), "-");
        assert_eq!(second.last().unwrap(), out.as_os_str());
        assert!(first.windows(2).any(|v| v[0] == "-pass" && v[1] == "1"));
        assert!(second.windows(2).any(|v| v[0] == "-pass" && v[1] == "2"));
        assert!(first.iter().any(|x| x == "-an"));
        assert!(first.iter().any(|x| x == "-f"));
        assert!(!second.iter().any(|x| x == "-f"));
        let path = dir.0.clone();
        drop(dir);
        assert!(!path.exists());
        o.use_video_bitrate = false;
        assert!(validate_options(&o).is_err());
        o.use_video_bitrate = true;
        o.hardware_encoder = HardwareEncoder::Nvenc;
        assert!(validate_options(&o).is_err());
        o.hardware_encoder = HardwareEncoder::Software;
        o.preset = Preset::H265;
        assert!(validate_options(&o).is_ok());
        let hevc_first = build_two_pass_args(&input, &out, &o, 1, &dir.prefix()).unwrap();
        assert!(hevc_first.windows(2).any(|v| v[0] == "-x265-params"
            && v[1].to_string_lossy().contains("pass=1:stats=encode.stats")));
        assert!(!hevc_first.iter().any(|value| value == "-passlogfile"));
        std::fs::remove_file(input).unwrap();
    }

    #[test]
    fn invalid_hardware_codec_rejected() {
        let mut o = options(Preset::ProRes);
        o.hardware_encoder = HardwareEncoder::Nvenc;
        assert!(validate_options(&o).is_err());
    }

}
