//! Per-epoch DSP pipeline shared by the desktop replay tool and the mobile engine.
//!
//! Order per block: integrity check → anchor validation → sample conversion →
//! energy envelope → cue searches → onset detector. Pure with respect to I/O: it never
//! touches storage, network, or UI. After an integrity failure the pipeline stops
//! producing candidates; the next attempt needs a new epoch.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};
use squib_domain::{Candidate, CueKind, DetectorConfig, QualityEvent, QualityKind, Severity, frames_to_ns};

use crate::block::{BlockHeader, EpochFormat, IntegrityChecker, SampleFormat};
use crate::clock::{AnchorPolicy, AnchorVerdict, ClockEvidence, ClockMapper};
use crate::cue::{CueMatch, CueRequest, cue_template, match_cue};
use crate::detector::OnsetDetector;
use crate::envelope::{EnvelopeBuilder, EnvelopeChunk};

/// Raw history kept for cue searches whose window starts in the past (seconds).
pub const HISTORY_S: f64 = 2.0;
/// Delivery delay beyond which a diagnostic quality event is reported (ns).
pub const DELIVERY_DELAY_REPORT_NS: i64 = 250_000_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DspEvent {
    Candidate(Candidate),
    Cue(CueMatch),
    Quality(QualityEvent),
    Envelope(EnvelopeChunk),
}

/// Delivery-delay statistics: how late blocks arrived relative to their mapped
/// sample time. Affects display latency only, never event timestamps (A03).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DeliveryStats {
    pub samples: u64,
    pub max_ns: i64,
    pub sum_ns: i128,
}

