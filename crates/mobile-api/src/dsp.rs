//! DSP worker: consumes the bounded PCM queue on its own thread and runs the shared
//! pipeline. No database, network, or UI calls. Output goes to a bounded control
//! channel; if that channel fills, the epoch is failed rather than silently dropping.

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use squib_domain::{DetectorConfig, QualityEvent, QualityKind, Severity, TimestampQuality};
use squib_timing::block::{BlockHeader, EpochFormat, SampleFormat};
use squib_timing::calibration::{AmbientAnalyzer, AmbientStats};
use squib_timing::clock::FrameAnchor;
use squib_timing::cue::CueRequest;
use squib_timing::pipeline::{DspEvent, Pipeline, PipelineSummary};

use crate::queue::PcmQueue;

pub const OUT_CAPACITY: usize = 4096;
const HIST_BUCKET_NS: u64 = 50_000;
const HIST_BUCKETS: usize = 400;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WorkerMode {
    Run,
    /// Ambient statistics over the first `ambient_frames`, then probe detections.
    Calibration {
        ambient_frames: i64,
        highpass_hz: f32,
    },
}

pub enum WorkerCmd {
    RequestCue(CueRequest),
    Finish,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MappingSnapshot {
    pub reference: Option<FrameAnchor>,
    pub quality: TimestampQuality,
    pub sample_rate_hz: u32,
}

#[derive(Debug)]
pub enum WorkerOut {
    Dsp(DspEvent),
    Progress { processed_end: i64 },
    Mapping(MappingSnapshot),
    Ambient(AmbientStats),
    Finished(Box<PipelineSummary>),
}

/// Per-block processing-time histogram for the p99 < 50% block duration budget.
#[derive(Debug, Clone)]
pub struct ProcStats {
    pub blocks: u64,
    pub max_ns: u64,
    pub total_ns: u64,
    pub block_duration_ns: u64,
    hist: Vec<u64>,
}

impl ProcStats {
    fn new() -> Self {
        Self { blocks: 0, max_ns: 0, total_ns: 0, block_duration_ns: 0, hist: vec![0; HIST_BUCKETS + 1] }
    }
    fn record(&mut self, ns: u64, block_ns: u64) {
        self.blocks += 1;
        self.max_ns = self.max_ns.max(ns);
        self.total_ns += ns;
        self.block_duration_ns = block_ns;
        let i = ((ns / HIST_BUCKET_NS) as usize).min(HIST_BUCKETS);
        self.hist[i] += 1;
    }
    /// Upper bound of the bucket containing the requested quantile.
    pub fn quantile_ns(&self, q: f64) -> u64 {
        if self.blocks == 0 {
            return 0;
        }
        let target = ((self.blocks as f64) * q).ceil() as u64;
        let mut acc = 0;
        for (i, &c) in self.hist.iter().enumerate() {
            acc += c;
            if acc >= target {
                return if i == HIST_BUCKETS { self.max_ns } else { (i as u64 + 1) * HIST_BUCKET_NS };
            }
        }
        self.max_ns
    }
}

pub struct DspWorker {
    pub queue: Arc<PcmQueue>,
    cmd_tx: Sender<WorkerCmd>,
    pub out_rx: Receiver<WorkerOut>,
    thread: Option<JoinHandle<()>>,
    pub control_overflow: Arc<AtomicBool>,
    pub stats: Arc<Mutex<ProcStats>>,
    /// Frames processed with all resulting events already sent (diagnostics/tests).
    pub published_end: Arc<AtomicI64>,
}

impl DspWorker {
    pub fn spawn(fmt: EpochFormat, detector: Option<DetectorConfig>, queue: Arc<PcmQueue>, mode: WorkerMode) -> Self {
        let (cmd_tx, cmd_rx) = bounded::<WorkerCmd>(64);
        let (out_tx, out_rx) = bounded::<WorkerOut>(OUT_CAPACITY);
        let control_overflow = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(Mutex::new(ProcStats::new()));
        let q = queue.clone();
        let ov = control_overflow.clone();
        let st = stats.clone();
        let published_end = Arc::new(AtomicI64::new(0));
        let pe = published_end.clone();
        let thread = std::thread::Builder::new()
            .name("squib-dsp".into())
            .spawn(move || run_worker(fmt, detector, q, mode, cmd_rx, out_tx, ov, st, pe))
            .expect("spawn dsp thread");
        Self { queue, cmd_tx, out_rx, thread: Some(thread), control_overflow, stats, published_end }
    }

    pub fn request_cue(&self, r: CueRequest) {
        let _ = self.cmd_tx.send(WorkerCmd::RequestCue(r));
        self.wake();
    }

    pub fn finish(&self) {
        let _ = self.cmd_tx.send(WorkerCmd::Finish);
        self.wake();
    }

    pub fn wake(&self) {
        if let Some(t) = &self.thread {
            t.thread().unpark();
        }
    }

    pub fn thread_handle(&self) -> Option<std::thread::Thread> {
        self.thread.as_ref().map(|t| t.thread().clone())
    }

    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished())
    }
}

