# Validation and regression notes

This project is a new Rust application with an evolving GUI and feature set.

## Source-level / media-command checks

- Initial encoding command patterns were exercised with synthetic FFmpeg media for 11 presets (H.264/H.265/AV1/VP9/ProRes/DNxHR/MKV remux/WAV/FLAC/MP3/AAC) on the source-authoring environment.
- FFmpeg still-frame PNG extraction for a short test MP4 was also exercised.
- The authoring environment did not provide a Rust toolchain; **no successful Rust build or real GUI launch is claimed** by these checks.
- GitHub Actions should build and test `main` on Windows, Linux and macOS; any failures must be fixed before distributing binaries.

## GUI acceptance tests

1. Check 760×600, 1040×780 and 1440×900 windows, including high DPI, for accessible controls and scrolling.
2. Test drag/drop and file picker with audio-only/video, corrupt files, Unicode paths and short clips.
3. Change presets after adding files; verify only queued files change and active encoding stays unchanged.
4. Start/pause/cancel/retry/remove queue items; ensure no zombie media subprocesses or overwritten files.
5. Check selected-file previews, error messages, missing FFmpeg/FFprobe guidance and theme/settings persistence.
6. Validate codecs, mixed streams, subtitles/metadata, quality and A/V sync against reference media; smoke commands are not enough.
7. Verify portable archives on fresh Windows/macOS/Linux machines with FFmpeg placed beside the binary or in PATH.

## Known missing GUI features

Still preview exists; real-time video playback, scrubbable timeline, waveforms, advanced per-stream settings and most professional editing controls are **not implemented**.