impl DeliveryStats {
    pub fn mean_ns(&self) -> Option<i64> {
        (self.samples > 0).then(|| (self.sum_ns / i128::from(self.samples)) as i64)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PipelineSummary {
    pub format: EpochFormat,
    pub blocks: u64,
    pub frames: i64,
    pub clipped_samples: u64,
    pub nonfinite_samples: u64,
    pub integrity_failed: bool,
    pub clock: ClockEvidence,
    pub delivery: DeliveryStats,
    pub detector_floor_dbfs: f32,
}

struct Search {
    req: CueRequest,
    template: Vec<f32>,
    buf: Vec<f32>,
    buf_first: i64,
    incomplete: bool,
}

pub struct Pipeline {
    fmt: EpochFormat,
    integrity: IntegrityChecker,
    clock: ClockMapper,
    detector: Option<OnsetDetector>,
    envelope: EnvelopeBuilder,
    history: VecDeque<f32>,
    history_first: i64,
    history_cap: usize,
    searches: Vec<Search>,
    scratch: Vec<f32>,
    quality_buf: Vec<QualityEvent>,
    blocks: u64,
    clipped: u64,
    nonfinite: u64,
    clip_reported_until: i64,
    anchor_rejections_reported: u32,
    delivery: DeliveryStats,
    failed: bool,
    processed_end: i64,
}

impl Pipeline {
    /// `detector` is `None` for capture without event detection (cue test only).
    pub fn new(fmt: EpochFormat, detector: Option<DetectorConfig>) -> Self {
        assert_eq!(fmt.channels, 1, "M1 pipeline processes mono capture");
        let history_cap = (HISTORY_S * f64::from(fmt.sample_rate_hz)) as usize;
        Self {
            integrity: IntegrityChecker::new(fmt),
            clock: ClockMapper::new(fmt.sample_rate_hz, AnchorPolicy::default()),
            detector: detector.map(|d| OnsetDetector::new(d, fmt.sample_rate_hz)),
            envelope: EnvelopeBuilder::new(fmt.sample_rate_hz),
            history: VecDeque::with_capacity(history_cap),
            history_first: 0,
            history_cap,
            searches: Vec::new(),
            scratch: Vec::with_capacity(8192),
            quality_buf: Vec::with_capacity(8),
            blocks: 0,
            clipped: 0,
            nonfinite: 0,
            clip_reported_until: i64::MIN,
            anchor_rejections_reported: 0,
            delivery: DeliveryStats::default(),
            failed: false,
            processed_end: 0,
            fmt,
        }
    }

    pub fn format(&self) -> EpochFormat {
        self.fmt
    }

    pub fn clock(&self) -> &ClockMapper {
        &self.clock
    }

    pub fn failed(&self) -> bool {
        self.failed
    }

    /// End (exclusive) of frames processed so far.
    pub fn processed_end(&self) -> i64 {
        self.processed_end
    }

    pub fn detector_mut(&mut self) -> Option<&mut OnsetDetector> {
        self.detector.as_mut()
    }

    /// Mark the epoch failed by an external integrity event (route change, etc.).
    pub fn fail(&mut self) {
        self.failed = true;
    }

    /// Register a cue search. Samples already in history are used immediately.
    pub fn request_cue(&mut self, req: CueRequest, out: &mut impl FnMut(DspEvent)) {
        let template = cue_template(req.kind, self.fmt.sample_rate_hz);
        let need = (req.window_end_frame - req.window_start_frame) as usize + template.len();
        let mut s = Search { req, template, buf: Vec::with_capacity(need), buf_first: req.window_start_frame, incomplete: false };
        if req.window_start_frame < self.history_first && self.processed_end > req.window_start_frame {
            // Part of the window is older than retained history: start at the oldest
            // retained frame and record the search as incomplete.
            s.buf_first = self.history_first;
            s.incomplete = true;
        }
        for (i, &x) in self.history.iter().enumerate() {
            let f = self.history_first + i as i64;
            if f >= s.buf_first && s.buf.len() < need {
                s.buf.push(x);
            }
        }
        self.searches.push(s);
        self.complete_searches(false, out);
    }

    /// Process one block. `out` receives events in deterministic order.
    pub fn process_i16(&mut self, h: &BlockHeader, samples: &[i16], out: &mut impl FnMut(DspEvent)) {
        debug_assert_eq!(h.format, SampleFormat::Pcm16);
        self.scratch.clear();
        self.scratch.extend(samples.iter().map(|&s| f32::from(s) / 32768.0));
        self.process_scratch(h, out);
    }

    pub fn process_f32(&mut self, h: &BlockHeader, samples: &[f32], out: &mut impl FnMut(DspEvent)) {
        self.scratch.clear();
        let mut bad = 0u64;
        self.scratch.extend(samples.iter().map(|&s| {
            if s.is_finite() {
                s.clamp(-1.0, 1.0)
            } else {
                bad += 1;
                0.0
            }
        }));
        if bad > 0 {
            self.nonfinite += bad;
            out(DspEvent::Quality(
                QualityEvent::new(
                    QualityKind::NonfiniteInput,
                    Severity::Warning,
                    format!("{bad} non-finite samples replaced by zero"),
                )
                .frames(h.first_frame, h.first_frame + i64::from(h.frame_count)),
            ));
        }
        self.process_scratch(h, out);
    }

    fn process_scratch(&mut self, h: &BlockHeader, out: &mut impl FnMut(DspEvent)) {
        if self.failed {
            return;
        }
        self.blocks += 1;
        let n = self.scratch.len();
        if n as u32 != h.frame_count {
            self.failed = true;
            out(DspEvent::Quality(QualityEvent::new(
                QualityKind::SequenceDiscontinuity,
                Severity::Integrity,
                format!("header says {} frames, payload has {n}", h.frame_count),
            )));
            return;
        }
        self.quality_buf.clear();
        self.integrity.check(h, &mut self.quality_buf);
        let mut integrity_lost = false;
        for q in self.quality_buf.drain(..) {
            integrity_lost |= q.interrupts();
            out(DspEvent::Quality(q));
        }
        if integrity_lost {
            self.failed = true;
            self.fail_searches(out);
            return;
        }
        let end = h.first_frame + n as i64;

        if let Some(a) = h.anchor {
            // Frame-origin probe: an anchor must not describe frames beyond what the
            // platform buffer could hold past the delivered stream.
            let limit = self.fmt.platform_buffer_frames.map(|cap| end + i64::from(cap));
            if limit.is_some_and(|l| a.frame > l) {
                self.failed = true;
                out(DspEvent::Quality(
                    QualityEvent::new(
                        QualityKind::CaptureOverrun,
                        Severity::Integrity,
                        format!("anchor frame {} beyond delivered {} + buffer", a.frame, end),
                    )
                    .frames(end, a.frame)
                    .at(a.mono_ns),
                ));
                self.fail_searches(out);
                return;
            }
            match self.clock.offer(a) {
                AnchorVerdict::Accepted { .. } => {}
                AnchorVerdict::Rejected { reason } => {
                    if self.anchor_rejections_reported < 3 {
                        self.anchor_rejections_reported += 1;
                        out(DspEvent::Quality(
                            QualityEvent::new(QualityKind::AnchorRejected, Severity::Warning, reason).at(a.mono_ns),
                        ));
                    }
                }
                AnchorVerdict::Discontinuity { residual_ns } => {
                    self.failed = true;
                    out(DspEvent::Quality(
                        QualityEvent::new(
                            QualityKind::ClockDiscontinuity,
                            Severity::Integrity,
                            format!("anchor residual {residual_ns} ns"),
                        )
                        .frames(h.first_frame, end)
                        .at(a.mono_ns),
                    ));
                    self.fail_searches(out);
                    return;
                }
            }
        }
        if let Some(arrival) = h.delivered_ns {
            if self.clock.evidence().accepted == 0 {
                self.clock.note_delivery(end, arrival);
            } else if let Some((mapped, _)) = self.clock.frame_to_ns(end) {
                let d = arrival - mapped;
                self.delivery.samples += 1;
                self.delivery.sum_ns += i128::from(d);
                if d > self.delivery.max_ns {
                    self.delivery.max_ns = d;
                }
                if d > DELIVERY_DELAY_REPORT_NS {
                    out(DspEvent::Quality(
                        QualityEvent::new(
                            QualityKind::DeliveryDelay,
                            Severity::Info,
                            format!("block delivered {} ms after capture", d / 1_000_000),
                        )
                        .frames(h.first_frame, end)
                        .at(arrival),
                    ));
                }
            }
        }

        let mut clipped_here = 0u64;
        for &x in &self.scratch {
            if x.abs() >= 0.999 {
                clipped_here += 1;
            }
        }
        if clipped_here > 0 {
            self.clipped += clipped_here;
            if h.first_frame >= self.clip_reported_until {
                self.clip_reported_until = h.first_frame + i64::from(self.fmt.sample_rate_hz);
                out(DspEvent::Quality(
                    QualityEvent::new(
                        QualityKind::ClippedInput,
                        Severity::Warning,
                        format!("{clipped_here} samples at full scale"),
                    )
                    .frames(h.first_frame, end),
                ));
            }
        }

        let scratch = std::mem::take(&mut self.scratch);
        self.envelope.push(h.first_frame, &scratch, &mut |c| out(DspEvent::Envelope(c)));

        // History and cue searches.
        if self.blocks == 1 {
            self.history_first = h.first_frame;
        }
        for &x in &scratch {
            if self.history.len() == self.history_cap {
                self.history.pop_front();
                self.history_first += 1;
            }
            self.history.push_back(x);
        }
        for s in &mut self.searches {
            let need = (s.req.window_end_frame - s.req.window_start_frame) as usize + s.template.len();
            for (i, &x) in scratch.iter().enumerate() {
                let f = h.first_frame + i as i64;
                if f >= s.buf_first + s.buf.len() as i64 && s.buf.len() < need {
                    s.buf.push(x);
                }
            }
        }
        if let Some(d) = self.detector.as_mut() {
            d.process(h.first_frame, &scratch, &mut |c| out(DspEvent::Candidate(c)));
        }
        self.scratch = scratch;
        self.processed_end = end;
        self.complete_searches(false, out);
    }

    fn complete_searches(&mut self, force: bool, out: &mut impl FnMut(DspEvent)) {
        let mut i = 0;
        while i < self.searches.len() {
            let s = &self.searches[i];
            let need_end = s.req.window_end_frame + s.template.len() as i64;
            if force || self.processed_end >= need_end {
                let s = self.searches.remove(i);
                let incomplete = s.incomplete || s.buf_first + (s.buf.len() as i64) < need_end;
                let m = match_cue(&s.req, &s.template, &s.buf, s.buf_first, incomplete);
                out(DspEvent::Cue(m));
            } else {
                i += 1;
            }
        }
    }

    fn fail_searches(&mut self, out: &mut impl FnMut(DspEvent)) {
        for s in self.searches.drain(..) {
            out(DspEvent::Cue(CueMatch {
                cue_id: s.req.cue_id,
                kind: s.req.kind,
                template_version: s.req.kind.template_version().into(),
                onset_frame: None,
                end_frame: None,
                best_ncc: 0.0,
                second_ncc: 0.0,
                window_start_frame: s.req.window_start_frame,
                window_end_frame: s.req.window_end_frame,
                incomplete: true,
            }));
        }
    }

    /// End of capture: resolve pending searches with what was observed and flush the
    /// envelope. Candidates still inside their fixed feature window are not emitted;
    /// the run controller's drain accounts for that latency.
    pub fn finish(&mut self, out: &mut impl FnMut(DspEvent)) {
        self.complete_searches(true, out);
        self.envelope.flush(&mut |c| out(DspEvent::Envelope(c)));
    }

    pub fn summary(&self) -> PipelineSummary {
        PipelineSummary {
            format: self.fmt,
            blocks: self.blocks,
            frames: self.processed_end,
            clipped_samples: self.clipped,
            nonfinite_samples: self.nonfinite,
            integrity_failed: self.failed,
            clock: self.clock.evidence(),
            delivery: self.delivery.clone(),
            detector_floor_dbfs: self.detector.as_ref().map(|d| d.floor_dbfs()).unwrap_or(f32::NAN),
        }
    }

    /// Detector output latency: frames after onset before a candidate is emitted.
    pub fn detector_latency_frames(&self) -> i64 {
        (0.040 * f64::from(self.fmt.sample_rate_hz)) as i64
    }

    pub fn frames_to_ns(&self, frames: i64) -> i64 {
        frames_to_ns(frames, self.fmt.sample_rate_hz)
    }
}

/// Convenience for kinds stored on cue matches.
pub fn is_start(kind: CueKind) -> bool {
    kind == CueKind::Start
}
