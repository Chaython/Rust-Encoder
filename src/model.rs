use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Preset {
    H264,
    H265,
    Av1,
    Vp9,
    ProRes,
    Dnxhr,
    RemuxMkv,
    Wav,
    Flac,
    Mp3,
    Aac,
}

impl Preset {
    pub const ALL: [Preset; 11] = [
        Self::H264, Self::H265, Self::Av1, Self::Vp9, Self::ProRes,
        Self::Dnxhr, Self::RemuxMkv, Self::Wav, Self::Flac, Self::Mp3, Self::Aac,
    ];
    pub const fn label(self) -> &'static str {
        match self {
            Self::H264 => "H.264 / MP4", Self::H265 => "H.265 / MP4",
            Self::Av1 => "AV1 / MKV", Self::Vp9 => "VP9 / WebM",
            Self::ProRes => "ProRes / MOV", Self::Dnxhr => "DNxHR / MOV",
            Self::RemuxMkv => "Rewrap / MKV (no encoding)",
            Self::Wav => "Audio / WAV", Self::Flac => "Audio / FLAC",
            Self::Mp3 => "Audio / MP3", Self::Aac => "Audio / AAC (M4A)",
        }
    }
    pub const fn extension(self) -> &'static str {
        match self {
            Self::H264 | Self::H265 => "mp4", Self::Av1 | Self::RemuxMkv => "mkv",
            Self::Vp9 => "webm", Self::ProRes | Self::Dnxhr => "mov",
            Self::Wav => "wav", Self::Flac => "flac", Self::Mp3 => "mp3", Self::Aac => "m4a",
        }
    }
    pub const fn video(self) -> bool {
        matches!(self, Self::H264 | Self::H265 | Self::Av1 | Self::Vp9 | Self::ProRes | Self::Dnxhr)
    }
    pub const fn audio_only(self) -> bool {
        matches!(self, Self::Wav | Self::Flac | Self::Mp3 | Self::Aac)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub preset: Preset,
    pub quality: u8,
    pub target_width: u32,
    pub audio_bitrate_kbps: u16,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub normalize_audio: bool,
    pub output_dir: String,
    pub ffmpeg_path: String,
    pub ffprobe_path: String,
    pub dark_theme: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            preset: Preset::H264, quality: 23, target_width: 0,
            audio_bitrate_kbps: 192, start_seconds: 0.0, end_seconds: 0.0,
            normalize_audio: false, output_dir: String::new(),
            ffmpeg_path: String::new(), ffprobe_path: String::new(), dark_theme: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct EncodeOptions {
    pub preset: Preset,
    pub quality: u8,
    pub target_width: u32,
    pub audio_bitrate_kbps: u16,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub normalize_audio: bool,
    pub output_dir: Option<PathBuf>,
    pub ffmpeg: PathBuf,
}

impl Settings {
    pub fn encode_options(&self) -> EncodeOptions {
        EncodeOptions {
            preset: self.preset,
            quality: self.quality, target_width: self.target_width,
            audio_bitrate_kbps: self.audio_bitrate_kbps,
            start_seconds: self.start_seconds, end_seconds: self.end_seconds,
            normalize_audio: self.normalize_audio,
            output_dir: (!self.output_dir.is_empty()).then(|| PathBuf::from(&self.output_dir)),
            ffmpeg: resolve_tool("ffmpeg", &self.ffmpeg_path),
        }
    }
}

/// Use an explicit path first, then bundled tool adjacent to executable, then PATH.
pub fn resolve_tool(name: &str, configured: &str) -> PathBuf {
    if !configured.trim().is_empty() {
        return PathBuf::from(configured);
    }
    let executable = if cfg!(windows) { format!("{name}.exe") } else { name.to_owned() };
    if let Ok(parent) = std::env::current_exe()
        .and_then(|exe| exe.parent().map(Path::to_path_buf).ok_or(std::io::ErrorKind::NotFound.into()))
    {
        let bundled = parent.join(&executable);
        if bundled.is_file() {
            return bundled;
        }
    }
    PathBuf::from(executable)
}

pub fn settings_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("rust-encoder").join("settings.json"))
}

pub fn load_settings() -> Settings {
    let Some(path) = settings_path() else { return Settings::default() };
    std::fs::read_to_string(path).ok()
        .and_then(|contents| serde_json::from_str(&contents).ok()).unwrap_or_default()
}

pub fn save_settings(settings: &Settings) -> std::io::Result<()> {
    let path = settings_path().ok_or(std::io::Error::other("config directory unavailable"))?;
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
    let content = serde_json::to_vec_pretty(settings).map_err(std::io::Error::other)?;
    // A failed write never truncates the last good settings file.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, content)?;
    #[cfg(windows)]
    if path.exists() { std::fs::remove_file(&path)?; }
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn each_preset_has_extension_and_label() {
        for preset in Preset::ALL {
            assert!(!preset.extension().is_empty());
            assert!(!preset.label().is_empty());
        }
    }
    #[test]
    fn settings_json_roundtrip() {
        let mut a = Settings::default();
        a.preset = Preset::Flac;
        a.target_width = 1280;
        let b: Settings = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(b.preset, Preset::Flac);
        assert_eq!(b.target_width, 1280);
    }
}
