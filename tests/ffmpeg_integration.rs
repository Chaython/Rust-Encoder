//! End-to-end FFmpeg tests. Skipped on development machines without FFmpeg.
//! Linux CI installs FFmpeg so these are exercised there.
use rust_encoder::{encode, preview_stream, audio_preview, model::{HardwareEncoder, EncodeOptions, Preset}};
use std::{fs, path::{Path, PathBuf}, process::Command,
    sync::{Arc, atomic::AtomicBool}, time::Duration};

fn options(preset: Preset) -> EncodeOptions {
    EncodeOptions {
        preset, quality: 25,
        output_bit_depth: 0, color_signal: rust_encoder::model::ColorSignal::Automatic,
        use_video_bitrate: false, two_pass: false, video_bitrate_kbps: 4000,
        max_video_bitrate_kbps: 0, encoder_speed: 1, encoder_threads: 0,
        keyframe_interval: 0, faststart: true,
        target_width: 0, audio_bitrate_kbps: 128,
        start_seconds: 0.0, end_seconds: 0.0, normalize_audio: false, normalization_target_lufs: -16, audio_fade_in_seconds: 0.0, video_fade_in_seconds: 0.0, audio_fade_out_seconds: 0.0, video_fade_out_seconds: 0.0,
        hardware_encoder: HardwareEncoder::Software, frame_rate: 0, frame_rate_numerator: 0, frame_rate_denominator: 1,
        crop_width: 0, crop_height: 0, crop_x: 0, crop_y: 0,
        rotate_degrees: 0, flip_horizontal: false, flip_vertical: false,
        audio_stream: -1, audio_mix_secondary_stream: -1, subtitle_stream: -1, video_stream: 0,
        deinterlace: false, broadcast_video_limiter: false,
        video_detail_amount: 0, video_grain_amount: 0,
        video_nlmeans_strength: 0, video_deflicker_amount: 0,
        denoise: false, sharpen: false,
        brightness: 0.0, contrast: 1.0, saturation: 1.0, gamma: 1.0,
        audio_channels: 0, audio_sample_rate: 0, audio_gain_db: 0.0, audio_offset_seconds: 0.0, audio_highpass_hz: 0, audio_lowpass_hz: 0, audio_equalizer_db: 0.0, audio_equalizer_hz: 1000, audio_eq_bands_db: [0.0; 5],
        output_dir: None, ffmpeg: PathBuf::from("ffmpeg"),
    }
}

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg").arg("-version").output().is_ok()
}

fn command_ok(cmd: &mut Command) {
    let output = cmd.output().expect("failed to launch ffmpeg");
    assert!(output.status.success(), "FFmpeg failed: {}", String::from_utf8_lossy(&output.stderr));
}

