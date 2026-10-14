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
    Opus,
    RemuxMp4,
    VideoOnly,
    PngStill,
    JpegStill,
    ExtractSrt,
    ExtractAss,
    FrameMd5,
    BlackDetect,
    FreezeDetect,
    SilenceDetect,
    LoudnessReport,
}

impl Preset {
    pub const ALL: [Preset; 23] = [
        Self::H264, Self::H265, Self::Av1, Self::Vp9, Self::ProRes,
        Self::Dnxhr, Self::RemuxMkv, Self::RemuxMp4, Self::VideoOnly,
        Self::Wav, Self::Flac, Self::Mp3, Self::Aac, Self::Opus,
        Self::PngStill, Self::JpegStill, Self::ExtractSrt, Self::ExtractAss,
        Self::FrameMd5, Self::BlackDetect, Self::FreezeDetect,
        Self::SilenceDetect, Self::LoudnessReport,
    ];
    pub const fn label(self) -> &'static str {
        match self {
            Self::H264 => "H.264 / MP4", Self::H265 => "H.265 / MP4",
            Self::Av1 => "AV1 / MKV", Self::Vp9 => "VP9 / WebM",
            Self::ProRes => "ProRes / MOV", Self::Dnxhr => "DNxHR / MOV",
            Self::RemuxMkv => "Rewrap / MKV (stream copy)",
            Self::RemuxMp4 => "Rewrap / MP4 (stream copy)",
            Self::VideoOnly => "Extract video / MKV (stream copy)",
            Self::PngStill => "Extract still frame / PNG",
            Self::JpegStill => "Extract still frame / JPEG",
            Self::ExtractSrt => "Extract subtitles / SRT",
            Self::ExtractAss => "Extract subtitles / ASS",
            Self::FrameMd5 => "Frame checksum / MD5",
            Self::BlackDetect => "Detect black frames / TXT",
            Self::FreezeDetect => "Detect frozen video / TXT",
            Self::SilenceDetect => "Detect silent audio / TXT",
            Self::LoudnessReport => "Measure EBU R128 loudness / TXT",
            Self::Wav => "Audio / WAV", Self::Flac => "Audio / FLAC",
            Self::Mp3 => "Audio / MP3", Self::Aac => "Audio / AAC (M4A)",
            Self::Opus => "Audio / Opus",
        }
    }
    pub const fn extension(self) -> &'static str {
        match self {
            Self::H264 | Self::H265 => "mp4", Self::Av1 | Self::RemuxMkv => "mkv",
            Self::Vp9 => "webm", Self::ProRes | Self::Dnxhr => "mov",
            Self::RemuxMp4 => "mp4", Self::VideoOnly => "mkv",
            Self::PngStill => "png", Self::JpegStill => "jpg",
            Self::ExtractSrt => "srt", Self::ExtractAss => "ass", Self::FrameMd5 => "md5",
            Self::BlackDetect | Self::FreezeDetect | Self::SilenceDetect | Self::LoudnessReport => "txt",
            Self::Wav => "wav", Self::Flac => "flac", Self::Mp3 => "mp3", Self::Aac => "m4a", Self::Opus => "opus",
        }
    }
    pub const fn video(self) -> bool {
        matches!(self, Self::H264 | Self::H265 | Self::Av1 | Self::Vp9 | Self::ProRes | Self::Dnxhr)
    }
    pub const fn report(self) -> bool {
        matches!(self, Self::BlackDetect | Self::FreezeDetect | Self::SilenceDetect | Self::LoudnessReport)
    }
    pub const fn audio_only(self) -> bool {
        matches!(self, Self::Wav | Self::Flac | Self::Mp3 | Self::Aac | Self::Opus)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum HardwareEncoder {
    #[default]
    Software,
    Nvenc,
    Qsv,
    Amf,
    VideoToolbox,
}

impl HardwareEncoder {
    pub const ALL: [Self; 5] = [Self::Software, Self::Nvenc, Self::Qsv, Self::Amf, Self::VideoToolbox];
    pub const fn label(self) -> &'static str {
        match self {
            Self::Software => "CPU / Software",
            Self::Nvenc => "NVIDIA NVENC",
            Self::Qsv => "Intel Quick Sync",
            Self::Amf => "AMD AMF",
            Self::VideoToolbox => "Apple VideoToolbox",
        }
    }
    pub const fn codec(self, preset: Preset) -> Option<&'static str> {
        match (self, preset) {
            (Self::Nvenc, Preset::H264) => Some("h264_nvenc"),
            (Self::Nvenc, Preset::H265) => Some("hevc_nvenc"),
            (Self::Qsv, Preset::H264) => Some("h264_qsv"),
            (Self::Qsv, Preset::H265) => Some("hevc_qsv"),
            (Self::Amf, Preset::H264) => Some("h264_amf"),
            (Self::Amf, Preset::H265) => Some("hevc_amf"),
            (Self::VideoToolbox, Preset::H264) => Some("h264_videotoolbox"),
            (Self::VideoToolbox, Preset::H265) => Some("hevc_videotoolbox"),
            _ => None,
        }
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
    pub hardware_encoder: HardwareEncoder,
    pub frame_rate: u16, // 0 keeps the source frame rate
    pub crop_width: u32, // 0 disables crop
    pub crop_height: u32,
    pub crop_x: u32,
    pub crop_y: u32,
    pub rotate_degrees: u16, // 0/90/180/270
    pub flip_horizontal: bool,
    pub flip_vertical: bool,
    pub audio_stream: i32, // -1 all, -2 none, otherwise zero-based track index
    pub subtitle_stream: i32, // -1 none, otherwise zero-based track index
    pub video_stream: u32, // zero-based track index
    pub deinterlace: bool,
    pub denoise: bool,
    pub sharpen: bool,
    pub brightness: f32, // -1.0..1.0
    pub contrast: f32,   // 0.0..2.0
    pub saturation: f32, // 0.0..3.0
    pub gamma: f32,      // 0.1..4.0
    pub audio_channels: u8, // 0 source, 1 mono, 2 stereo
    pub audio_sample_rate: u32, // 0 original
    pub audio_gain_db: f32, // -30..30 dB
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
            normalize_audio: false,
            hardware_encoder: HardwareEncoder::Software, frame_rate: 0,
            crop_width: 0, crop_height: 0, crop_x: 0, crop_y: 0,
            rotate_degrees: 0, flip_horizontal: false, flip_vertical: false,
            audio_stream: -1, subtitle_stream: -1, video_stream: 0,
            deinterlace: false, denoise: false, sharpen: false,
            brightness: 0.0, contrast: 1.0, saturation: 1.0, gamma: 1.0,
            audio_channels: 0, audio_sample_rate: 0, audio_gain_db: 0.0,
            output_dir: String::new(),
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
    pub hardware_encoder: HardwareEncoder,
    pub frame_rate: u16, // 0 keeps the source frame rate
    pub crop_width: u32, // 0 disables crop
    pub crop_height: u32,
    pub crop_x: u32,
    pub crop_y: u32,
    pub rotate_degrees: u16, // 0/90/180/270
    pub flip_horizontal: bool,
    pub flip_vertical: bool,
    pub audio_stream: i32, // -1 all, -2 none, otherwise zero-based track index
    pub subtitle_stream: i32, // -1 none, otherwise zero-based track index
    pub video_stream: u32, // zero-based track index
    pub deinterlace: bool,
    pub denoise: bool,
    pub sharpen: bool,
    pub brightness: f32, // -1.0..1.0
    pub contrast: f32,   // 0.0..2.0
    pub saturation: f32, // 0.0..3.0
    pub gamma: f32,      // 0.1..4.0
    pub audio_channels: u8, // 0 source, 1 mono, 2 stereo
    pub audio_sample_rate: u32, // 0 original
    pub audio_gain_db: f32, // -30..30 dB
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
            hardware_encoder: self.hardware_encoder, frame_rate: self.frame_rate,
            crop_width: self.crop_width, crop_height: self.crop_height,
            crop_x: self.crop_x, crop_y: self.crop_y,
            rotate_degrees: self.rotate_degrees,
            flip_horizontal: self.flip_horizontal, flip_vertical: self.flip_vertical,
            audio_stream: self.audio_stream, subtitle_stream: self.subtitle_stream,
            video_stream: self.video_stream,
            deinterlace: self.deinterlace, denoise: self.denoise, sharpen: self.sharpen,
            brightness: self.brightness, contrast: self.contrast,
            saturation: self.saturation, gamma: self.gamma,
            audio_channels: self.audio_channels, audio_sample_rate: self.audio_sample_rate,
            audio_gain_db: self.audio_gain_db,
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
