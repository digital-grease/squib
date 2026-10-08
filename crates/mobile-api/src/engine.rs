//! `SquibEngine`: the control-plane facade. One mutex-guarded actor processes commands
//! and worker/store results serially; the platform drives it with `poll(now)`.
//!
//! Monotonic timebase: all `*_ns` arguments are `CLOCK_MONOTONIC` nanoseconds
//! (Android `System.nanoTime()`, which AudioRecord/AudioTrack timestamps use with
//! `TIMEBASE_MONOTONIC`). Calendar times are UTC milliseconds supplied separately.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use squib_domain::*;
use squib_storage::{
    CalibrationRecord, CueObservation, EpochRecord, FinalRecord, ObservationBatch, RecoveredRun, Repository, RouteInfo,
    RunIntent, RunProgress, StorageError, classify_context,
};
use squib_timing::block::{EpochFormat, SampleFormat};
use squib_timing::calibration::{AmbientStats, CalibrationSuggestion, suggest_threshold};
use squib_timing::cue::{CueMatch, CueRequest, cue_pcm16};
use squib_timing::envelope::{EnvelopeChunk, dequantize_db};
use squib_timing::pipeline::{DspEvent, PipelineSummary};
use squib_timing::run::{Effect, Input, LifecycleKind, Phase, RunMachine, START_CUE_ID};

use crate::dsp::{DspWorker, MappingSnapshot, WorkerMode, WorkerOut};
use crate::ffi::*;
use crate::queue::PcmQueue;
use crate::registry;
use crate::store::{StoreActor, StoreCmd, StoreReply};

/// Observations are flushed to storage at least this often during a run.
pub const BATCH_WINDOW_NS: i64 = 250 * NS_PER_MS;
/// Ambient baseline required before arming a microphone run.
pub const BASELINE_MS: i64 = 500;
/// Capture queue target duration (docs/squib/04: roughly 250 ms).
pub const QUEUE_MS: u32 = 250;
/// Detector emission latency plus margin; drain waits this far past the cutoff.
pub const DRAIN_MARGIN_MS: i64 = 45;
const CALIBRATION_AMBIENT_S: i64 = 3;
const CUE_TEST_LEAD_MS: i64 = 600;
const COMMIT_WAIT_NS: i64 = 1_500 * NS_PER_MS;
pub const CLOCK_DOMAIN: &str = "CLOCK_MONOTONIC";
pub const CUE_GAIN: f32 = 0.5;

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn route_info(r: &RouteReport) -> RouteInfo {
    RouteInfo {
        input_device: r.input_device.clone(),
        output_device: r.output_device.clone(),
        audio_source: r.audio_source.clone(),
        unprocessed_supported: r.unprocessed_supported,
        effects: r.effects.clone(),
        os_build: r.os_build.clone(),
        device_model: r.device_model.clone(),
    }
}

struct Capture {
    handle: u64,
    worker: DspWorker,
    rate: u32,
    mapping: Option<MappingSnapshot>,
    processed_end: i64,
    summary: Option<PipelineSummary>,
    stop_requested: bool,
    overflow_reported: bool,
}

impl Capture {
    fn ns_to_frame(&self, ns: i64) -> Option<i64> {
        let r = self.mapping?.reference?;
        Some(r.frame + ns_to_frames(ns - r.mono_ns, self.rate))
    }
    fn frame_to_ns(&self, f: i64) -> Option<i64> {
        let r = self.mapping?.reference?;
        Some(r.mono_ns + frames_to_ns(f - r.frame, self.rate))
    }
    fn finished(&self) -> bool {
        self.summary.is_some()
    }
}

struct ActiveRun {
    run_id: String,
    session_id: String,
    config: RunConfig,
    created_utc_ms: i64,
    tz_offset_min: i32,
    candidates: Vec<Candidate>,
    quality: Vec<(u64, Option<String>, QualityEvent)>,
    envelope: Vec<EnvelopeChunk>,
    cue_matches: BTreeMap<u32, CueMatch>,
    epoch: Option<EpochRecord>,
    epoch_summary: Option<String>,
    rate: Option<u32>,
    sent: (usize, usize, usize),
    unsent_since_ns: Option<i64>,
    next_batch_seq: u64,
    durable_seq: u64,
    batch_failure_reported: bool,
    drain_cutoff_frame: Option<i64>,
    pending_commit: Option<(Outcome, i64)>,
    commit_inflight: bool,
    baseline_signalled: bool,
    timestamp_quality: Option<TimestampQuality>,
    progress_sent: Option<RunProgress>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeKind {
    Calibration,
    CueTest,
}

struct Probe {
    kind: ProbeKind,
    route: RouteReport,
    stage: String,
    ambient: Option<AmbientStats>,
    impulses: Vec<f32>,
    suggestion: Option<CalibrationSuggestion>,
    cue_issued_ns: Option<i64>,
    cue_requested: bool,
    cue_result: Option<CueMatch>,
    message: String,
    started_ns: i64,
}

struct Inner {
    machine: RunMachine,
    run: Option<ActiveRun>,
    capture: Option<Capture>,
    probe: Option<Probe>,
    effects: Vec<NativeEffect>,
}

#[derive(uniffi::Object)]
pub struct SquibEngine {
    inner: Mutex<Inner>,
    cond: Mutex<crate::conditions::CondState>,
    store: StoreActor,
    read: Mutex<Repository>,
    recovered: Vec<RecoveredRun>,
}

fn rejected(e: impl ToString) -> SquibError {
    SquibError::Rejected(e.to_string())
}

impl SquibEngine {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn read(&self) -> MutexGuard<'_, Repository> {
        self.read.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub(crate) fn run_active(&self) -> bool {
        let g = self.lock();
        g.machine.is_active() || g.probe.is_some()
    }

    pub(crate) fn current_run_id(&self) -> Option<String> {
        self.lock().machine.run_id.clone()
    }

    pub(crate) fn read_repo(&self) -> MutexGuard<'_, Repository> {
        self.read()
    }

    pub(crate) fn store_actor(&self) -> &StoreActor {
        &self.store
    }

    pub(crate) fn cond_state(&self) -> &Mutex<crate::conditions::CondState> {
        &self.cond
    }

    /// Feed an input to the machine and interpret effects until quiescent.
    fn feed(&self, g: &mut Inner, input: Input, now_ns: i64) -> Result<(), SquibError> {
        let mut queue = vec![input];
        while let Some(i) = queue.pop() {
            let fx = g.machine.handle(i).map_err(|r| rejected(r.reason))?;
            for e in fx {
                if let Some(next) = self.apply(g, e, now_ns) {
                    queue.insert(0, next);
                }
            }
        }
        Ok(())
    }