impl Drop for DspWorker {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(WorkerCmd::Finish);
        if let Some(t) = self.thread.take() {
            t.thread().unpark();
            let _ = t.join();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_worker(
    fmt: EpochFormat,
    detector: Option<DetectorConfig>,
    queue: Arc<PcmQueue>,
    mode: WorkerMode,
    cmd_rx: Receiver<WorkerCmd>,
    out_tx: Sender<WorkerOut>,
    overflow: Arc<AtomicBool>,
    stats: Arc<Mutex<ProcStats>>,
    published_end: Arc<AtomicI64>,
) {
    let mut p = Pipeline::new(fmt, detector);
    let mut ambient = match mode {
        WorkerMode::Calibration { highpass_hz, .. } => Some(AmbientAnalyzer::new(fmt.sample_rate_hz, highpass_hz)),
        WorkerMode::Run => None,
    };
    let ambient_frames = match mode {
        WorkerMode::Calibration { ambient_frames, .. } => ambient_frames,
        WorkerMode::Run => 0,
    };
    let mut ambient_done = ambient.is_none();
    let mut scratch: Vec<f32> = Vec::with_capacity(queue.max_frames());
    let mut finishing = false;
    let mut overflow_reported = false;
    let mut last_mapping: Option<MappingSnapshot> = None;
    let send = |o: WorkerOut| {
        if let Err(TrySendError::Full(_)) = out_tx.try_send(o) {
            overflow.store(true, Ordering::Release);
        }
    };
    loop {
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                WorkerCmd::RequestCue(r) => p.request_cue(r, &mut |e| send(WorkerOut::Dsp(e))),
                WorkerCmd::Finish => finishing = true,
            }
        }
        if !overflow_reported && queue.take_overflow() {
            overflow_reported = true;
            let end = p.processed_end();
            send(WorkerOut::Dsp(DspEvent::Quality(
                QualityEvent::new(
                    QualityKind::QueueOverflow,
                    Severity::Integrity,
                    "capture queue full; producer could not enqueue",
                )
                .frames(end, end),
            )));
            p.fail();
        }
        let mut did = false;
        while let Some(b) = queue.pop() {
            did = true;
            let h = BlockHeader {
                sequence: b.sequence,
                first_frame: b.first_frame,
                frame_count: b.frame_count,
                sample_rate_hz: fmt.sample_rate_hz,
                channels: 1,
                format: SampleFormat::Pcm16,
                anchor: b.anchor,
                delivered_ns: b.delivered_ns,
                flags: b.flags,
            };
            let t0 = Instant::now();
            let samples = &b.samples[..b.frame_count as usize];
            p.process_i16(&h, samples, &mut |e| send(WorkerOut::Dsp(e)));
            if !ambient_done && let Some(a) = ambient.as_mut() {
                scratch.clear();
                scratch.extend(samples.iter().map(|&s| f32::from(s) / 32768.0));
                a.push(&scratch);
                if p.processed_end() >= ambient_frames {
                    ambient_done = true;
                    if let Some(st) = a.stats() {
                        send(WorkerOut::Ambient(st));
                    }
                }
            }
            let ns = t0.elapsed().as_nanos() as u64;
            let block_ns = u64::from(b.frame_count) * 1_000_000_000 / u64::from(fmt.sample_rate_hz);
            if let Ok(mut s) = stats.lock() {
                s.record(ns, block_ns);
            }
            queue.recycle(b);
        }
        if did {
            send(WorkerOut::Progress { processed_end: p.processed_end() });
            let snap = MappingSnapshot {
                reference: p.clock().reference(),
                quality: p.clock().quality(),
                sample_rate_hz: fmt.sample_rate_hz,
            };
            if last_mapping != Some(snap) {
                last_mapping = Some(snap);
                send(WorkerOut::Mapping(snap));
            }
            published_end.store(p.processed_end(), Ordering::Release);
        }
        if finishing && queue.is_empty() {
            p.finish(&mut |e| send(WorkerOut::Dsp(e)));
            send(WorkerOut::Finished(Box::new(p.summary())));
            return;
        }
        if !did {
            std::thread::park_timeout(Duration::from_millis(5));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a22_producer_overflow_fails_epoch_without_blocking_producer() {
        let fmt = EpochFormat { sample_rate_hz: 48_000, channels: 1, format: SampleFormat::Pcm16, platform_buffer_frames: None };
        let q = Arc::new(PcmQueue::new(4, 480));
        let block = [0i16; 480];
        // Fill before the consumer starts: the 5th push must fail immediately.
        for i in 0..4 {
            assert_eq!(q.push(i, i as i64 * 480, &block, None, None), crate::queue::PushResult::Ok);
        }
        let t0 = Instant::now();
        assert_eq!(q.push(4, 4 * 480, &block, None, None), crate::queue::PushResult::Overflow);
        assert!(t0.elapsed() < Duration::from_millis(5), "producer never waits");
        let w = DspWorker::spawn(fmt, Some(DetectorConfig::default()), q.clone(), WorkerMode::Run);
        // The producer continues with the next block; the frames of block 4 are lost.
        while q.push(5, 5 * 480, &block, None, None) != crate::queue::PushResult::Ok {
            std::thread::yield_now();
        }
        w.wake();
        w.finish();
        let mut kinds = vec![];
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            match w.out_rx.recv_timeout(Duration::from_millis(50)) {
                Ok(WorkerOut::Dsp(DspEvent::Quality(q))) => kinds.push((q.kind, q.severity)),
                Ok(WorkerOut::Finished(s)) => {
                    assert!(s.integrity_failed);
                    break;
                }
                _ => {}
            }
        }
        assert!(kinds.contains(&(QualityKind::QueueOverflow, Severity::Integrity)), "{kinds:?}");
    }

    #[test]
    fn processing_histogram_quantiles() {
        let mut s = ProcStats::new();
        for i in 0..100 {
            s.record(if i == 99 { 2_000_000 } else { 40_000 }, 10_000_000);
        }
        assert_eq!(s.quantile_ns(0.5), 50_000);
        assert_eq!(s.quantile_ns(0.99), 50_000);
        assert_eq!(s.quantile_ns(1.0), 2_050_000);
    }
}
