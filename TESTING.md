# Rust Encoder test plan

## Automated coverage

- Unit tests cover settings migration/JSON, FFprobe parsing, preset mapping, argument validation, progress interpretation, collision avoidance and layout root rendering.
- FFmpeg integration tests generate a 1-second sample A/V clip using lavfi. They exercise H.264 with crop/rotation/fps; MKV/MP4 rewrap; video-only extraction; WAV/FLAC/MP3/AAC; still PNG/JPEG extraction; frame MD5; black/frozen/silent detection and EBU R128 loudness reports. FFmpeg is installed explicitly in Linux CI. Tests skip gracefully if FFmpeg is absent on a local machine.
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

## Additional color and audio parity regression coverage

- All advanced settings are serialized with backward-compatible defaults, and are snapshotted into pending encoding jobs.
- Argument tests assert exactly one `-vf`/`-af` chain for combined effects, reject non-finite controls, and reject video/audio processing on stream-copy presets.
- The Linux generated-media integration test now exercises YADIF, HQDN3D, unsharp and `eq` filters with H.264, including mono/48 kHz conversion and audio gain; FLAC additionally verifies resampling and gain.
- These are simple FFmpeg processing modes; color-managed HDR, LUTs, per-channel mixing and two-pass broadcast loudness are not implemented.

## Sampled preview and trim timeline

- Media integration tests create mixed video/audio FFmpeg fixtures, probe the source frame rate, extract a seeked frame and generate/parse the 900×120 waveform PNG.
- GUI tests check trimmed bounds (including invalid older settings) and preview panels without a window.
- The preview decodes **silent sampled frames** rather than replaying synchronized audio. Multiple rapid scrub requests are serialized per selection; stale results are discarded.
- Manual GUI checks remain essential for scrubbing, repeated switching between files, input focus, small-window layout, memory usage and longer videos.
