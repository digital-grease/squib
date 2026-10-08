//! Generic practice scoring (docs/squib/07 "Scoring baseline").
//!
//! No official discipline is implemented or named: every built-in profile is
//! "Generic practice". Inputs are integer counts; results are computed unrounded and
//! rounded only for display, per profile. Missing scores are incomplete, never zero.
//! Score entries are revisions independent of detector observations.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use squib_domain::hash::content_hash;
use thiserror::Error;

pub const SCORING_CALC_VERSION: &str = "generic-scoring-v1";
pub const MAX_COUNT: u32 = 1000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProfileKind {
    /// Elapsed time from a valid start to the final accepted event.
    TimeOnly,
    /// Net points / elapsed seconds. `categories` are scoring zones; `penalties` are
    /// point deductions (negative values).
    PointsHitFactor { categories: Vec<(String, i32)>, penalties: Vec<(String, i32)> },
    /// Elapsed time plus explicit penalty time per counted penalty.
    TimePlus { penalties_ms: Vec<(String, u32)> },
    /// Entered elapsed time and/or score with its own precision; no detector involved.
    ManualResult,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoringProfile {
    pub id: String,
    pub version: u32,
    /// Always "Generic practice" for built-in profiles.
    pub discipline_label: String,
    pub title: String,
    pub kind: ProfileKind,
    /// Decimal places for display/report only.
    pub display_decimals: u8,
}

/// Built-in generic profiles. Official profiles require verified rule fixtures first.
pub fn builtin_profiles() -> Vec<ScoringProfile> {
    let generic = "Generic practice".to_string();
    vec![
        ScoringProfile {
            id: "generic-time".into(),
            version: 1,
            discipline_label: generic.clone(),
            title: "Time only".into(),
            kind: ProfileKind::TimeOnly,
            display_decimals: 2,
        },
        ScoringProfile {
            id: "generic-points-hf".into(),
            version: 1,
            discipline_label: generic.clone(),
            title: "Points and hit factor".into(),
            kind: ProfileKind::PointsHitFactor {
                categories: vec![("A".into(), 5), ("C".into(), 3), ("D".into(), 1), ("Miss".into(), 0)],
                penalties: vec![("Miss penalty".into(), -10), ("Procedural".into(), -10), ("No-shoot".into(), -10)],
            },
            display_decimals: 4,
        },
        ScoringProfile {
            id: "generic-time-plus".into(),
            version: 1,
            discipline_label: generic.clone(),
            title: "Time plus penalties".into(),
            kind: ProfileKind::TimePlus {
                penalties_ms: vec![("Point down".into(), 1000), ("Miss".into(), 5000), ("Procedural".into(), 3000)],
            },
            display_decimals: 2,
        },
        ScoringProfile {
            id: "generic-manual".into(),
            version: 1,
            discipline_label: generic,
            title: "Manual result".into(),
            kind: ProfileKind::ManualResult,
            display_decimals: 2,
        },
    ]
}

pub fn builtin(id: &str, version: u32) -> Option<ScoringProfile> {
    builtin_profiles().into_iter().find(|p| p.id == id && p.version == version)
}

/// What the user entered for one run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoreEntry {
    pub profile_id: String,
    pub profile_version: u32,
    /// Counts per category or penalty name.
    pub counts: BTreeMap<String, u32>,
    /// The user marked the entry complete (all targets scored).
    pub complete: bool,
    /// Manual result: elapsed time and its precision.
    pub manual_elapsed_ns: Option<i64>,
    pub manual_precision_ns: Option<i64>,
    /// Manual result: points, if the user enters a score directly.
    pub manual_points: Option<i64>,
    pub notes: String,
}

#[derive(Debug, Clone, PartialEq, Error, Serialize, Deserialize)]
pub enum ScoreError {
    #[error("unknown scoring profile")]
    UnknownProfile,
    #[error("unknown category or penalty '{0}' for this profile")]
    UnknownName(String),
    #[error("count for '{0}' exceeds {MAX_COUNT}")]
    CountTooLarge(String),
    #[error("notes exceed 2000 characters")]
    NotesTooLong,
    #[error("manual elapsed time must be positive")]
    ManualTime,
}

