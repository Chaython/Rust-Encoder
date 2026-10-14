use serde::Deserialize;
use std::{path::{Path, PathBuf}, process::Command};

#[derive(Clone, Debug, Default)]
pub struct MediaInfo {
    pub duration_sec: Option<f64>,
    pub video_codec: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub audio_codec: Option<String>,
    pub audio_channels: Option<u32>,
}

impl MediaInfo {
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if let (Some(w), Some(h)) = (self.width, self.height) {
            parts.push(format!("{w}×{h}"));
        }
        if let Some(codec) = &self.video_codec { parts.push(codec.to_uppercase()); }
        if let Some(codec) = &self.audio_codec { parts.push(codec.to_uppercase()); }
        if let Some(duration) = self.duration_sec { parts.push(format!("{duration:.1}s")); }
        parts.join(" · ")
    }
}

#[derive(Deserialize)]
struct ProbeOutput { format: Option<ProbeFormat>, #[serde(default)] streams: Vec<ProbeStream> }
#[derive(Deserialize)]
struct ProbeFormat { duration: Option<String> }
#[derive(Deserialize)]
struct ProbeStream {
    codec_type: Option<String>, codec_name: Option<String>,
    width: Option<u32>, height: Option<u32>, channels: Option<u32>, duration: Option<String>,
}

pub fn probe_file(binary: &Path, path: &Path) -> Result<MediaInfo, String> {
    if !path.is_file() { return Err("Input file does not exist".into()); }
    let output = Command::new(binary)
        .args(["-v", "error", "-show_format", "-show_streams", "-of", "json"])
        .arg(path).output().map_err(|e| format!("Could not start {}: {e}", binary.display()))?;
    if !output.status.success() {
        return Err(format!("ffprobe: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    parse_probe(&output.stdout)
}

fn parse_probe(data: &[u8]) -> Result<MediaInfo, String> {
    let parsed: ProbeOutput = serde_json::from_slice(data).map_err(|e| format!("Invalid ffprobe output: {e}"))?;
    let mut info = MediaInfo { duration_sec: parsed.format.and_then(|f| f.duration).and_then(|s| s.parse::<f64>().ok()), ..Default::default() };
    for stream in parsed.streams {
        match stream.codec_type.as_deref() {
            Some("video") if info.video_codec.is_none() => {
                info.video_codec = stream.codec_name;
                info.width = stream.width; info.height = stream.height;
                if info.duration_sec.is_none() {
                    info.duration_sec = stream.duration.and_then(|s| s.parse::<f64>().ok());
                }
            }
            Some("audio") if info.audio_codec.is_none() => {
                info.audio_codec = stream.codec_name;
                info.audio_channels = stream.channels;
            }
            _ => {}
        }
    }
    info.duration_sec = info.duration_sec.filter(|v| v.is_finite() && *v > 0.0);
    Ok(info)
}

pub fn probe_async(id: u64, tool: PathBuf, path: PathBuf, sender: crossbeam_channel::Sender<crate::encode::Event>) {
    std::thread::spawn(move || {
        let result = probe_file(&tool, &path);
        let _ = sender.send(crate::encode::Event::Probed { id, result });
    });
}

/// A decoded RGBA thumbnail, sent from a worker without touching the GUI thread.
#[derive(Debug)]
pub struct Thumbnail {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

pub fn thumbnail_file(ffmpeg: &Path, path: &Path) -> Result<Thumbnail, String> {
    if !path.is_file() { return Err("Input file not found".into()); }
    let output = Command::new(ffmpeg)
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-map", "0:v:0", "-frames:v", "1", "-vf", "scale=640:-2", "-f", "image2pipe", "-vcodec", "png", "pipe:1"])
        .output()
        .map_err(|e| format!("Could not start preview generator: {e}"))?;
    if !output.status.success() {
        return Err(format!("Preview unavailable: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    let img = image::load_from_memory_with_format(&output.stdout, image::ImageFormat::Png)
        .map_err(|e| format!("Could not decode preview: {e}"))?.to_rgba8();
    let (width, height) = img.dimensions();
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return Err("Preview image dimensions are invalid".into());
    }
    Ok(Thumbnail { width: width as usize, height: height as usize, rgba: img.into_raw() })
}

pub fn thumbnail_async(id: u64, ffmpeg: PathBuf, path: PathBuf, sender: crossbeam_channel::Sender<crate::encode::Event>) {
    std::thread::spawn(move || {
        let result = thumbnail_file(&ffmpeg, &path);
        let _ = sender.send(crate::encode::Event::PreviewReady { id, result });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_typical_ffprobe_json() {
        let json = br#"{"format":{"duration":"12.5"},"streams":[{"codec_type":"video","codec_name":"h264","width":1920,"height":1080},{"codec_type":"audio","codec_name":"aac","channels":2}]}"#;
        let info = parse_probe(json).unwrap();
        assert_eq!(info.duration_sec, Some(12.5));
        assert_eq!(info.width, Some(1920));
        assert_eq!(info.audio_codec.as_deref(), Some("aac"));
    }
    #[test]
    fn missing_duration_is_valid() {
        let info = parse_probe(br#"{"streams":[]}"#).unwrap();
        assert!(info.duration_sec.is_none());
    }
}