    fn apply(&self, g: &mut Inner, e: Effect, now_ns: i64) -> Option<Input> {
        match e {
            Effect::PersistIntent => {
                let run = g.run.as_ref()?;
                let intent = RunIntent {
                    run_id: run.run_id.clone(),
                    session_id: run.session_id.clone(),
                    created_utc_ms: run.created_utc_ms,
                    tz_offset_min: run.tz_offset_min,
                    config: run.config.clone(),
                };
                let r = self.store.call(|tx| StoreCmd::Intent(Box::new(intent), tx));
                Some(Input::IntentPersisted { ok: r.is_ok(), error: r.err().map(|e| e.to_string()), now_ns })
            }
            Effect::StartCapture => {
                g.effects.push(NativeEffect::StartCapture { preferred_rate_hz: 48_000 });
                None
            }
            Effect::StopCapture => {
                self.stop_capture(g);
                None
            }
            Effect::PlayCue { cue_id, kind, .. } => {
                g.effects.push(NativeEffect::PlayCue { cue_id, cue: kind.into() });
                None
            }
            Effect::ExpectCue { cue_id, kind, reference_ns, render_known } => {
                let cap = g.capture.as_ref();
                match cap.and_then(|c| c.ns_to_frame(reference_ns).map(|f| (c, f))) {
                    Some((c, frame)) => {
                        c.worker.request_cue(CueRequest::around(cue_id, kind, frame, render_known, c.rate));
                        None
                    }
                    None => {
                        let q = QualityEvent::new(
                            QualityKind::AnchorUnavailable,
                            Severity::Warning,
                            "no frame mapping for cue search",
                        )
                        .at(now_ns);
                        if let Some(r) = g.run.as_mut() {
                            let seq = r.quality.len() as u64;
                            r.quality.push((seq, None, q));
                        }
                        Some(Input::CueUnresolved { cue_id, now_ns })
                    }
                }
            }
            Effect::BeginDrain { cutoff_ns } => {
                let frame = g.capture.as_ref().and_then(|c| c.ns_to_frame(cutoff_ns).or(Some(c.processed_end)));
                if let Some(r) = g.run.as_mut() {
                    r.drain_cutoff_frame = frame;
                }
                if frame.is_none() {
                    return Some(Input::Drained { complete: false, now_ns });
                }
                None
            }
            Effect::CommitFinal { outcome } => {
                if let Some(r) = g.run.as_mut() {
                    r.pending_commit = Some((outcome, now_ns));
                    r.commit_inflight = false;
                }
                None
            }
            Effect::Quality(q) => {
                if let Some(r) = g.run.as_mut() {
                    let seq = r.quality.len() as u64;
                    let epoch = r.epoch.as_ref().map(|e| e.epoch_id.clone());
                    r.quality.push((seq, epoch, q));
                    r.unsent_since_ns.get_or_insert(now_ns);
                }
                None
            }
            Effect::KeepScreenOn(on) => {
                g.effects.push(NativeEffect::KeepScreenOn { on });
                None
            }
        }
    }

    fn stop_capture(&self, g: &mut Inner) {
        if let Some(c) = g.capture.as_mut()
            && !c.stop_requested
        {
            c.stop_requested = true;
            registry::unregister(c.handle);
            c.worker.finish();
            g.effects.push(NativeEffect::StopCapture);
        }
    }

    fn drain_worker(&self, g: &mut Inner, now_ns: i64) {
        let mut inputs = Vec::new();
        {
            let Some(cap) = g.capture.as_mut() else { return };
            if cap.worker.control_overflow.load(std::sync::atomic::Ordering::Acquire) && !cap.overflow_reported {
                cap.overflow_reported = true;
                inputs.push(Input::Quality {
                    event: QualityEvent::new(QualityKind::ControlQueueOverflow, Severity::Integrity, "DSP output queue full"),
                    now_ns,
                });
            }
            while let Ok(o) = cap.worker.out_rx.try_recv() {
                match o {
                    WorkerOut::Progress { processed_end } => cap.processed_end = processed_end,
                    WorkerOut::Mapping(m) => cap.mapping = Some(m),
                    WorkerOut::Finished(s) => {
                        if let Some(r) = g.run.as_mut() {
                            r.epoch_summary = serde_json::to_string(&*s).ok();
                            r.timestamp_quality = Some(s.clock.quality);
                        }
                        cap.summary = Some(*s);
                    }
                    WorkerOut::Ambient(a) => {
                        if let Some(p) = g.probe.as_mut()
                            && p.kind == ProbeKind::Calibration
                        {
                            p.ambient = Some(a);
                            p.stage = "impulses".into();
                            p.message =
                                "Ambient measured. Make 2–5 test impulses at normal range distance, then tap Finish.".into();
                        }
                    }
                    WorkerOut::Dsp(DspEvent::Candidate(c)) => {
                        if let Some(p) = g.probe.as_mut() {
                            let ambient_end = i64::from(cap.rate) * CALIBRATION_AMBIENT_S;
                            if p.kind == ProbeKind::Calibration && p.ambient.is_some() && c.onset_frame > ambient_end {
                                p.impulses.push(c.features.floor_ratio_db);
                            }
                        } else if let Some(r) = g.run.as_mut() {
                            r.candidates.push(c);
                            r.unsent_since_ns.get_or_insert(now_ns);
                        }
                    }
                    WorkerOut::Dsp(DspEvent::Envelope(e)) => {
                        if let Some(r) = g.run.as_mut() {
                            r.envelope.push(e);
                            r.unsent_since_ns.get_or_insert(now_ns);
                        }
                    }
                    WorkerOut::Dsp(DspEvent::Quality(q)) => {
                        if g.probe.is_some() {
                            if q.interrupts()
                                && let Some(p) = g.probe.as_mut()
                            {
                                p.stage = "failed".into();
                                p.message = format!("{}: {}", q.kind.label(), q.detail);
                            }
                        } else {
                            inputs.push(Input::Quality { event: q, now_ns });
                        }
                    }
                    WorkerOut::Dsp(DspEvent::Cue(m)) => {
                        if let Some(p) = g.probe.as_mut() {
                            p.cue_result = Some(m);
                            continue;
                        }
                        let onset_ns = m.onset_frame.and_then(|f| cap.frame_to_ns(f));
                        let cue_id = m.cue_id;
                        let input = match m.onset_frame {
                            Some(f) => Input::CueResolved { cue_id, onset_frame: f, onset_ns, now_ns },
                            None => Input::CueUnresolved { cue_id, now_ns },
                        };
                        if let Some(r) = g.run.as_mut() {
                            r.cue_matches.insert(cue_id, m);
                            r.unsent_since_ns.get_or_insert(now_ns);
                        }
                        inputs.push(input);
                    }
                }
            }
        }
        // Baseline readiness for microphone runs.
        if let (Some(cap), Some(run)) = (g.capture.as_ref(), g.run.as_mut()) {
            let baseline = i64::from(cap.rate) * BASELINE_MS / 1000;
            if g.machine.phase == Phase::Preparing && !run.baseline_signalled && cap.processed_end >= baseline {
                run.baseline_signalled = true;
                inputs.push(Input::CaptureReady { now_ns });
            }
            if g.machine.phase == Phase::Completing
                && let Some(cut) = run.drain_cutoff_frame
                && cap.processed_end >= cut + i64::from(cap.rate) * DRAIN_MARGIN_MS / 1000
            {
                run.drain_cutoff_frame = None;
                inputs.push(Input::Drained { complete: true, now_ns });
            }
        }
        for i in inputs {
            let _ = self.feed(g, i, now_ns);
        }
    }

    fn drain_store(&self, g: &mut Inner, now_ns: i64) {
        let mut inputs = Vec::new();
        while let Ok(r) = self.store.replies.try_recv() {
            match r {
                StoreReply::BatchAck { run_id, result, .. } => {
                    if let Some(run) = g.run.as_mut().filter(|x| x.run_id == run_id) {
                        match result {
                            Ok(seq) => run.durable_seq = run.durable_seq.max(seq),
                            Err(e) => {
                                if !run.batch_failure_reported {
                                    run.batch_failure_reported = true;
                                    inputs.push(Input::Quality {
                                        event: QualityEvent::new(
                                            QualityKind::StorageWriteFailed,
                                            Severity::Warning,
                                            e.to_string(),
                                        ),
                                        now_ns,
                                    });
                                }
                            }
                        }
                    }
                }
                StoreReply::Finalized { run_id, result } => {
                    if g.run.as_ref().is_some_and(|x| x.run_id == run_id) {
                        if let Some(run) = g.run.as_mut() {
                            run.commit_inflight = false;
                            if result.is_ok() {
                                run.pending_commit = None;
                                run.durable_seq = run.next_batch_seq;
                            }
                        }
                        inputs.push(Input::FinalCommitted {
                            ok: result.is_ok(),
                            error: result.err().map(|e| e.to_string()),
                            now_ns,
                        });
                    }
                }
            }
        }
        for i in inputs {
            let _ = self.feed(g, i, now_ns);
        }
    }

