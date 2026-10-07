//! Derived results recomputed from one specific revision.

use serde::{Deserialize, Serialize};

use crate::revision::RunRevision;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunResults {
    pub revision_number: u32,
    pub count: u32,
    /// Requires a resolved start reference; otherwise unavailable, not guessed.
    pub first_ns: Option<i64>,
    /// Requires a resolved start reference.
    pub last_ns: Option<i64>,
    /// Differences between consecutive accepted events (same clock).
    pub splits_ns: Vec<i64>,
    /// Equal timestamps: zero split, insufficient resolution.
    pub zero_splits: u32,
    /// Review hint from the configuration; never used to alter detections.
    pub expected_count: Option<u32>,
}

impl RunResults {
    /// `Some(true/false)` when an expected count hint exists.
    pub fn count_matches_hint(&self) -> Option<bool> {
        self.expected_count.map(|n| n == self.count)
    }
}

pub fn compute_results(rev: &RunRevision, expected_count: Option<u32>) -> RunResults {
    let ev = &rev.content.accepted;
    let resolved = rev.content.origin_resolved;
    let splits_ns: Vec<i64> = ev.windows(2).map(|w| w[1].timeline_ns - w[0].timeline_ns).collect();
    RunResults {
        revision_number: rev.content.number,
        count: u32::try_from(ev.len()).unwrap_or(u32::MAX),
        first_ns: if resolved { ev.first().map(|e| e.timeline_ns) } else { None },
        last_ns: if resolved { ev.last().map(|e| e.timeline_ns) } else { None },
        zero_splits: u32::try_from(splits_ns.iter().filter(|&&s| s == 0).count()).unwrap_or(u32::MAX),
        splits_ns,
        expected_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::{ClassifiedCandidate, DetectorSuggestion};
    use crate::revision::{RevisionAction, apply_revision, initial_revision};

    fn cc(seq: u64, t: i64) -> ClassifiedCandidate {
        ClassifiedCandidate {
            sequence: seq,
            onset_frame: 0,
            relative_ns: Some(t),
            timeline_ns: t,
            classification: DetectorSuggestion::SuggestedAccepted,
            reasons: vec![],
            detector_score: 0.9,
        }
    }

    #[test]
    fn first_splits_last() {
        let cs = vec![cc(0, 1_000), cc(1, 1_250), cc(2, 1_600)];
        let r = compute_results(&initial_revision("r", true, &cs, 0), Some(4));
        assert_eq!(r.first_ns, Some(1_000));
        assert_eq!(r.last_ns, Some(1_600));
        assert_eq!(r.splits_ns, vec![250, 350]);
        assert_eq!(r.count, 3);
        assert_eq!(r.count_matches_hint(), Some(false), "hint is reported, never enforced");
    }

    #[test]
    fn unresolved_origin_keeps_splits_only() {
        let cs = vec![cc(0, 5_000), cc(1, 5_400)];
        let r = compute_results(&initial_revision("r", false, &cs, 0), None);
        assert_eq!(r.first_ns, None);
        assert_eq!(r.last_ns, None);
        assert_eq!(r.splits_ns, vec![400]);
    }

    #[test]
    fn equal_times_flagged_and_recomputed_after_move() {
        let cs = vec![cc(0, 1_000), cc(1, 2_000)];
        let r1 = initial_revision("r", true, &cs, 0);
        let r2 = apply_revision(&r1, &cs, vec![RevisionAction::AddManual { id: "m".into(), timeline_ns: 2_000 }], "e", 1, None)
            .unwrap();
        let res = compute_results(&r2, None);
        assert_eq!(res.zero_splits, 1);
        assert_eq!(res.revision_number, 2);
    }
}
