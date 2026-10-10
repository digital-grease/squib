//! Single-owner run state machine (docs/squib/04 "Run state machine").
//!
//! A pure reducer: `handle(input) -> effects`. It never reads clocks; every input
//! carries the monotonic time supplied by the adapter. The engine interprets effects
//! (persist, start capture, play cue, drain, commit) and feeds results back as inputs.
//!
//! Start references:
//! - `phone_live`: the start cue template onset found in the microphone stream of the
//!   same capture epoch (`AcousticCue`). If not found, the run is interrupted; a loud
//!   transient is never promoted to a start (A04).
//! - `par_only`: the platform playback timestamp of the cue's first frame
//!   (`ScheduledRender`), or only the request time when that is unavailable
//!   (`RequestedOnly`, degraded and labelled). Never claimed as microphone-verified.

use serde::{Deserialize, Serialize};
use squib_domain::{
    CueKind, NS_PER_MS, Outcome, PersistState, QualityEvent, QualityKind, RunConfig, Severity, SourceMode, StartReferenceMethod,
    StopPolicy,
};

/// A start cue played this late after its deadline is reported (diagnostic).
pub const CUE_LATE_REPORT_NS: i64 = 20 * NS_PER_MS;
/// A par cue that cannot start within this lateness is skipped and reported missed.
pub const PAR_MISS_TOLERANCE_NS: i64 = 250 * NS_PER_MS;
/// Playback must report rendering within this time of the request.
pub const RENDER_TIMEOUT_NS: i64 = 2_000 * NS_PER_MS;
/// Acoustic cue resolution must complete within this time of rendering.
pub const CUE_RESOLVE_TIMEOUT_NS: i64 = 3_000 * NS_PER_MS;
/// Capture preparation (open + ambient baseline) must finish within this time.
pub const PREPARE_TIMEOUT_NS: i64 = 5_000 * NS_PER_MS;
/// Drain after stop must finish within this time or the tail is unprocessed.
pub const DRAIN_TIMEOUT_NS: i64 = 2_000 * NS_PER_MS;

pub const START_CUE_ID: u32 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Ready,
    Preparing,
    Armed,
    AwaitingCue,
    Running,
    Completing,
    Interrupted,
    Saved,
    SavePending,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LifecycleKind {
    ForegroundLost,
    RouteChanged,
    MicSilenced,
    AudioFocusLost,
    PermissionRevoked,
    CaptureError(String),
}

impl LifecycleKind {
    fn quality(&self) -> QualityEvent {
        let (k, d) = match self {
            LifecycleKind::ForegroundLost => (QualityKind::ForegroundLost, String::new()),
            LifecycleKind::RouteChanged => (QualityKind::RouteChanged, String::new()),
            LifecycleKind::MicSilenced => (QualityKind::MicSilenced, String::new()),
            LifecycleKind::AudioFocusLost => (QualityKind::AudioFocusLost, String::new()),
            LifecycleKind::PermissionRevoked => (QualityKind::PermissionRevoked, String::new()),
            LifecycleKind::CaptureError(e) => (QualityKind::CaptureError, e.clone()),
        };
        QualityEvent::new(k, Severity::Integrity, d)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)] // Arm is rare; boxing adds noise to every match.