    fn cue_observations(g: &Inner) -> Vec<CueObservation> {
        let Some(run) = g.run.as_ref() else { return vec![] };
        let rate = run.rate;
        let epoch = run.epoch.as_ref().map(|e| e.epoch_id.clone());
        g.machine
            .cues
            .iter()
            .map(|c| {
                let m = run.cue_matches.get(&c.cue_id);
                let tlen = rate.map(|r| i64::from(c.kind.duration_ms()) * i64::from(r) / 1000).unwrap_or(0);
                let (ss, se, exact) = match m {
                    Some(m) if m.onset_frame.is_some() => (m.onset_frame, m.end_frame, Some(true)),
                    Some(m) => (Some(m.window_start_frame), Some(m.window_end_frame + tlen), Some(false)),
                    None => (None, None, None),
                };
                CueObservation {
                    cue_id: c.cue_id,
                    kind: c.kind,
                    par_index: c.par_index,
                    template_version: c.kind.template_version().into(),
                    requested_mono_ns: c.requested_ns,
                    issued_mono_ns: c.issued_ns,
                    render_mono_ns: c.render_ns,
                    epoch_id: if m.is_some() { epoch.clone() } else { None },
                    acoustic_onset_frame: c.acoustic_onset_frame,
                    acoustic_onset_mono_ns: c.acoustic_onset_ns,
                    match_ncc: m.map(|m| m.best_ncc),
                    span_start_frame: ss,
                    span_end_frame: se,
                    span_exact: exact,
                    missed: c.missed,
                    heard: c.heard,
                }
            })
            .collect()
    }

    fn progress(g: &Inner) -> RunProgress {
        RunProgress {
            start_method: g.machine.start_method,
            start_ref_frame: g.machine.start_ref_frame,
            start_ref_mono_ns: g.machine.start_ref_ns,
            armed_mono_ns: g.machine.armed_ns,
        }
    }

    fn maybe_flush(&self, g: &mut Inner, now_ns: i64) {
        let cues = Self::cue_observations(g);
        let progress = Self::progress(g);
        let Some(run) = g.run.as_mut() else { return };
        if run.pending_commit.is_some() || g.machine.persist == Some(PersistState::Saved) {
            return;
        }
        // A new start reference is flushed immediately so recovery after termination
        // keeps the run's origin; observations are batched within the window.
        let progress_changed = run.progress_sent.as_ref() != Some(&progress);
        if !progress_changed && run.unsent_since_ns.is_none_or(|t| now_ns - t < BATCH_WINDOW_NS) {
            return;
        }
        run.progress_sent = Some(progress.clone());
        run.next_batch_seq += 1;
        let epoch_id = run.epoch.as_ref().map(|e| e.epoch_id.clone()).unwrap_or_default();
        let batch = ObservationBatch {
            batch_seq: run.next_batch_seq,
            epoch: run.epoch.clone(),
            progress: Some(progress),
            cues,
            candidates: run.candidates[run.sent.0..].iter().map(|c| (epoch_id.clone(), c.clone())).collect(),
            quality: run.quality[run.sent.1..].to_vec(),
            envelope: run.envelope[run.sent.2..].iter().map(|e| (epoch_id.clone(), e.clone())).collect(),
        };
        run.sent = (run.candidates.len(), run.quality.len(), run.envelope.len());
        run.unsent_since_ns = None;
        self.store.send(StoreCmd::Batch { run_id: run.run_id.clone(), batch: Box::new(batch) });
    }

    fn classify_run(g: &Inner) -> Vec<ClassifiedCandidate> {
        let Some(run) = g.run.as_ref() else { return vec![] };
        let Some(rate) = run.rate else { return vec![] };
        let cues = Self::cue_observations(g);
        let stop = g.machine.stop_ns.and_then(|ns| g.capture.as_ref().and_then(|c| c.ns_to_frame(ns)));
        classify(&run.candidates, &classify_context(rate, g.machine.start_ref_frame, &cues, stop))
    }

    fn maybe_commit(&self, g: &mut Inner, now_ns: i64) {
        let ready = {
            let Some(run) = g.run.as_ref() else { return };
            let Some((_, since)) = run.pending_commit else { return };
            if run.commit_inflight {
                return;
            }
            g.capture.as_ref().is_none_or(|c| c.finished()) || now_ns - since > COMMIT_WAIT_NS
        };
        if !ready {
            return;
        }
        let classified = Self::classify_run(g);
        let cues = Self::cue_observations(g);
        let progress = Self::progress(g);
        let stop_frame = g.machine.stop_ns.and_then(|ns| g.capture.as_ref().and_then(|c| c.ns_to_frame(ns)));
        let interrupt = g.machine.interrupt_reason.map(|k| k.as_str().to_string());
        let mode = g.machine.config.as_ref().map(|c| c.source_mode);
        let resolved = g.machine.start_method == Some(StartReferenceMethod::AcousticCue);
        let Some(run) = g.run.as_mut() else { return };
        let Some((outcome, _)) = run.pending_commit else { return };
        let utc = run.created_utc_ms.max(0);
        let rev = (mode == Some(SourceMode::PhoneLive) && run.epoch.is_some())
            .then(|| initial_revision(&run.run_id, resolved, &classified, utc));
        let epoch_id = run.epoch.as_ref().map(|e| e.epoch_id.clone()).unwrap_or_default();
        run.next_batch_seq += 1;
        let remaining = ObservationBatch {
            batch_seq: run.next_batch_seq,
            epoch: run.epoch.clone(),
            progress: Some(progress),
            cues,
            candidates: run.candidates.iter().map(|c| (epoch_id.clone(), c.clone())).collect(),
            quality: run.quality.clone(),
            envelope: run.envelope.iter().map(|e| (epoch_id.clone(), e.clone())).collect(),
        };
        let f = FinalRecord {
            run_id: run.run_id.clone(),
            outcome,
            remaining,
            stop_mono_ns: g.machine.stop_ns,
            stop_frame,
            interrupt_reason: interrupt,
            epoch_summary: run.epoch.as_ref().and_then(|e| run.epoch_summary.clone().map(|s| (e.epoch_id.clone(), s))),
            initial_revision: rev,
            finalized_utc_ms: utc,
            error: None,
        };
        run.commit_inflight = true;
        self.store.send(StoreCmd::Finalize(Box::new(f)));
    }

