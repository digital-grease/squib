//! Detected candidates (immutable observations) and run-level classification.
//!
//! Onset estimation (the detector), classification relative to cues and the start
//! reference (this module), and reviewed acceptance (revisions) are separate steps.

use serde::{Deserialize, Serialize};

/// Convert a signed frame difference to nanoseconds, rounding half away from zero.
/// Uses 128-bit intermediates so large frame counters cannot overflow.
pub fn frames_to_ns(delta_frames: i64, sample_rate_hz: u32) -> i64 {
    assert!(sample_rate_hz > 0, "sample rate must be positive");
    let num = i128::from(delta_frames) * 1_000_000_000i128;
    let den = i128::from(sample_rate_hz);
    let q = num / den;
    let r = num % den;
    let adj = if 2 * r.abs() >= den { num.signum() } else { 0 };
    let v = q + adj;
    i64::try_from(v).unwrap_or(if v > 0 { i64::MAX } else { i64::MIN })
}

/// Convert a signed nanosecond difference to frames, rounding half away from zero.
pub fn ns_to_frames(delta_ns: i64, sample_rate_hz: u32) -> i64 {
    assert!(sample_rate_hz > 0, "sample rate must be positive");
    let num = i128::from(delta_ns) * i128::from(sample_rate_hz);
    let den = 1_000_000_000i128;
    let q = num / den;
    let r = num % den;
    let adj = if 2 * r.abs() >= den { num.signum() } else { 0 };
    let v = q + adj;
    i64::try_from(v).unwrap_or(if v > 0 { i64::MAX } else { i64::MIN })
}

/// Transparent features measured by the detector. Values are finite or the candidate
/// is not emitted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateFeatures {
    /// Peak absolute sample value (dBFS).
    pub peak_dbfs: f32,
    /// Noise floor at trigger (dBFS, mean-square of high-passed signal).
    pub floor_dbfs: f32,
    /// Fast energy over noise floor at trigger (dB).
    pub floor_ratio_db: f32,
    /// Fast energy over pre-onset medium energy (dB).
    pub attack_db: f32,
    /// Onset to peak, frames.
    pub rise_frames: u32,
    /// Early (0–5 ms) energy over later (15–35 ms) energy (dB). Larger = faster decay.
    pub decay_db: f32,
    /// Samples at or above 99.9% of full scale within the analysis window.
    pub clipped_samples: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectorSuggestion {
    SuggestedAccepted,
    Uncertain,
    Rejected,
}

impl DetectorSuggestion {
    pub fn as_str(self) -> &'static str {
        match self {
            DetectorSuggestion::SuggestedAccepted => "suggested_accepted",
            DetectorSuggestion::Uncertain => "uncertain",
            DetectorSuggestion::Rejected => "rejected",
        }
    }
}

/// Why a candidate carries its classification. Reasons are additive labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateReason {
    /// Below the automatic acceptance margins.
    LowMargin,
    /// Much quieter impulse shortly after a louder one.
    PossibleEcho,
    /// Input reached full scale; morphology may be unreliable.
    Clipped,
    /// Onset before the start reference: diagnostic only.
    PreCue,
    /// Onset inside the identified start cue span (could be the cue itself or a shot).
    OverlapsStartCue,
    /// Onset inside an identified par cue span.
    OverlapsParCue,
    /// Onset coincides with the start of an identified app cue: the cue itself.
    AppCue,
    /// Onset after the stop cutoff.
    AfterStop,
    /// Start origin unavailable; only relative splits are meaningful.
    NoStartReference,
}

/// An immutable detector observation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    /// Strictly increasing within a capture epoch.
    pub sequence: u64,
    pub onset_frame: i64,
    pub peak_frame: i64,
    pub features: CandidateFeatures,
    pub suggestion: DetectorSuggestion,
    pub reasons: Vec<CandidateReason>,
    /// Bounded ranking score in [0, 1]. **Not a probability.**
    pub detector_score: f32,
    pub algorithm_version: String,
    pub config_hash: String,
}

