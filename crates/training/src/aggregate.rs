//! Repeated-string aggregates (docs/squib/07 scoring table: "Repeated-string aggregate
//! | Named keep/drop/count policy over completed strings | No universal drop-one rule").
//!
//! The policy is part of a drill version, so it is versioned with the drill. Strings are
//! the drill's runs in time order. Only strings that completed with a complete score are
//! counted; interrupted, cancelled, and unscored strings are listed but never counted,
//! and a set with fewer counted strings than the drill requires is Incomplete, never a
//! partial total presented as final.

use serde::{Deserialize, Serialize};

use crate::analytics::RunRecord;
use crate::scoring::{ProfileKind, ScoreStatus, ScoringProfile};

pub const AGGREGATE_CALC_VERSION: &str = "string-aggregate-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Combine {
    Sum,
    Mean,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregatePolicy {
    pub combine: Combine,
    /// Keep only this many best strings of the required set (drop the rest). `None`
    /// keeps every required string.
    pub keep_best: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    /// Final time in seconds (with penalty time where the profile adds it); lower is better.
    Seconds,
    /// Hit factor (points per second); higher is better.
    HitFactor,
}

pub fn metric_for(profile: &ScoringProfile) -> Metric {
    match profile.kind {
        ProfileKind::PointsHitFactor { .. } => Metric::HitFactor,
        _ => Metric::Seconds,
    }
}

impl AggregatePolicy {
    /// Checked against the drill's string count and scoring profile.
    pub fn validate(&self, repeats: u32, metric: Metric) -> Result<(), String> {
        if repeats < 2 {
            return Err("a set result needs at least two strings".into());
        }
        if let Some(k) = self.keep_best
            && !(1..=repeats).contains(&k)
        {
            return Err(format!("keep best must be 1 to {repeats}"));
        }
        if self.combine == Combine::Sum && metric == Metric::HitFactor {
            return Err("hit factors cannot be totalled; use the average".into());
        }
        Ok(())
    }