    fn view(g: &mut Inner) -> EngineView {
        let m = &g.machine;
        let classified = Self::classify_run(g);
        let live: Vec<LiveEvent> = classified
            .iter()
            .filter(|c| c.classification != DetectorSuggestion::Rejected)
            .map(|c| LiveEvent {
                timeline_ns: c.timeline_ns,
                accepted: c.classification == DetectorSuggestion::SuggestedAccepted,
            })
            .collect();
        let accepted: Vec<i64> = live.iter().filter(|e| e.accepted).map(|e| e.timeline_ns).collect();
        let resolved = m.start_method.is_some_and(|s| s.supports_origin_relative_times()) && m.start_ref_frame.is_some();
        let pars_total = m.config.as_ref().map(|c| c.pars_ms.len() as u32).unwrap_or(0);
        let phase = match m.phase {
            Phase::Ready if g.probe.is_some() => "calibrating",
            Phase::Ready => "ready",
            Phase::Preparing => "preparing",
            Phase::Armed => "armed",
            Phase::AwaitingCue => "awaiting_cue",
            Phase::Running => "running",
            Phase::Completing => "completing",
            Phase::Interrupted => "interrupted",
            Phase::Saved => "saved",
            Phase::SavePending => "save_pending",
            Phase::Cancelled => "cancelled",
            Phase::Failed => "failed",
        };
        let tsq =
            g.capture.as_ref().and_then(|c| c.mapping.map(|m| m.quality)).or(g.run.as_ref().and_then(|r| r.timestamp_quality));
        EngineView {
            phase: phase.into(),
            run_id: m.run_id.clone(),
            mode: m.config.as_ref().map(|c| if c.source_mode == SourceMode::PhoneLive { Mode::PhoneLive } else { Mode::ParOnly }),
            outcome: m.outcome.map(|o| o.as_str().into()),
            persist: m.persist.map(|p| p.as_str().into()),
            start_method: m.start_method.map(|s| s.as_str().into()),
            timestamp_quality: tsq.map(|q| q.as_str().into()),
            error: m.error.clone(),
            interrupt_reason: m.interrupt_reason.map(|k| k.label().into()),
            accepted_count: accepted.len() as u32,
            uncertain_count: live.iter().filter(|e| !e.accepted).count() as u32,
            first_ns: if resolved { accepted.first().copied() } else { None },
            last_ns: if resolved { accepted.last().copied() } else { None },
            live_events: live,
            pars_played: m.cues.iter().filter(|c| c.kind == CueKind::Par && !c.missed).count() as u32,
            pars_total,
            quality_labels: m
                .quality
                .iter()
                .filter(|q| q.severity != Severity::Info)
                .map(|q| q.kind.label().to_string())
                .collect(),
            next_deadline_ns: m.next_deadline_ns(),
            durable_seq: g.run.as_ref().map(|r| r.durable_seq).unwrap_or(0),
            effects: std::mem::take(&mut g.effects),
            active: m.is_active(),
        }
    }

    fn tick_probe(&self, g: &mut Inner, now_ns: i64) {
        let Some(p) = g.probe.as_mut() else { return };
        if p.kind != ProbeKind::CueTest || p.stage == "failed" || p.stage == "done" {
            return;
        }
        let Some(cap) = g.capture.as_ref() else { return };
        if p.cue_issued_ns.is_none() && now_ns - p.started_ns >= CUE_TEST_LEAD_MS * NS_PER_MS && cap.processed_end > 0 {
            p.cue_issued_ns = Some(now_ns);
            p.stage = "playing".into();
            g.effects.push(NativeEffect::PlayCue { cue_id: START_CUE_ID, cue: CueType::Start });
        }
        if let Some(m) = p.cue_result.take() {
            p.stage = "done".into();
            p.message = if m.accepted() {
                format!("Cue heard (match score {:.2}). Acoustic start timing can be attempted on this route.", m.best_ncc)
            } else {
                format!(
                    "Cue not heard (best match score {:.2}). Raise media volume, uncover the microphone, or use par-only mode.",
                    m.best_ncc
                )
            };
            p.cue_result = Some(m);
            self.stop_capture(g);
        }
    }
}

#[uniffi::export]
impl SquibEngine {
    /// Open the journal at `db_path`, migrating and recovering interrupted runs.
    #[uniffi::constructor]
    pub fn new(db_path: String, now_utc_ms: i64) -> Result<Arc<Self>, SquibError> {
        let path = PathBuf::from(&db_path);
        let (store, recovered) = StoreActor::start(path.clone(), now_utc_ms)?;
        let read = Repository::open_read_only(&path)?;
        let engine = Arc::new(Self {
            inner: Mutex::new(Inner { machine: RunMachine::new(), run: None, capture: None, probe: None, effects: vec![] }),
            cond: Mutex::new(crate::conditions::CondState::default()),
            store,
            read: Mutex::new(read),
            recovered,
        });
        engine.restore_place();
        engine.seed_drills(now_utc_ms);
        Ok(engine)
    }

    /// Runs recovered as interrupted at startup (process termination).
    pub fn recovered_runs(&self) -> Vec<RecoveredRunView> {
        self.recovered
            .iter()
            .map(|r| RecoveredRunView {
                run_id: r.run_id.clone(),
                last_durable_seq: r.last_durable_seq,
                committed_candidates: r.committed_candidates,
            })
            .collect()
    }

    /// Cue PCM for platform playback; the same template the matcher searches for.
    pub fn cue_pcm(&self, cue: CueType, sample_rate_hz: u32) -> Vec<i16> {
        cue_pcm16(cue.into(), sample_rate_hz.clamp(8_000, 192_000), CUE_GAIN)
    }

    /// Route/cue preflight shown before Arm (A05, A23).
    pub fn preflight(&self, mode: Mode, route: RouteReport, expected_rate_hz: u32) -> Preflight {
        let mut blocking = Vec::new();
        let mut warn = Vec::new();
        let builtin_out = route.output_device == "builtin_speaker";
        match mode {
            Mode::ParOnly => {
                if route.media_volume <= 0.0 {
                    warn.push("Media volume is off: cues will be silent. The screen still shows the start.".to_string());
                }
                if !builtin_out {
                    warn.push(format!(
                        "Cue output is {}. Playback timing on this route is not verified; start time uses the platform render report.",
                        route.output_device
                    ));
                }
                warn.push("Par-only timing: no microphone is used and no shots are detected.".into());
                return Preflight {
                    can_arm: true,
                    level: RouteLevel::Experimental,
                    messages: warn,
                    route_signature: None,
                    calibration_id: None,
                    calibration_threshold_db: None,
                    calibration_label: None,
                };
            }
            Mode::PhoneLive => {
                if !route.mic_permission {
                    blocking.push("Microphone permission is off. Par-only mode remains available.".to_string());
                }
                if route.input_device != "builtin_mic" {
                    blocking.push(format!(
                        "Input is {}: only the built-in microphone is supported for acoustic timing. Disconnect it or use par-only.",
                        route.input_device
                    ));
                }
                if !builtin_out {
                    blocking.push(format!(
                        "Cue output is {}: the start cue must play from the built-in speaker to be heard by the microphone.",
                        route.output_device
                    ));
                }
                if route.media_volume <= 0.0 {
                    blocking
                        .push("Media volume is off: the start cue cannot be heard. Squib will not change your volume.".into());
                } else if route.media_volume < 0.3 {
                    warn.push("Media volume is low: run the cue test before arming.".into());
                }
                if route.audio_source != "unprocessed" {
                    warn.push("Raw (unprocessed) input is not available; voice-recognition input may alter onsets.".into());
                }
                if !route.effects.is_empty() {
                    warn.push(format!("Platform audio effects active: {}.", route.effects.join(", ")));
                }
                warn.push("Experimental: shot timing on this phone and route has not been field-qualified.".into());
            }
        }
        let sig = route_info(&route).signature(expected_rate_hz);
        let cal = self.read().latest_calibration(&sig).ok().flatten();
        if cal.is_none() {
            warn.push("No sensitivity setup for this route: default sensitivity will be used.".into());
        }
        let can = blocking.is_empty();
        blocking.extend(warn);
        Preflight {
            can_arm: can,
            level: if can { RouteLevel::Experimental } else { RouteLevel::Unsupported },
            messages: blocking,
            route_signature: Some(sig),
            calibration_id: cal.as_ref().map(|c| c.id.clone()),
            calibration_threshold_db: cal.as_ref().map(|c| c.detector_config.threshold_db),
            calibration_label: cal.and_then(|c| c.label),
        }
    }

