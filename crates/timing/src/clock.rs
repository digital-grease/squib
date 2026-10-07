//! Frame ↔ monotonic-clock mapping (docs/squib/04 "Time model").
//!
//! Policy `frame-anchor-v1`:
//! - An anchor associates a frame index (in the capture's frame domain) with a
//!   monotonic nanosecond time reported by the platform for that same frame.
//! - Anchors are validated for monotonicity, plausible implied rate, and residual
//!   against the previous accepted anchor at the negotiated nominal rate.
//! - Mapping uses the latest accepted anchor at the nominal rate, so drift between
//!   anchors stays bounded by the anchor interval. Drift is estimated for evidence.
//! - A residual beyond the bound is a clock discontinuity: the epoch cannot support a
//!   verified run and the run is interrupted. Committed timestamps are never rewritten.
//! - Delivery (callback/read return) times are never anchors. An optional
//!   delivery-derived mapping exists only under the `Approximate` label.

use serde::{Deserialize, Serialize};
use squib_domain::{TimestampQuality, frames_to_ns, ns_to_frames};

pub const MAPPING_POLICY_VERSION: &str = "frame-anchor-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameAnchor {
    pub frame: i64,
    pub mono_ns: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AnchorPolicy {
    /// Allowed relative deviation of the implied rate from nominal.
    pub max_rate_deviation: f64,
    /// Fixed residual allowance (ns).
    pub residual_base_ns: i64,
    /// Additional residual allowance proportional to the interval since the last anchor.
    pub residual_per_interval: f64,
    /// Minimum frame span between two anchors before the implied rate is checked.
    pub min_rate_check_span_s: f64,
    /// Span over which two consistent anchors make the mapping verified.
    pub verify_span_s: f64,
}

