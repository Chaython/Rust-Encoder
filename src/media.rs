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
    pub video_frame_rate: Option<f64>,
    pub video_tracks: Vec<MediaTrack>,
    pub audio_tracks: Vec<MediaTrack>,
    pub subtitle_tracks: Vec<MediaTrack>,
}

#[derive(Clone, Debug, Default)]
pub struct MediaTrack {
    pub codec: String,
    pub language: Option<String>,
    pub title: Option<String>,
    pub channels: Option<u32>,
}

impl MediaTrack {
    pub fn description(&self, index: usize) -> String {
        let mut s = format!("#{} {}", index, self.codec);
        if let Some(lang) = &self.language { s.push_str(&format!(" [{lang}]")); }
        if let Some(ch) = self.channels { s.push_str(&format!(" {ch}ch")); }
        if let Some(title) = &self.title { s.push_str(&format!(" — {title}")); }
        s
    }
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
    avg_frame_rate: Option<String>,
    tags: Option<ProbeTags>,
}

#[derive(Deserialize)]
struct ProbeTags { language: Option<String>, title: Option<String> }

/// Parse the rational frame rate reported by ffprobe (e.g. 30000/1001).
fn parse_frame_rate(value: &str) -> Option<f64> {
    let rate = if let Some((num, den)) = value.split_once('/') {
        let numerator = num.parse::<f64>().ok()?;
        let denominator = den.parse::<f64>().ok()?;
        numerator / denominator
    } else {
        value.parse::<f64>().ok()?
    };
    (rate.is_finite() && (1.0..=240.0).contains(&rate)).then_some(rate)
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
        let track = MediaTrack {
            codec: stream.codec_name.clone().unwrap_or_else(|| "unknown".into()),
            language: stream.tags.as_ref().and_then(|t| t.language.clone()),
            title: stream.tags.as_ref().and_then(|t| t.title.clone()),
            channels: stream.channels,
        };
        match stream.codec_type.as_deref() {
            Some("video") => info.video_tracks.push(track),
            Some("audio") => info.audio_tracks.push(track),
            Some("subtitle") => info.subtitle_tracks.push(track),
            _ => {}
        }
        match stream.codec_type.as_deref() {
            Some("video") if info.video_codec.is_none() => {
                info.video_codec = stream.codec_name;
                info.width = stream.width; info.height = stream.height;
                info.video_frame_rate = stream.avg_frame_rate.as_deref().and_then(parse_frame_rate);
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
    thumbnail_file_at(ffmpeg, path, 0.0)
}

pub fn thumbnail_file_at(ffmpeg: &Path, path: &Path, seek_seconds: f64) -> Result<Thumbnail, String> {
    if !seek_seconds.is_finite() || seek_seconds < 0.0 { return Err("Invalid thumbnail seek position".into()); }
    if !path.is_file() { return Err("Input file not found".into()); }
    let output = Command::new(ffmpeg)
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-ss"])
        .arg(format!("{seek_seconds:.3}"))
        .arg("-i").arg(path)
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

/// Render a compact all-audio waveform in the FFmpeg worker, never on the UI thread.
/// FFmpeg's showwavespic emits one bounded 900x120 PNG rather than raw PCM.
pub fn waveform_file(ffmpeg: &Path, path: &Path) -> Result<Thumbnail, String> {
    if !path.is_file() { return Err("Input file not found".into()); }
    let output = Command::new(ffmpeg)
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-filter_complex", "[0:a:0]showwavespic=s=900x120:colors=0x54b5e5",
            "-frames:v", "1", "-f", "image2pipe", "-vcodec", "png", "pipe:1"])
        .output()
        .map_err(|e| format!("Could not start waveform generator: {e}"))?;
    if !output.status.success() {
        return Err(format!("Waveform unavailable: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    let img = image::load_from_memory_with_format(&output.stdout, image::ImageFormat::Png)
        .map_err(|e| format!("Cannot decode waveform: {e}"))?.to_rgba8();
    let (width, height) = img.dimensions();
    if width != 900 || height != 120 { return Err("Unexpected waveform dimensions".into()); }
    Ok(Thumbnail { width: width as usize, height: height as usize, rgba: img.into_raw() })
}

pub fn waveform_async(id: u64, ffmpeg: PathBuf, path: PathBuf,
    sender: crossbeam_channel::Sender<crate::encode::Event>) {
    std::thread::spawn(move || {
        let result = waveform_file(&ffmpeg, &path);
        let _ = sender.send(crate::encode::Event::WaveformReady { id, result });
    });
}

pub fn thumbnail_async(id: u64, seek_ms: u64, ffmpeg: PathBuf, path: PathBuf, sender: crossbeam_channel::Sender<crate::encode::Event>) {
    std::thread::spawn(move || {
        let result = thumbnail_file_at(&ffmpeg, &path, seek_ms as f64 / 1000.0);
        let _ = sender.send(crate::encode::Event::PreviewReady { id, seek_ms, result });
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
        assert_eq!(info.video_tracks.len(), 1);
        assert_eq!(info.audio_tracks.len(), 1);
    }
    #[test]
    fn frame_rate_parser_guards_zero_and_invalid_rates() {
        assert_eq!(parse_frame_rate("30000/1001").unwrap().round(), 30.0);
        assert_eq!(parse_frame_rate("25"), Some(25.0));
        for rate in ["0/0", "1/0", "400/1", "NaN", "", "0"] {
            assert_eq!(parse_frame_rate(rate), None);
        }
    }
    #[test]
    fn missing_duration_is_valid() {
        let info = parse_probe(br#"{"streams":[]}"#).unwrap();
        assert!(info.duration_sec.is_none());
    }
}