    /// Arm a run. The configuration is persisted durably before the run is armed.
    pub fn arm(&self, cmd_id: String, req: ArmRequest, now_ns: i64) -> Result<EngineView, SquibError> {
        let mut g = self.lock();
        if g.probe.is_some() {
            return Err(rejected("finish sensitivity setup or the cue test first"));
        }
        if matches!(g.machine.phase, Phase::Saved | Phase::Cancelled | Phase::Failed) {
            let _ = g.machine.handle(Input::Reset);
            g.run = None;
            g.capture = None;
        }
        if g.machine.arm_cmd.as_deref() == Some(cmd_id.as_str()) {
            return Ok(Self::view(&mut g));
        }
        if g.machine.phase != Phase::Ready {
            return Err(rejected("a run is already in progress"));
        }
        if !(8_000..=192_000).contains(&req.expected_rate_hz) {
            return Err(SquibError::Invalid(format!("unsupported expected rate {} Hz", req.expected_rate_hz)));
        }
        let policy: DelayPolicy = req.delay.into();
        policy.validate().map_err(rejected)?;
        let mode: SourceMode = req.mode.into();
        let route = req.route.as_ref().map(route_info);
        let mut detector = None;
        let mut calibration_id = None;
        if mode == SourceMode::PhoneLive {
            let mut d = DetectorConfig::default();
            if let Some(cid) = &req.calibration_id
                && let Some(sig) = route.as_ref().map(|r| r.signature(req.expected_rate_hz))
                && let Ok(Some(c)) = self.read().latest_calibration(&sig)
                && &c.id == cid
            {
                d = c.detector_config.clone();
                calibration_id = Some(c.id);
            }
            if let Some(t) = req.threshold_db {
                d.threshold_db = t;
            }
            detector = Some(d);
        }
        let config = RunConfig {
            schema_version: DOMAIN_SCHEMA_VERSION,
            source_mode: mode,
            delay_policy: policy,
            selected_delay_ms: policy.select(req.unit_random),
            pars_ms: req.pars_ms.clone(),
            stop_policy: match req.auto_stop_grace_ms {
                Some(grace_ms) => StopPolicy::AfterLastPar { grace_ms },
                None => StopPolicy::Manual,
            },
            expected_count: req.expected_count,
            start_cue_template: CueKind::Start.template_version().into(),
            par_cue_template: CueKind::Par.template_version().into(),
            detector,
            calibration_profile_id: calibration_id,
            route_signature: if mode == SourceMode::PhoneLive {
                route.as_ref().map(|r| r.signature(req.expected_rate_hz))
            } else {
                None
            },
            shooter_id: self.active_shooter(),
            drill_version_id: req.drill_id.clone().zip(req.drill_version).map(|(d, v)| squib_storage::drill_ref(&d, v)),
            equipment_version_id: None,
            environment_snapshot_id: None,
            timestamp_mapping_method: TIMESTAMP_MAPPING_METHOD.into(),
            app_build: req.app_build.clone(),
        };
        // Conditions are pinned before the run intent so the snapshot reference is valid.
        let environment_snapshot_id = self.pin_conditions(req.now_utc_ms)?;
        let config = RunConfig { environment_snapshot_id, ..config };
        config.validate().map_err(rejected)?;
        let session_id = self.store.call(|reply| StoreCmd::Session {
            shooter: self.active_shooter(),
            new_id: new_id(),
            now_utc_ms: req.now_utc_ms,
            tz: req.tz_offset_min,
            reply,
        })?;
        let run_id = new_id();
        g.run = Some(ActiveRun {
            run_id: run_id.clone(),
            session_id,
            config: config.clone(),
            created_utc_ms: req.now_utc_ms,
            tz_offset_min: req.tz_offset_min,
            candidates: vec![],
            quality: vec![],
            envelope: vec![],
            cue_matches: BTreeMap::new(),
            epoch: None,
            epoch_summary: None,
            rate: None,
            sent: (0, 0, 0),
            unsent_since_ns: None,
            next_batch_seq: 0,
            durable_seq: 0,
            batch_failure_reported: false,
            drain_cutoff_frame: None,
            pending_commit: None,
            commit_inflight: false,
            baseline_signalled: false,
            timestamp_quality: None,
            progress_sent: None,
        });
        g.capture = None;
        self.feed(&mut g, Input::Arm { cmd_id, run_id, config, now_ns }, now_ns)?;
        Ok(Self::view(&mut g))
    }

    /// The platform opened capture with this actual format/route. Returns the data-plane
    /// handle for `PcmBridge.nativePush`, or 0 if capture is not expected now.
    pub fn capture_started(&self, meta: CaptureMeta, now_ns: i64) -> Result<u64, SquibError> {
        let mut g = self.lock();
        if g.capture.as_ref().is_some_and(|c| !c.stop_requested) {
            return Err(rejected("capture already open"));
        }
        let rate = meta.sample_rate_hz;
        if !(8_000..=192_000).contains(&rate) || meta.block_frames == 0 || meta.block_frames > 16_384 {
            return Err(SquibError::Invalid(format!("unsupported capture format {rate} Hz / {} frames", meta.block_frames)));
        }
        let fmt = EpochFormat {
            sample_rate_hz: rate,
            channels: 1,
            format: SampleFormat::Pcm16,
            platform_buffer_frames: meta.platform_buffer_frames,
        };
        let queue = Arc::new(PcmQueue::for_duration(rate, meta.block_frames as usize, QUEUE_MS));
        let (detector, mode) = if let Some(p) = g.probe.as_ref() {
            match p.kind {
                ProbeKind::Calibration => {
                    let probe = DetectorConfig { threshold_db: 12.0, attack_min_db: 6.0, ..DetectorConfig::default() };
                    let hp = probe.highpass_hz;
                    (
                        Some(probe),
                        WorkerMode::Calibration { ambient_frames: i64::from(rate) * CALIBRATION_AMBIENT_S, highpass_hz: hp },
                    )
                }
                ProbeKind::CueTest => (None, WorkerMode::Run),
            }
        } else {
            if g.machine.phase != Phase::Preparing || g.machine.mode_is_live() == Some(false) {
                return Ok(0);
            }
            let route = route_info(&meta.route);
            let expected = g.machine.config.as_ref().and_then(|c| c.route_signature.clone());
            if expected.as_deref() != Some(route.signature(rate).as_str()) {
                let reason = format!("actual route {} differs from preflight", route.signature(rate));
                self.feed(&mut g, Input::CaptureFailed { reason, now_ns }, now_ns)?;
                return Ok(0);
            }
            (g.machine.config.as_ref().and_then(|c| c.detector.clone()), WorkerMode::Run)
        };
        let worker = DspWorker::spawn(fmt, detector, queue.clone(), mode);
        let handle = registry::register(queue, worker.thread_handle());
        let epoch_id = new_id();
        let probing = g.probe.is_some();
        if let Some(r) = g.run.as_mut().filter(|_| !probing) {
            r.rate = Some(rate);
            r.epoch = Some(EpochRecord {
                epoch_id: epoch_id.clone(),
                run_id: r.run_id.clone(),
                sample_rate_hz: rate,
                channels: 1,
                sample_format: "pcm16".into(),
                route: route_info(&meta.route),
                clock_domain: meta.clock_domain.clone(),
                started_mono_ns: Some(meta.started_mono_ns),
            });
            r.unsent_since_ns.get_or_insert(now_ns);
        }
        g.capture = Some(Capture {
            handle,
            worker,
            rate,
            mapping: None,
            processed_end: 0,
            summary: None,
            stop_requested: false,
            overflow_reported: false,
        });
        Ok(handle)
    }

