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

/// Stream metadata only: changing the tag does NOT change source color or
/// perform HDR/SDR tone mapping. Use only on correctly graded source material.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorSignal {
    #[default]
    Automatic,
    Bt709,
    Bt2020Pq,
    Bt2020Hlg,
}

impl ColorSignal {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Automatic => "Auto / source",
            Self::Bt709 => "Rec.709 SDR metadata",
            Self::Bt2020Pq => "BT.2020 / PQ HDR10 metadata",
            Self::Bt2020Hlg => "BT.2020 / HLG metadata",
        }
    }

    pub const fn ffmpeg_values(self) -> Option<(&'static str, &'static str, &'static str)> {
        match self {
            Self::Automatic => None,
            Self::Bt709 => Some(("bt709", "bt709", "bt709")),
            Self::Bt2020Pq => Some(("bt2020", "smpte2084", "bt2020nc")),
            Self::Bt2020Hlg => Some(("bt2020", "arib-std-b67", "bt2020nc")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub preset: Preset,
    pub quality: u8,
    pub output_bit_depth: u8, // 0 encoder default, 8 or 10
    pub color_signal: ColorSignal,
    pub use_video_bitrate: bool,
    pub two_pass: bool,
    pub video_bitrate_kbps: u32,
    pub max_video_bitrate_kbps: u32, // 0 = no maximum
    pub encoder_speed: u8, // 0 faster, 1 balanced, 2 better compression
    pub encoder_threads: u8, // 0 = automatic
    pub keyframe_interval: u16, // 0 = automatic
    pub faststart: bool,
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
    pub ffplay_path: String,
    pub dark_theme: bool,
    pub workspace_left_fraction: f32,
    pub workspace_right_fraction: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            preset: Preset::H264, quality: 23,
            output_bit_depth: 0, color_signal: ColorSignal::Automatic,
            use_video_bitrate: false, two_pass: false, video_bitrate_kbps: 4000,
            max_video_bitrate_kbps: 0, encoder_speed: 1, encoder_threads: 0,
            keyframe_interval: 0, faststart: true, target_width: 0,
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
            ffmpeg_path: String::new(), ffprobe_path: String::new(),
            ffplay_path: String::new(), dark_theme: true,
            workspace_left_fraction: 0.21, workspace_right_fraction: 0.29,
        }
    }
}

#[derive(Clone, Debug)]
pub struct EncodeOptions {
    pub preset: Preset,
    pub quality: u8,
    pub output_bit_depth: u8,
    pub color_signal: ColorSignal,
    pub use_video_bitrate: bool,
    pub two_pass: bool,
    pub video_bitrate_kbps: u32,
    pub max_video_bitrate_kbps: u32,
    pub encoder_speed: u8,
    pub encoder_threads: u8,
    pub keyframe_interval: u16,
    pub faststart: bool,
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
            quality: self.quality, output_bit_depth: self.output_bit_depth,
            color_signal: self.color_signal, use_video_bitrate: self.use_video_bitrate,
            two_pass: self.two_pass,
            video_bitrate_kbps: self.video_bitrate_kbps,
            max_video_bitrate_kbps: self.max_video_bitrate_kbps,
            encoder_speed: self.encoder_speed, encoder_threads: self.encoder_threads,
            keyframe_interval: self.keyframe_interval, faststart: self.faststart,
            target_width: self.target_width,
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
    if let Some(managed) = crate::dependencies::managed_binary(name) {
        return managed;
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
    save_settings_bytes(&path, &content)
}

/// Rename replaces the destination on modern Windows and Unix. In particular,
/// never delete the previous settings before the replacement: if replacement
/// fails (e.g. antivirus holds the file), the old settings remain intact.
fn save_settings_bytes(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, content)?;
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
    fn settings_replace_keeps_only_new_valid_document() {
        let root = std::env::temp_dir().join(format!("encoder-settings-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("settings.json");
        std::fs::write(&path, br#"{"preset":"H264"}"#).unwrap();
        save_settings_bytes(&path, br#"{"preset":"Flac"}"#).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), br#"{"preset":"Flac"}"#);
        let _ = std::fs::remove_dir_all(root);
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