    pub fn label(&self, repeats: u32) -> String {
        let what = match self.combine {
            Combine::Sum => "Total",
            Combine::Mean => "Average",
        };
        match self.keep_best {
            Some(k) if k < repeats => format!("{what} of the best {k} of {repeats} strings"),
            _ => format!("{what} of {repeats} strings"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetString {
    pub run_id: String,
    pub created_utc_ms: i64,
    /// The string's value in the policy's metric, when it has a complete score.
    pub value: Option<f64>,
    /// Counted toward the result.
    pub counted: bool,
    /// Scored and within the set, but dropped by keep-best.
    pub dropped: bool,
    /// Why a string is not counted: `not_complete`, `score_incomplete`, `extra`.
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetResult {
    pub calc_version: String,
    pub label: String,
    pub metric: Metric,
    pub required: u32,
    pub scored: u32,
    pub value: Option<f64>,
    pub status: ScoreStatus,
    pub strings: Vec<SetString>,
}

fn value_of(r: &RunRecord, metric: Metric) -> Option<f64> {
    match metric {
        Metric::Seconds => r.final_ns.map(|ns| ns as f64 / 1e9),
        Metric::HitFactor => r.hit_factor,
    }
}

/// `strings` are the set's runs in time order.
pub fn aggregate(strings: &[&RunRecord], policy: &AggregatePolicy, repeats: u32, metric: Metric) -> SetResult {
    let mut out: Vec<SetString> = Vec::new();
    let mut in_set: Vec<usize> = Vec::new();
    for r in strings {
        let complete = r.outcome == Some(squib_domain::Outcome::Complete);
        let scored = complete && (r.score_complete || !r.score_required);
        let value = if scored { value_of(r, metric) } else { None };
        let note = if !complete {
            Some("not_complete".to_string())
        } else if value.is_none() {
            Some("score_incomplete".to_string())
        } else if in_set.len() as u32 >= repeats {
            Some("extra".to_string())
        } else {
            None
        };
        if note.is_none() {
            in_set.push(out.len());
        }
        out.push(SetString {
            run_id: r.run_id.clone(),
            created_utc_ms: r.created_utc_ms,
            value,
            counted: false,
            dropped: false,
            note,
        });
    }
    let scored = in_set.len() as u32;
    let label = policy.label(repeats);
    if scored < repeats {
        return SetResult {
            calc_version: AGGREGATE_CALC_VERSION.into(),
            label,
            metric,
            required: repeats,
            scored,
            value: None,
            status: ScoreStatus::Incomplete(format!("{scored} of {repeats} strings completed and scored")),
            strings: out,
        };
    }
    // Rank the set's strings best first; ties keep time order.
    let mut ranked = in_set.clone();
    ranked.sort_by(|a, b| {
        let (x, y) = (out[*a].value.unwrap_or(0.0), out[*b].value.unwrap_or(0.0));
        let ord = match metric {
            Metric::Seconds => x.partial_cmp(&y),
            Metric::HitFactor => y.partial_cmp(&x),
        };
        ord.unwrap_or(std::cmp::Ordering::Equal).then(a.cmp(b))
    });
    let keep = policy.keep_best.unwrap_or(repeats).min(repeats) as usize;
    for (pos, i) in ranked.iter().enumerate() {
        if pos < keep {
            out[*i].counted = true;
        } else {
            out[*i].dropped = true;
        }
    }
    let kept: Vec<f64> = out.iter().filter(|s| s.counted).filter_map(|s| s.value).collect();
    let total: f64 = kept.iter().sum();
    let value = match policy.combine {
        Combine::Sum => total,
        Combine::Mean => total / kept.len() as f64,
    };
    SetResult {
        calc_version: AGGREGATE_CALC_VERSION.into(),
        label,
        metric,
        required: repeats,
        scored,
        value: Some(value),
        status: ScoreStatus::Complete,
        strings: out,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use squib_domain::Outcome;

    fn rec(id: &str, at: i64, final_s: Option<f64>, outcome: Outcome) -> RunRecord {
        RunRecord {
            run_id: id.into(),
            created_utc_ms: at,
            shooter_id: "me".into(),
            drill_id: Some("d".into()),
            drill_version: Some(1),
            mode: "par_only".into(),
            scoring_profile: None,
            equipment_tags: vec![],
            equipment_id: None,
            timing_method: "manual".into(),
            outcome: Some(outcome),
            review_state: None,
            edited: false,
            manual: true,
            origin_resolved: true,
            revision_number: Some(1),
            score_revision: None,
            first_ns: None,
            final_ns: final_s.map(|s| (s * 1e9) as i64),
            splits_ns: vec![],
            hit_factor: None,
            score_complete: final_s.is_some(),
            score_required: false,
        }
    }

    fn close(a: Option<f64>, b: f64) -> bool {
        a.is_some_and(|v| (v - b).abs() < 1e-9)
    }

    #[test]
    fn total_and_keep_best_count_only_completed_scored_strings() {
        let rs = [
            rec("a", 1, Some(2.0), Outcome::Complete),
            rec("b", 2, Some(9.0), Outcome::Interrupted), // aborted: listed, never counted
            rec("c", 3, Some(3.0), Outcome::Complete),
            rec("d", 4, Some(2.5), Outcome::Complete),
            rec("e", 5, Some(1.0), Outcome::Complete), // beyond the set: extra
        ];
        let refs: Vec<&RunRecord> = rs.iter().collect();
        let all = aggregate(&refs, &AggregatePolicy { combine: Combine::Sum, keep_best: None }, 3, Metric::Seconds);
        assert_eq!(all.status, ScoreStatus::Complete);
        assert!(close(all.value, 7.5), "{all:?}");
        assert_eq!(all.label, "Total of 3 strings");
        let note = |id: &str| all.strings.iter().find(|s| s.run_id == id).unwrap().note.clone();
        assert_eq!((note("b").as_deref(), note("e").as_deref()), (Some("not_complete"), Some("extra")));

        let best = aggregate(&refs, &AggregatePolicy { combine: Combine::Sum, keep_best: Some(2) }, 3, Metric::Seconds);
        assert!(close(best.value, 4.5), "drops the slowest 3.0 s string");
        assert!(best.strings.iter().find(|s| s.run_id == "c").unwrap().dropped);
        assert_eq!(best.label, "Total of the best 2 of 3 strings");
    }

    #[test]
    fn short_sets_are_incomplete_never_partial_totals() {
        let rs = [rec("a", 1, Some(2.0), Outcome::Complete), rec("b", 2, None, Outcome::Complete)];
        let refs: Vec<&RunRecord> = rs.iter().collect();
        let r = aggregate(&refs, &AggregatePolicy { combine: Combine::Mean, keep_best: None }, 2, Metric::Seconds);
        assert_eq!(r.value, None);
        assert_eq!(r.status, ScoreStatus::Incomplete("1 of 2 strings completed and scored".into()));
        assert_eq!(r.strings[1].note.as_deref(), Some("score_incomplete"));
    }

    #[test]
    fn hit_factor_sets_average_best_first_and_refuse_totals() {
        let mut a = rec("a", 1, Some(2.0), Outcome::Complete);
        a.hit_factor = Some(5.0);
        let mut b = rec("b", 2, Some(2.0), Outcome::Complete);
        b.hit_factor = Some(7.0);
        let mut c = rec("c", 3, Some(2.0), Outcome::Complete);
        c.hit_factor = Some(6.0);
        let refs = vec![&a, &b, &c];
        let p = AggregatePolicy { combine: Combine::Mean, keep_best: Some(2) };
        assert!(p.validate(3, Metric::HitFactor).is_ok());
        let r = aggregate(&refs, &p, 3, Metric::HitFactor);
        assert!(close(r.value, 6.5), "keeps the two highest hit factors");
        assert!(AggregatePolicy { combine: Combine::Sum, keep_best: None }.validate(3, Metric::HitFactor).is_err());
        assert!(p.validate(1, Metric::Seconds).is_err(), "one string is not a set");
        assert!(AggregatePolicy { combine: Combine::Sum, keep_best: Some(4) }.validate(3, Metric::Seconds).is_err());
    }
}
