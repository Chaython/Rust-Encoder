//! Native optional PCM audio playback for the embedded media preview.
//! FFmpeg decodes any supported source into fixed-format stereo f32le;
//! rodio owns the OS output device. A bounded FIFO and nonblocking Source
//! protect the audio callback against process I/O stalls.
use crossbeam_channel::{Receiver, SendTimeoutError, TryRecvError, bounded};
use rodio::{OutputStream, OutputStreamBuilder, Sink, Source};
use std::{
    ffi::OsString, io::{BufReader, Read}, path::Path,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, atomic::{AtomicBool, AtomicU64, Ordering}},
    thread, time::Duration,
};

pub const RATE: u32 = 48_000;
pub const CHANNELS: u16 = 2;
const SAMPLES_PER_BLOCK: usize = 4096;
const PCM_BYTES: usize = SAMPLES_PER_BLOCK * 4;

/// Fill a PCM block, retaining a final short (but channel-aligned) block.
/// read_exact() would discard the last samples at EOF.
fn read_pcm_block<R: Read>(reader: &mut R, dst: &mut [u8]) -> Result<usize, String> {
    let mut filled = 0;
    while filled < dst.len() {
        match reader.read(&mut dst[filled..]) {
            Ok(0) => break,
            Ok(size) => filled += size,
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(format!("PCM pipe read failed: {err}")),
        }
    }
    if filled % (CHANNELS as usize * 4) != 0 {
        return Err("Truncated interleaved stereo PCM frame".into());
    }
    Ok(filled)
}


pub fn audio_args(input: &Path, seek: f64, end: Option<f64>, stream_index: usize)
    -> Result<Vec<OsString>, String> {
    if !input.is_file() { return Err(format!("Audio source does not exist: {}", input.display())); }
    if !seek.is_finite() || seek < 0.0 { return Err("Audio start must be finite and nonnegative".into()); }
    if let Some(end) = end
        && (!end.is_finite() || end <= seek) {
        return Err("Audio Out point must be later than playback start".into());
    }
    if stream_index > 31 { return Err("Audio track index must be 0–31".into()); }
    let mut args: Vec<OsString> = [
        "-nostdin", "-hide_banner", "-loglevel", "error", "-ss",
    ].into_iter().map(OsString::from).collect();
    args.push(format!("{seek:.3}").into());
    args.push(OsString::from("-i"));
    args.push(input.as_os_str().to_owned());
    if let Some(end) = end {
        args.push(OsString::from("-t"));
        args.push(format!("{:.3}", end - seek).into());
    }
    args.extend([
        OsString::from("-map"), OsString::from(format!("0:a:{stream_index}")),
        OsString::from("-vn"), OsString::from("-sn"), OsString::from("-dn"),
        OsString::from("-ac"), OsString::from(CHANNELS.to_string()),
        OsString::from("-ar"), OsString::from(RATE.to_string()),
        OsString::from("-f"), OsString::from("f32le"),
        OsString::from("-c:a"), OsString::from("pcm_f32le"),
        OsString::from("pipe:1"),
    ]);
    Ok(args)
}

struct PcmSource {
    rx: Receiver<Vec<f32>>,
    current: std::vec::IntoIter<f32>,
    completed: Arc<AtomicBool>,
    underruns: Arc<AtomicU64>,
    played_samples: Arc<AtomicU64>,
}

impl Iterator for PcmSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(sample) = self.current.next() {
                self.played_samples.fetch_add(1, Ordering::Relaxed);
                return Some(sample);
            }
            match self.rx.try_recv() {
                Ok(block) => self.current = block.into_iter(),
                Err(TryRecvError::Disconnected) => return None,
                Err(TryRecvError::Empty) if self.completed.load(Ordering::Acquire) => return None,
                Err(TryRecvError::Empty) => {
                    // Never block a CPAL callback on a subprocess pipe.
                    self.underruns.fetch_add(1, Ordering::Relaxed);
                    return Some(0.0);
                }
            }
        }
    }
}

impl Source for PcmSource {
    fn current_span_len(&self) -> Option<usize> { None }
    fn channels(&self) -> u16 { CHANNELS }
    fn sample_rate(&self) -> u32 { RATE }
    fn total_duration(&self) -> Option<Duration> { None }
}

pub struct NativeAudioPreview {
    pub job_id: u64,
    seek: f64,
    // Sink must be destroyed before its OS audio stream.
    sink: Arc<Sink>,
    _stream: OutputStream,
    child: Arc<Mutex<Child>>,
    stop: Arc<AtomicBool>,
    finished: Receiver<Result<(), String>>,
    underruns: Arc<AtomicU64>,
    played_samples: Arc<AtomicU64>,
}

