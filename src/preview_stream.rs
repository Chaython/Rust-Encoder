//! Bounded, cancellable FFmpeg-to-egui streaming preview.
//!
//! Continuous native *video* in egui, without re-launching FFmpeg each frame.
//! Sound is deliberately not handled here: the optional FFplay companion is
//! currently the synchronized audio/video path.
use crossbeam_channel::{Receiver, TryRecvError, TrySendError, bounded};
use std::{
    ffi::OsString,
    io::{BufReader, Read},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

pub const WIDTH: usize = 640;
pub const HEIGHT: usize = 360;
pub const FPS: u32 = 24;
const FRAME_BYTES: usize = WIDTH * HEIGHT * 4;

#[derive(Debug)]
pub struct StreamFrame {
    pub rgba: Vec<u8>,
    pub position: f64,
}

pub struct NativeVideoPreview {
    pub job_id: u64,
    frames: Receiver<StreamFrame>,
    finished: Receiver<Result<(), String>>,
    child: Arc<Mutex<Child>>,
    stop: Arc<AtomicBool>,
}

pub fn decode_args(input: &Path, start: f64, end: Option<f64>) -> Result<Vec<OsString>, String> {
    if !input.is_file() {
        return Err(format!("Preview input does not exist: {}", input.display()));
    }
    if !start.is_finite() || start < 0.0 {
        return Err("Preview seek position is invalid".into());
    }
    if let Some(end) = end
        && (!end.is_finite() || end <= start) {
        return Err("Preview Out point must be later than the playhead".into());
    }
    // Use the input's real-time timestamps to pace the decoder. Resample into
    // a fixed frame size and rate, avoiding unbounded image allocations.
    let filter = format!(
        "fps={FPS},scale={WIDTH}:{HEIGHT}:force_original_aspect_ratio=decrease:flags=bicubic,         pad={WIDTH}:{HEIGHT}:(ow-iw)/2:(oh-ih)/2"
    );
    let mut args: Vec<OsString> = [
        "-nostdin", "-hide_banner", "-loglevel", "error",
        "-ss",
    ].into_iter().map(OsString::from).collect();
    args.push(format!("{start:.3}").into());
    args.extend([OsString::from("-re"), OsString::from("-i"), input.as_os_str().to_owned()]);
    if let Some(end) = end {
        args.extend([OsString::from("-t"), format!("{:.3}", end - start).into()]);
    }
    args.extend([
        OsString::from("-map"), OsString::from("0:v:0"),
        OsString::from("-an"), OsString::from("-sn"), OsString::from("-dn"),
        OsString::from("-vf"), OsString::from(filter),
        OsString::from("-pix_fmt"), OsString::from("rgba"),
        OsString::from("-f"), OsString::from("rawvideo"),
        OsString::from("-c:v"), OsString::from("rawvideo"),
        OsString::from("pipe:1"),
    ]);
    Ok(args)
}

/// Consume only frames that have reached the audio playback clock. Keep the
/// earliest future frame pending, rather than replacing it with newer frames.
pub fn frame_at_clock(
    frames: &Receiver<StreamFrame>,
    held: &mut Option<StreamFrame>,
    deadline: Option<f64>,
) -> Option<StreamFrame> {
    let ready = |position: f64| deadline.is_none_or(|limit| position <= limit);
    let mut newest = None;
    if let Some(pending) = held.take() {
        if ready(pending.position) {
            newest = Some(pending);
        } else {
            *held = Some(pending);
            return None;
        }
    }
    while let Ok(frame) = frames.try_recv() {
        if ready(frame.position) {
            newest = Some(frame);
        } else {
            *held = Some(frame);
            break;
        }
    }
    newest
}

impl NativeVideoPreview {
    pub fn launch(job_id: u64, ffmpeg: &Path, input: &Path,
                  start: f64, end: Option<f64>) -> Result<Self, String> {
        let args = decode_args(input, start, end)?;
        let mut child = Command::new(ffmpeg)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            // Never leave an unread stderr pipe blocking the decoder.
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Cannot start embedded preview ({}): {e}", ffmpeg.display()))?;
        let stdout = child.stdout.take().ok_or("FFmpeg preview pipe unavailable")?;
        let child = Arc::new(Mutex::new(child));
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, frames) = bounded::<StreamFrame>(2);
        let (done_tx, finished) = bounded::<Result<(), String>>(1);
        let child_worker = Arc::clone(&child);
        let stop_worker = Arc::clone(&stop);

        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut frame_idx = 0u64;
            let mut clean_end = false;
            loop {
                if stop_worker.load(Ordering::Acquire) { break; }
                let mut rgba = vec![0; FRAME_BYTES];
                if let Err(error) = reader.read_exact(&mut rgba) {
                    clean_end = error.kind() == std::io::ErrorKind::UnexpectedEof;
                    break;
                }
                let frame = StreamFrame {
                    rgba, position: start + frame_idx as f64 / FPS as f64,
                };
                frame_idx += 1;
                // The GUI consumes only the newest visible frames. Unlike
                // unbounded send(), this cannot accumulate minutes of video.
                match tx.try_send(frame) {
                    Ok(()) | Err(TrySendError::Full(_)) => {}
                    Err(TrySendError::Disconnected(_)) => break,
                }
            }
            // Drop the buffered reader (and thus the pipe) before waiting.
            drop(reader);
            // Do not hold the Child mutex across a blocking wait: the GUI
            // needs that same lock to kill the decoder immediately on Stop.
            let result = loop {
                let status = match child_worker.lock() {
                    Ok(mut child) => {
                        if stop_worker.load(Ordering::Acquire) {
                            let _ = child.kill();
                        }
                        child.try_wait()
                    }
                    Err(_) => break Err("Preview process lock poisoned".into()),
                };
                match status {
                    Ok(Some(status)) if status.success() || stop_worker.load(Ordering::Acquire) => break Ok(()),
                    Ok(Some(status)) => break Err(format!("Embedded decoder exited with {status}")),
                    Ok(None) => thread::sleep(std::time::Duration::from_millis(25)),
                    Err(error) => break Err(format!("Embedded decoder could not be reaped: {error}")),
                }
            };
            // An EOF between frames is normal at the natural end of a clip.
            // Process status, not the pipe EOF alone, determines failure.
            let _ = clean_end;
            let _ = done_tx.try_send(result);
        });

        Ok(Self { job_id, frames, finished, child, stop })
    }

    pub fn newest_frame(&self) -> Option<StreamFrame> {
        let mut newest = None;
        while let Ok(frame) = self.frames.try_recv() {
            newest = Some(frame);
        }
        newest
    }

    pub fn frames(&self) -> &Receiver<StreamFrame> { &self.frames }

    pub fn try_finished(&self) -> Option<Result<(), String>> {
        match self.finished.try_recv() {
            Ok(done) => Some(done),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
        }
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
        }
    }
}

