//! Companion synchronized audio/video preview via FFplay.
//! This is a managed *external* SDL player window, not an embedded egui decoder.
use crate::model::{resolve_tool, Settings};
use std::{ffi::OsString, path::{Path, PathBuf}, process::{Child, Command, ExitStatus, Stdio}};

/// Prefer an explicit override, then FFplay beside the configured FFmpeg,
/// then a bundle beside Rust Encoder, and finally the system PATH.
pub fn resolve_ffplay(settings: &Settings) -> PathBuf {
    if !settings.ffplay_path.trim().is_empty() {
        return PathBuf::from(&settings.ffplay_path);
    }
    if !settings.ffmpeg_path.trim().is_empty() {
        let binary = if cfg!(windows) { "ffplay.exe" } else { "ffplay" };
        let sibling = Path::new(&settings.ffmpeg_path).with_file_name(binary);
        if sibling.is_file() { return sibling; }
    }
    resolve_tool("ffplay", "")
}

/// Build an OS-safe argument array without involving a command shell.
/// `end` is an absolute time in the input, not a playback duration.
pub fn playback_args(input: &Path, seek: f64, end: Option<f64>) -> Result<Vec<OsString>, String> {
    if !input.is_file() { return Err(format!("Media file does not exist: {}", input.display())); }
    if !seek.is_finite() || seek < 0.0 { return Err("Invalid playback position".into()); }
    if let Some(end) = end
        && (!end.is_finite() || end <= seek) {
        return Err("Playback end must be later than the start position".into());
    }
    let mut args: Vec<OsString> = ["-hide_banner", "-loglevel", "error", "-autoexit",
        "-sync", "audio", "-ss"]
        .iter().map(OsString::from).collect();
    args.push(format!("{seek:.3}").into());
    if let Some(end) = end {
        args.push(OsString::from("-t"));
        args.push(format!("{:.3}", end - seek).into());
    }
    args.extend([
        OsString::from("-window_title"), OsString::from("Rust Encoder - A/V Preview"),
        OsString::from("-i"), input.as_os_str().to_owned(),
    ]);
    Ok(args)
}

pub struct PlaybackSession {
    pub job_id: u64,
    child: Child,
}

impl PlaybackSession {
    pub fn launch(job_id: u64, binary: &Path, input: &Path,
                  seek: f64, end: Option<f64>) -> Result<Self, String> {
        let args = playback_args(input, seek, end)?;
        let child = Command::new(binary)
            .args(&args)
            .stdin(Stdio::null())
            // Never pipe player output without a reader: verbose players can deadlock.
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!(
                "Cannot launch FFplay ({}): {error}. Install FFplay or set its path under External tools.",
                binary.display()
            ))?;
        Ok(Self { job_id, child })
    }

    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }
}

impl Drop for PlaybackSession {
    fn drop(&mut self) {
        // Reap all started children, whether they exited naturally or we stop them.
        match self.child.try_wait() {
            Ok(Some(_)) => {}
            _ => {
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn playback_args_preserve_unicode_and_special_paths() {
        let path = std::env::temp_dir().join(format!(
            "rust-encoder-player-{}-🦀-special & ;.mkv", std::process::id()));
        std::fs::write(&path, b"preview fixture").unwrap();
        let args = playback_args(&path, 1.25, Some(3.75)).unwrap();
        assert!(args.windows(2).any(|p| p[0] == "-ss" && p[1] == "1.250"));
        assert!(args.windows(2).any(|p| p[0] == "-t" && p[1] == "2.500"));
        assert_eq!(args.last().unwrap(), path.as_os_str());
        assert!(!args.iter().any(|a| a == "sh" || a == "cmd"));
        let all = playback_args(&path, 0.0, None).unwrap();
        assert!(!all.iter().any(|a| a == "-t"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_invalid_ranges_and_missing_files() {
        let path = std::env::temp_dir().join(format!(
            "rust-encoder-player-validation-{}.mp4", std::process::id()));
        std::fs::write(&path, b"media").unwrap();
        for start in [f64::NAN, f64::INFINITY, -0.1] {
            assert!(playback_args(&path, start, None).is_err());
        }
        for end in [0.0, 1.0, f64::INFINITY, f64::NAN] {
            assert!(playback_args(&path, 1.0, Some(end)).is_err());
        }
        std::fs::remove_file(&path).unwrap();
        assert!(playback_args(&path, 0.0, None).is_err());
    }

    #[test]
    fn user_configured_player_path_is_respected() {
        let mut settings = Settings::default();
        settings.ffplay_path = "custom-ffplay".into();
        assert_eq!(resolve_ffplay(&settings), PathBuf::from("custom-ffplay"));
        let parsed: Settings = serde_json::from_str("{}").unwrap();
        assert!(parsed.ffplay_path.is_empty());
    }
}