    pub fn capture_failed(&self, reason: String, now_ns: i64) -> EngineView {
        let mut g = self.lock();
        if let Some(p) = g.probe.as_mut() {
            p.stage = "failed".into();
            p.message = reason;
        } else {
            let _ = self.feed(&mut g, Input::CaptureFailed { reason, now_ns }, now_ns);
        }
        Self::view(&mut g)
    }

    /// Platform began playing a cue. `render_ns` is the playback timestamp of the cue's
    /// first frame when the platform reports it.
    pub fn cue_rendered(&self, cue_id: u32, render_ns: Option<i64>, now_ns: i64) {
        let mut g = self.lock();
        if let Some(p) = g.probe.as_mut() {
            if p.kind == ProbeKind::CueTest && !p.cue_requested {
                p.cue_requested = true;
                let reference = render_ns.unwrap_or(p.cue_issued_ns.unwrap_or(now_ns));
                if let Some(c) = g.capture.as_ref() {
                    match c.ns_to_frame(reference) {
                        Some(f) => {
                            c.worker.request_cue(CueRequest::around(cue_id, CueKind::Start, f, render_ns.is_some(), c.rate))
                        }
                        None => {
                            if let Some(p) = g.probe.as_mut() {
                                p.stage = "failed".into();
                                p.message = "No frame mapping available to locate the cue.".into();
                            }
                        }
                    }
                }
            }
            return;
        }
        let _ = self.feed(&mut g, Input::CueRendered { cue_id, render_ns, now_ns }, now_ns);
    }

    pub fn lifecycle(&self, event: LifecycleEvent, now_ns: i64) -> EngineView {
        let mut g = self.lock();
        if g.probe.is_some() {
            if let Some(p) = g.probe.as_mut() {
                p.stage = "failed".into();
                p.message = "Interrupted.".into();
            }
            self.stop_capture(&mut g);
        } else {
            let kind = match event {
                LifecycleEvent::ForegroundLost => LifecycleKind::ForegroundLost,
                LifecycleEvent::RouteChanged => LifecycleKind::RouteChanged,
                LifecycleEvent::MicSilenced => LifecycleKind::MicSilenced,
                LifecycleEvent::AudioFocusLost => LifecycleKind::AudioFocusLost,
                LifecycleEvent::PermissionRevoked => LifecycleKind::PermissionRevoked,
            };
            let _ = self.feed(&mut g, Input::Lifecycle { kind, now_ns }, now_ns);
        }
        Self::view(&mut g)
    }

    pub fn capture_error(&self, message: String, now_ns: i64) -> EngineView {
        let mut g = self.lock();
        let _ = self.feed(&mut g, Input::Lifecycle { kind: LifecycleKind::CaptureError(message), now_ns }, now_ns);
        Self::view(&mut g)
    }

    pub fn cancel(&self, cmd_id: String, now_ns: i64) -> Result<EngineView, SquibError> {
        let mut g = self.lock();
        self.feed(&mut g, Input::Cancel { cmd_id, now_ns }, now_ns)?;
        Ok(Self::view(&mut g))
    }

    pub fn stop(&self, cmd_id: String, now_ns: i64) -> Result<EngineView, SquibError> {
        let mut g = self.lock();
        self.feed(&mut g, Input::Stop { cmd_id, now_ns }, now_ns)?;
        Ok(Self::view(&mut g))
    }

    pub fn retry_save(&self, cmd_id: String, now_ns: i64) -> Result<EngineView, SquibError> {
        let mut g = self.lock();
        self.feed(&mut g, Input::RetrySave { cmd_id, now_ns }, now_ns)?;
        Ok(Self::view(&mut g))
    }

    /// Return to Ready after a terminal phase (Saved/Cancelled/Failed).
    pub fn reset(&self) -> Result<EngineView, SquibError> {
        let mut g = self.lock();
        g.machine.handle(Input::Reset).map_err(|r| rejected(r.reason))?;
        g.run = None;
        g.capture = None;
        Ok(Self::view(&mut g))
    }

    /// Drive the engine: drain DSP/storage results, advance deadlines, flush batches.
    pub fn poll(&self, now_ns: i64) -> EngineView {
        let mut g = self.lock();
        self.drain_worker(&mut g, now_ns);
        self.drain_store(&mut g, now_ns);
        if g.probe.is_none() {
            let _ = self.feed(&mut g, Input::Tick { now_ns }, now_ns);
        }
        self.tick_probe(&mut g, now_ns);
        self.maybe_flush(&mut g, now_ns);
        self.maybe_commit(&mut g, now_ns);
        if g.capture.as_ref().is_some_and(|c| c.stop_requested && c.finished()) && g.probe.is_none() {
            // Worker summary captured; keep mapping for the view until reset.
        }
        Self::view(&mut g)
    }

    pub fn capture_diagnostics(&self) -> Option<CaptureDiagnostics> {
        let g = self.lock();
        let c = g.capture.as_ref()?;
        let s = c.worker.stats.lock().ok()?.clone();
        let q = c.worker.queue.stats();
        let summary = c.summary.clone();
        Some(CaptureDiagnostics {
            blocks: s.blocks,
            processed_frames: c.worker.published_end.load(std::sync::atomic::Ordering::Acquire),
            dsp_p50_ns: s.quantile_ns(0.5),
            dsp_p99_ns: s.quantile_ns(0.99),
            dsp_max_ns: s.max_ns,
            block_duration_ns: s.block_duration_ns,
            queue_capacity_blocks: q.capacity_blocks,
            queue_high_water_blocks: q.high_water_blocks,
            queue_overflows: q.overflows,
            timestamp_quality: c.mapping.map(|m| m.quality).unwrap_or(TimestampQuality::Unavailable).as_str().into(),
            anchors_accepted: summary.as_ref().map(|s| s.clock.accepted).unwrap_or(0),
            anchors_rejected: summary.as_ref().map(|s| s.clock.rejected).unwrap_or(0),
            drift_ppm: summary.as_ref().and_then(|s| s.clock.drift_ppm),
            max_anchor_residual_ns: summary.as_ref().map(|s| s.clock.max_abs_residual_ns).unwrap_or(0),
            delivery_mean_ns: summary.as_ref().and_then(|s| s.delivery.mean_ns()),
            delivery_max_ns: summary.as_ref().map(|s| s.delivery.max_ns).unwrap_or(0),
        })
    }

    pub fn list_runs(&self, limit: u32) -> Result<Vec<RunListItem>, SquibError> {
        Ok(self
            .read()
            .list_runs(limit.min(1000))?
            .into_iter()
            .map(|s| RunListItem {
                run_id: s.row.run_id,
                created_utc_ms: s.row.created_utc_ms,
                tz_offset_min: s.row.tz_offset_min,
                mode: s.row.source_mode,
                outcome: s.row.outcome.map(|o| o.as_str().into()),
                persist: s.row.persist_state.as_str().into(),
                review_state: s.row.review_state.map(|r| r.as_str().into()),
                start_method: s.row.start_method.map(|m| m.as_str().into()),
                count: s.accepted_count,
                first_ns: s.first_ns,
                last_ns: s.last_ns,
                expected_count: s.expected_count,
                quality_warnings: s.quality_warnings,
                edited: s.edited,
            })
            .collect())
    }

