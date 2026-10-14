# Rust Encoder development roadmap

This is a feature roadmap, not a claim of existing functionality.

## Desktop foundation
- [x] Responsive GUI, file list, encoder controls, theme, and still-frame preview (source implementation)
- [x] Sequential job queue, nonblocking probe and progress/cancel events (source implementation)
- [x] 11 basic codec/stream-copy presets (FFmpeg command smoke-tested)
- [ ] Verified build, GUI lifecycle, and media end-to-end tests on Windows/macOS/Linux
- [ ] Real preview player, audio waveform, timeline, precise cuts, video-audio sync

## Professional encoding
- [ ] Enumerate installed FFmpeg encoders, hardware GPUs and codec capabilities
- [ ] HDR color management, 10/12-bit, ProRes/DNxHR profile validation
- [ ] Stream mapping for subtitles, attachment metadata and multiple audio tracks
- [ ] GPU acceleration selection/fallback; two-pass audio normalization
- [ ] Cropping, denoise, LUT, subtitles, watermark and overlays

## Packaging and quality
- [x] Cross-platform source builds and CI artifact packaging workflow
- [ ] Integration test corpus for mixed-media formats and Unicode paths
- [ ] Code signing, notarization, native installers and reproducible releases
- [ ] Fully offline/portable mode with dependency guidance
- [ ] Crash recovery, localization, keyboard accessibility
