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

**Limitations:** MKV rewrap copies all streams, whereas MP4 rewrap copies audio/video only (MP4 cannot handle arbitrary stream types). Subtitles for encoded video are converted to a codec appropriate for the container where possible; the source must contain an accessible subtitle track. Audio extraction selects one track. Still-frame extraction exports one frame. Analysis functions scan media and save FFmpeg diagnostic reports as text. Additional filters require decoding/re-encoding, and are rejected for stream-copy modes instead of being silently ignored. Loudness normalization is available only for audio/video encoding, not rewrap, analysis, or extraction without audio; switching to a non-encoding function or choosing No audio clears incompatible audio-processing settings. Audio output conversion uses FFmpeg channel conversion and resampling; it is not an isolated per-channel mixer. Analysis thresholds are currently fixed defaults (0.5-second black/frozen/silence intervals, -50 dB silence, EBU R128 loudness). Cut accuracy when stream-copying depends on keyframes. Crop values must fit the input picture. Hardware encoders may require drivers and different rate control options.

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

## Automatic FFmpeg setup

Rust Encoder checks **both** FFmpeg and FFprobe as soon as the app launches. When either is unavailable, it downloads the appropriate platform build from [binmgr/ffmpeg GitHub Releases](https://github.com/binmgr/ffmpeg/releases) to the per-user application data directory (`rust-encoder/tools/ffmpeg`). The download runs in a worker thread, requires **no administrator privileges**, and displays the current stage in the GUI. Builds are checked against the upstream SHA-256 checksum manifest before any executable is installed.

Previously working binaries on PATH and valid manually configured executable paths are respected. Invalid overrides are cleared after successful installation. File imports made during setup are queued until FFprobe is ready. If automatic download fails (network/permissions/unsupported architecture), use **Recheck / install tools** or supply executable paths under **External tools**.

The automatically downloaded FFmpeg binaries are **separate third-party GPL media software**, not included with Rust Encoder's rolling-release ZIP. Installation requires an Internet connection, and supported platforms are Windows/Linux/macOS on x64/ARM64; see the upstream release for build licensing and provenance.

## Rolling release

Every successful build of `main` updates the [Rolling prerelease](https://github.com/Chaython/Rust-Encoder/releases/tag/rolling) with verified Windows, Linux, and macOS portable ZIPs and `SHA256SUMS.txt`.
All three platforms must finish compiling and testing before the release is updated. Pull requests and version-tag builds cannot overwrite the rolling release.
The reusable `rolling` tag advances to the exact tested commit; the release URL stays the same between builds.
FFmpeg, FFprobe, optional FFplay, code signing, and native installers are **not** included in these portable archives.
## Streamlined workspace and encoding performance

The desktop layout is adaptive, with draggable pane handles whose proportions persist between launches: **three separate panes** on a wide screen (searchable file queue and function selector; large central media preview; independently scrolling encoding options). Medium screens use two columns and narrow windows stack panels. A **Compact** queue option uses egui virtual scrolling to render only visible jobs, even in large batches; turn it off for detailed file cards and per-job metadata. Presets are grouped by video, stream copy, audio, images/subtitles and analysis.

Video functions now support **constant-quality (CRF) or target-bitrate** encoding (including optional real two-pass software H.264/H.265/VP9), a configurable optional peak bitrate/buffer, Fast/Balanced/Quality software encoder speed, a CPU thread limit, adjustable GOP/keyframe interval and MP4 faststart toggle. AV1, VP9, x264 and x265 use codec-appropriate speed controls. Hardware choices are probed asynchronously when FFmpeg becomes available: a codec must be listed and must successfully encode a one-frame test clip on the actual device/driver before it is selectable. The GUI supports rescanning after hardware or FFmpeg changes. Target-bitrate support still depends on hardware and driver capabilities.

For identical output quality, speed settings change encoding time and compression efficiency rather than promising a universal speedup; the hardware backend and video filters remain performance bottlenecks. Estimates based on input duration and video/audio bitrates are approximate, not guarantees. Existing JSON profiles load with backward-compatible defaults.

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
- **Native continuous embedded A/V and audio-only playback (experimental):** choose **Embedded A/V** to decode video frames in a bounded 24 FPS stream and, when an audio track exists, decode stereo 48 kHz PCM into a bounded Rust rodio/CPAL output stream. Volume is adjustable. The preview uses rodio's playback clock as a guide to video timing, dropping late frames or holding early frames. Audio-only media can now play directly from the timeline. The clock counts real consumed PCM samples, not silence inserted during underruns. A short prebuffer reduces startup underruns, and the decoder preserves the final partial PCM block at EOF. Pause/Resume preserves the playhead and restarts the decoder at that position, while the audio tail continues if the video decoder finishes first. Both FFmpeg subprocesses are stopped on seeking, changing files, or exiting. Linux builds require ALSA development headers.
- Embedded audio/video synchronization is **experimental** and uses independent FFmpeg decoders with approximate timestamps, not exact source PTS; device latency and variable-frame-rate clips may drift. FFplay remains the reliable synchronized A/V alternative. A proper shared demux/decode clock, multitrack editing and frame-accurate editing remain future milestones.

### Video precision and color tags

For H.264/H.265/AV1/VP9, the codec pane now offers an encoder-default or explicit 8-bit 4:2:0 pixel format. **Software H.265 and AV1** additionally offer 10-bit 4:2:0 (HEVC Main10). The color signal options are automatic, BT.709 SDR, BT.2020 PQ/HDR10 and BT.2020 HLG. HDR metadata choices require 10-bit H.265/AV1, and H.265 writes the corresponding x265 VUI tags. These are **metadata only, not tone mapping, grading, gamut conversion or SDR-to-HDR**: select HDR signaling only for source footage already mastered in the specified transfer. The program does not guarantee HDR10 static mastering metadata (MaxCLL/MaxFALL) yet. Hardware 10-bit output is disabled until hardware-specific compatibility checks are implemented.

### Genuine two-pass HEVC (x265)

H.265/HEVC now supports two-pass target-bitrate software encoding. It uses **x265-native `pass` and `stats` parameters**, rather than relying on FFmpeg's generic `-passlogfile`, which can return success without producing usable x265 statistics. Both passes run in an isolated per-job working directory; relative input/output paths are resolved to absolute paths; pass-one stats must exist and be nonempty. The directory is cleaned on success, error, or cancellation. This can be combined with Main10 output and explicit Rec.709/PQ/HLG signaling. Full HDR10 mastering metadata and true tone-mapping remain unimplemented.

**Color metadata reliability:** FFmpeg generic color switches may not populate AV1 sequence metadata in every SVT build. Explicit AV1 color signal selection now sends matching `-svtav1-params` color primaries, transfer and matrix; x265 uses native VUI. Integration tests verify these metadata fields with FFprobe. Selecting an HDR tag on SDR footage remains a potentially incorrect retag, NOT a color transform; do not use HDR tags unless the input is already HDR-mastered.