    pub fn load_review(&self, run_id: String) -> Result<ReviewView, SquibError> {
        build_review(&self.read(), &run_id)
    }

    /// Apply review actions as a new revision on top of `base_revision`.
    pub fn apply_review(
        &self,
        run_id: String,
        base_revision: u32,
        actions: Vec<ReviewAction>,
        reason: Option<String>,
        now_utc_ms: i64,
    ) -> Result<ReviewView, SquibError> {
        let (latest, classified) = {
            let repo = self.read();
            let d = repo.load_run(&run_id)?;
            let latest = d.revisions.last().cloned().ok_or_else(|| SquibError::Invalid("run has no reviewable events".into()))?;
            (latest, d.classified)
        };
        if latest.content.number != base_revision {
            return Err(SquibError::Rejected(format!(
                "review is based on revision {base_revision} but revision {} exists; reload",
                latest.content.number
            )));
        }
        let mut acts = Vec::with_capacity(actions.len());
        for a in actions {
            acts.push(match a {
                ReviewAction::Accept { sequence } => RevisionAction::AcceptCandidate { sequence },
                ReviewAction::Reject { sequence } => RevisionAction::RejectCandidate { sequence },
                ReviewAction::AddManual { timeline_ns } => {
                    RevisionAction::AddManual { id: new_id()[..8].to_string(), timeline_ns }
                }
                ReviewAction::Move { reference, timeline_ns } => {
                    RevisionAction::MoveEvent { event: event_ref(&reference)?, timeline_ns }
                }
                ReviewAction::RemoveManual { manual_id } => RevisionAction::RemoveManual { id: manual_id },
                ReviewAction::Note { text } => RevisionAction::Note { text },
            });
        }
        let rev = apply_revision(&latest, &classified, acts, "local-device", now_utc_ms, reason)
            .map_err(|e| SquibError::Invalid(e.to_string()))?;
        self.store.call(|tx| StoreCmd::Revision(Box::new(rev), tx))?;
        build_review(&self.read(), &run_id)
    }

    // ---- Guided sensitivity setup and cue test -------------------------------------

    pub fn start_calibration(&self, route: RouteReport, now_ns: i64) -> Result<CalibrationView, SquibError> {
        self.start_probe(ProbeKind::Calibration, route, now_ns)
    }

    pub fn start_cue_test(&self, route: RouteReport, now_ns: i64) -> Result<CalibrationView, SquibError> {
        self.start_probe(ProbeKind::CueTest, route, now_ns)
    }

    pub fn probe_status(&self, now_ns: i64) -> CalibrationView {
        let mut g = self.lock();
        self.drain_worker(&mut g, now_ns);
        self.tick_probe(&mut g, now_ns);
        probe_view(&mut g)
    }

    /// Finish calibration; when `save`, store a route-scoped profile.
    pub fn finish_probe(
        &self,
        save: bool,
        label: Option<String>,
        now_utc_ms: i64,
        now_ns: i64,
    ) -> Result<CalibrationView, SquibError> {
        let mut g = self.lock();
        self.drain_worker(&mut g, now_ns);
        let rate = g.capture.as_ref().map(|c| c.rate).unwrap_or(48_000);
        self.stop_capture(&mut g);
        let Some(p) = g.probe.as_mut() else { return Err(rejected("no setup in progress")) };
        if p.kind == ProbeKind::Calibration {
            match p.ambient.clone() {
                None => {
                    p.stage = "failed".into();
                    p.message = "Ambient measurement incomplete; keep the phone still for 3 seconds.".into();
                }
                Some(a) => {
                    let s = suggest_threshold(&a, &p.impulses);
                    p.stage = "done".into();
                    p.message = match s.verdict {
                        squib_timing::calibration::CalibrationVerdict::Separated => "Test impulses are well above ambient.".into(),
                        squib_timing::calibration::CalibrationVerdict::AmbientOnly => "No test impulses detected; sensitivity set from ambient only.".into(),
                        squib_timing::calibration::CalibrationVerdict::PoorSeparation => {
                            "Test impulses are close to ambient noise: this setup may miss or invent events. Par-only remains available.".into()
                        }
                        squib_timing::calibration::CalibrationVerdict::AmbientClipped => "Ambient input clipped: move the phone farther or check the microphone.".into(),
                    };
                    if save {
                        let detector = DetectorConfig { threshold_db: s.threshold_db, ..DetectorConfig::default() };
                        let rec = CalibrationRecord {
                            id: new_id(),
                            route_signature: route_info(&p.route).signature(rate),
                            label: label.filter(|l| !l.trim().is_empty()).map(|l| l.chars().take(80).collect()),
                            algorithm_version: s.algorithm_version.clone(),
                            detector_config: detector,
                            suggestion_json: serde_json::to_string(&s).unwrap_or_default(),
                            verdict: serde_json::to_value(&s.verdict)
                                .ok()
                                .and_then(|v| v.as_str().map(String::from))
                                .unwrap_or_default(),
                            impulse_count: s.impulse_ratios_db.len() as u32,
                            os_build: Some(p.route.os_build.clone()),
                            created_utc_ms: now_utc_ms,
                        };
                        self.store.call(|tx| StoreCmd::Calibration(Box::new(rec), tx))?;
                    }
                    p.suggestion = Some(s);
                }
            }
        }
        let v = probe_view(&mut g);
        g.probe = None;
        Ok(v)
    }

    /// Testing aid: fail the next `count` storage writes (A08 / storage-failure flows).
    pub fn debug_inject_storage_faults(&self, count: u32) {
        self.store.send(StoreCmd::InjectFaults(count));
    }
}

impl SquibEngine {
    fn start_probe(&self, kind: ProbeKind, route: RouteReport, now_ns: i64) -> Result<CalibrationView, SquibError> {
        let mut g = self.lock();
        if g.machine.is_active() || g.probe.is_some() {
            return Err(rejected("busy"));
        }
        if !route.mic_permission {
            return Err(rejected("microphone permission is required for this check"));
        }
        let _ = g.machine.handle(Input::Reset);
        g.run = None;
        g.capture = None;
        g.probe = Some(Probe {
            kind,
            route,
            stage: if kind == ProbeKind::Calibration { "ambient".into() } else { "listening".into() },
            ambient: None,
            impulses: vec![],
            suggestion: None,
            cue_issued_ns: None,
            cue_requested: false,
            cue_result: None,
            message: if kind == ProbeKind::Calibration {
                "Keep the phone where it will sit during practice. Measuring ambient sound for 3 seconds…".into()
            } else {
                "Listening, then playing the start cue…".into()
            },
            started_ns: now_ns,
        });
        g.effects.push(NativeEffect::StartCapture { preferred_rate_hz: 48_000 });
        Ok(probe_view(&mut g))
    }
}

trait MachineExt {
    fn mode_is_live(&self) -> Option<bool>;
}

impl MachineExt for RunMachine {
    fn mode_is_live(&self) -> Option<bool> {
        self.config.as_ref().map(|c| c.source_mode == SourceMode::PhoneLive)
    }
}

