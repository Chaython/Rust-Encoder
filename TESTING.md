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

## Experimental native embedded audio playback
- Rust tests validate FFmpeg float PCM arguments, seek/trimming and selected audio-track routing; an additional test confirms the playback clock excludes underrun silence; a nonblocking audio Source test checks that the callback cannot stall waiting for FFmpeg data. Additional tests cover short final PCM blocks, malformed partial sample frames and EOF.
- Linux generated-media integration tests also validate FFmpeg decodes real audio into 48 kHz two-channel f32le samples. Hardware output is **not** tested in headless CI.
- Manual testing must cover Windows WASAPI, macOS CoreAudio, Linux ALSA/Pulse/PipeWire, volume, rapid seeking, device removal, video-only files, audio-only embedded playback, PCM files shorter than the prebuffer threshold, and variable-frame-rate A/V synchronization. Specifically verify Pause/Resume across the In/Out range, and audio finishing after the last video frame. In-app synchronization remains approximate; FFplay is the fallback.

## Playback clock queue and video/audio EOF handling
- Deterministic tests cover holding the first future video frame until its audio deadline and draining to the latest frame in video-only playback.
- When video decoding ends, audio playback is intentionally allowed to drain rather than forcibly being cut off. This and Pause/Resume require live desktop validation with real A/V output.

## Continuous embedded silent video decoding
- The Rust decoder module validates Unicode/odd-character arguments, invalid positions and trim bounds.
- Linux FFmpeg integration tests start a real 24 FPS rawvideo stream, verify the returned frame size and cancel without hanging.
- Manual inspection remains required for GPU texture changes, end-of-file, repeated seek/stop, responsiveness and native Windows/macOS codec installations.
- This is **video-only** playback. It is not synchronized native audio/video and frame timestamps for variable-frame-rate files are approximate.

## Optional synchronized A/V companion preview
- Pure Rust tests validate FFplay command arguments for Unicode/shell-metacharacter filenames, fractional seek and Out bounds, invalid/NaN times, no overwrite or shell invocation, and settings migration.
- Manual testing must verify FFplay actually opens and plays with sound on Windows/Linux/macOS, that separate windows exit on Stop, selection changes or application shutdown, and that missing FFplay is reported without freezing the GUI.
- FFplay is **not bundled** in CI ZIPs. This opens a separate synchronized playback window; the embedded Rust egui preview is still sampled and silent. CI compilation and argument tests do not test a live audio device or FFplay UI.

## Sampled preview and trim timeline

- Media integration tests create mixed video/audio FFmpeg fixtures, probe the source frame rate, extract a seeked frame and generate/parse the 900×120 waveform PNG.
- GUI tests check trimmed bounds (including invalid older settings) and preview panels without a window.
- The preview decodes **silent sampled frames** rather than replaying synchronized audio. Multiple rapid scrub requests are serialized per selection; stale results are discarded.
- Manual GUI checks remain essential for scrubbing, repeated switching between files, input focus, small-window layout, memory usage and longer videos.
