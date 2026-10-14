//! Find a working FFmpeg/FFprobe pair and provision missing binaries without
//! touching PATH or requiring administrator privileges.
use crate::model;
use sha2::{Digest, Sha256};
use crossbeam_channel::Sender;
use serde::Deserialize;
use std::{
    fs::{self, File}, io::{self, Read, Write}, path::{Path, PathBuf},
    process::Command, time::Duration,
};

const OWNER: &str = "binmgr";
const REPO: &str = "ffmpeg";
const MAX_ARCHIVE_BYTES: u64 = 400 * 1024 * 1024;
const MAX_TOOL_BYTES: u64 = 220 * 1024 * 1024;

#[derive(Clone, Debug)]
pub enum State {
    Checking,
    Downloading(String),
    Extracting,
    Ready { installed: bool, directory: Option<PathBuf> },
    Failed(String),
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}
struct Platform {
    asset: &'static str,
    windows: bool,
}

fn platform() -> Result<Platform, String> {
    let name = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "ffmpeg-windows-amd64.zip",
        ("windows", "aarch64") => "ffmpeg-windows-arm64.zip",
        ("linux", "x86_64") => "ffmpeg-linux-amd64.tar.gz",
        ("linux", "aarch64") => "ffmpeg-linux-arm64.tar.gz",
        ("macos", "x86_64") => "ffmpeg-darwin-amd64.tar.gz",
        ("macos", "aarch64") => "ffmpeg-darwin-arm64.tar.gz",
        _ => return Err(format!(
            "Automatic FFmpeg download is not available for {}/{}. Set FFmpeg and FFprobe paths manually.",
            std::env::consts::OS, std::env::consts::ARCH)),
    };
    Ok(Platform { asset: name, windows: cfg!(windows) })
}

pub fn tools_directory() -> Result<PathBuf, String> {
    dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .ok_or("Cannot locate your user data directory".to_owned())
        .map(|root| root.join("rust-encoder").join("tools").join("ffmpeg"))
}

pub fn managed_binary(name: &str) -> Option<PathBuf> {
    let directory = tools_directory().ok()?;
    let name = if cfg!(windows) { format!("{name}.exe") } else { name.to_owned() };
    let path = directory.join(name);
    path.is_file().then_some(path)
}

pub fn usable(binary: &Path) -> bool {
    Command::new(binary)
        .arg("-version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status().is_ok_and(|status| status.success())
}

pub fn missing_tools(ffmpeg: &str, ffprobe: &str) -> Vec<&'static str> {
    let tools = [("ffmpeg", ffmpeg), ("ffprobe", ffprobe)];
    tools.into_iter()
        .filter_map(|(name, configured)| {
            (!usable(&model::resolve_tool(name, configured))).then_some(name)
        })
        .collect()
}

fn asset<'a>(release: &'a Release, name: &str) -> Result<&'a str, String> {
    release.assets.iter().find(|asset| asset.name == name)
        .map(|a| a.browser_download_url.as_str())
        .ok_or_else(|| format!("Release {} has no asset {name}", release.tag_name))
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(25))
        .timeout_read(Duration::from_secs(150))
        .timeout_write(Duration::from_secs(45))
        .build()
}

fn get_reader<'a>(agent: &'a ureq::Agent, url: &str) -> Result<Box<dyn Read + 'a>, String> {
    if !url.starts_with("https://") {
        return Err("Refusing an unencrypted FFmpeg download".to_owned());
    }
    let response = agent.get(url)
        .set("User-Agent", "Rust-Encoder-dependency-bootstrap")
        .set("Accept", "application/octet-stream")
        .call().map_err(|error| format!("Download request failed: {error}"))?;
    if !(200..300).contains(&response.status()) {
        return Err(format!("Unexpected download HTTP status {}", response.status()));
    }
    Ok(Box::new(response.into_reader()))
}

fn read_small(agent: &ureq::Agent, url: &str, max: u64) -> Result<Vec<u8>, String> {
    let mut result = Vec::new();
    get_reader(agent, url)?.take(max + 1).read_to_end(&mut result)
        .map_err(|err| format!("Cannot read download: {err}"))?;
    if result.len() as u64 > max { return Err("Downloaded metadata is too large".into()); }
    Ok(result)
}