fn probe_view(g: &mut Inner) -> CalibrationView {
    let effects = std::mem::take(&mut g.effects);
    match g.probe.as_ref() {
        None => CalibrationView {
            stage: "idle".into(),
            ambient_median_dbfs: None,
            ambient_p99_dbfs: None,
            clipped_fraction: None,
            impulse_ratios_db: vec![],
            suggested_threshold_db: None,
            verdict: None,
            message: String::new(),
            effects,
        },
        Some(p) => CalibrationView {
            stage: p.stage.clone(),
            ambient_median_dbfs: p.ambient.as_ref().map(|a| a.median_dbfs),
            ambient_p99_dbfs: p.ambient.as_ref().map(|a| a.p99_dbfs),
            clipped_fraction: p.ambient.as_ref().map(|a| a.clipped_fraction),
            impulse_ratios_db: p.impulses.clone(),
            suggested_threshold_db: p.suggestion.as_ref().map(|s| s.threshold_db),
            verdict: p
                .suggestion
                .as_ref()
                .and_then(|s| serde_json::to_value(&s.verdict).ok())
                .and_then(|v| v.as_str().map(String::from)),
            message: p.message.clone(),
            effects,
        },
    }
}

fn event_ref(r: &EventRefFfi) -> Result<EventRef, SquibError> {
    match (r.kind.as_str(), r.sequence, &r.manual_id) {
        ("candidate", Some(sequence), _) => Ok(EventRef::Candidate { sequence }),
        ("manual", _, Some(id)) => Ok(EventRef::Manual { id: id.clone() }),
        _ => Err(SquibError::Invalid("bad event reference".into())),
    }
}

fn reason_label(r: CandidateReason) -> &'static str {
    match r {
        CandidateReason::LowMargin => "Low margin",
        CandidateReason::PossibleEcho => "Possible echo",
        CandidateReason::Clipped => "Clipped input",
        CandidateReason::PreCue => "Before start",
        CandidateReason::OverlapsStartCue => "During start cue",
        CandidateReason::OverlapsParCue => "During par cue",
        CandidateReason::AppCue => "App cue",
        CandidateReason::AfterStop => "After stop",
        CandidateReason::NoStartReference => "No start reference",
    }
}

pub(crate) fn build_review(repo: &Repository, run_id: &str) -> Result<ReviewView, SquibError> {
    let d = repo.load_run(run_id)?;
    let rate = d.epoch.as_ref().map(|e| e.record.sample_rate_hz);
    let latest = d.revisions.last();
    let resolved = latest.map(|r| r.content.origin_resolved).unwrap_or(false);
    let to_timeline = |frame: i64| -> Option<i64> {
        let rate = rate?;
        Some(match d.row.start_ref_frame {
            Some(s) => frames_to_ns(frame - s, rate),
            None => frames_to_ns(frame, rate),
        })
    };
    let mut events = Vec::new();
    for c in &d.classified {
        let ev = EventRef::Candidate { sequence: c.sequence };
        let accepted = latest.and_then(|r| r.content.accepted.iter().find(|e| e.event == ev));
        let state = match (accepted, latest) {
            (Some(_), _) => "accepted",
            (None, Some(r)) if r.content.unresolved.contains(&c.sequence) => "uncertain",
            (None, Some(_)) => "rejected",
            (None, None) => c.classification.as_str(),
        };
        events.push(ReviewEvent {
            reference: EventRefFfi { kind: "candidate".into(), sequence: Some(c.sequence), manual_id: None },
            timeline_ns: accepted.map(|a| a.timeline_ns).unwrap_or(c.timeline_ns),
            state: state.into(),
            origin: match accepted.map(|a| a.origin) {
                Some(EventOrigin::Moved) => "moved".into(),
                _ => "detected".into(),
            },
            reasons: c.reasons.iter().map(|r| reason_label(*r).to_string()).collect(),
            detector_score: Some(c.detector_score),
        });
    }
    if let Some(r) = latest {
        for a in &r.content.accepted {
            if let EventRef::Manual { id } = &a.event {
                events.push(ReviewEvent {
                    reference: EventRefFfi { kind: "manual".into(), sequence: None, manual_id: Some(id.clone()) },
                    timeline_ns: a.timeline_ns,
                    state: "accepted".into(),
                    origin: "manual".into(),
                    reasons: vec![],
                    detector_score: None,
                });
            }
        }
    }
    events.sort_by_key(|e| e.timeline_ns);
    let results = latest.map(|r| compute_results(r, d.config.expected_count));
    let cues = d
        .cues
        .iter()
        .map(|c| {
            let label = match c.kind {
                CueKind::Start => "Start cue".to_string(),
                CueKind::Par => format!("Par {}", c.par_index.map(|i| i + 1).unwrap_or(0)),
            };
            let t = match (c.acoustic_onset_frame, c.kind) {
                (Some(f), _) => to_timeline(f),
                (None, CueKind::Par) => {
                    d.config.pars_ms.get(c.par_index.unwrap_or(0) as usize).map(|p| i64::from(*p) * NS_PER_MS)
                }
                (None, CueKind::Start) => {
                    d.row.start_method.filter(|m| *m != StartReferenceMethod::Unresolved && rate.is_none()).map(|_| 0)
                }
            };
            CueMarker { label, timeline_ns: t, heard: c.heard, missed: c.missed }
        })
        .collect();
    let quality = d
        .quality
        .iter()
        .map(|q| QualityView {
            label: q.kind.label().into(),
            severity: q.severity.as_str().into(),
            detail: q.detail.clone(),
            timeline_ns: q.start_frame.and_then(to_timeline),
        })
        .collect();
    let (energy, hop_ns, start_ns) = match (rate, d.envelope.first()) {
        (Some(rate), Some(first)) => {
            let mut v = Vec::new();
            for c in &d.envelope {
                for pair in c.data.chunks(2) {
                    v.push(dequantize_db(pair[0]));
                }
            }
            (v, frames_to_ns(i64::from(first.hop_frames), rate), to_timeline(first.first_frame).unwrap_or(0))
        }
        _ => (vec![], 0, 0),
    };
    let tsq = d
        .epoch
        .as_ref()
        .and_then(|e| e.summary_json.as_ref())
        .and_then(|s| serde_json::from_str::<PipelineSummary>(s).ok())
        .map(|s| s.clock.quality.as_str().to_string());
    Ok(ReviewView {
        run_id: d.row.run_id.clone(),
        created_utc_ms: d.row.created_utc_ms,
        mode: d.row.source_mode.clone(),
        outcome: d.row.outcome.map(|o| o.as_str().into()),
        persist: d.row.persist_state.as_str().into(),
        review_state: d.row.review_state.map(|r| r.as_str().into()),
        start_method: d.row.start_method.map(|m| m.as_str().into()),
        timestamp_quality: tsq,
        origin_resolved: resolved,
        revision_number: latest.map(|r| r.content.number).unwrap_or(0),
        edited: latest.is_some_and(|r| r.is_edited()),
        uncommitted_tail_possible: d.row.uncommitted_tail_possible,
        interrupt_reason: d.row.interrupt_reason.clone(),
        events,
        cues,
        quality,
        count: results.as_ref().map(|r| r.count).unwrap_or(0),
        first_ns: results.as_ref().and_then(|r| r.first_ns),
        last_ns: results.as_ref().and_then(|r| r.last_ns),
        splits_ns: results.as_ref().map(|r| r.splits_ns.clone()).unwrap_or_default(),
        zero_splits: results.as_ref().map(|r| r.zero_splits).unwrap_or(0),
        expected_count: d.config.expected_count,
        energy_db: energy,
        energy_hop_ns: hop_ns,
        energy_start_ns: start_ns,
        pars_ms: d.config.pars_ms.clone(),
        delay_ms: d.config.selected_delay_ms,
        detector_version: d.row.detector_version.clone(),
        threshold_db: d.config.detector.as_ref().map(|x| x.threshold_db),
    })
}

impl From<StorageError> for Box<SquibError> {
    fn from(e: StorageError) -> Self {
        Box::new(e.into())
    }
}