impl Drop for NativeVideoPreview {
    fn drop(&mut self) {
        // Cancel immediately; the worker reaps without blocking the UI thread.
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streamed_preview_args_preserve_paths_and_bounds() {
        let path = std::env::temp_dir().join(format!(
            "rust-encoder-stream-{}-🦀 &.mkv", std::process::id()));
        std::fs::write(&path, b"sample").unwrap();
        let args = decode_args(&path, 1.5, Some(3.0)).unwrap();
        assert!(args.windows(2).any(|a| a[0] == "-ss" && a[1] == "1.500"));
        assert!(args.windows(2).any(|a| a[0] == "-t" && a[1] == "1.500"));
        assert!(args.windows(2).any(|a| a[0] == "-i" && a[1] == path.as_os_str()));
        assert!(args.iter().any(|a| a.to_string_lossy().contains("fps=24")));
        assert!(args.iter().any(|a| a.to_string_lossy().contains("pad=640:360")));
        assert_eq!(args.last().unwrap(), "pipe:1");
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn clock_holds_first_future_frame_until_due() {
        let (tx, rx) = bounded(4);
        for position in [0.0, 0.1, 0.2, 0.3] {
            tx.send(StreamFrame { rgba: vec![], position }).unwrap();
        }
        let mut held = None;
        assert_eq!(frame_at_clock(&rx, &mut held, Some(0.05)).unwrap().position, 0.0);
        assert_eq!(held.as_ref().unwrap().position, 0.1);
        assert!(frame_at_clock(&rx, &mut held, Some(0.05)).is_none());
        assert_eq!(held.as_ref().unwrap().position, 0.1);
        assert_eq!(frame_at_clock(&rx, &mut held, Some(0.25)).unwrap().position, 0.2);
        assert_eq!(held.as_ref().unwrap().position, 0.3);
        assert_eq!(frame_at_clock(&rx, &mut held, Some(0.35)).unwrap().position, 0.3);
        assert!(held.is_none());
    }

    #[test]
    fn clock_free_mode_drains_to_newest_frame() {
        let (tx, rx) = bounded(2);
        tx.send(StreamFrame { rgba: vec![], position: 1.0 }).unwrap();
        tx.send(StreamFrame { rgba: vec![], position: 1.1 }).unwrap();
        assert_eq!(frame_at_clock(&rx, &mut None, None).unwrap().position, 1.1);
    }

    #[test]
    fn stream_rejects_nonfinite_positions() {
        let path = std::env::temp_dir().join(format!(
            "rust-encoder-stream-invalid-{}.mp4", std::process::id()));
        std::fs::write(&path, b"sample").unwrap();
        for invalid in [f64::INFINITY, f64::NAN, -0.01] {
            assert!(decode_args(&path, invalid, None).is_err());
            assert!(decode_args(&path, 0.0, Some(invalid)).is_err());
        }
        assert!(decode_args(&path, 5.0, Some(3.0)).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
