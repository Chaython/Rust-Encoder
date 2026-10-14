# Rust Encoder

A native Rust desktop media converter and batch-processing application, powered by external FFmpeg and FFprobe executables. The application is written in Rust using `eframe`/`egui` and does not use Electron, a WebView, or a JVM.

> **Development status:** Early implementation. The full editing, advanced processing, and professional media workflows described in the roadmap are not yet available. Desktop builds and GUI behavior must be verified on real platforms.

## Features available in source

- Responsive native GUI with dark/light mode, drag-and-drop, file picker, scrollable queue/settings, and asynchronous still-frame preview.
- Background `ffprobe` media inspection and nonblocking `ffmpeg` encoding with progress, errors, and cancellation.
- Batch queue with retry, removal, and pause-after-current behavior. Pending jobs follow current settings; running jobs keep their own settings.
- 11 initial presets: H.264, H.265, AV1, VP9, ProRes, DNxHR, MKV remux, WAV, FLAC, MP3, and AAC.
- Basic quality, bitrate, resize, trim, audio normalization, output directory, and codec tool-path settings.
- Collision-safe output naming and temporary output files during encoding.

**Dependencies:** Rust toolchain 1.95 or newer for building; FFmpeg and FFprobe must be installed separately, in `PATH`, placed next to the application binary, or configured in the GUI. The release archives intentionally do not bundle FFmpeg. Verify any third-party encoder and codec licensing before redistributing them.

## Build locally

```sh
cargo build --release
cargo test --all-targets
```

Run from the source tree with `cargo run --release`. The executable will be `target/release/rust-encoder` (or `rust-encoder.exe` on Windows).

On Ubuntu/Debian the native GUI dependencies may require:

```sh
sudo apt-get update
sudo apt-get install -y libgtk-3-dev libxkbcommon-dev libwayland-dev libxcb-shape0-dev libxcb-xfixes0-dev libxcb-render0-dev libssl-dev pkg-config
```

## GitHub Actions

The [desktop build workflow](.github/workflows/build.yml) runs formatting, `cargo check`, tests, Clippy, and release compilation on Windows, macOS, and Linux. It publishes **portable build artifacts** for each platform on successful runs. It runs on pushes to `main`, pull requests and manual dispatch. Git tags matching `v*` also trigger builds. These downloadable archives contain the application and this README/license; FFmpeg remains a separate installation.

Build artifacts are not automatically signed, notarized, packaged as installers, or published as GitHub Releases. A passing CI compile is not proof of real-GUI feature coverage; review [TESTING.md](TESTING.md) for manual tests.

## Media output behavior

Imported files can be selected first, and conversion presets adjusted afterward. `ffprobe` inspects the source on a worker thread. Start queue processes jobs sequentially. Output uses a suffix such as `_h264` beside source unless an alternate output folder is chosen. Existing target files are not intentionally replaced.

Remuxing streams without re-encoding requires compatible containers and generally has keyframe-aligned rather than frame-accurate cutting. Some presets do not preserve subtitle and attachment tracks. Basic normalization is single-pass and not studio-verified loudness matching. Available encoders depend on your FFmpeg build.

## Planned work

Full video playback and scrubbing; waveform/timeline editing; per-stream audio/subtitle controls; GPU encoder detection; HDR/10-bit profiles; crop/pad/LUT/overlay; multitrack and two-pass normalization; image sequences; RAW and metadata utilities; additional filters, installers, update checks, and cross-platform accessibility testing. See [ROADMAP.md](ROADMAP.md).

## License

The Rust source in this repository is provided under **GPL-3.0-or-later**; see [LICENSE](LICENSE). It is a standalone application, not affiliated with any external encoder GUI product. FFmpeg and any other optional third-party binaries have independent licensing and distribution requirements.