fn download(agent: &ureq::Agent, url: &str, path: &Path) -> Result<[u8; 32], String> {
    let mut response = get_reader(agent, url)?;
    let mut file = File::create(path).map_err(|err| format!("Cannot create {}: {err}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0u8; 128 * 1024];
    let mut count = 0;
    loop {
        let len = response.read(&mut buffer).map_err(|err| format!("Cannot read archive: {err}"))?;
        if len == 0 { break; }
        count += len as u64;
        if count > MAX_ARCHIVE_BYTES { return Err("FFmpeg archive exceeds allowed size".into()); }
        file.write_all(&buffer[..len]).map_err(|err| format!("Cannot write FFmpeg archive: {err}"))?;
        digest.update(&buffer[..len]);
    }
    file.flush().map_err(|err| format!("Cannot flush FFmpeg archive: {err}"))?;
    if count == 0 { return Err("FFmpeg download is empty".into()); }
    Ok(digest.finalize().into())
}

fn checksum(checksums: &str, archive: &str) -> Result<[u8; 32], String> {
    for line in checksums.lines() {
        let mut fields = line.split_whitespace();
        if let (Some(hash), Some(name)) = (fields.next(), fields.next())
            && name.trim_start_matches('*').trim_start_matches("./") == archive
        {
            if hash.len() != 64 { return Err("Invalid SHA-256 digest length".into()); }
            let mut result = [0u8; 32];
            for (index, byte) in result.iter_mut().enumerate() {
                *byte = u8::from_str_radix(&hash[index * 2..index * 2 + 2], 16)
                    .map_err(|_| "Invalid SHA-256 checksum hexadecimal".to_owned())?;
            }
            return Ok(result);
        }
    }
    Err(format!("Published SHA-256 checksum for {archive} not found"))
}

fn tool_name(path: &Path, windows: bool) -> Option<&'static str> {
    match path.file_name()?.to_str()? {
        "ffmpeg.exe" if windows => Some("ffmpeg.exe"),
        "ffprobe.exe" if windows => Some("ffprobe.exe"),
        "ffplay.exe" if windows => Some("ffplay.exe"),
        "ffmpeg" if !windows => Some("ffmpeg"),
        "ffprobe" if !windows => Some("ffprobe"),
        "ffplay" if !windows => Some("ffplay"),
        _ => None,
    }
}

fn copy_limited(reader: &mut impl Read, length: u64, to: &Path) -> Result<(), String> {
    if length == 0 || length > MAX_TOOL_BYTES {
        return Err(format!("Invalid FFmpeg executable size ({length} bytes)"));
    }
    let mut output = File::create(to).map_err(|err| format!("Cannot create {}: {err}", to.display()))?;
    let copied = io::copy(&mut reader.take(length), &mut output)
        .map_err(|err| format!("Failed to extract {}: {err}", to.display()))?;
    if copied != length { return Err("Truncated FFmpeg archive entry".into()); }
    drop(output);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(to, fs::Permissions::from_mode(0o755))
            .map_err(|err| format!("Cannot mark FFmpeg executable: {err}"))?;
    }
    Ok(())
}

fn extract(archive: &Path, directory: &Path, platform: &Platform) -> Result<(), String> {
    if platform.windows {
        let file = File::open(archive).map_err(|err| err.to_string())?;
        let mut zip = zip::ZipArchive::new(file).map_err(|err| format!("Invalid FFmpeg ZIP: {err}"))?;
        if zip.len() > 1500 { return Err("Too many FFmpeg archive entries".into()); }
        for index in 0..zip.len() {
            let mut entry = zip.by_index(index).map_err(|err| err.to_string())?;
            let Some(path) = entry.enclosed_name() else { return Err("Unsafe FFmpeg ZIP entry path".into()); };
            let Some(name) = tool_name(&path, true) else { continue };
            if !entry.is_file() { continue; }
            let target = directory.join(name);
            if target.exists() { return Err(format!("Duplicate executable {name} in archive")); }
            let size = entry.size();
            copy_limited(&mut entry, size, &target)?;
        }
    } else {
        let file = File::open(archive).map_err(|err| err.to_string())?;
        let gzip = flate2::read::GzDecoder::new(file);
        let mut tar = tar::Archive::new(gzip);
        for (index, entry) in tar.entries().map_err(|err| err.to_string())?.enumerate() {
            if index > 1500 { return Err("Too many FFmpeg archive entries".into()); }
            let mut entry = entry.map_err(|err| format!("Invalid FFmpeg TAR: {err}"))?;
            if !entry.header().entry_type().is_file() { continue; }
            let path = entry.path().map_err(|err| err.to_string())?;
            let Some(name) = tool_name(path.as_ref(), false) else { continue };
            let target = directory.join(name);
            if target.exists() { return Err(format!("Duplicate executable {name} in archive")); }
            let size = entry.size();
            copy_limited(&mut entry, size, &target)?;
        }
    }
    for name in ["ffmpeg", "ffprobe"] {
        let binary = directory.join(if platform.windows { format!("{name}.exe") } else { name.to_owned() });
        if !binary.is_file() { return Err(format!("FFmpeg package did not contain {name}")); }
        if !usable(&binary) { return Err(format!("Extracted {name} is not executable on this system")); }
    }
    Ok(())
}