/// Frame span `[start, end)` of an identified app cue in the capture stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CueSpan {
    pub start_frame: i64,
    pub end_frame: i64,
    pub is_start_cue: bool,
    /// True when the span comes from an acoustic template match. A played-but-unheard
    /// cue contributes an inexact span: overlaps become uncertain, never rejected.
    pub exact: bool,
}

/// Context for classifying candidates within a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClassifyContext {
    pub sample_rate_hz: u32,
    /// Start reference frame in the same epoch, when acoustically resolved.
    pub start_frame: Option<i64>,
    pub cue_spans: Vec<CueSpan>,
    /// Candidates with onset after this frame are outside the run.
    pub stop_frame: Option<i64>,
}

/// Onsets within this many milliseconds of a cue span start are the cue itself.
pub const APP_CUE_ONSET_TOLERANCE_MS: i64 = 3;

/// Run-level classification of a candidate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClassifiedCandidate {
    pub sequence: u64,
    pub onset_frame: i64,
    /// Relative to the start reference; `None` when the origin is unavailable.
    pub relative_ns: Option<i64>,
    /// Position on the run timeline: relative to the start reference when resolved,
    /// otherwise relative to the epoch's frame 0 (splits only; never a first-shot time).
    pub timeline_ns: i64,
    pub classification: DetectorSuggestion,
    pub reasons: Vec<CandidateReason>,
    pub detector_score: f32,
}

