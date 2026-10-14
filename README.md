# Rust Encoder

Rust-native desktop media converter and batch processing tool built with **egui/eframe** and external **FFmpeg/FFprobe** executables. No Java runtime, WebView or Electron. The application and encoder orchestration are Rust; codecs remain provided by FFmpeg.

> **Scope:** This is **not yet a 1:1 replacement** for a fully featured post-production suite. The source can compile on Windows, Linux and macOS, but support for complex editing, AI-assisted processing, RAW, downloads and some professional workflows is still missing. See [FEATURE_PARITY.md](FEATURE_PARITY.md) and [ROADMAP.md](ROADMAP.md).

## Available features

- Desktop GUI with resize-friendly queue and settings, dark/light theme, drag/drop, file picker and per-file still-frame preview **with seek position** (not a realtime player).
- Background FFprobe metadata inspection with video/audio/subtitle track listings (including language and channel count when available).
- Queued jobs, batch execution, retry, pause-after-current, cancellation, FFmpeg progress, safe temporary outputs and non-overwriting filenames.
- **23 functions:** H.264, H.265/HEVC, AV1, VP9, ProRes, DNxHR, MKV rewrap, MP4 rewrap, video-only stream copy, WAV, FLAC, MP3, AAC, Opus, PNG still, JPEG still, SRT subtitle extraction, ASS subtitle extraction, frame MD5 checksums, black-frame detection, frozen-frame detection, silence detection, and EBU R128 loudness analysis.
- Encoding controls for CRF, audio bitrate, resize, crop, rotation, flips, output FPS, trim, loudness normalization (one-pass), manual audio/video/subtitle track mapping. Video restoration/color tools include YADIF deinterlace, HQDN3D denoise, sharpening, brightness, contrast, saturation and gamma. Audio processing supports mono/stereo output, configurable sample rate and gain in dB.
- Optional manual NVENC, Intel Quick Sync, AMD AMF, Apple VideoToolbox selection for H.264 and H.265. FFmpeg must include the selected encoder and compatible drivers; GPU capability detection/fallback is **not** implemented.
- Save/load JSON settings profiles. Newer settings load older profiles with defaults for unknown fields.

**Limitations:** MKV rewrap copies all streams, whereas MP4 rewrap copies audio/video only (MP4 cannot handle arbitrary stream types). Subtitles for encoded video are converted to a codec appropriate for the container where possible; the source must contain an accessible subtitle track. Audio extraction selects one track. Still-frame extraction exports one frame. Analysis functions scan media and save FFmpeg diagnostic reports as text. Additional filters require decoding/re-encoding, and are rejected for stream-copy modes instead of being silently ignored. Audio output conversion uses FFmpeg channel conversion and resampling; it is not an isolated per-channel mixer. Analysis thresholds are currently fixed defaults (0.5-second black/frozen/silence intervals, -50 dB silence, EBU R128 loudness). Cut accuracy when stream-copying depends on keyframes. Crop values must fit the input picture. Hardware encoders may require drivers and different rate control options.

## Dependencies

Rust 1.95+ (build only); FFmpeg, FFprobe and optional FFplay **are not included** in the portable archives. Put them in PATH, beside the binary, or configure paths under *External tools*. A codec can only be used if present in that FFmpeg build. Consult FFmpeg/codec licensing before redistributing binaries.

## Build and test

```sh
cargo build --release
cargo test --all-targets
cargo clippy --all-targets
```

Binary: `target/release/rust-encoder` or `rust-encoder.exe`. Check `rust-encoder --version` without opening a desktop window.

On Ubuntu/Debian, install the GUI dependencies and FFmpeg with:

```sh
sudo apt-get update
sudo apt-get install -y libgtk-3-dev libxkbcommon-dev libwayland-dev libxcb-shape0-dev libxcb-xfixes0-dev libxcb-render0-dev libssl-dev pkg-config ffmpeg
```

## CI / portable builds

[Build workflow](.github/workflows/build.yml) runs source checks, tests, lint, release compilation, a headless `--version` startup test and uploads portable ZIPs on Windows/macOS/Linux. On Linux it also executes the FFmpeg integration tests against generated media. Binaries are not code-signed or notarized; a successful headless startup test does not prove that every native GUI control renders correctly.

## Media output / presets

1. Add source files via picker or drop. Wait for FFprobe to inspect them.
2. Choose a function. Current settings apply to queued jobs. Active and completed jobs are unaffected by later changes.
3. Optionally configure video and audio streams, filters, trim and destination. If transcoding with hardware, verify your FFmpeg encoder list with `ffmpeg -encoders`.
4. Start function. A queued job gets a unique name such as `_h264.mp4`; failed/cancelled partial files are not committed as outputs.
5. Use *Save profile* and *Load profile* to exchange JSON profiles; settings also persist automatically.

## License

Rust Encoder source is GPL-3.0-or-later; see [LICENSE](LICENSE). It is not affiliated with any other media encoder GUI. FFmpeg and optional external utilities have their own licenses.

## Timeline and media inspection

- Click or drag the timeline to seek a source file. Step forward/back by the probed frame duration; playback previews up to 4 silent still frames per second, decoded asynchronously via FFmpeg.
- Mark In, Mark Out or clear the selection directly on the timeline. Marks update shared trim settings for all queued jobs; running jobs do not change. Stream-copy cuts remain keyframe-inexact.
- An FFmpeg `showwavespic` waveform of the first audio stream appears under the preview. Waveform generation runs off the GUI thread and can take time for long media.
- **Synchronized audio/video preview:** use **Play A/V (FFplay window)** from the timeline. Rust Encoder discovers FFplay beside the configured FFmpeg, beside the application, or in PATH; an explicit FFplay path can also be set under **External tools**. The external player starts at the timeline playhead and stops at the selected Out point, and is stopped when changing files or closing Rust Encoder. Playback controls within the FFplay window remain separate from the egui timeline.
- **Native continuous embedded video:** choose **Embedded video (silent)** to use a single, bounded, cancellable FFmpeg raw-frame decoder directly in the egui preview at up to 24 FPS. It automatically stops on file changes, manual seeking, or closing the window. FFmpeg remains required but no additional Rust video dependency is introduced.
- The embedded player does **not** include synchronized audio playback; FFplay remains the audio/video alternative. A native audio backend, multitrack editor and frame-accurate editing remain future milestones.