fn install_once(tx: &Sender<State>) -> Result<PathBuf, String> {
    let platform = platform()?;
    let directory = tools_directory()?;
    let parent = directory.parent().ok_or("Invalid tool installation directory")?;
    fs::create_dir_all(parent).map_err(|e| format!("Cannot create FFmpeg directory: {e}"))?;
    let temp = parent.join(format!(".ffmpeg-install-{}", std::process::id()));
    let archive = parent.join(format!(".ffmpeg-download-{}.archive", std::process::id()));
    let result = (|| {
        // Refuse stale staging directories instead of following existing symlinks.
        fs::create_dir(&temp).map_err(|e| format!("Cannot stage FFmpeg files: {e}"))?;
        let agent = agent();
        let json = read_small(&agent,
            &format!("https://api.github.com/repos/{OWNER}/{REPO}/releases/latest"),
            512 * 1024)?;
        let release: Release = serde_json::from_slice(&json)
            .map_err(|err| format!("Invalid FFmpeg release metadata: {err}"))?;
        let checksum_url = asset(&release, "SHA256SUMS.txt")?;
        let checksums = read_small(&agent, checksum_url, 128 * 1024)?;
        let checksums = String::from_utf8(checksums)
            .map_err(|err| format!("Invalid FFmpeg checksum text: {err}"))?;
        let expected = checksum(&checksums, platform.asset)?;
        tx.send(State::Downloading(format!("Downloading verified FFmpeg ({})…", release.tag_name))).ok();
        let actual = download(&agent, asset(&release, platform.asset)?, &archive)?;
        if actual != expected { return Err("FFmpeg SHA-256 checksum mismatch; refusing to install".into()); }
        tx.send(State::Extracting).ok();
        extract(&archive, &temp, &platform)?;
        // Swap only after the full pair has passed -version. Keep existing
        // installation as a backup in case the final move fails.
        let backup = parent.join(format!(".ffmpeg-previous-{}", std::process::id()));
        if directory.exists() {
            fs::rename(&directory, &backup)
                .map_err(|e| format!("Cannot back up previous FFmpeg install: {e}"))?;
        }
        if let Err(err) = fs::rename(&temp, &directory) {
            if backup.exists() { let _ = fs::rename(&backup, &directory); }
            return Err(format!("Cannot activate FFmpeg installation: {err}"));
        }
        if backup.exists() { let _ = fs::remove_dir_all(backup); }
        Ok(directory.clone())
    })();
    let _ = fs::remove_file(&archive);
    if temp.exists() { let _ = fs::remove_dir_all(&temp); }
    result
}

/// Run only on a worker thread. UI rendering must never wait for the network.
pub fn check_and_install(
    configured_ffmpeg: String, configured_ffprobe: String, tx: Sender<State>,
) {
    let _ = tx.send(State::Checking);
    let missing = missing_tools(&configured_ffmpeg, &configured_ffprobe);
    if missing.is_empty() {
        let _ = tx.send(State::Ready { installed: false, directory: None });
        return;
    }
    let _ = tx.send(State::Downloading(format!(
        "Missing {}. Downloading FFmpeg and FFprobe…", missing.join(", ")
    )));
    match install_once(&tx) {
        Ok(directory) => {
            let _ = tx.send(State::Ready { installed: true, directory: Some(directory) });
        }
        Err(error) => {
            let _ = tx.send(State::Failed(format!(
                "{error}. FFmpeg/FFprobe could not be installed automatically. Set the executable paths in External tools."
            )));
        }
    }
}

pub fn start_check(ffmpeg: String, ffprobe: String, tx: Sender<State>) {
    std::thread::spawn(move || check_and_install(ffmpeg, ffprobe, tx));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_named_checksum_not_a_similar_filename() {
        let data = "aabbcc\n";
        assert!(checksum(data, "ffmpeg-windows-amd64.zip").is_err());
        let hash = "ab".repeat(32);
        let manifest = format!("{hash}  ffmpeg-windows-amd64.zip\n{hash}  ffmpeg-windows-arm64.zip\n");
        assert_eq!(checksum(&manifest, "ffmpeg-windows-amd64.zip").unwrap(), [0xabu8; 32]);
        assert!(checksum(&manifest, "ffmpeg-windows-amd64.zip.bak").is_err());
    }

    #[test]
    fn archive_names_are_restricted_to_executable_basename() {
        assert_eq!(tool_name(Path::new("ffmpeg.exe"), true), Some("ffmpeg.exe"));
        assert_eq!(tool_name(Path::new("nested/bin/ffprobe.exe"), true), Some("ffprobe.exe"));
        assert_eq!(tool_name(Path::new("something/else.dll"), true), None);
        assert_eq!(tool_name(Path::new("ffmpeg"), false), Some("ffmpeg"));
    }

    #[test]
    fn unsupported_archive_entries_are_not_extracted() {
        // Only executable basenames can be selected for installation.
        assert!(tool_name(Path::new("../unrelated.exe"), true).is_none());
        assert!(tool_name(Path::new("x/ffmpeg.dll"), true).is_none());
    }
}
