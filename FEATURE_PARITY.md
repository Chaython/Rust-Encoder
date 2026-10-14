# Rust Encoder compatibility / parity inventory

This matrix is an **acceptance-tracking document**. Existing controls are not proof of parity. Any missing item means the rewrite is not a 1:1 substitute for a production post-production application.

| Area | Current Rust implementation | Important remaining work |
| --- | --- | --- |
| Video encoding | H.264, H.265, AV1, VP9, ProRes, DNxHR; CRF/basic profiles | Broad profiles, hardware probing, bitrate/two-pass options, HDR/color, more codec variants |
| Rewrap / lossless | MKV full copy, MP4 A/V copy, video-only copy, start/end trim | Multi-cut EDL, precise boundaries, metadata/track editor, more containers |
| Audio | WAV, FLAC, MP3, AAC, Opus, one-pass loudnorm | Track replacement, channel remix/downmix, split, separation, accurate two-pass loudness |
| Stream handling | Video/audio track indices, subtitle index/convert/extract; FFprobe metadata | Multi-language subtitle editing, external subs, burn-in, attachment editor, batch track routing |
| Video filters | Resize, crop, flip, rotate, output FPS | Interpolation, VFR/CFR management, LUT/color grading, deinterlacing, overlays, advanced filters |
| Image processing | Single PNG/JPEG frame extraction, seek position preview | Whole image-sequence workflows, RAW conversion, EXIF/meta, image batch editor |
| Detection and analysis | Frame MD5, metadata inspection | Black/freeze/silence/offline detection, loudness graph, VMAF, reports |
| Native UI | File queue, themes, metadata/preview, filters, persisted settings/profiles | Real-time player, timeline/waveform, docking, richer per-job editors, keyboard/accessibility |
| Automation | Batch queue, cancel/retry, JSON profiles | Watch folders, command jobs, network upload, job recovery, schedule, scripting |
| AI / utility tools | Not implemented | Upscaling, background removal, transcript/translation, yt-dlp, specialist utilities |
| Packaging | CI portable ZIPs, no JVM/Electron | Bundled codecs under compliant licenses, installer, signing, notarization, application tests |

Parity requires a feature-by-feature acceptance test (including negative/error paths), real input corpus and qualified cross-platform builds. Hardware/transcoding results depend on the FFmpeg build and GPU drivers.