fn fixture(dir: &Path) -> PathBuf {
    let input = dir.join("video test 🦀.mkv");
    command_ok(Command::new("ffmpeg").args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=128x96:rate=10:duration=1", "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=44100:duration=1", "-shortest", "-c:v", "mpeg4", "-c:a", "aac"]).arg(&input));
    input
}

#[test]
fn conversion_preview_audio_and_remux_finish() {
    if !ffmpeg_available() { eprintln!("Skipping media test: FFmpeg is not installed"); return; }
    let dir = std::env::temp_dir().join(format!("rust-encoder-e2e-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let input = fixture(&dir);
    for (index, preset) in [Preset::H264, Preset::RemuxMkv, Preset::RemuxMp4, Preset::VideoOnly,
                   Preset::Wav, Preset::Flac, Preset::Mp3, Preset::Aac,
                   Preset::PngStill, Preset::JpegStill, Preset::FrameMd5,
                   Preset::BlackDetect, Preset::FreezeDetect, Preset::SilenceDetect,
                   Preset::LoudnessReport].into_iter().enumerate() {
        let output = dir.join(format!("test-{preset:?}.{}", preset.extension()));
        let mut options = options(preset);
        if preset == Preset::H264 {
            options.crop_width = 96;
            options.crop_height = 64;
            options.rotate_degrees = 90;
            options.frame_rate = 5;
            options.deinterlace = true;
            options.denoise = true;
            options.broadcast_video_limiter = true;
            options.video_detail_amount = 3;
            options.video_grain_amount = 20;
            options.video_nlmeans_strength = 2;
            options.video_deflicker_amount = 10;
            options.sharpen = true;
            options.brightness = 0.12;
            options.contrast = 1.1;
            options.saturation = 1.25;
            options.gamma = 0.95;
            options.audio_channels = 1;
            options.audio_sample_rate = 48_000;
            options.audio_gain_db = -2.5;
            options.audio_fade_in_seconds = 0.25;
            options.video_fade_in_seconds = 0.25;
        }
        if preset == Preset::Flac {
            options.audio_channels = 2;
            options.audio_sample_rate = 48_000;
            options.audio_gain_db = -6.0;
        }
        if preset.report() {
            // Run the real background worker, not only its command builder;
            // the worker captures filter diagnostics and finalizes an analysis TXT.
            let (tx, rx) = crossbeam_channel::unbounded();
            encode::start_job(900 + index as u64, input.clone(), options,
                tx, Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
            let report_path = loop {
                match rx.recv_timeout(Duration::from_secs(25)).expect("analyzer timed out") {
                    encode::Event::Completed { output, .. } => break output,
                    encode::Event::Failed { message, .. } => panic!("{preset:?}: {message}"),
                    encode::Event::Cancelled { .. } => panic!("Unexpected cancellation"),
                    _ => {}
                }
            };
            assert!(report_path.is_file(), "missing analysis file for {preset:?}");
            assert!(fs::metadata(report_path).unwrap().len() > 30);
        } else {
            let args = encode::build_args(&input, &output, &options).unwrap();
            command_ok(Command::new("ffmpeg").args(args));
            assert!(output.exists(), "missing output for {preset:?}");
            assert!(fs::metadata(output).unwrap().len() > 0, "empty {preset:?}");
        }
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn media_waveform_and_frame_rate_probe_work() {
    if !ffmpeg_available() { eprintln!("Skipping media preview: FFmpeg is not installed"); return; }
    let dir = std::env::temp_dir().join(format!("rust-encoder-timeline-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let input = fixture(&dir);
    let result = rust_encoder::media::probe_file(Path::new("ffprobe"), &input).unwrap();
    assert_eq!(result.video_frame_rate, Some(10.0));
    assert!(result.audio_tracks.len() >= 1);
    let waveform = rust_encoder::media::waveform_file(Path::new("ffmpeg"), &input).unwrap();
    assert_eq!((waveform.width, waveform.height), (900, 120));
    assert_eq!(waveform.rgba.len(), 900 * 120 * 4);
    let still = rust_encoder::media::thumbnail_file_at(Path::new("ffmpeg"), &input, 0.5).unwrap();
    assert!(still.width > 0 && still.height > 0);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn embedded_preview_streams_real_decoded_frames_and_can_cancel() {
    if !ffmpeg_available() { eprintln!("Skipping stream preview: FFmpeg missing"); return; }
    let dir = std::env::temp_dir().join(format!("rust-encoder-stream-e2e-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let input = fixture(&dir);
    let stream = preview_stream::NativeVideoPreview::launch(
        10, Path::new("ffmpeg"), &input, 0.0, Some(0.8)).unwrap();
    let start = std::time::Instant::now();
    let frame = loop {
        if let Some(frame) = stream.newest_frame() { break frame; }
        if start.elapsed() > Duration::from_secs(20) { panic!("No decoded video frame"); }
        std::thread::sleep(Duration::from_millis(30));
    };
    assert_eq!(frame.rgba.len(), preview_stream::WIDTH * preview_stream::HEIGHT * 4);
    assert!(frame.position.is_finite());
    stream.stop();
    drop(stream);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn embedded_audio_pcm_decoder_produces_real_stereo_samples() {
    if !ffmpeg_available() { eprintln!("Skipping audio PCM integration: FFmpeg missing"); return; }
    let dir = std::env::temp_dir().join(format!("rust-encoder-pcm-e2e-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let input = fixture(&dir);
    let args = audio_preview::audio_args(&input, 0.0, Some(0.5), 0).unwrap();
    let output = Command::new("ffmpeg").args(args).output().unwrap();
    assert!(output.status.success(), "PCM FFmpeg failed: {}", String::from_utf8_lossy(&output.stderr));
    assert!(output.stdout.len() >= 8192);
    assert_eq!(output.stdout.len() % 8, 0, "Expected interleaved stereo f32 samples");
    assert!(output.stdout.chunks_exact(4).take(1024)
        .map(|x| f32::from_le_bytes(x.try_into().unwrap()))
        .any(|sample| sample.is_finite() && sample.abs() > 0.00001));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn h264_two_pass_produces_valid_file_and_cleans_logs() {
    if !ffmpeg_available() { eprintln!("Skipping two-pass FFmpeg test"); return; }
    let dir = std::env::temp_dir().join(format!("rust-encoder-two-pass-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let input = fixture(&dir);
    let mut options = options(Preset::H264);
    options.use_video_bitrate = true;
    options.video_bitrate_kbps = 500;
    options.two_pass = true;
    let (tx, rx) = crossbeam_channel::unbounded();
    encode::start_job(803, input, options, tx,
        Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
    let mut last_pass = 0;
    let path = loop {
        match rx.recv_timeout(Duration::from_secs(45)).expect("Two-pass encode timed out") {
            encode::Event::Progress { pass, .. } => { last_pass = last_pass.max(pass); }
            encode::Event::Completed { output, .. } => break output,
            encode::Event::Failed { message, .. } => panic!("Two-pass failed: {message}"),
            encode::Event::Cancelled { .. } => panic!("Unexpected two-pass cancellation"),
            _ => {}
        }
    };
    assert_eq!(last_pass, 2);
    assert!(path.is_file());
    assert!(fs::metadata(&path).unwrap().len() > 1000);
    let probe = rust_encoder::media::probe_file(Path::new("ffprobe"), &path).unwrap();
    assert!(probe.video_codec.is_some());
    assert!(!fs::read_dir(&dir).unwrap().any(|entry| {
        entry.unwrap().file_name().to_string_lossy().starts_with(".rust-encoder-pass-")
    }));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn h265_two_pass_main10_rec709_signaling_is_verified() {
    if !ffmpeg_available() { eprintln!("Skipping HEVC test; FFmpeg missing"); return; }
    let encoders = Command::new("ffmpeg").args(["-hide_banner", "-encoders"]).output().unwrap();
    if !String::from_utf8_lossy(&encoders.stdout).contains("libx265") {
        eprintln!("Skipping HEVC test; libx265 unavailable");
        return;
    }
    let dir = std::env::temp_dir().join(format!("rust-encoder-hevc-main10-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let input = fixture(&dir);
    let mut o = options(Preset::H265);
    o.two_pass = true;
    o.use_video_bitrate = true;
    o.video_bitrate_kbps = 500;
    o.encoder_speed = 0;
    o.output_bit_depth = 10;
    o.color_signal = rust_encoder::model::ColorSignal::Bt709;
    let (tx, rx) = crossbeam_channel::unbounded();
    encode::start_job(842, input, o, tx,
        Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
    let output = loop {
        match rx.recv_timeout(Duration::from_secs(75)).expect("HEVC two-pass encode timed out") {
            encode::Event::Completed { output, .. } => break output,
            encode::Event::Failed { message, .. } => panic!("HEVC two-pass failed: {message}"),
            encode::Event::Cancelled { .. } => panic!("Unexpected HEVC cancellation"),
            _ => {}
        }
    };
    let probe = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries",
            "stream=codec_name,pix_fmt,color_primaries,color_transfer,color_space",
            "-of", "default=noprint_wrappers=1"])
        .arg(&output).output().unwrap();
    assert!(probe.status.success(), "FFprobe failed");
    let metadata = String::from_utf8_lossy(&probe.stdout);
    for field in ["codec_name=hevc", "pix_fmt=yuv420p10le",
                  "color_primaries=bt709", "color_transfer=bt709", "color_space=bt709"] {
        assert!(metadata.contains(field), "Expected {field}:\n{metadata}");
    }
    assert!(!fs::read_dir(&dir).unwrap().any(|entry| {
        entry.unwrap().file_name().to_string_lossy().starts_with(".rust-encoder-pass-")
    }), "x265 pass statistics must be removed after completion");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn av1_main10_pq_sequence_metadata_roundtrip() {
    if !ffmpeg_available() { eprintln!("Skipping AV1 metadata test: FFmpeg unavailable"); return; }
    let encoders = Command::new("ffmpeg").args(["-hide_banner", "-encoders"]).output().unwrap();
    if !String::from_utf8_lossy(&encoders.stdout).contains("libsvtav1") {
        eprintln!("Skipping AV1 metadata test: libsvtav1 unavailable");
        return;
    }
    let dir = std::env::temp_dir().join(format!("rust-encoder-av1-color-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let input = fixture(&dir);
    let mut settings = options(Preset::Av1);
    settings.encoder_speed = 0;
    settings.output_bit_depth = 10;
    settings.color_signal = rust_encoder::model::ColorSignal::Bt2020Pq;
    let output = dir.join("tagged.mkv");
    // The input fixture is SDR; this test checks OUTPUT METADATA ONLY, not
    // SDR-to-HDR conversion, tone mapping or transfer/color correctness.
    command_ok(Command::new("ffmpeg").args(encode::build_args(&input, &output, &settings).unwrap()));
    let report = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries",
            "stream=codec_name,pix_fmt,color_primaries,color_transfer,color_space",
            "-of", "default=noprint_wrappers=1"])
        .arg(&output).output().unwrap();
    assert!(report.status.success());
    let details = String::from_utf8_lossy(&report.stdout);
    for entry in ["codec_name=av1", "pix_fmt=yuv420p10le", "color_primaries=bt2020",
        "color_transfer=smpte2084", "color_space=bt2020nc"] {
        assert!(details.contains(entry), "{entry} missing from:\n{details}");
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn vp9_ten_bit_profile_two_roundtrip() {
    if !ffmpeg_available() { eprintln!("Skipping VP9 10-bit test: FFmpeg unavailable"); return; }
    let encoders = Command::new("ffmpeg").args(["-hide_banner", "-encoders"]).output().unwrap();
    if !String::from_utf8_lossy(&encoders.stdout).contains("libvpx-vp9") {
        eprintln!("Skipping VP9 10-bit test: libvpx-vp9 unavailable");
        return;
    }
    let dir = std::env::temp_dir().join(format!("rust-encoder-vp9-profile2-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let input = fixture(&dir);
    let mut settings = options(Preset::Vp9);
    settings.output_bit_depth = 10;
    settings.encoder_speed = 0;
    settings.audio_stream = -2;
    // Exercise the real queue worker, including temporary output creation,
    // collision-safe finalization and completion signaling.
    let (tx, rx) = crossbeam_channel::unbounded();
    encode::start_job(9042, input, settings, tx,
        Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
    let output = loop {
        match rx.recv_timeout(Duration::from_secs(90)).expect("VP9 Profile 2 encode timed out") {
            encode::Event::Completed { output, .. } => break output,
            encode::Event::Failed { message, .. } => panic!("VP9 Profile 2 failed: {message}"),
            encode::Event::Cancelled { .. } => panic!("VP9 Profile 2 cancelled unexpectedly"),
            _ => {}
        }
    };
    assert!(output.exists());
    let probe = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries",
            "stream=codec_name,profile,pix_fmt", "-of", "default=noprint_wrappers=1"])
        .arg(&output).output().unwrap();
    assert!(probe.status.success());
    let metadata = String::from_utf8_lossy(&probe.stdout);
    for field in ["codec_name=vp9", "profile=Profile 2", "pix_fmt=yuv420p10le"] {
        assert!(metadata.contains(field), "Expected {field}:\n{metadata}");
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn two_selected_audio_streams_mix_into_one_encodable_track() {
    if !ffmpeg_available()
        || Command::new("ffprobe").arg("-version").output().is_err() {
        eprintln!("Skipping mix integration: FFmpeg/FFprobe unavailable");
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "rust-encoder-mix-e2e-{}-{:?}", std::process::id(), std::thread::current().id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let input = dir.join("two-tracks.mkv");
    command_ok(Command::new("ffmpeg").args([
        "-nostdin", "-hide_banner", "-loglevel", "error", "-y",
        "-f", "lavfi", "-i", "testsrc2=size=128x96:rate=10:duration=1",
        "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000:duration=1",
        "-f", "lavfi", "-i", "sine=frequency=880:sample_rate=48000:duration=1",
        "-map", "0:v", "-map", "1:a", "-map", "2:a",
        "-c:v", "mpeg4", "-c:a", "aac", "-shortest",
    ]).arg(&input));

    for (preset, name) in [(Preset::H264, "mixed.mp4"), (Preset::Wav, "mixed.wav")] {
        let mut o = options(preset);
        o.audio_stream = 0;
        o.audio_mix_secondary_stream = 1;
        o.audio_gain_db = -3.0;
        let output = dir.join(name);
        let args = encode::build_args(&input, &output, &o).unwrap();
        command_ok(Command::new("ffmpeg").args(args));
        let streams = Command::new("ffprobe").args([
            "-v", "error", "-select_streams", "a",
            "-show_entries", "stream=index", "-of", "csv=p=0",
        ]).arg(&output).output().unwrap();
        assert!(streams.status.success());
        assert_eq!(String::from_utf8_lossy(&streams.stdout).lines().count(), 1,
            "expected one mixed audio stream in {}", name);
    }
    fs::remove_dir_all(dir).unwrap();
}
