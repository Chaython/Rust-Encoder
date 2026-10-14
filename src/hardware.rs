//! Hardware encoding is a capability of the actual device *and* FFmpeg build.
//! A listed encoder is not necessarily usable; run a tiny real encode to null.
//! Device/driver probes run on an app worker thread, never the render thread.
use crate::model::{HardwareEncoder, Preset};
use crossbeam_channel::Sender;
use std::{
    path::{Path, PathBuf}, process::{Command, Stdio},
    thread, time::{Duration, Instant},
};

const ENCODERS: [HardwareEncoder; 4] = [
    HardwareEncoder::Nvenc, HardwareEncoder::Qsv,
    HardwareEncoder::Amf, HardwareEncoder::VideoToolbox,
];

#[derive(Clone, Debug, Default)]
pub struct Capabilities {
    pub h264: Vec<HardwareEncoder>,
    pub h265: Vec<HardwareEncoder>,
    pub ffmpeg: PathBuf,
}

impl Capabilities {
    pub fn supports(&self, encoder: HardwareEncoder, preset: Preset) -> bool {
        if encoder == HardwareEncoder::Software { return true; }
        match preset {
            Preset::H264 => self.h264.contains(&encoder),
            Preset::H265 => self.h265.contains(&encoder),
            _ => false,
        }
    }
}

fn available_encoders(output: &str) -> Vec<String> {
    output.lines().filter_map(|line| {
        let mut tokens = line.split_whitespace();
        let flags = tokens.next()?;
        let name = tokens.next()?;
        (flags.starts_with('V') && flags.len() >= 5).then_some(name.to_owned())
    }).collect()
}

/// Time-bounded smoke test: do not rely on -encoders alone (an FFmpeg binary
/// built with NVENC may be running on a machine without NVIDIA hardware).
fn usable_codec(ffmpeg: &Path, codec: &str) -> bool {
    let mut child = match Command::new(ffmpeg)
        .args(["-nostdin", "-hide_banner", "-loglevel", "error",
            "-f", "lavfi", "-i", "color=c=black:s=128x128:r=1:d=1",
            "-frames:v", "1", "-an", "-c:v", codec, "-f", "null", "-"])
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .spawn() {
        Ok(child) => child,
        Err(_) => return false,
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Err(_) => { let _ = child.kill(); let _ = child.wait(); return false; }
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
            Ok(None) => thread::sleep(Duration::from_millis(60)),
        }
    }
}

pub fn detect(ffmpeg: &Path) -> Result<Capabilities, String> {
    let output = Command::new(ffmpeg)
        .args(["-hide_banner", "-encoders"])
        .stdin(Stdio::null()).output()
        .map_err(|err| format!("Cannot inspect {}: {err}", ffmpeg.display()))?;
    if !output.status.success() {
        return Err(format!("FFmpeg failed to list encoders: {}", ffmpeg.display()));
    }
    let available = available_encoders(&String::from_utf8_lossy(&output.stdout));
    let mut result = Capabilities { ffmpeg: ffmpeg.to_owned(), ..Default::default() };
    for encoder in ENCODERS {
        for preset in [Preset::H264, Preset::H265] {
            let codec = encoder.codec(preset).expect("hardware H264/HEVC mapping");
            if available.iter().any(|name| name == codec) && usable_codec(ffmpeg, codec) {
                match preset {
                    Preset::H264 => result.h264.push(encoder),
                    Preset::H265 => result.h265.push(encoder),
                    _ => unreachable!(),
                }
            }
        }
    }
    Ok(result)
}

pub fn start_scan(ffmpeg: PathBuf, tx: Sender<Result<Capabilities, String>>) {
    thread::spawn(move || { let _ = tx.send(detect(&ffmpeg)); });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_encoder_list_excludes_decoders_and_audio() {
        let output = "Encoders:\n V..... h264_nvenc NVIDIA\n V..... libx264 H264\n A..... aac audio\n D..... bogus decoder\n";
        let parsed = available_encoders(output);
        assert!(parsed.contains(&"h264_nvenc".to_owned()));
        assert!(parsed.contains(&"libx264".to_owned()));
        assert!(!parsed.contains(&"aac".to_owned()));
        assert!(!parsed.contains(&"bogus".to_owned()));
    }
    #[test]
    fn a_report_only_supports_verified_hardware_per_codec() {
        let report = Capabilities {
            ffmpeg: PathBuf::from("ffmpeg"),
            h264: vec![HardwareEncoder::Nvenc],
            h265: vec![HardwareEncoder::Qsv],
        };
        assert!(report.supports(HardwareEncoder::Nvenc, Preset::H264));
        assert!(!report.supports(HardwareEncoder::Nvenc, Preset::H265));
        assert!(report.supports(HardwareEncoder::Qsv, Preset::H265));
        assert!(report.supports(HardwareEncoder::Software, Preset::H264));
    }
}
