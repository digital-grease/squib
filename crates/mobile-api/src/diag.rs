//! Opt-in diagnostic recording (M5 experiment 3). The DSP worker offers each block it
//! has processed to a `DiagTap`; a separate writer thread turns blocks into a mono
//! PCM16 WAV. The DSP thread never waits: blocks travel in preallocated buffers through
//! bounded channels, and when no buffer is free the block is skipped and later written
//! as silence with its frame range listed as a gap, so file positions stay exact.
//!
//! Nothing is written unless the run's configuration asked for it.

use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::thread::JoinHandle;

use crossbeam_channel::{Receiver, Sender, bounded};

/// Longest recording kept (frames are capped at this many seconds).
pub const MAX_SECONDS: i64 = 60;
/// Buffers in flight between the DSP thread and the writer (about 5 s at 48 kHz / 480).
pub const POOL_BUFFERS: usize = 512;

/// DSP-side handle. Dropping it ends the recording.
pub struct DiagTap {
    free_rx: Receiver<Vec<i16>>,
    /// Returns a buffer the tap could not use, so the pool never shrinks.
    free_back: Sender<Vec<i16>>,
    full_tx: Sender<(i64, Vec<i16>)>,
    dropped: Arc<AtomicI64>,
}

impl DiagTap {
    /// Called on the DSP thread after a block is processed. Never blocks or allocates
    /// (buffers are preallocated with the queue's maximum block size).
    pub fn offer(&self, first_frame: i64, samples: &[i16]) {
        match self.free_rx.try_recv() {
            Ok(mut buf) if buf.capacity() >= samples.len() => {
                buf.clear();
                buf.extend_from_slice(samples);
                if self.full_tx.try_send((first_frame, buf)).is_err() {
                    self.dropped.fetch_add(samples.len() as i64, Ordering::Relaxed);
                }
            }
            Ok(buf) => {
                let _ = self.free_back.try_send(buf);
                self.dropped.fetch_add(samples.len() as i64, Ordering::Relaxed);
            }
            Err(_) => {
                self.dropped.fetch_add(samples.len() as i64, Ordering::Relaxed);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiagOutcome {
    pub path: PathBuf,
    pub sample_rate_hz: u32,
    pub first_epoch_frame: i64,
    pub frames: i64,
    pub gaps: Vec<(i64, i64)>,
    pub truncated: bool,
    pub dropped_frames: i64,
    pub bytes: u64,
}

/// Writer side, owned by the engine until the run is finalized.
pub struct DiagWriter {
    pub relative_path: String,
    thread: Option<JoinHandle<std::io::Result<Option<DiagOutcome>>>>,
}

impl DiagWriter {
    /// Wait for the writer to finish (the tap must have been dropped). `None` when no
    /// audio arrived.
    pub fn join(mut self) -> std::io::Result<Option<DiagOutcome>> {
        match self.thread.take().map(|t| t.join()) {
            Some(Ok(r)) => r,
            Some(Err(_)) => Err(std::io::Error::other("diagnostic writer panicked")),
            None => Ok(None),
        }
    }
}

fn wav_header(w: &mut impl Write, rate: u32, data_bytes: u32) -> std::io::Result<()> {
    w.write_all(b"RIFF")?;
    w.write_all(&(36 + data_bytes).to_le_bytes())?;
    w.write_all(b"WAVEfmt ")?;
    w.write_all(&16u32.to_le_bytes())?;
    w.write_all(&1u16.to_le_bytes())?; // PCM
    w.write_all(&1u16.to_le_bytes())?; // mono
    w.write_all(&rate.to_le_bytes())?;
    w.write_all(&(rate * 2).to_le_bytes())?;
    w.write_all(&2u16.to_le_bytes())?;
    w.write_all(&16u16.to_le_bytes())?;
    w.write_all(b"data")?;
    w.write_all(&data_bytes.to_le_bytes())
}

/// Start a recording to `root/relative_path`. `max_block` is the largest block the
/// queue can deliver.
pub fn start(root: &Path, relative_path: &str, rate: u32, max_block: usize) -> std::io::Result<(DiagTap, DiagWriter)> {
    let path = root.join(relative_path);
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let partial = path.with_extension("wav.partial");
    let file = std::fs::File::create(&partial)?;
    let (free_tx, free_rx) = bounded::<Vec<i16>>(POOL_BUFFERS);
    let (full_tx, full_rx) = bounded::<(i64, Vec<i16>)>(POOL_BUFFERS);
    for _ in 0..POOL_BUFFERS {
        let _ = free_tx.send(Vec::with_capacity(max_block));
    }
    let free_back = free_tx.clone();
    let dropped = Arc::new(AtomicI64::new(0));
    let d2 = dropped.clone();
    let max_frames = MAX_SECONDS * i64::from(rate);
    let thread =
        std::thread::Builder::new().name("squib-diag-writer".into()).spawn(move || -> std::io::Result<Option<DiagOutcome>> {
            let mut w = BufWriter::with_capacity(64 * 1024, file);
            wav_header(&mut w, rate, 0)?;
            let mut first: Option<i64> = None;
            let mut written: i64 = 0;
            let mut gaps: Vec<(i64, i64)> = Vec::new();
            let mut truncated = false;
            let silence = vec![0u8; 4096];
            while let Ok((frame, buf)) = full_rx.recv() {
                let base = *first.get_or_insert(frame);
                let expected = base + written;
                if !truncated && frame > expected {
                    // Frames the recorder never received: silence, and say so.
                    let missing = (frame - expected).min(max_frames - written);
                    gaps.push((expected, expected + missing));
                    let mut left = missing * 2;
                    while left > 0 {
                        let n = left.min(silence.len() as i64) as usize;
                        w.write_all(&silence[..n])?;
                        left -= n as i64;
                    }
                    written += missing;
                }
                if !truncated && frame + buf.len() as i64 > base + written {
                    let skip = (base + written - frame).max(0) as usize;
                    let room = (max_frames - written).max(0) as usize;
                    let take = (buf.len() - skip).min(room);
                    for s in &buf[skip..skip + take] {
                        w.write_all(&s.to_le_bytes())?;
                    }
                    written += take as i64;
                    if take < buf.len() - skip || written >= max_frames {
                        truncated = written >= max_frames;
                    }
                }
                let _ = free_tx.try_send(buf);
            }
            let Some(base) = first else {
                drop(w);
                let _ = std::fs::remove_file(&partial);
                return Ok(None);
            };
            let data = (written * 2) as u32;
            w.seek(SeekFrom::Start(0))?;
            wav_header(&mut w, rate, data)?;
            let file = w.into_inner().map_err(|e| e.into_error())?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&partial, &path)?;
            Ok(Some(DiagOutcome {
                path,
                sample_rate_hz: rate,
                first_epoch_frame: base,
                frames: written,
                gaps,
                truncated,
                dropped_frames: d2.load(Ordering::Relaxed),
                bytes: 44 + u64::from(data),
            }))
        })?;
    Ok((
        DiagTap { free_rx, free_back, full_tx, dropped },
        DiagWriter { relative_path: relative_path.into(), thread: Some(thread) },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("squib-diag-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn pcm(path: &Path) -> Vec<i16> {
        let b = std::fs::read(path).unwrap();
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(b[40..44].try_into().unwrap()) as usize, b.len() - 44);
        b[44..].chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect()
    }

    #[test]
    fn blocks_are_written_in_order_and_gaps_become_listed_silence() {
        let d = dir("gaps");
        let (tap, w) = start(&d, "diagnostics/r1.wav", 1000, 4).unwrap();
        tap.offer(0, &[1, 2, 3, 4]);
        tap.offer(4, &[5, 6, 7, 8]);
        tap.offer(12, &[9, 10]); // frames 8..12 never arrived
        drop(tap);
        let o = w.join().unwrap().unwrap();
        assert_eq!((o.first_epoch_frame, o.frames, o.gaps.clone(), o.truncated), (0, 14, vec![(8, 12)], false));
        assert_eq!(pcm(&o.path), vec![1, 2, 3, 4, 5, 6, 7, 8, 0, 0, 0, 0, 9, 10]);
        assert!(!o.path.with_extension("wav.partial").exists());
    }

    #[test]
    fn length_is_capped_and_marked_truncated() {
        let d = dir("cap");
        let rate = 10; // 60 s = 600 frames
        let (tap, w) = start(&d, "x.wav", rate, 100).unwrap();
        let block = [7i16; 100];
        for i in 0..8 {
            tap.offer(i * 100, &block);
        }
        drop(tap);
        let o = w.join().unwrap().unwrap();
        assert_eq!((o.frames, o.truncated), (600, true));
        assert_eq!(pcm(&o.path).len(), 600);
    }

    #[test]
    fn no_audio_means_no_file() {
        let d = dir("empty");
        let (tap, w) = start(&d, "y.wav", 48_000, 480).unwrap();
        drop(tap);
        assert_eq!(w.join().unwrap(), None);
        assert!(!d.join("y.wav").exists() && !d.join("y.wav.partial").exists());
    }

    #[test]
    fn a_full_pool_drops_without_blocking() {
        let d = dir("full");
        let (tap, w) = start(&d, "z.wav", 48_000, 4).unwrap();
        // Oversized blocks cannot use the preallocated buffers: dropped, never allocated.
        tap.offer(0, &[1; 8]);
        tap.offer(8, &[2; 4]);
        drop(tap);
        let o = w.join().unwrap().unwrap();
        assert_eq!(o.dropped_frames, 8);
        assert_eq!(o.first_epoch_frame, 8, "the file starts at the first frame it received");
    }
}
