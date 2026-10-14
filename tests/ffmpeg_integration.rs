//! End-to-end FFmpeg tests. Skipped on development machines without FFmpeg.
//! Linux CI installs FFmpeg so these are exercised there.
use rust_encoder::{encode, model::{HardwareEncoder, EncodeOptions, Preset}};
use std::{fs, path::{Path, PathBuf}, process::Command};

fn options(preset: Preset) -> EncodeOptions {
    EncodeOptions {
        preset, quality: 25, target_width: 0, audio_bitrate_kbps: 128,
        start_seconds: 0.0, end_seconds: 0.0, normalize_audio: false,
        hardware_encoder: HardwareEncoder::Software, frame_rate: 0,
        crop_width: 0, crop_height: 0, crop_x: 0, crop_y: 0,
        rotate_degrees: 0, flip_horizontal: false, flip_vertical: false,
        audio_stream: -1, subtitle_stream: -1, video_stream: 0,
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
    for preset in [Preset::H264, Preset::RemuxMkv, Preset::RemuxMp4, Preset::VideoOnly,
                   Preset::Wav, Preset::Flac, Preset::Mp3, Preset::Aac,
                   Preset::PngStill, Preset::JpegStill, Preset::FrameMd5] {
        let output = dir.join(format!("test-{preset:?}.{}", preset.extension()));
        let mut options = options(preset);
        if preset == Preset::H264 {
            options.crop_width = 96;
            options.crop_height = 64;
            options.rotate_degrees = 90;
            options.frame_rate = 5;
        }
        let args = encode::build_args(&input, &output, &options).unwrap();
        command_ok(Command::new("ffmpeg").args(args));
        assert!(output.exists(), "missing output for {preset:?}");
        assert!(fs::metadata(output).unwrap().len() > 0, "empty {preset:?}");
    }
    fs::remove_dir_all(dir).unwrap();
}