impl ScoreEntry {
    pub fn validate(&self) -> Result<ScoringProfile, ScoreError> {
        let p = builtin(&self.profile_id, self.profile_version).ok_or(ScoreError::UnknownProfile)?;
        let known: Vec<&str> = match &p.kind {
            ProfileKind::PointsHitFactor { categories, penalties } => {
                categories.iter().chain(penalties.iter()).map(|(n, _)| n.as_str()).collect()
            }
            ProfileKind::TimePlus { penalties_ms } => penalties_ms.iter().map(|(n, _)| n.as_str()).collect(),
            _ => vec![],
        };
        for (name, n) in &self.counts {
            // An unknown penalty is an error, never silently ignored.
            if !known.contains(&name.as_str()) {
                return Err(ScoreError::UnknownName(name.clone()));
            }
            if *n > MAX_COUNT {
                return Err(ScoreError::CountTooLarge(name.clone()));
            }
        }
        if self.notes.chars().count() > 2000 {
            return Err(ScoreError::NotesTooLong);
        }
        if self.manual_elapsed_ns.is_some_and(|t| t <= 0) {
            return Err(ScoreError::ManualTime);
        }
        Ok(p)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", content = "reason", rename_all = "snake_case")]
pub enum ScoreStatus {
    Complete,
    /// Required input is missing; the result is not treated as zero.
    Incomplete(String),
    /// Inputs are present but cannot produce a result (e.g. zero time).
    Invalid(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoreResult {
    pub profile_id: String,
    pub profile_version: u32,
    pub calc_version: String,
    pub status: ScoreStatus,
    /// Elapsed time used (raw or with penalties), ns.
    pub elapsed_ns: Option<i64>,
    pub penalty_ns: Option<i64>,
    pub final_time_ns: Option<i64>,
    pub net_points: Option<i64>,
    /// Unrounded hit factor (points per second).
    pub hit_factor: Option<f64>,
    /// Rounded for display using the profile's rule.
    pub display: Option<String>,
    /// Manual or imported result: no detector involvement.
    pub manual: bool,
}

/// `run_elapsed_ns` is the final accepted event relative to a valid start reference,
/// or `None` when unavailable (no start origin, no events).
pub fn compute(entry: Option<&ScoreEntry>, profile: &ScoringProfile, run_elapsed_ns: Option<i64>) -> ScoreResult {
    let mut r = ScoreResult {
        profile_id: profile.id.clone(),
        profile_version: profile.version,
        calc_version: SCORING_CALC_VERSION.into(),
        status: ScoreStatus::Complete,
        elapsed_ns: None,
        penalty_ns: None,
        final_time_ns: None,
        net_points: None,
        hit_factor: None,
        display: None,
        manual: false,
    };
    let dec = usize::from(profile.display_decimals);
    let secs = |ns: i64| ns as f64 / 1e9;
    match &profile.kind {
        ProfileKind::TimeOnly => match run_elapsed_ns {
            Some(t) if t > 0 => {
                r.elapsed_ns = Some(t);
                r.final_time_ns = Some(t);
                r.display = Some(format!("{:.dec$} s", secs(t)));
            }
            Some(_) => r.status = ScoreStatus::Invalid("elapsed time is not positive".into()),
            None => r.status = ScoreStatus::Incomplete("no valid start and final event".into()),
        },
        ProfileKind::PointsHitFactor { categories, penalties } => {
            let Some(e) = entry.filter(|e| e.complete) else {
                r.status = ScoreStatus::Incomplete("score not entered".into());
                r.elapsed_ns = run_elapsed_ns;
                return r;
            };
            let value = |name: &str| categories.iter().chain(penalties.iter()).find(|(n, _)| n == name).map(|(_, v)| *v);
            let net: i64 = e.counts.iter().map(|(n, c)| i64::from(value(n).unwrap_or(0)) * i64::from(*c)).sum();
            r.net_points = Some(net);
            match run_elapsed_ns {
                None => r.status = ScoreStatus::Incomplete("no valid elapsed time".into()),
                Some(t) if t <= 0 => r.status = ScoreStatus::Invalid("zero or negative time".into()),
                Some(t) => {
                    r.elapsed_ns = Some(t);
                    r.final_time_ns = Some(t);
                    // Generic practice floors the hit factor at zero (documented choice).
                    let hf = net.max(0) as f64 / secs(t);
                    r.hit_factor = Some(hf);
                    r.display = Some(format!("{:.dec$} HF ({net} pts / {:.2} s)", hf, secs(t)));
                }
            }
        }
        ProfileKind::TimePlus { penalties_ms } => {
            let Some(t) = run_elapsed_ns else {
                r.status = ScoreStatus::Incomplete("no valid elapsed time".into());
                return r;
            };
            if t <= 0 {
                r.status = ScoreStatus::Invalid("zero or negative time".into());
                return r;
            }
            r.elapsed_ns = Some(t);
            let Some(e) = entry.filter(|e| e.complete) else {
                r.status = ScoreStatus::Incomplete("penalties not entered".into());
                return r;
            };
            let mut pen = 0i64;
            for (name, c) in &e.counts {
                match penalties_ms.iter().find(|(n, _)| n == name) {
                    Some((_, ms)) => pen += i64::from(*ms) * 1_000_000 * i64::from(*c),
                    None => {
                        r.status = ScoreStatus::Invalid(format!("unknown penalty '{name}'"));
                        return r;
                    }
                }
            }
            r.penalty_ns = Some(pen);
            r.final_time_ns = Some(t + pen);
            r.display = Some(format!("{:.dec$} s ({:.2} + {:.2})", secs(t + pen), secs(t), secs(pen)));
        }
        ProfileKind::ManualResult => {
            r.manual = true;
            let Some(e) = entry else {
                r.status = ScoreStatus::Incomplete("no result entered".into());
                return r;
            };
            r.elapsed_ns = e.manual_elapsed_ns;
            r.final_time_ns = e.manual_elapsed_ns;
            r.net_points = e.manual_points;
            if e.manual_elapsed_ns.is_none() && e.manual_points.is_none() {
                r.status = ScoreStatus::Incomplete("no result entered".into());
            } else {
                let mut parts = vec![];
                if let Some(t) = e.manual_elapsed_ns {
                    parts.push(format!("{:.dec$} s", secs(t)));
                }
                if let Some(p) = e.manual_points {
                    parts.push(format!("{p} pts"));
                }
                r.display = Some(parts.join(", ") + " (manual)");
            }
        }
    }
    r
}

/// A score entry revision: changes to scores never change timestamps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoreRevision {
    pub run_id: String,
    pub number: u32,
    pub parent_hash: Option<String>,
    pub entry: ScoreEntry,
    pub editor: String,
    pub created_utc_ms: i64,
    pub content_hash: String,
}

impl ScoreRevision {
    pub fn new(
        run_id: &str,
        parent: Option<&ScoreRevision>,
        entry: ScoreEntry,
        editor: &str,
        now: i64,
    ) -> Result<Self, ScoreError> {
        entry.validate()?;
        let number = parent.map(|p| p.number + 1).unwrap_or(1);
        let parent_hash = parent.map(|p| p.content_hash.clone());
        let content_hash = content_hash(&(run_id, number, &parent_hash, &entry, editor, now));
        Ok(Self { run_id: run_id.into(), number, parent_hash, entry, editor: editor.into(), created_utc_ms: now, content_hash })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(profile: &str, counts: &[(&str, u32)], complete: bool) -> ScoreEntry {
        ScoreEntry {
            profile_id: profile.into(),
            profile_version: 1,
            counts: counts.iter().map(|(k, v)| ((*k).to_string(), *v)).collect(),
            complete,
            manual_elapsed_ns: None,
            manual_precision_ns: None,
            manual_points: None,
            notes: String::new(),
        }
    }

    #[test]
    fn time_only_requires_a_valid_start() {
        let p = builtin("generic-time", 1).unwrap();
        assert_eq!(compute(None, &p, Some(3_210_000_000)).display.as_deref(), Some("3.21 s"));
        assert!(matches!(compute(None, &p, None).status, ScoreStatus::Incomplete(_)));
        assert!(matches!(compute(None, &p, Some(0)).status, ScoreStatus::Invalid(_)));
    }

    #[test]
    fn a15_hit_factor_incomplete_and_zero_time() {
        let p = builtin("generic-points-hf", 1).unwrap();
        let e = entry("generic-points-hf", &[("A", 8), ("C", 2), ("Miss penalty", 1)], true);
        e.validate().unwrap();
        let r = compute(Some(&e), &p, Some(4_000_000_000));
        assert_eq!(r.net_points, Some(36));
        assert!((r.hit_factor.unwrap() - 9.0).abs() < 1e-12, "unrounded HF");
        assert_eq!(r.display.as_deref(), Some("9.0000 HF (36 pts / 4.00 s)"));
        // Missing score is incomplete, not zero misses or full points.
        let r = compute(None, &p, Some(4_000_000_000));
        assert!(matches!(r.status, ScoreStatus::Incomplete(_)));
        assert_eq!(r.net_points, None);
        let r = compute(Some(&entry("generic-points-hf", &[("A", 1)], false)), &p, Some(4_000_000_000));
        assert!(matches!(r.status, ScoreStatus::Incomplete(_)), "unfinished entry is incomplete");
        assert!(matches!(compute(Some(&e), &p, Some(0)).status, ScoreStatus::Invalid(_)));
        assert!(matches!(compute(Some(&e), &p, None).status, ScoreStatus::Incomplete(_)));
        // Negative net points floor the HF at zero.
        let neg = entry("generic-points-hf", &[("Procedural", 3)], true);
        assert_eq!(compute(Some(&neg), &p, Some(1_000_000_000)).hit_factor, Some(0.0));
    }

    #[test]
    fn time_plus_and_unknown_penalties() {
        let p = builtin("generic-time-plus", 1).unwrap();
        let e = entry("generic-time-plus", &[("Point down", 3), ("Miss", 1)], true);
        let r = compute(Some(&e), &p, Some(5_500_000_000));
        assert_eq!(r.final_time_ns, Some(13_500_000_000));
        assert_eq!(r.penalty_ns, Some(8_000_000_000));
        let bad = entry("generic-time-plus", &[("Wrong", 1)], true);
        assert_eq!(bad.validate().unwrap_err(), ScoreError::UnknownName("Wrong".into()));
        assert!(matches!(compute(Some(&bad), &p, Some(1)).status, ScoreStatus::Invalid(_)), "unknown penalty never ignored");
    }

    #[test]
    fn manual_results_are_marked_manual() {
        let p = builtin("generic-manual", 1).unwrap();
        let mut e = entry("generic-manual", &[], true);
        e.manual_elapsed_ns = Some(4_560_000_000);
        e.manual_precision_ns = Some(10_000_000);
        let r = compute(Some(&e), &p, None);
        assert!(r.manual);
        assert_eq!(r.display.as_deref(), Some("4.56 s (manual)"));
        assert!(matches!(compute(None, &p, None).status, ScoreStatus::Incomplete(_)));
    }

    #[test]
    fn score_revisions_chain() {
        let e = entry("generic-points-hf", &[("A", 5)], true);
        let r1 = ScoreRevision::new("run", None, e.clone(), "local", 1).unwrap();
        let r2 = ScoreRevision::new("run", Some(&r1), e, "local", 2).unwrap();
        assert_eq!(r2.number, 2);
        assert_eq!(r2.parent_hash.as_deref(), Some(r1.content_hash.as_str()));
        assert!(builtin_profiles().iter().all(|p| p.discipline_label == "Generic practice"), "no official labels");
    }
}
