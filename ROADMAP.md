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
- [x] Audio-only embedded playback, initial PCM prebuffer, and full-length short PCM tail handling
- [x] Playback Pause/Resume with playhead preservation; allow audio to finish after video EOF
- [x] Audio-clock frame staging without overwriting the next frame before presentation
- [x] PCM sample-count playhead ignores decoder-underrun silence (output-device latency still needs calibration)
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

## Usability and performance work
- [x] Responsive wide-screen queue / preview / options layout inspired by Shutter Encoder, plus searchable compact queues and grouped functions
- [x] CRF or target video bitrate, peak bitrate, codec speed tiers, thread/GOP controls and faststart preference
- [x] Adjustable pane widths persist across launches; a reset control restores defaults
- [x] Virtualized compact file list renders only visible rows and supports 10,000-file search regression testing
- [ ] Measure frame rendering time and memory with thousands of actual probed files, preserve keyboard focus across pane resizing
- [ ] True two-pass encoding, encoder capability detection, measurable speed benchmarks and a media corpus

- [x] FFmpeg two-pass H.264/VP9 target bitrate with cancellable passes, isolated stats and cleanup
- [ ] Extend tested two-pass support to HEVC and AV1 via encoder-specific pass handling

- [x] Background hardware encoder probing with actual one-frame encoding tests for NVENC/QSV/AMF/VideoToolbox H.264/HEVC

- [x] 8/10-bit software H.265/AV1 pixel format control and tagged Rec.709/BT.2020 PQ/HLG output
- [ ] Source-aware HDR metadata propagation; HDR→SDR tone mapping and LUTs; mastering-display/MaxCLL/MaxFALL; 10-bit GPU capability verification