/// Classify candidates against cue spans, start reference, and stop cutoff.
///
/// Overlapping candidates are preserved as `Uncertain` rather than blacked out.
/// Pre-cue impulses remain as rejected diagnostics.
pub fn classify(candidates: &[Candidate], ctx: &ClassifyContext) -> Vec<ClassifiedCandidate> {
    let tol = ns_to_frames(APP_CUE_ONSET_TOLERANCE_MS * 1_000_000, ctx.sample_rate_hz);
    candidates
        .iter()
        .map(|c| {
            let mut reasons = c.reasons.clone();
            let mut class = c.suggestion;
            let demote = |class: &mut DetectorSuggestion, to: DetectorSuggestion| {
                let rank = |s: DetectorSuggestion| match s {
                    DetectorSuggestion::SuggestedAccepted => 0,
                    DetectorSuggestion::Uncertain => 1,
                    DetectorSuggestion::Rejected => 2,
                };
                if rank(to) > rank(*class) {
                    *class = to;
                }
            };
            for span in &ctx.cue_spans {
                if c.onset_frame >= span.start_frame - tol && c.onset_frame < span.end_frame {
                    if span.exact && (c.onset_frame - span.start_frame).abs() <= tol {
                        reasons.push(CandidateReason::AppCue);
                        demote(&mut class, DetectorSuggestion::Rejected);
                    } else if span.is_start_cue {
                        reasons.push(CandidateReason::OverlapsStartCue);
                        demote(&mut class, DetectorSuggestion::Uncertain);
                    } else {
                        reasons.push(CandidateReason::OverlapsParCue);
                        demote(&mut class, DetectorSuggestion::Uncertain);
                    }
                }
            }
            let relative_ns = ctx.start_frame.map(|s| frames_to_ns(c.onset_frame - s, ctx.sample_rate_hz));
            match ctx.start_frame {
                Some(s) if c.onset_frame < s => {
                    reasons.push(CandidateReason::PreCue);
                    demote(&mut class, DetectorSuggestion::Rejected);
                }
                None => reasons.push(CandidateReason::NoStartReference),
                _ => {}
            }
            if let Some(stop) = ctx.stop_frame
                && c.onset_frame > stop
            {
                reasons.push(CandidateReason::AfterStop);
                demote(&mut class, DetectorSuggestion::Rejected);
            }
            reasons.sort();
            reasons.dedup();
            ClassifiedCandidate {
                sequence: c.sequence,
                onset_frame: c.onset_frame,
                relative_ns,
                timeline_ns: relative_ns.unwrap_or_else(|| frames_to_ns(c.onset_frame, ctx.sample_rate_hz)),
                classification: class,
                reasons,
                detector_score: c.detector_score,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn cand(seq: u64, onset: i64, s: DetectorSuggestion) -> Candidate {
        Candidate {
            sequence: seq,
            onset_frame: onset,
            peak_frame: onset + 10,
            features: CandidateFeatures {
                peak_dbfs: -3.0,
                floor_dbfs: -60.0,
                floor_ratio_db: 40.0,
                attack_db: 30.0,
                rise_frames: 10,
                decay_db: 12.0,
                clipped_samples: 0,
            },
            suggestion: s,
            reasons: vec![],
            detector_score: 0.9,
            algorithm_version: "t".into(),
            config_hash: "h".into(),
        }
    }

    #[test]
    fn frame_conversion_rounds_and_handles_sign() {
        assert_eq!(frames_to_ns(48_000, 48_000), 1_000_000_000);
        assert_eq!(frames_to_ns(1, 48_000), 20_833); // 20833.33
        assert_eq!(frames_to_ns(-1, 48_000), -20_833);
        assert_eq!(frames_to_ns(1, 44_100), 22_676); // 22675.7
        assert_eq!(frames_to_ns(-1, 44_100), -22_676);
        assert_eq!(ns_to_frames(1_000_000_000, 44_100), 44_100);
        assert_eq!(ns_to_frames(-11_338, 44_100), -1); // 0.5000058 frames
        // No overflow with very large frame counters.
        let big = i64::MAX / 2;
        assert_eq!(frames_to_ns(big, 48_000), i64::MAX);
        assert_eq!(frames_to_ns(-big, 48_000), i64::MIN);
    }

    #[test]
    fn classification_preserves_overlap_and_rejects_cue_and_pre_cue() {
        let rate = 48_000;
        let ctx = ClassifyContext {
            sample_rate_hz: rate,
            start_frame: Some(10_000),
            cue_spans: vec![
                CueSpan { start_frame: 10_000, end_frame: 17_200, is_start_cue: true, exact: true },
                CueSpan { start_frame: 100_000, end_frame: 105_760, is_start_cue: false, exact: true },
            ],
            stop_frame: Some(200_000),
        };
        let cs = vec![
            cand(0, 5_000, DetectorSuggestion::SuggestedAccepted),   // pre-cue
            cand(1, 10_020, DetectorSuggestion::SuggestedAccepted),  // the cue itself
            cand(2, 15_000, DetectorSuggestion::SuggestedAccepted),  // shot during cue
            cand(3, 20_000, DetectorSuggestion::SuggestedAccepted),  // clean
            cand(4, 102_000, DetectorSuggestion::SuggestedAccepted), // during par cue
            cand(5, 250_000, DetectorSuggestion::SuggestedAccepted), // after stop
        ];
        let out = classify(&cs, &ctx);
        assert_eq!(out[0].classification, DetectorSuggestion::Rejected);
        assert!(out[0].reasons.contains(&CandidateReason::PreCue));
        assert_eq!(out[1].classification, DetectorSuggestion::Rejected);
        assert!(out[1].reasons.contains(&CandidateReason::AppCue));
        assert_eq!(out[2].classification, DetectorSuggestion::Uncertain);
        assert!(out[2].reasons.contains(&CandidateReason::OverlapsStartCue));
        assert_eq!(out[3].classification, DetectorSuggestion::SuggestedAccepted);
        assert_eq!(out[3].relative_ns, Some(frames_to_ns(10_000, rate)));
        assert_eq!(out[4].classification, DetectorSuggestion::Uncertain);
        assert_eq!(out[5].classification, DetectorSuggestion::Rejected);
    }

    #[test]
    fn missing_start_reference_has_no_relative_time() {
        let ctx = ClassifyContext { sample_rate_hz: 48_000, start_frame: None, cue_spans: vec![], stop_frame: None };
        let out = classify(&[cand(0, 500, DetectorSuggestion::SuggestedAccepted)], &ctx);
        assert_eq!(out[0].relative_ns, None);
        assert!(out[0].reasons.contains(&CandidateReason::NoStartReference));
    }
}
