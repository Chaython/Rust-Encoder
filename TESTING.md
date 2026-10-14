# Rust Encoder test plan

## Automated coverage

- Unit tests cover settings migration/JSON, FFprobe parsing, preset mapping, argument validation, progress interpretation, collision avoidance and layout root rendering.
- FFmpeg integration tests generate a 1-second sample A/V clip using lavfi. They exercise H.264 with crop/rotation/fps; MKV/MP4 rewrap; video-only extraction; WAV/FLAC/MP3/AAC; still PNG/JPEG extraction; frame MD5. FFmpeg is installed explicitly in Linux CI. Tests skip gracefully if FFmpeg is absent on a local machine.
- GitHub Actions builds Windows/Linux/macOS, tests all targets, runs clippy, packages portable ZIPs and executes the compiled binary with `--version`.
- Synthetic tests do **not** establish codec, platform, professional video or GUI feature parity.

## Required manual desktop acceptance

1. Run at 740×550, 1040×780 and 1440×900, with Windows scaling 100/150/200%, both themes; confirm independent scroll and accessible queue buttons.
2. Drag/drop Unicode or special-character paths; test multiple video, audio-only and damaged files; select files while probes or previews are running.
3. Seek still-frame preview repeatedly, select another file during processing, verify obsolete preview events are discarded.
4. Change a preset while jobs are queued vs running; load/save a profile; verify persisted controls on restart.
5. Exercise start/pause-after-current/cancel/retry on long media; close the GUI while encoding; inspect child FFmpeg processes and temp output cleanup.
6. Test multi-video, multi-audio, subtitle tracks, language tags, existing output path collision, metadata, destination permissions, disk-full and race conditions.
7. Test each hardware backend with supported GPU/driver and actual device, including missing/unsupported encoder error handling.
8. Compare transcodes (duration, stream mapping, frames, audio loudness, color fidelity) against input with FFprobe, perceptual checks and frame hashes.
9. Verify remote/UNC paths, symlinks, long paths, Unicode and non-ASCII filenames on native Windows/macOS/Linux.
10. Verify portable archives on machines **without** developer tools; test executable and external FFmpeg discovery.

## Known limitations

No realtime video player, synchronized waveform/editor timeline, bulk image sequences, complete HDR/color/subtitle workflows, AI/media utility integrations, or full original feature equivalence. A one-pass `loudnorm` filter does not guarantee a measured integrated loudness target. Hardware encoders require compatible FFmpeg binaries and may not support software encoder settings.