pub enum Input {
    /// Request to arm with a validated, immutable configuration.
    Arm {
        cmd_id: String,
        run_id: String,
        config: RunConfig,
        now_ns: i64,
    },
    /// Repository acknowledged (or failed) the durable run intent.
    IntentPersisted {
        ok: bool,
        error: Option<String>,
        now_ns: i64,
    },
    /// Capture is open with an ambient baseline (microphone modes).
    CaptureReady {
        now_ns: i64,
    },
    CaptureFailed {
        reason: String,
        now_ns: i64,
    },
    Tick {
        now_ns: i64,
    },
    /// Platform began rendering a cue. `render_ns` is the platform playback timestamp
    /// of the cue's first frame when available.
    CueRendered {
        cue_id: u32,
        render_ns: Option<i64>,
        now_ns: i64,
    },
    /// Acoustic match result for a cue (microphone modes).
    CueResolved {
        cue_id: u32,
        onset_frame: i64,
        onset_ns: Option<i64>,
        now_ns: i64,
    },
    CueUnresolved {
        cue_id: u32,
        now_ns: i64,
    },
    Quality {
        event: QualityEvent,
        now_ns: i64,
    },
    Lifecycle {
        kind: LifecycleKind,
        now_ns: i64,
    },
    Cancel {
        cmd_id: String,
        now_ns: i64,
    },
    Stop {
        cmd_id: String,
        now_ns: i64,
    },
    /// DSP processed through the stop cutoff (`complete`) or timed out.
    Drained {
        complete: bool,
        now_ns: i64,
    },
    FinalCommitted {
        ok: bool,
        error: Option<String>,
        now_ns: i64,
    },
    RetrySave {
        cmd_id: String,
        now_ns: i64,
    },
    /// Return to Ready from a terminal phase (Repeat / new attempt).
    Reset,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Effect {
    PersistIntent,
    StartCapture,
    StopCapture,
    PlayCue {
        cue_id: u32,
        kind: CueKind,
        par_index: Option<u32>,
        requested_ns: i64,
    },
    /// Search the capture stream for this cue around `reference_ns`.
    ExpectCue {
        cue_id: u32,
        kind: CueKind,
        reference_ns: i64,
        render_known: bool,
    },
    BeginDrain {
        cutoff_ns: i64,
    },
    CommitFinal {
        outcome: Outcome,
    },
    Quality(QualityEvent),
    KeepScreenOn(bool),
}

/// Observation of one app cue for persistence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CueRecord {
    pub cue_id: u32,
    pub kind: CueKind,
    pub par_index: Option<u32>,
    pub requested_ns: i64,
    pub issued_ns: i64,
    pub render_ns: Option<i64>,
    pub acoustic_onset_frame: Option<i64>,
    pub acoustic_onset_ns: Option<i64>,
    pub missed: bool,
    pub heard: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rejection {
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunMachine {
    pub phase: Phase,
    pub run_id: Option<String>,
    pub config: Option<RunConfig>,
    pub arm_cmd: Option<String>,
    pub stop_cmd: Option<String>,
    pub outcome: Option<Outcome>,
    pub persist: Option<PersistState>,
    pub error: Option<String>,
    pub interrupt_reason: Option<QualityKind>,
    pub prepare_started_ns: Option<i64>,
    pub armed_ns: Option<i64>,
    start_deadline_ns: Option<i64>,
    pub start_method: Option<StartReferenceMethod>,
    pub start_ref_ns: Option<i64>,
    pub start_ref_frame: Option<i64>,
    pub stop_ns: Option<i64>,
    drain_started_ns: Option<i64>,
    resolve_from_ns: Option<i64>,
    pub tail_complete: Option<bool>,
    pub cues: Vec<CueRecord>,
    next_par: usize,
    cue_busy_until_ns: i64,
    auto_stop_ns: Option<i64>,
    pub quality: Vec<QualityEvent>,
}

impl Default for RunMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl RunMachine {
    pub fn new() -> Self {
        Self {
            phase: Phase::Ready,
            run_id: None,
            config: None,
            arm_cmd: None,
            stop_cmd: None,
            outcome: None,
            persist: None,
            error: None,
            interrupt_reason: None,
            prepare_started_ns: None,
            armed_ns: None,
            start_deadline_ns: None,
            start_method: None,
            start_ref_ns: None,
            start_ref_frame: None,
            stop_ns: None,
            drain_started_ns: None,
            resolve_from_ns: None,
            tail_complete: None,
            cues: vec![],
            next_par: 0,
            cue_busy_until_ns: i64::MIN,
            auto_stop_ns: None,
            quality: vec![],
        }
    }

    /// Whether a run is in progress (shooter/profile switching is disabled).
    pub fn is_active(&self) -> bool {
        matches!(
            self.phase,
            Phase::Preparing | Phase::Armed | Phase::AwaitingCue | Phase::Running | Phase::Completing | Phase::Interrupted
        )
    }

    /// Next monotonic time at which a `Tick` would change state, for adapter scheduling.
    /// Never exposed to the UI as a countdown.
    pub fn next_deadline_ns(&self) -> Option<i64> {
        match self.phase {
            Phase::Armed => self.start_deadline_ns,
            Phase::Running => {
                let par = self.par_deadline(self.next_par);
                match (par, self.auto_stop_ns) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                }
            }
            _ => None,
        }
    }

    fn mode(&self) -> Option<SourceMode> {
        self.config.as_ref().map(|c| c.source_mode)
    }

    fn par_deadline(&self, i: usize) -> Option<i64> {
        let base = self.start_ref_ns?;
        let par = *self.config.as_ref()?.pars_ms.get(i)?;
        Some(base + i64::from(par) * NS_PER_MS)
    }

    fn record_quality(&mut self, q: QualityEvent, fx: &mut Vec<Effect>) {
        self.quality.push(q.clone());
        fx.push(Effect::Quality(q));
    }

    fn interrupt(&mut self, q: QualityEvent, fx: &mut Vec<Effect>) {
        let kind = q.kind;
        self.record_quality(q, fx);
        match self.phase {
            Phase::Preparing => {
                self.phase = Phase::Failed;
                self.error = Some(kind.label().to_string());
                self.outcome = Some(Outcome::FailedToStart);
                self.persist = Some(PersistState::Pending);
                fx.push(Effect::StopCapture);
                fx.push(Effect::CommitFinal { outcome: Outcome::FailedToStart });
                fx.push(Effect::KeepScreenOn(false));
            }
            Phase::Armed | Phase::AwaitingCue | Phase::Running | Phase::Completing => {
                self.phase = Phase::Interrupted;
                self.interrupt_reason = Some(kind);
                self.outcome = Some(Outcome::Interrupted);
                self.persist = Some(PersistState::Pending);
                fx.push(Effect::StopCapture);
                fx.push(Effect::CommitFinal { outcome: Outcome::Interrupted });
                fx.push(Effect::KeepScreenOn(false));
            }
            _ => {}
        }
    }

    fn enter_armed(&mut self, now_ns: i64) {
        let delay = self.config.as_ref().map(|c| i64::from(c.selected_delay_ms)).unwrap_or(0);
        self.phase = Phase::Armed;
        self.armed_ns = Some(now_ns);
        // Delay is measured from entry into Armed, after preparation, not the UI tap.
        self.start_deadline_ns = Some(now_ns + delay * NS_PER_MS);
    }

    fn begin_running(&mut self, method: StartReferenceMethod, ref_ns: Option<i64>, ref_frame: Option<i64>) {
        self.phase = Phase::Running;
        self.start_method = Some(method);
        self.start_ref_ns = ref_ns;
        self.start_ref_frame = ref_frame;
        if let (Some(cfg), Some(base)) = (&self.config, ref_ns)
            && let (StopPolicy::AfterLastPar { grace_ms }, Some(&last)) = (cfg.stop_policy, cfg.pars_ms.last())
        {
            self.auto_stop_ns = Some(base + (i64::from(last) + i64::from(grace_ms)) * NS_PER_MS);
        }
    }

    fn begin_completing(&mut self, now_ns: i64, fx: &mut Vec<Effect>) {
        self.phase = Phase::Completing;
        self.stop_ns = Some(now_ns);
        self.drain_started_ns = Some(now_ns);
        match self.mode() {
            Some(SourceMode::PhoneLive) => fx.push(Effect::BeginDrain { cutoff_ns: now_ns }),
            _ => {
                self.tail_complete = Some(true);
                self.commit(Outcome::Complete, fx);
            }
        }
    }

    fn commit(&mut self, outcome: Outcome, fx: &mut Vec<Effect>) {
        self.outcome = Some(outcome);
        self.persist = Some(PersistState::Pending);
        fx.push(Effect::StopCapture);
        fx.push(Effect::CommitFinal { outcome });
        fx.push(Effect::KeepScreenOn(false));
    }

    pub fn handle(&mut self, input: Input) -> Result<Vec<Effect>, Rejection> {
        let mut fx = Vec::new();
        let reject = |r: &str| Err(Rejection { reason: r.to_string() });
        match input {
            Input::Arm { cmd_id, run_id, config, now_ns } => {
                if self.arm_cmd.as_deref() == Some(cmd_id.as_str()) {
                    return Ok(fx); // duplicate delivery of the same command
                }
                if self.phase != Phase::Ready {
                    return reject("a run is already in progress");
                }
                if !config.source_mode.armable() {
                    return reject("this mode records an existing result and is not armed");
                }
                if let Err(e) = config.validate() {
                    return reject(&e.to_string());
                }
                *self = RunMachine::new();
                self.phase = Phase::Preparing;
                self.run_id = Some(run_id);
                self.config = Some(config);
                self.arm_cmd = Some(cmd_id);
                self.prepare_started_ns = Some(now_ns);
                fx.push(Effect::KeepScreenOn(true));
                fx.push(Effect::PersistIntent);
            }
            Input::IntentPersisted { ok, error, now_ns } => {
                if self.phase != Phase::Preparing {
                    return Ok(fx);
                }
                if !ok {
                    // Nothing durable exists; no run record can be claimed.
                    self.phase = Phase::Failed;
                    self.error = Some(format!("could not save run intent: {}", error.unwrap_or_default()));
                    self.persist = Some(PersistState::Failed);
                    fx.push(Effect::KeepScreenOn(false));
                    return Ok(fx);
                }
                if self.mode() == Some(SourceMode::PhoneLive) {
                    fx.push(Effect::StartCapture);
                } else {
                    self.enter_armed(now_ns);
                }
            }
            Input::CaptureReady { now_ns } => {
                if self.phase == Phase::Preparing && self.mode() == Some(SourceMode::PhoneLive) {
                    self.enter_armed(now_ns);
                }
            }
            Input::CaptureFailed { reason, now_ns } => {
                let q = QualityEvent::new(QualityKind::CaptureError, Severity::Integrity, reason).at(now_ns);
                self.interrupt(q, &mut fx);
            }
            Input::Tick { now_ns } => self.tick(now_ns, &mut fx),
            Input::CueRendered { cue_id, render_ns, now_ns } => self.on_rendered(cue_id, render_ns, now_ns, &mut fx),
            Input::CueResolved { cue_id, onset_frame, onset_ns, .. } => {
                if let Some(c) = self.cues.iter_mut().find(|c| c.cue_id == cue_id) {
                    c.acoustic_onset_frame = Some(onset_frame);
                    c.acoustic_onset_ns = onset_ns;
                    c.heard = Some(true);
                }
                if cue_id == START_CUE_ID && self.phase == Phase::AwaitingCue {
                    // Par deadlines need a monotonic base; fall back to the render time
                    // when the epoch mapping is unavailable (start origin stays acoustic).
                    let base = onset_ns.or_else(|| self.cues.first().and_then(|c| c.render_ns.or(Some(c.issued_ns))));
                    self.begin_running(StartReferenceMethod::AcousticCue, base, Some(onset_frame));
                }
            }
            Input::CueUnresolved { cue_id, now_ns } => {
                if let Some(c) = self.cues.iter_mut().find(|c| c.cue_id == cue_id) {
                    c.heard = Some(false);
                }
                if cue_id == START_CUE_ID && self.phase == Phase::AwaitingCue {
                    let q = QualityEvent::new(QualityKind::CueUnresolved, Severity::Integrity, "start cue template not found")
                        .at(now_ns);
                    self.interrupt(q, &mut fx);
                } else if cue_id != START_CUE_ID {
                    let q =
                        QualityEvent::new(QualityKind::ParCueNotHeard, Severity::Info, format!("par cue {cue_id}")).at(now_ns);
                    self.record_quality(q, &mut fx);
                }
            }
            Input::Quality { event, .. } => {
                if event.interrupts() && self.is_active() && self.phase != Phase::Interrupted {
                    self.interrupt(event, &mut fx);
                } else {
                    self.record_quality(event, &mut fx);
                }
            }
            Input::Lifecycle { kind, now_ns } => {
                if self.is_active() && self.phase != Phase::Interrupted {
                    self.interrupt(kind.quality().at(now_ns), &mut fx);
                }
            }
            Input::Cancel { cmd_id, now_ns } => match self.phase {
                Phase::Preparing | Phase::Armed | Phase::AwaitingCue => {
                    let _ = (cmd_id, now_ns);
                    self.phase = Phase::Cancelled;
                    self.commit(Outcome::Cancelled, &mut fx);
                }
                Phase::Cancelled => {}
                _ => return reject("cancel is only available before the start cue; use Stop"),
            },
            Input::Stop { cmd_id, now_ns } => {
                if self.stop_cmd.as_deref() == Some(cmd_id.as_str()) {
                    return Ok(fx);
                }
                match self.phase {
                    Phase::Running => {
                        self.stop_cmd = Some(cmd_id);
                        self.begin_completing(now_ns, &mut fx);
                    }
                    Phase::Completing | Phase::Interrupted | Phase::Saved | Phase::SavePending => {}
                    Phase::Preparing | Phase::Armed | Phase::AwaitingCue => {
                        // Stop before the start reference is a cancel: no timing exists.
                        self.phase = Phase::Cancelled;
                        self.commit(Outcome::Cancelled, &mut fx);
                    }
                    _ => return reject("no run to stop"),
                }
            }
            Input::Drained { complete, now_ns } => {
                if self.phase == Phase::Completing {
                    self.tail_complete = Some(complete);
                    if complete {
                        self.commit(Outcome::Complete, &mut fx);
                    } else {
                        let q =
                            QualityEvent::new(QualityKind::TailUnprocessed, Severity::Integrity, "drain timed out").at(now_ns);
                        self.interrupt(q, &mut fx);
                    }
                }
            }
            Input::FinalCommitted { ok, error, .. } => {
                if ok {
                    self.persist = Some(PersistState::Saved);
                    if matches!(self.phase, Phase::Completing | Phase::Interrupted | Phase::SavePending) {
                        self.phase = Phase::Saved;
                    }
                } else {
                    self.persist = Some(PersistState::Failed);
                    self.error = error;
                    if matches!(self.phase, Phase::Completing | Phase::Interrupted) {
                        self.phase = Phase::SavePending;
                    }
                }
            }
            Input::RetrySave { .. } => {
                if self.phase == Phase::SavePending {
                    if let Some(o) = self.outcome {
                        self.persist = Some(PersistState::Pending);
                        fx.push(Effect::CommitFinal { outcome: o });
                    }
                } else {
                    return reject("nothing to retry");
                }
            }
            Input::Reset => match self.phase {
                Phase::Saved | Phase::Cancelled | Phase::Failed | Phase::Ready => *self = RunMachine::new(),
                Phase::SavePending => return reject("save the run before starting another"),
                _ => return reject("a run is in progress"),
            },
        }
        Ok(fx)
    }

    fn tick(&mut self, now_ns: i64, fx: &mut Vec<Effect>) {
        match self.phase {
            Phase::Preparing if self.prepare_started_ns.is_some_and(|t| now_ns - t > PREPARE_TIMEOUT_NS) => {
                let q =
                    QualityEvent::new(QualityKind::CaptureError, Severity::Integrity, "capture preparation timed out").at(now_ns);
                self.interrupt(q, fx);
            }
            Phase::Armed => {
                let Some(deadline) = self.start_deadline_ns else { return };
                if now_ns < deadline {
                    return;
                }
                let late = now_ns - deadline;
                if late > CUE_LATE_REPORT_NS {
                    let q = QualityEvent::new(
                        QualityKind::CueLate,
                        Severity::Info,
                        format!("start cue {} ms late", late / NS_PER_MS),
                    )
                    .at(now_ns);
                    self.record_quality(q, fx);
                }
                self.cues.push(CueRecord {
                    cue_id: START_CUE_ID,
                    kind: CueKind::Start,
                    par_index: None,
                    requested_ns: deadline,
                    issued_ns: now_ns,
                    render_ns: None,
                    acoustic_onset_frame: None,
                    acoustic_onset_ns: None,
                    missed: false,
                    heard: None,
                });
                self.cue_busy_until_ns = now_ns + i64::from(CueKind::Start.duration_ms()) * NS_PER_MS;
                self.phase = Phase::AwaitingCue;
                fx.push(Effect::PlayCue { cue_id: START_CUE_ID, kind: CueKind::Start, par_index: None, requested_ns: deadline });
            }
            Phase::AwaitingCue => {
                let Some(c) = self.cues.first() else { return };
                let rendered = c.render_ns.or(self.resolve_from_ns);
                match rendered {
                    None if now_ns - c.issued_ns > RENDER_TIMEOUT_NS => {
                        let q = QualityEvent::new(
                            QualityKind::CueRenderUnavailable,
                            Severity::Integrity,
                            "start cue playback not confirmed",
                        )
                        .at(now_ns);
                        self.interrupt(q, fx);
                    }
                    Some(r) if now_ns - r > CUE_RESOLVE_TIMEOUT_NS => {
                        let q =
                            QualityEvent::new(QualityKind::CueUnresolved, Severity::Integrity, "start cue resolution timed out")
                                .at(now_ns);
                        self.interrupt(q, fx);
                    }
                    _ => {}
                }
            }
            Phase::Running => {
                while let Some(deadline) = self.par_deadline(self.next_par) {
                    if now_ns < deadline {
                        break;
                    }
                    let late = now_ns - deadline;
                    if late <= PAR_MISS_TOLERANCE_NS && now_ns < self.cue_busy_until_ns {
                        break; // previous cue still playing; cues never overlap
                    }
                    let i = self.next_par;
                    self.next_par += 1;
                    let cue_id = (i + 1) as u32;
                    let mut rec = CueRecord {
                        cue_id,
                        kind: CueKind::Par,
                        par_index: Some(i as u32),
                        requested_ns: deadline,
                        issued_ns: now_ns,
                        render_ns: None,
                        acoustic_onset_frame: None,
                        acoustic_onset_ns: None,
                        missed: false,
                        heard: None,
                    };
                    if late > PAR_MISS_TOLERANCE_NS {
                        rec.missed = true;
                        self.cues.push(rec);
                        let q = QualityEvent::new(
                            QualityKind::CueMissed,
                            Severity::Warning,
                            format!("par {} skipped, {} ms late", i + 1, late / NS_PER_MS),
                        )
                        .at(now_ns);
                        self.record_quality(q, fx);
                        continue;
                    }
                    if late > CUE_LATE_REPORT_NS {
                        let q = QualityEvent::new(
                            QualityKind::CueLate,
                            Severity::Info,
                            format!("par {} {} ms late", i + 1, late / NS_PER_MS),
                        )
                        .at(now_ns);
                        self.record_quality(q, fx);
                    }
                    self.cues.push(rec);
                    self.cue_busy_until_ns = now_ns + i64::from(CueKind::Par.duration_ms()) * NS_PER_MS;
                    fx.push(Effect::PlayCue { cue_id, kind: CueKind::Par, par_index: Some(i as u32), requested_ns: deadline });
                    break; // at most one cue starts per tick
                }
                if self.auto_stop_ns.is_some_and(|t| now_ns >= t) {
                    self.auto_stop_ns = None;
                    self.stop_cmd = Some("auto-stop".into());
                    self.begin_completing(now_ns, fx);
                }
            }
            Phase::Completing
                if self.drain_started_ns.is_some_and(|t| now_ns - t > DRAIN_TIMEOUT_NS) && self.tail_complete.is_none() =>
            {
                self.tail_complete = Some(false);
                let q = QualityEvent::new(QualityKind::TailUnprocessed, Severity::Integrity, "drain timed out").at(now_ns);
                self.interrupt(q, fx);
            }
            _ => {}
        }
    }

    fn on_rendered(&mut self, cue_id: u32, render_ns: Option<i64>, now_ns: i64, fx: &mut Vec<Effect>) {
        let Some(c) = self.cues.iter_mut().find(|c| c.cue_id == cue_id) else { return };
        if c.render_ns.is_some() {
            return;
        }
        c.render_ns = render_ns;
        let issued = c.issued_ns;
        let kind = c.kind;
        if render_ns.is_none() {
            let q = QualityEvent::new(
                QualityKind::CueRenderUnavailable,
                Severity::Warning,
                format!("cue {cue_id}: only request time known"),
            )
            .at(now_ns);
            self.record_quality(q, fx);
        }
        let mic = self.mode() == Some(SourceMode::PhoneLive);
        if cue_id == START_CUE_ID {
            if self.phase != Phase::AwaitingCue {
                return;
            }
            if mic {
                // Resolution timeout runs from the render (or request) time.
                self.resolve_from_ns = Some(render_ns.unwrap_or(issued));
                fx.push(Effect::ExpectCue {
                    cue_id,
                    kind,
                    reference_ns: render_ns.unwrap_or(issued),
                    render_known: render_ns.is_some(),
                });
            } else {
                match render_ns {
                    Some(r) => self.begin_running(StartReferenceMethod::ScheduledRender, Some(r), None),
                    None => self.begin_running(StartReferenceMethod::RequestedOnly, Some(issued), None),
                }
            }
        } else if mic && self.phase == Phase::Running {
            fx.push(Effect::ExpectCue {
                cue_id,
                kind,
                reference_ns: render_ns.unwrap_or(issued),
                render_known: render_ns.is_some(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use squib_domain::{DelayPolicy, DetectorConfig, TIMESTAMP_MAPPING_METHOD};

    const MS: i64 = NS_PER_MS;

    fn cfg(mode: SourceMode, delay: u32, pars: Vec<u32>, stop: StopPolicy) -> RunConfig {
        RunConfig {
            schema_version: 1,
            source_mode: mode,
            delay_policy: DelayPolicy::Fixed { ms: delay },
            selected_delay_ms: delay,
            pars_ms: pars,
            stop_policy: stop,
            expected_count: Some(3),
            start_cue_template: CueKind::Start.template_version().into(),
            par_cue_template: CueKind::Par.template_version().into(),
            detector: (mode == SourceMode::PhoneLive).then(DetectorConfig::default),
            calibration_profile_id: None,
            route_signature: None,
            shooter_id: "default".into(),
            drill_version_id: None,
            equipment_version_id: None,
            environment_snapshot_id: None,
            timestamp_mapping_method: TIMESTAMP_MAPPING_METHOD.into(),
            capture_video: false,
            diagnostic_recording: false,
            app_build: "test".into(),
        }
    }

    fn arm(m: &mut RunMachine, c: RunConfig, now: i64) -> Vec<Effect> {
        m.handle(Input::Arm { cmd_id: "a1".into(), run_id: "r1".into(), config: c, now_ns: now }).unwrap()
    }

    fn par_only_running(pars: Vec<u32>, stop: StopPolicy) -> RunMachine {
        let mut m = RunMachine::new();
        arm(&mut m, cfg(SourceMode::ParOnly, 2000, pars, stop), 1_000 * MS);
        m.handle(Input::IntentPersisted { ok: true, error: None, now_ns: 1_010 * MS }).unwrap();
        let fx = m.handle(Input::Tick { now_ns: 3_010 * MS }).unwrap();
        assert!(fx.iter().any(|e| matches!(e, Effect::PlayCue { kind: CueKind::Start, .. })));
        m.handle(Input::CueRendered { cue_id: 0, render_ns: Some(3_040 * MS), now_ns: 3_045 * MS }).unwrap();
        assert_eq!(m.phase, Phase::Running);
        m
    }

    #[test]
    fn par_only_full_cycle_without_capture() {
        let mut m = RunMachine::new();
        let fx = arm(&mut m, cfg(SourceMode::ParOnly, 2000, vec![1000], StopPolicy::Manual), 1_000 * MS);
        assert_eq!(fx, vec![Effect::KeepScreenOn(true), Effect::PersistIntent]);
        assert_eq!(m.phase, Phase::Preparing);
        let fx = m.handle(Input::IntentPersisted { ok: true, error: None, now_ns: 1_010 * MS }).unwrap();
        assert!(!fx.contains(&Effect::StartCapture), "par-only never opens the microphone");
        assert_eq!(m.phase, Phase::Armed);
        // Delay measured from Armed (1010 ms), not the tap (1000 ms).
        assert!(m.handle(Input::Tick { now_ns: 3_005 * MS }).unwrap().is_empty());
        assert_eq!(m.next_deadline_ns(), Some(3_010 * MS));
        m.handle(Input::Tick { now_ns: 3_010 * MS }).unwrap();
        assert_eq!(m.phase, Phase::AwaitingCue);
        m.handle(Input::CueRendered { cue_id: 0, render_ns: Some(3_040 * MS), now_ns: 3_045 * MS }).unwrap();
        assert_eq!(m.start_method, Some(StartReferenceMethod::ScheduledRender));
        assert_eq!(m.start_ref_ns, Some(3_040 * MS));
        let fx = m.handle(Input::Tick { now_ns: 4_040 * MS }).unwrap();
        assert!(fx.iter().any(|e| matches!(e, Effect::PlayCue { kind: CueKind::Par, par_index: Some(0), .. })));
        let fx = m.handle(Input::Stop { cmd_id: "s".into(), now_ns: 5_000 * MS }).unwrap();
        assert!(fx.contains(&Effect::CommitFinal { outcome: Outcome::Complete }));
        assert_eq!(m.persist, Some(PersistState::Pending));
        assert_eq!(m.phase, Phase::Completing, "not Saved before durable ack");
        m.handle(Input::FinalCommitted { ok: true, error: None, now_ns: 5_010 * MS }).unwrap();
        assert_eq!(m.phase, Phase::Saved);
        assert_eq!(m.outcome, Some(Outcome::Complete));
        m.handle(Input::Reset).unwrap();
        assert_eq!(m.phase, Phase::Ready);
    }

    #[test]
    fn instant_delay_cues_on_first_tick() {
        let mut m = RunMachine::new();
        arm(&mut m, cfg(SourceMode::ParOnly, 0, vec![], StopPolicy::Manual), 0);
        m.handle(Input::IntentPersisted { ok: true, error: None, now_ns: 5 }).unwrap();
        let fx = m.handle(Input::Tick { now_ns: 5 }).unwrap();
        assert!(fx.iter().any(|e| matches!(e, Effect::PlayCue { .. })));
    }

    #[test]
    fn missing_render_timestamp_is_degraded_not_hidden() {
        let mut m = RunMachine::new();
        arm(&mut m, cfg(SourceMode::ParOnly, 0, vec![], StopPolicy::Manual), 0);
        m.handle(Input::IntentPersisted { ok: true, error: None, now_ns: 1 }).unwrap();
        m.handle(Input::Tick { now_ns: 2 }).unwrap();
        let fx = m.handle(Input::CueRendered { cue_id: 0, render_ns: None, now_ns: 10 }).unwrap();
        assert_eq!(m.start_method, Some(StartReferenceMethod::RequestedOnly));
        assert!(fx.iter().any(|e| matches!(e, Effect::Quality(q) if q.kind == QualityKind::CueRenderUnavailable)));
    }

    #[test]
    fn duplicate_arm_and_stop_are_idempotent_and_busy_rejected() {
        let mut m = par_only_running(vec![], StopPolicy::Manual);
        assert!(
            m.handle(Input::Arm {
                cmd_id: "a1".into(),
                run_id: "r1".into(),
                config: cfg(SourceMode::ParOnly, 0, vec![], StopPolicy::Manual),
                now_ns: 0
            })
            .unwrap()
            .is_empty()
        );
        assert!(
            m.handle(Input::Arm {
                cmd_id: "a2".into(),
                run_id: "r2".into(),
                config: cfg(SourceMode::ParOnly, 0, vec![], StopPolicy::Manual),
                now_ns: 0
            })
            .is_err()
        );
        let fx1 = m.handle(Input::Stop { cmd_id: "s".into(), now_ns: 9_000 * MS }).unwrap();
        let fx2 = m.handle(Input::Stop { cmd_id: "s".into(), now_ns: 9_001 * MS }).unwrap();
        assert!(!fx1.is_empty());
        assert!(fx2.is_empty());
        assert!(m.handle(Input::Stop { cmd_id: "s2".into(), now_ns: 9_002 * MS }).unwrap().is_empty());
    }

    #[test]
    fn cancel_before_cue_and_not_after_start() {
        let mut m = RunMachine::new();
        arm(&mut m, cfg(SourceMode::ParOnly, 3000, vec![], StopPolicy::Manual), 0);
        m.handle(Input::IntentPersisted { ok: true, error: None, now_ns: 1 }).unwrap();
        let fx = m.handle(Input::Cancel { cmd_id: "c".into(), now_ns: 100 }).unwrap();
        assert_eq!(m.phase, Phase::Cancelled);
        assert!(fx.contains(&Effect::CommitFinal { outcome: Outcome::Cancelled }));
        let mut m = par_only_running(vec![], StopPolicy::Manual);
        assert!(m.handle(Input::Cancel { cmd_id: "c".into(), now_ns: 1 }).is_err());
    }

    #[test]
    fn invalid_config_rejected_before_any_effect() {
        let mut m = RunMachine::new();
        let mut c = cfg(SourceMode::ParOnly, 0, vec![], StopPolicy::Manual);
        c.delay_policy = DelayPolicy::Random { min_ms: 5, max_ms: 1 };
        assert!(m.handle(Input::Arm { cmd_id: "a".into(), run_id: "r".into(), config: c, now_ns: 0 }).is_err());
        assert_eq!(m.phase, Phase::Ready);
    }

    #[test]
    fn intent_persist_failure_blocks_arming() {
        let mut m = RunMachine::new();
        arm(&mut m, cfg(SourceMode::ParOnly, 0, vec![], StopPolicy::Manual), 0);
        let fx = m.handle(Input::IntentPersisted { ok: false, error: Some("disk full".into()), now_ns: 1 }).unwrap();
        assert_eq!(m.phase, Phase::Failed);
        assert_eq!(m.persist, Some(PersistState::Failed));
        assert!(!fx.iter().any(|e| matches!(e, Effect::PlayCue { .. } | Effect::StartCapture)));
        assert!(m.handle(Input::Tick { now_ns: 10_000 * MS }).unwrap().is_empty(), "no hidden countdown");
    }

    #[test]
    fn delayed_tick_misses_par_and_overlapping_cues_are_serialized() {
        let mut m = par_only_running(vec![1000, 1300, 1600], StopPolicy::Manual);
        // Tick arrives 400 ms after par 1 deadline (start ref 3040 → par1 at 4040).
        let fx = m.handle(Input::Tick { now_ns: 4_440 * MS }).unwrap();
        // par 1 (400 ms late) missed; par 2 (100 ms late) plays; par 3 not yet due.
        assert!(fx.iter().any(|e| matches!(e, Effect::Quality(q) if q.kind == QualityKind::CueMissed)));
        let plays: Vec<_> = fx.iter().filter(|e| matches!(e, Effect::PlayCue { .. })).collect();
        assert_eq!(plays.len(), 1);
        assert!(matches!(plays[0], Effect::PlayCue { par_index: Some(1), .. }));
        // Par 3 due at 4640 while par 2 plays until 4560: plays once due.
        let fx = m.handle(Input::Tick { now_ns: 4_640 * MS }).unwrap();
        assert!(fx.iter().any(|e| matches!(e, Effect::PlayCue { par_index: Some(2), .. })));
        assert_eq!(m.cues.iter().filter(|c| c.missed).count(), 1);
    }

    #[test]
    fn auto_stop_after_last_par() {
        let mut m = par_only_running(vec![1000], StopPolicy::AfterLastPar { grace_ms: 500 });
        m.handle(Input::Tick { now_ns: 4_040 * MS }).unwrap();
        let fx = m.handle(Input::Tick { now_ns: 4_540 * MS }).unwrap();
        assert!(fx.contains(&Effect::CommitFinal { outcome: Outcome::Complete }));
    }

    fn live_awaiting() -> RunMachine {
        let mut m = RunMachine::new();
        arm(&mut m, cfg(SourceMode::PhoneLive, 1000, vec![1000], StopPolicy::Manual), 0);
        let fx = m.handle(Input::IntentPersisted { ok: true, error: None, now_ns: 10 * MS }).unwrap();
        assert!(fx.contains(&Effect::StartCapture));
        assert_eq!(m.phase, Phase::Preparing, "not armed until capture and baseline are ready");
        m.handle(Input::CaptureReady { now_ns: 600 * MS }).unwrap();
        assert_eq!(m.phase, Phase::Armed);
        m.handle(Input::Tick { now_ns: 1_600 * MS }).unwrap();
        let fx = m.handle(Input::CueRendered { cue_id: 0, render_ns: Some(1_620 * MS), now_ns: 1_630 * MS }).unwrap();
        assert!(fx.iter().any(|e| matches!(e, Effect::ExpectCue { render_known: true, .. })));
        assert_eq!(m.phase, Phase::AwaitingCue, "render alone is not an acoustic reference");
        m
    }

    #[test]
    fn live_acoustic_start_and_drain() {
        let mut m = live_awaiting();
        m.handle(Input::CueResolved { cue_id: 0, onset_frame: 77_000, onset_ns: Some(1_625 * MS), now_ns: 1_900 * MS }).unwrap();
        assert_eq!(m.start_method, Some(StartReferenceMethod::AcousticCue));
        assert_eq!(m.start_ref_frame, Some(77_000));
        let fx = m.handle(Input::Stop { cmd_id: "s".into(), now_ns: 5_000 * MS }).unwrap();
        assert_eq!(fx, vec![Effect::BeginDrain { cutoff_ns: 5_000 * MS }]);
        let fx = m.handle(Input::Drained { complete: true, now_ns: 5_100 * MS }).unwrap();
        assert!(fx.contains(&Effect::CommitFinal { outcome: Outcome::Complete }));
    }

    #[test]
    fn unresolved_cue_interrupts_and_never_yields_reaction_time() {
        let mut m = live_awaiting();
        let fx = m.handle(Input::CueUnresolved { cue_id: 0, now_ns: 2_000 * MS }).unwrap();
        assert_eq!(m.phase, Phase::Interrupted);
        assert!(fx.contains(&Effect::CommitFinal { outcome: Outcome::Interrupted }));
        assert_eq!(m.start_method, None);
        assert_eq!(m.interrupt_reason, Some(QualityKind::CueUnresolved));
        m.handle(Input::FinalCommitted { ok: true, error: None, now_ns: 2_010 * MS }).unwrap();
        assert_eq!(m.phase, Phase::Saved);
        assert_eq!(m.outcome, Some(Outcome::Interrupted), "saved does not imply complete");
    }

    #[test]
    fn cue_resolution_timeout_interrupts() {
        let mut m = live_awaiting();
        m.handle(Input::Tick { now_ns: 1_620 * MS + CUE_RESOLVE_TIMEOUT_NS + 1 }).unwrap();
        assert_eq!(m.phase, Phase::Interrupted);
    }

    #[test]
    fn capture_integrity_and_lifecycle_interrupt_every_active_phase() {
        let gap = QualityEvent::new(QualityKind::CaptureGap, Severity::Integrity, "x");
        let mut m = live_awaiting();
        m.handle(Input::Quality { event: gap.clone(), now_ns: 0 }).unwrap();
        assert_eq!(m.phase, Phase::Interrupted);

        let mut m = live_awaiting();
        m.handle(Input::CueResolved { cue_id: 0, onset_frame: 1, onset_ns: Some(1), now_ns: 1 }).unwrap();
        m.handle(Input::Lifecycle { kind: LifecycleKind::RouteChanged, now_ns: 2 }).unwrap();
        assert_eq!(m.interrupt_reason, Some(QualityKind::RouteChanged));

        let mut m = RunMachine::new();
        arm(&mut m, cfg(SourceMode::PhoneLive, 0, vec![], StopPolicy::Manual), 0);
        m.handle(Input::IntentPersisted { ok: true, error: None, now_ns: 1 }).unwrap();
        m.handle(Input::CaptureFailed { reason: "permission denied".into(), now_ns: 2 }).unwrap();
        assert_eq!(m.phase, Phase::Failed);
        assert_eq!(m.outcome, Some(Outcome::FailedToStart));

        // Non-integrity quality is recorded without interrupting.
        let mut m = par_only_running(vec![], StopPolicy::Manual);
        let info = QualityEvent::new(QualityKind::DeliveryDelay, Severity::Info, "x");
        m.handle(Input::Quality { event: info, now_ns: 0 }).unwrap();
        assert_eq!(m.phase, Phase::Running);
    }

    #[test]
    fn foreground_loss_interrupts_par_only_too() {
        let mut m = par_only_running(vec![], StopPolicy::Manual);
        m.handle(Input::Lifecycle { kind: LifecycleKind::ForegroundLost, now_ns: 10 }).unwrap();
        assert_eq!(m.phase, Phase::Interrupted);
    }

    #[test]
    fn drain_timeout_cannot_be_complete() {
        let mut m = live_awaiting();
        m.handle(Input::CueResolved { cue_id: 0, onset_frame: 1, onset_ns: Some(1_625 * MS), now_ns: 1 }).unwrap();
        m.handle(Input::Stop { cmd_id: "s".into(), now_ns: 5_000 * MS }).unwrap();
        let fx = m.handle(Input::Tick { now_ns: 5_000 * MS + DRAIN_TIMEOUT_NS + 1 }).unwrap();
        assert!(fx.contains(&Effect::CommitFinal { outcome: Outcome::Interrupted }));
    }

    #[test]
    fn storage_failure_then_retry() {
        let mut m = par_only_running(vec![], StopPolicy::Manual);
        m.handle(Input::Stop { cmd_id: "s".into(), now_ns: 9_000 * MS }).unwrap();
        m.handle(Input::FinalCommitted { ok: false, error: Some("disk full".into()), now_ns: 9_001 * MS }).unwrap();
        assert_eq!(m.phase, Phase::SavePending);
        assert_eq!(m.persist, Some(PersistState::Failed));
        assert!(m.handle(Input::Reset).is_err(), "cannot silently abandon an unsaved run");
        let fx = m.handle(Input::RetrySave { cmd_id: "r".into(), now_ns: 9_100 * MS }).unwrap();
        assert_eq!(fx, vec![Effect::CommitFinal { outcome: Outcome::Complete }]);
        m.handle(Input::FinalCommitted { ok: true, error: None, now_ns: 9_101 * MS }).unwrap();
        assert_eq!(m.phase, Phase::Saved);
    }

    #[test]
    fn preparation_timeout_fails_cleanly() {
        let mut m = RunMachine::new();
        arm(&mut m, cfg(SourceMode::PhoneLive, 0, vec![], StopPolicy::Manual), 0);
        m.handle(Input::IntentPersisted { ok: true, error: None, now_ns: 1 }).unwrap();
        m.handle(Input::Tick { now_ns: PREPARE_TIMEOUT_NS + 1 }).unwrap();
        assert_eq!(m.phase, Phase::Failed);
    }
}
