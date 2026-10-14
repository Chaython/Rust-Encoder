# Rust Encoder feature-parity roadmap

See [FEATURE_PARITY.md](FEATURE_PARITY.md) for a more granular list. A checked entry means source implementation exists, **not** that independent full-workflow qualification has passed.

## Encoding foundation
- [x] Rust GUI, queued jobs, portable cross-platform CI builds
- [x] Asynchronous metadata/progress, status, error reporting, cancellation
- [x] 23 FFmpeg-backed functions, including rewrap, audio extraction, still extraction, subtitles, MD5 and media diagnostics
- [x] Seekable still-frame previews; track language/channel metadata
- [x] Trim, resize, crop, rotate, flips, output FPS, one-pass normalization, stream selection
- [x] YADIF deinterlace, HQDN3D denoise, sharpen, basic color adjustments; mono/stereo/sample-rate conversion and audio gain
- [x] Manual H.264/H.265 GPU encoder selections; saved JSON profiles
- [ ] Verified UI workflow at multiple resolutions, DPI and real OS themes
- [ ] Color space, 10/12-bit, HDR metadata, LUTs, subtitle burn-in, watermark, advanced deinterlace cadence
- [ ] GPU capability detection, parameter compatibility/fallback and 2-pass normalization
- [ ] More codecs, containers, formats, professional profiles, caption workflows

## Editing, analysis and specialist tools
- [x] Initial interactive timeline: seeking, In/Out marks, frame stepping and sampled video preview with first-audio waveform
- [x] Optional managed FFplay companion for synchronized audio/video review from a timeline position and Out point
- [x] Continuous 24 FPS embedded video with bounded decoder
- [x] Experimental native stereo PCM audio output via rodio/CPAL, approximate audio-master playback clock and volume
- [ ] Verified presentation-time-aware native A/V synchronization, multi-track waveforms and multi-point editing
- [ ] Frame-accurate cuts, EDL/XML/project interchange, retiming and merge pipelines
- [ ] Advanced detection thresholds, offline detection, VMAF and comparative analysis
- [ ] Audio track replacement, merging, separation and multi-track control
- [ ] Image sequences, still batch, RAW/EXIF, professional image workflows
- [ ] Download integrations, transcription/translation and AI imaging utilities

## Release quality
- [x] Win/macOS/Linux build, tests, release artifact packaging and executable startup check
- [x] FFmpeg integration test for representative basic operations (Linux CI)
- [ ] Integration test corpus across supported formats, codecs, languages, GPUs and edge cases
- [ ] Bundled dependency options, offline packaging, native installers and code signing/notarization
- [ ] Crash recovery, queue persistence, updates, keyboard accessibility, localization
