# Rust Encoder feature-parity roadmap

See [FEATURE_PARITY.md](FEATURE_PARITY.md) for a more granular list. A checked entry means source implementation exists, **not** that independent full-workflow qualification has passed.

## Encoding foundation
- [x] Rust GUI, queued jobs, portable cross-platform CI builds
- [x] Asynchronous metadata/progress, status, error reporting, cancellation
- [x] 19 FFmpeg-backed functions, including rewrap, audio extraction, still extraction, subtitles and MD5
- [x] Seekable still-frame previews; track language/channel metadata
- [x] Trim, resize, crop, rotate, flips, output FPS, one-pass normalization, stream selection
- [x] Manual H.264/H.265 GPU encoder selections; saved JSON profiles
- [ ] Verified UI workflow at multiple resolutions, DPI and real OS themes
- [ ] Color space, 10/12-bit, HDR metadata, LUTs, burn-in, watermark and deinterlacing
- [ ] GPU capability detection, parameter compatibility/fallback and 2-pass normalization
- [ ] More codecs, containers, formats, professional profiles, caption workflows

## Editing, analysis and specialist tools
- [ ] Native video player, synchronized waveform and timeline, multi-point editing
- [ ] Frame-accurate cuts, EDL/XML/project interchange, retiming and merge pipelines
- [ ] Black/silence/offline detection, full loudness analysis, VMAF and logs
- [ ] Audio track replacement, merging, separation and multi-track control
- [ ] Image sequences, still batch, RAW/EXIF, professional image workflows
- [ ] Download integrations, transcription/translation and AI imaging utilities

## Release quality
- [x] Win/macOS/Linux build, tests, release artifact packaging and executable startup check
- [x] FFmpeg integration test for representative basic operations (Linux CI)
- [ ] Integration test corpus across supported formats, codecs, languages, GPUs and edge cases
- [ ] Bundled dependency options, offline packaging, native installers and code signing/notarization
- [ ] Crash recovery, queue persistence, updates, keyboard accessibility, localization
