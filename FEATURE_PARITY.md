# Rust Encoder compatibility / parity inventory

This matrix is an **acceptance-tracking document**. Existing controls are not proof of parity. Any missing item means the rewrite is not a 1:1 substitute for a production post-production application.

| Area | Current Rust implementation | Important remaining work |
| --- | --- | --- |
| Video encoding | H.264, H.265, AV1, VP9, ProRes, DNxHR; CRF and target bitrate, peak bitrate, speed tiers, thread and GOP tuning, MP4 faststart | Two-pass encoding, advanced codec profiles, hardware probing, HDR and color management |
| Rewrap / lossless | MKV full copy, MP4 A/V copy, video-only copy, start/end trim | Multi-cut EDL, precise boundaries, metadata/track editor, more containers |
| Audio | WAV, FLAC, MP3, AAC, Opus, one-pass loudnorm, mono/stereo conversion, sample-rate conversion and gain | Track replacement, channel-by-channel remix, split, separation, accurate two-pass loudness |
| Stream handling | Video/audio track indices, subtitle index/convert/extract; FFprobe metadata | Multi-language subtitle editing, external subs, burn-in, attachment editor, batch track routing |
| Video filters | Resize, crop, flip, rotate, output FPS, YADIF deinterlacing, HQDN3D denoising, sharpen, brightness/contrast/saturation/gamma | Frame interpolation, VFR/CFR management, LUT/advanced color grading, overlays and watermark filters |
| Image processing | Single PNG/JPEG frame extraction, seek position preview | Whole image-sequence workflows, RAW conversion, EXIF/meta, image batch editor |
| Detection and analysis | Frame MD5, black/freeze/silence detection, EBU R128 measurement, metadata inspection | Custom detection thresholds, offline detection, detailed loudness graph, VMAF, richer analysis reports |
| Native UI | Three-pane responsive workspace, searchable compact queue, grouped presets, themes, metadata/preview, basic interactive trim timeline, frame-step, silent 4 FPS sampled preview, continuously decoded 24 FPS embedded video and experimental native PCM audio and audio-only playback with prebuffering, EOF-safe decoding, Pause/Resume and independent audio tail completion, optional managed FFplay synchronized A/V companion window, first-audio waveform, filters, persisted settings/profiles | Verified native A/V synchronization from actual presentation timestamps, zero-drift clocks and precise audio/video seeking, multitrack waveform editor, multi-cut EDL, docking, richer per-job editors, keyboard/accessibility |
| Automation | Batch queue, cancel/retry, JSON profiles | Watch folders, command jobs, network upload, job recovery, schedule, scripting |
| AI / utility tools | Not implemented | Upscaling, background removal, transcript/translation, yt-dlp, specialist utilities |
| Packaging | CI portable ZIPs, no JVM/Electron | Bundled codecs under compliant licenses, installer, signing, notarization, application tests |

Parity requires a feature-by-feature acceptance test (including negative/error paths), real input corpus and qualified cross-platform builds. Hardware/transcoding results depend on the FFmpeg build and GPU drivers.

- 10-bit software HEVC Main10 and AV1 output with selectable BT.709, BT.2020 PQ or HLG color signaling (**tagging only**, not actual gamut conversion or HDR mastering/tone mapping). 8-bit/default choices remain available.

- Two-pass HEVC H.265 software encoding with verified native x265 analysis statistics and isolated cleanup; Main10 output plus Rec.709 tags verified with FFprobe.