impl NativeAudioPreview {
    pub fn launch(job_id: u64, ffmpeg: &Path, input: &Path, seek: f64,
                  end: Option<f64>, audio_stream: usize) -> Result<Self, String> {
        let args = audio_args(input, seek, end, audio_stream)?;
        let output = OutputStreamBuilder::open_default_stream()
            .map_err(|e| format!("Cannot open system audio output: {e}"))?;
        let sink = Arc::new(Sink::connect_new(output.mixer()));
        // Avoid advancing the playback clock by generating startup silence
        // before the first PCM buffers are ready.
        sink.pause();
        let mut child = Command::new(ffmpeg)
            .args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null())
            .spawn().map_err(|e| format!("Cannot start FFmpeg audio decoder: {e}"))?;
        let stdout = child.stdout.take().ok_or("PCM decoder stdout unavailable")?;
        let child = Arc::new(Mutex::new(child));
        let stop = Arc::new(AtomicBool::new(false));
        let completed = Arc::new(AtomicBool::new(false));
        let underruns = Arc::new(AtomicU64::new(0));
        let played_samples = Arc::new(AtomicU64::new(0));
        let (tx, rx) = bounded::<Vec<f32>>(12);
        let (done_tx, finished) = bounded::<Result<(), String>>(1);
        let worker_child = Arc::clone(&child);
        let worker_stop = Arc::clone(&stop);
        let worker_completed = Arc::clone(&completed);
        let worker_sink = Arc::clone(&sink);
        sink.append(PcmSource {
            rx, current: Vec::new().into_iter(), completed,
            underruns: Arc::clone(&underruns),
            played_samples: Arc::clone(&played_samples),
        });
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut bytes = vec![0u8; PCM_BYTES];
            let mut read_error = None;
            let mut buffered_blocks = 0;
            let mut started = false;
            'decode: loop {
                if worker_stop.load(Ordering::Acquire) { break; }
                let size = match read_pcm_block(&mut reader, &mut bytes) {
                    Ok(0) => break,
                    Ok(size) => size,
                    Err(error) => { read_error = Some(error); break; }
                };
                let mut block: Vec<f32> = bytes[..size].chunks_exact(4)
                    .map(|p| f32::from_le_bytes([p[0], p[1], p[2], p[3]])).collect();
                loop {
                    if worker_stop.load(Ordering::Acquire) { break 'decode; }
                    match tx.send_timeout(block, Duration::from_millis(60)) {
                        Ok(()) => {
                            buffered_blocks += 1;
                            if !started && buffered_blocks >= 3 {
                                worker_sink.play();
                                started = true;
                            }
                            break;
                        }
                        Err(SendTimeoutError::Timeout(pending)) => block = pending,
                        Err(SendTimeoutError::Disconnected(_)) => break 'decode,
                    }
                }
            }
            // Clips shorter than the prebuffer threshold must also play.
            if !started && buffered_blocks > 0 && !worker_stop.load(Ordering::Acquire) {
                worker_sink.play();
            }
            drop(reader);
            let result = loop {
                let wait = match worker_child.lock() {
                    Ok(mut child) => {
                        if worker_stop.load(Ordering::Acquire) { let _ = child.kill(); }
                        child.try_wait()
                    }
                    Err(_) => break Err("Audio decoder process lock poisoned".into()),
                };
                match wait {
                    Ok(Some(status)) if status.success() || worker_stop.load(Ordering::Acquire) => break Ok(()),
                    Ok(Some(status)) => break Err(format!("FFmpeg audio decoder failed ({status})")),
                    Ok(None) => thread::sleep(Duration::from_millis(20)),
                    Err(e) => break Err(format!("Audio decoder wait error: {e}")),
                }
            };
            worker_completed.store(true, Ordering::Release);
            let _ = done_tx.try_send(match read_error {
                Some(error) if !worker_stop.load(Ordering::Acquire) => Err(error),
                _ => result,
            });
        });
        Ok(Self { job_id, seek, sink, _stream: output, child, stop, finished, underruns, played_samples })
    }

    /// Audio-master timeline based on actual emitted PCM samples. Startup
    /// silence and underrun padding must not advance the source playhead.
    /// Hardware output latency remains approximate.
    pub fn playhead(&self) -> f64 {
        self.seek + self.played_samples.load(Ordering::Relaxed) as f64
            / f64::from(RATE * u32::from(CHANNELS))
    }
    pub fn volume(&self) -> f32 { self.sink.volume() }
    pub fn set_volume(&self, value: f32) { self.sink.set_volume(value.clamp(0.0, 1.0)); }
    pub fn underruns(&self) -> u64 { self.underruns.load(Ordering::Relaxed) }
    pub fn decoder_result(&self) -> Option<Result<(), String>> { self.finished.try_recv().ok() }
    pub fn finished_playback(&self) -> bool { self.sink.empty() }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        self.sink.stop();
        if let Ok(mut child) = self.child.lock() { let _ = child.kill(); }
    }
}
impl Drop for NativeAudioPreview {
    fn drop(&mut self) { self.stop(); }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pcm_arguments_preserve_seek_selection_and_path() {
        let path = std::env::temp_dir().join(format!("audio-preview-{}-🦀 &.mkv", std::process::id()));
        std::fs::write(&path, b"fixture").unwrap();
        let args = audio_args(&path, 1.25, Some(3.25), 2).unwrap();
        assert!(args.windows(2).any(|a| a[0] == "-map" && a[1] == "0:a:2"));
        assert!(args.windows(2).any(|a| a[0] == "-ss" && a[1] == "1.250"));
        assert!(args.windows(2).any(|a| a[0] == "-t" && a[1] == "2.000"));
        assert_eq!(args.last().unwrap(), "pipe:1");
        assert!(args.iter().any(|a| a == path.as_os_str()));
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn pcm_validation_rejects_invalid_offsets() {
        let path = std::env::temp_dir().join(format!("audio-preview-{}.mkv", std::process::id()));
        std::fs::write(&path, b"fixture").unwrap();
        for seek in [f64::NAN, f64::INFINITY, -0.1] {
            assert!(audio_args(&path, seek, None, 0).is_err());
        }
        assert!(audio_args(&path, 1.0, Some(0.5), 0).is_err());
        assert!(audio_args(&path, 1.0, None, 32).is_err());
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn final_short_pcm_block_is_not_lost() {
        use std::io::Cursor;
        let mut data = Vec::new();
        for sample in [0.5f32, -0.5, 0.25, -0.25, 0.75, -0.75] {
            data.extend_from_slice(&sample.to_le_bytes());
        }
        let mut reader = Cursor::new(data.clone());
        let mut buffer = vec![0u8; PCM_BYTES];
        assert_eq!(read_pcm_block(&mut reader, &mut buffer).unwrap(), data.len());
        assert_eq!(&buffer[..data.len()], data.as_slice());
        assert_eq!(read_pcm_block(&mut reader, &mut buffer).unwrap(), 0);
        let mut broken = Cursor::new(vec![0u8; 5]);
        assert!(read_pcm_block(&mut broken, &mut buffer).is_err());
    }

    #[test]
    fn pcm_clock_advances_on_real_samples_not_underrun_silence() {
        let (tx, rx) = bounded(2);
        let played = Arc::new(AtomicU64::new(0));
        let mut source = PcmSource {
            rx, current: Vec::new().into_iter(),
            completed: Arc::new(AtomicBool::new(false)),
            underruns: Arc::new(AtomicU64::new(0)),
            played_samples: Arc::clone(&played),
        };
        assert_eq!(source.next(), Some(0.0));
        assert_eq!(played.load(Ordering::Relaxed), 0);
        tx.send(vec![0.25, -0.25, 0.75, -0.75]).unwrap();
        for _ in 0..4 { assert!(source.next().is_some()); }
        assert_eq!(played.load(Ordering::Relaxed), 4);
        assert_eq!(source.next(), Some(0.0));
        assert_eq!(played.load(Ordering::Relaxed), 4);
    }

    #[test]
    fn audio_callback_does_not_block_without_samples() {
        let (_tx, rx) = bounded(1);
        let mut source = PcmSource {
            rx, current: Vec::new().into_iter(),
            completed: Arc::new(AtomicBool::new(false)),
            underruns: Arc::new(AtomicU64::new(0)),
            played_samples: Arc::new(AtomicU64::new(0)),
        };
        assert_eq!(source.next(), Some(0.0));
        assert_eq!(source.underruns.load(Ordering::Relaxed), 1);
        assert_eq!(source.played_samples.load(Ordering::Relaxed), 0);
        source.completed.store(true, Ordering::Release);
        assert_eq!(source.next(), None);
    }
}
