//! Review revisions over immutable observations (docs/squib/06 "Corrections").
//!
//! Revision 1 projects the detector's suggestions. Every edit creates a new revision
//! linked to its parent by number and content hash; observations are never modified.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::candidate::{ClassifiedCandidate, DetectorSuggestion};
use crate::hash::content_hash;
use crate::state::ReviewState;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventRef {
    Candidate { sequence: u64 },
    Manual { id: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventOrigin {
    /// Detector onset, unmodified.
    Detected,
    /// Detector candidate whose onset a person moved.
    Moved,
    /// Entered by a person; no fictional sample origin or detector score.
    Manual,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AcceptedEvent {
    pub event: EventRef,
    /// Run-timeline position (see `ClassifiedCandidate::timeline_ns`).
    pub timeline_ns: i64,
    pub origin: EventOrigin,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum RevisionAction {
    AcceptCandidate { sequence: u64 },
    RejectCandidate { sequence: u64 },
    AddManual { id: String, timeline_ns: i64 },
    MoveEvent { event: EventRef, timeline_ns: i64 },
    RemoveManual { id: String },
    Note { text: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RevisionContent {
    pub run_id: String,
    pub number: u32,
    pub parent_number: Option<u32>,
    pub parent_hash: Option<String>,
    /// Whether the timeline origin is a resolved start reference.
    pub origin_resolved: bool,
    pub actions: Vec<RevisionAction>,
    /// Strictly ordered by (timeline_ns, event).
    pub accepted: Vec<AcceptedEvent>,
    pub unresolved: Vec<u64>,
    pub rejected: Vec<u64>,
    pub notes: Vec<String>,
    /// `detector` for revision 1, otherwise the local editor identity.
    pub editor: String,
    pub created_utc_ms: i64,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunRevision {
    pub content: RevisionContent,
    pub content_hash: String,
}

impl RunRevision {
    fn seal(content: RevisionContent) -> Self {
        let content_hash = content_hash(&content);
        Self { content, content_hash }
    }

    pub fn verify_hash(&self) -> bool {
        content_hash(&self.content) == self.content_hash
    }

    pub fn review_state(&self) -> ReviewState {
        if !self.content.unresolved.is_empty() {
            ReviewState::NeedsReview
        } else if self.content.number > 1 {
            ReviewState::Reviewed
        } else {
            ReviewState::Automatic
        }
    }

    /// Edited by a person at least once.
    pub fn is_edited(&self) -> bool {
        self.content.number > 1
    }
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum RevisionError {
    #[error("unknown candidate {0}")]
    UnknownCandidate(u64),
    #[error("event {0:?} is not currently accepted")]
    NotAccepted(EventRef),
    #[error("negative time after the start reference is not an accepted event")]
    NegativeTime,
    #[error("candidate {0} is before the start reference and stays diagnostic")]
    PreCueCandidate(u64),
    #[error("manual event id {0} already exists")]
    DuplicateManualId(String),
    #[error("manual id must be non-empty and at most 64 bytes")]
    InvalidManualId,
    #[error("note exceeds 2000 bytes")]
    NoteTooLong,
    #[error("revision has no actions")]
    Empty,
    #[error("too many actions in one revision")]
    TooManyActions,
}

pub const DETECTOR_EDITOR: &str = "detector";
const MAX_ACTIONS: usize = 500;

fn sort_accepted(v: &mut [AcceptedEvent]) {
    v.sort_by(|a, b| a.timeline_ns.cmp(&b.timeline_ns).then_with(|| a.event.cmp(&b.event)));
}

/// Build revision 1 from classified candidates: suggested accepted → accepted,
/// uncertain → unresolved, rejected → rejected.
pub fn initial_revision(
    run_id: &str,
    origin_resolved: bool,
    candidates: &[ClassifiedCandidate],
    created_utc_ms: i64,
) -> RunRevision {
    let mut accepted = Vec::new();
    let mut unresolved = Vec::new();
    let mut rejected = Vec::new();
    for c in candidates {
        match c.classification {
            DetectorSuggestion::SuggestedAccepted => accepted.push(AcceptedEvent {
                event: EventRef::Candidate { sequence: c.sequence },
                timeline_ns: c.timeline_ns,
                origin: EventOrigin::Detected,
            }),
            DetectorSuggestion::Uncertain => unresolved.push(c.sequence),
            DetectorSuggestion::Rejected => rejected.push(c.sequence),
        }
    }
    sort_accepted(&mut accepted);
    RunRevision::seal(RevisionContent {
        run_id: run_id.to_string(),
        number: 1,
        parent_number: None,
        parent_hash: None,
        origin_resolved,
        actions: vec![],
        accepted,
        unresolved,
        rejected,
        notes: vec![],
        editor: DETECTOR_EDITOR.to_string(),
        created_utc_ms,
        reason: None,
    })
}

/// Apply review actions to `parent`, producing the next revision.
pub fn apply_revision(
    parent: &RunRevision,
    candidates: &[ClassifiedCandidate],
    actions: Vec<RevisionAction>,
    editor: &str,
    created_utc_ms: i64,
    reason: Option<String>,
) -> Result<RunRevision, RevisionError> {
    if actions.is_empty() {
        return Err(RevisionError::Empty);
    }
    if actions.len() > MAX_ACTIONS {
        return Err(RevisionError::TooManyActions);
    }
    let p = &parent.content;
    let resolved = p.origin_resolved;
    let mut accepted = p.accepted.clone();
    let mut unresolved: BTreeSet<u64> = p.unresolved.iter().copied().collect();
    let mut rejected: BTreeSet<u64> = p.rejected.iter().copied().collect();
    let mut notes = p.notes.clone();
    let find = |seq: u64| candidates.iter().find(|c| c.sequence == seq).ok_or(RevisionError::UnknownCandidate(seq));
    let check_time = |t: i64| if resolved && t < 0 { Err(RevisionError::NegativeTime) } else { Ok(()) };

    for a in &actions {
        match a {
            RevisionAction::AcceptCandidate { sequence } => {
                let c = find(*sequence)?;
                if resolved && c.relative_ns.is_some_and(|t| t < 0) {
                    return Err(RevisionError::PreCueCandidate(*sequence));
                }
                let key = EventRef::Candidate { sequence: *sequence };
                if !accepted.iter().any(|e| e.event == key) {
                    accepted.push(AcceptedEvent { event: key, timeline_ns: c.timeline_ns, origin: EventOrigin::Detected });
                }
                unresolved.remove(sequence);
                rejected.remove(sequence);
            }
            RevisionAction::RejectCandidate { sequence } => {
                find(*sequence)?;
                accepted.retain(|e| e.event != EventRef::Candidate { sequence: *sequence });
                unresolved.remove(sequence);
                rejected.insert(*sequence);
            }
            RevisionAction::AddManual { id, timeline_ns } => {
                if id.is_empty() || id.len() > 64 {
                    return Err(RevisionError::InvalidManualId);
                }
                check_time(*timeline_ns)?;
                let key = EventRef::Manual { id: id.clone() };
                if accepted.iter().any(|e| e.event == key) {
                    return Err(RevisionError::DuplicateManualId(id.clone()));
                }
                accepted.push(AcceptedEvent { event: key, timeline_ns: *timeline_ns, origin: EventOrigin::Manual });
            }
            RevisionAction::MoveEvent { event, timeline_ns } => {
                check_time(*timeline_ns)?;
                let e =
                    accepted.iter_mut().find(|e| &e.event == event).ok_or_else(|| RevisionError::NotAccepted(event.clone()))?;
                e.timeline_ns = *timeline_ns;
                if e.origin == EventOrigin::Detected {
                    e.origin = EventOrigin::Moved;
                }
            }
            RevisionAction::RemoveManual { id } => {
                let key = EventRef::Manual { id: id.clone() };
                let before = accepted.len();
                accepted.retain(|e| e.event != key);
                if accepted.len() == before {
                    return Err(RevisionError::NotAccepted(key));
                }
            }
            RevisionAction::Note { text } => {
                if text.len() > 2000 {
                    return Err(RevisionError::NoteTooLong);
                }
                notes.push(text.clone());
            }
        }
    }
    sort_accepted(&mut accepted);
    Ok(RunRevision::seal(RevisionContent {
        run_id: p.run_id.clone(),
        number: p.number + 1,
        parent_number: Some(p.number),
        parent_hash: Some(parent.content_hash.clone()),
        origin_resolved: resolved,
        actions,
        accepted,
        unresolved: unresolved.into_iter().collect(),
        rejected: rejected.into_iter().collect(),
        notes,
        editor: editor.to_string(),
        created_utc_ms,
        reason,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::CandidateReason;

    fn cc(seq: u64, t: i64, class: DetectorSuggestion) -> ClassifiedCandidate {
        ClassifiedCandidate {
            sequence: seq,
            onset_frame: t / 1000,
            relative_ns: Some(t),
            timeline_ns: t,
            classification: class,
            reasons: vec![CandidateReason::LowMargin],
            detector_score: 0.5,
        }
    }

    fn fixture() -> (Vec<ClassifiedCandidate>, RunRevision) {
        let cs = vec![
            cc(0, -50_000_000, DetectorSuggestion::Rejected),
            cc(1, 800_000_000, DetectorSuggestion::SuggestedAccepted),
            cc(2, 1_100_000_000, DetectorSuggestion::Uncertain),
            cc(3, 1_400_000_000, DetectorSuggestion::SuggestedAccepted),
        ];
        let r1 = initial_revision("run", true, &cs, 1);
        (cs, r1)
    }

    #[test]
    fn initial_projection_and_review_state() {
        let (_, r1) = fixture();
        assert_eq!(r1.content.accepted.len(), 2);
        assert_eq!(r1.content.unresolved, vec![2]);
        assert_eq!(r1.content.rejected, vec![0]);
        assert_eq!(r1.review_state(), ReviewState::NeedsReview);
        assert!(r1.verify_hash());
    }

    #[test]
    fn edits_create_linked_revisions_without_touching_parent() {
        let (cs, r1) = fixture();
        let snapshot = r1.clone();
        let r2 = apply_revision(
            &r1,
            &cs,
            vec![
                RevisionAction::AcceptCandidate { sequence: 2 },
                RevisionAction::RejectCandidate { sequence: 3 },
                RevisionAction::AddManual { id: "m1".into(), timeline_ns: 2_000_000_000 },
                RevisionAction::MoveEvent { event: EventRef::Candidate { sequence: 1 }, timeline_ns: 790_000_000 },
            ],
            "local-device",
            2,
            Some("missed shot".into()),
        )
        .unwrap();
        assert_eq!(r1, snapshot, "parent revision is immutable");
        assert_eq!(r2.content.number, 2);
        assert_eq!(r2.content.parent_hash.as_deref(), Some(r1.content_hash.as_str()));
        let times: Vec<i64> = r2.content.accepted.iter().map(|e| e.timeline_ns).collect();
        assert_eq!(times, vec![790_000_000, 1_100_000_000, 2_000_000_000]);
        assert_eq!(r2.content.accepted[0].origin, EventOrigin::Moved);
        assert_eq!(r2.content.accepted[2].origin, EventOrigin::Manual);
        assert_eq!(r2.content.rejected, vec![0, 3]);
        assert_eq!(r2.review_state(), ReviewState::Reviewed);
        assert!(r2.verify_hash());
    }

    #[test]
    fn invalid_edits_rejected() {
        let (cs, r1) = fixture();
        let err = |a: RevisionAction| apply_revision(&r1, &cs, vec![a], "e", 2, None).unwrap_err();
        assert_eq!(err(RevisionAction::AcceptCandidate { sequence: 0 }), RevisionError::PreCueCandidate(0));
        assert_eq!(err(RevisionAction::AcceptCandidate { sequence: 9 }), RevisionError::UnknownCandidate(9));
        assert_eq!(err(RevisionAction::AddManual { id: "x".into(), timeline_ns: -1 }), RevisionError::NegativeTime);
        assert!(matches!(
            err(RevisionAction::MoveEvent { event: EventRef::Candidate { sequence: 2 }, timeline_ns: 5 }),
            RevisionError::NotAccepted(_)
        ));
        assert_eq!(apply_revision(&r1, &cs, vec![], "e", 2, None).unwrap_err(), RevisionError::Empty);
    }

    #[test]
    fn tampered_revision_fails_hash() {
        let (_, mut r1) = fixture();
        r1.content.accepted.pop();
        assert!(!r1.verify_hash());
    }
}