impl Default for AnchorPolicy {
    fn default() -> Self {
        Self {
            max_rate_deviation: 0.02,
            residual_base_ns: 5_000_000,
            residual_per_interval: 0.005,
            min_rate_check_span_s: 0.2,
            verify_span_s: 0.5,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AnchorVerdict {
    Accepted {
        residual_ns: i64,
    },
    Rejected {
        reason: String,
    },
    /// The frame/clock relationship jumped: new epoch required.
    Discontinuity {
        residual_ns: i64,
    },
}

/// Evidence summary persisted with a capture epoch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClockEvidence {
    pub policy: String,
    pub quality: TimestampQuality,
    pub accepted: u32,
    pub rejected: u32,
    pub discontinuities: u32,
    pub max_abs_residual_ns: i64,
    pub first: Option<FrameAnchor>,
    pub last: Option<FrameAnchor>,
    /// (implied rate / nominal − 1) × 1e6 over first→last anchors, when span suffices.
    pub drift_ppm: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct ClockMapper {
    rate: u32,
    policy: AnchorPolicy,
    first: Option<FrameAnchor>,
    last: Option<FrameAnchor>,
    approximate: Option<FrameAnchor>,
    accepted: u32,
    rejected: u32,
    discontinuities: u32,
    max_abs_residual_ns: i64,
}

impl ClockMapper {
    pub fn new(sample_rate_hz: u32, policy: AnchorPolicy) -> Self {
        assert!(sample_rate_hz > 0);
        Self {
            rate: sample_rate_hz,
            policy,
            first: None,
            last: None,
            approximate: None,
            accepted: 0,
            rejected: 0,
            discontinuities: 0,
            max_abs_residual_ns: 0,
        }
    }

    pub fn sample_rate_hz(&self) -> u32 {
        self.rate
    }

    /// Offer a platform anchor. Callers must guarantee `frame` uses the same frame
    /// origin as delivered blocks; that association is the M0 platform experiment.
    pub fn offer(&mut self, a: FrameAnchor) -> AnchorVerdict {
        if a.frame < 0 || a.mono_ns <= 0 {
            self.rejected += 1;
            return AnchorVerdict::Rejected { reason: "negative or zero anchor".into() };
        }
        let Some(prev) = self.last else {
            self.first = Some(a);
            self.last = Some(a);
            self.accepted += 1;
            return AnchorVerdict::Accepted { residual_ns: 0 };
        };
        let d_frames = a.frame - prev.frame;
        let d_ns = a.mono_ns - prev.mono_ns;
        if d_ns <= 0 || d_frames < 0 {
            self.rejected += 1;
            return AnchorVerdict::Rejected { reason: format!("non-monotonic anchor (Δframes={d_frames}, Δns={d_ns})") };
        }
        let predicted = prev.mono_ns + frames_to_ns(d_frames, self.rate);
        let residual = a.mono_ns - predicted;
        let allowance = self.policy.residual_base_ns + (d_ns as f64 * self.policy.residual_per_interval) as i64;
        if residual.abs() > allowance {
            self.discontinuities += 1;
            return AnchorVerdict::Discontinuity { residual_ns: residual };
        }
        let span_s = d_frames as f64 / f64::from(self.rate);
        if span_s >= self.policy.min_rate_check_span_s {
            let implied = d_frames as f64 * 1e9 / d_ns as f64;
            let dev = implied / f64::from(self.rate) - 1.0;
            if dev.abs() > self.policy.max_rate_deviation {
                self.rejected += 1;
                return AnchorVerdict::Rejected { reason: format!("implied rate deviates {:.3}%", dev * 100.0) };
            }
        }
        self.last = Some(a);
        self.accepted += 1;
        self.max_abs_residual_ns = self.max_abs_residual_ns.max(residual.abs());
        AnchorVerdict::Accepted { residual_ns: residual }
    }

    /// Record a delivery-derived association. Only used when no platform anchor exists
    /// and always labelled `Approximate`.
    pub fn note_delivery(&mut self, frame_end: i64, arrival_ns: i64) {
        // Keep the earliest-arriving association per frame position: delivery can only
        // be late, so the minimum (arrival − frame time) is the least-biased bound.
        let cand = FrameAnchor { frame: frame_end, mono_ns: arrival_ns };
        match self.approximate {
            None => self.approximate = Some(cand),
            Some(old) => {
                let old_offset = old.mono_ns - frames_to_ns(old.frame, self.rate);
                let new_offset = arrival_ns - frames_to_ns(frame_end, self.rate);
                if new_offset < old_offset {
                    self.approximate = Some(cand);
                }
            }
        }
    }

    pub fn quality(&self) -> TimestampQuality {
        match (self.first, self.last) {
            (Some(f), Some(l)) if self.accepted >= 2 && self.discontinuities == 0 => {
                let span = (l.frame - f.frame) as f64 / f64::from(self.rate);
                if span >= self.policy.verify_span_s {
                    TimestampQuality::VerifiedSampleClock
                } else {
                    TimestampQuality::ProvisionalSampleClock
                }
            }
            (Some(_), _) => TimestampQuality::ProvisionalSampleClock,
            _ if self.approximate.is_some() => TimestampQuality::Approximate,
            _ => TimestampQuality::Unavailable,
        }
    }

    /// Anchor currently used for mapping (latest platform anchor, else delivery-derived).
    pub fn reference(&self) -> Option<FrameAnchor> {
        self.last.or(self.approximate)
    }

    /// Map a frame to monotonic ns with the mapping's current quality.
    pub fn frame_to_ns(&self, frame: i64) -> Option<(i64, TimestampQuality)> {
        let r = self.reference()?;
        Some((r.mono_ns.saturating_add(frames_to_ns(frame.saturating_sub(r.frame), self.rate)), self.quality()))
    }

    /// Map a monotonic time to the nearest frame.
    pub fn ns_to_frame(&self, ns: i64) -> Option<(i64, TimestampQuality)> {
        let r = self.reference()?;
        Some((r.frame.saturating_add(ns_to_frames(ns.saturating_sub(r.mono_ns), self.rate)), self.quality()))
    }

    pub fn evidence(&self) -> ClockEvidence {
        let drift_ppm = match (self.first, self.last) {
            (Some(f), Some(l)) if (l.frame - f.frame) as f64 / f64::from(self.rate) >= 1.0 => {
                let implied = (l.frame - f.frame) as f64 * 1e9 / (l.mono_ns - f.mono_ns) as f64;
                Some((implied / f64::from(self.rate) - 1.0) * 1e6)
            }
            _ => None,
        };
        ClockEvidence {
            policy: MAPPING_POLICY_VERSION.into(),
            quality: self.quality(),
            accepted: self.accepted,
            rejected: self.rejected,
            discontinuities: self.discontinuities,
            max_abs_residual_ns: self.max_abs_residual_ns,
            first: self.first,
            last: self.last,
            drift_ppm,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: u32 = 48_000;

    fn mapper() -> ClockMapper {
        ClockMapper::new(R, AnchorPolicy::default())
    }

    #[test]
    fn unavailable_without_anchor_and_approximate_from_delivery_only() {
        let mut m = mapper();
        assert_eq!(m.quality(), TimestampQuality::Unavailable);
        assert!(m.frame_to_ns(0).is_none());
        m.note_delivery(480, 1_000_000_000 + 30_000_000);
        m.note_delivery(960, 1_000_000_000 + 10_000_000 + 10_000_000); // earlier relative arrival
        assert_eq!(m.quality(), TimestampQuality::Approximate);
        let (t, q) = m.frame_to_ns(960).unwrap();
        assert_eq!(q, TimestampQuality::Approximate);
        assert_eq!(t, 1_020_000_000);
    }

    #[test]
    fn mapping_rounds_with_signed_offsets() {
        let mut m = mapper();
        m.offer(FrameAnchor { frame: 48_000, mono_ns: 5_000_000_000 });
        assert_eq!(m.frame_to_ns(48_001).unwrap().0, 5_000_020_833);
        assert_eq!(m.frame_to_ns(47_999).unwrap().0, 4_999_979_167);
        assert_eq!(m.frame_to_ns(0).unwrap().0, 4_000_000_000);
        assert_eq!(m.ns_to_frame(4_000_000_000).unwrap().0, 0);
        assert_eq!(m.quality(), TimestampQuality::ProvisionalSampleClock);
    }

    #[test]
    fn verified_after_consistent_span_and_drift_measured() {
        let mut m = mapper();
        m.offer(FrameAnchor { frame: 0, mono_ns: 1_000_000_000 });
        // Clock runs 50 ppm fast relative to nominal: 2 s of frames in 2.0001 s.
        let v = m.offer(FrameAnchor { frame: 96_000, mono_ns: 1_000_000_000 + 2_000_100_000 });
        assert!(matches!(v, AnchorVerdict::Accepted { .. }));
        assert_eq!(m.quality(), TimestampQuality::VerifiedSampleClock);
        let ppm = m.evidence().drift_ppm.unwrap();
        assert!((ppm + 50.0).abs() < 1.0, "ppm {ppm}");
    }

    #[test]
    fn rejects_non_monotonic_and_bad_rate_and_flags_discontinuity() {
        let mut m = mapper();
        m.offer(FrameAnchor { frame: 48_000, mono_ns: 2_000_000_000 });
        assert!(matches!(m.offer(FrameAnchor { frame: 47_000, mono_ns: 2_100_000_000 }), AnchorVerdict::Rejected { .. }));
        assert!(matches!(m.offer(FrameAnchor { frame: 49_000, mono_ns: 1_900_000_000 }), AnchorVerdict::Rejected { .. }));
        // 100 ms jump at one-second spacing: discontinuity.
        let v = m.offer(FrameAnchor { frame: 96_000, mono_ns: 3_100_000_000 });
        assert!(matches!(v, AnchorVerdict::Discontinuity { residual_ns: 100_000_000 }));
        assert_ne!(m.quality(), TimestampQuality::VerifiedSampleClock);
        assert_eq!(m.evidence().discontinuities, 1);
    }

    #[test]
    fn restart_requires_new_mapper() {
        // A capture restart resets frame counters to zero; offering that to the old
        // mapper is non-monotonic and rejected, never silently re-based.
        let mut m = mapper();
        m.offer(FrameAnchor { frame: 480_000, mono_ns: 20_000_000_000 });
        assert!(matches!(m.offer(FrameAnchor { frame: 480, mono_ns: 21_000_000_000 }), AnchorVerdict::Rejected { .. }));
    }

    #[test]
    fn large_frame_counters_do_not_overflow() {
        let mut m = mapper();
        let f = i64::MAX / 4;
        m.offer(FrameAnchor { frame: f, mono_ns: 1 });
        assert_eq!(m.frame_to_ns(f + 48_000).unwrap().0, 1_000_000_001);
        assert_eq!(m.frame_to_ns(0).unwrap().0, i64::MIN + 1, "saturates instead of wrapping");
    }
}
