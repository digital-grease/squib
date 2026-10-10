//! Comparable, explainable summaries (docs/squib/07 "Analytics").
//!
//! Every summary carries its denominator, its exclusions by reason, its inclusion
//! policy, the revision ids it used, and a calculation version. Fewer than
//! `MIN_TREND_N` comparable runs are labelled insufficient. Quantiles use linear
//! interpolation between order statistics (Hyndman-Fan type 7).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use squib_domain::{Outcome, ReviewState};

pub const ANALYTICS_CALC_VERSION: &str = "analytics-v1";
pub const MIN_TREND_N: usize = 5;

/// One run, projected from a specific review revision and score revision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    pub run_id: String,
    pub created_utc_ms: i64,
    pub shooter_id: String,
    pub drill_id: Option<String>,
    pub drill_version: Option<u32>,
    /// `par_only`, `phone_live`, `manual_entry`, ...
    pub mode: String,
    pub scoring_profile: Option<(String, u32)>,
    pub equipment_tags: Vec<String>,
    /// Equipment profile the run used (any version), when one was selected.
    pub equipment_id: Option<String>,
    /// `acoustic_cue`, `scheduled_render`, `manual`, ...
    pub timing_method: String,
    pub outcome: Option<Outcome>,
    pub review_state: Option<ReviewState>,
    pub edited: bool,
    /// Manual or imported source.
    pub manual: bool,
    pub origin_resolved: bool,
    pub revision_number: Option<u32>,
    pub score_revision: Option<u32>,
    pub first_ns: Option<i64>,
    pub final_ns: Option<i64>,
    pub splits_ns: Vec<i64>,
    pub hit_factor: Option<f64>,
    /// True when the drill's scoring profile requires a score and it is complete.
    pub score_complete: bool,
    pub score_required: bool,
}

/// Comparability filter. Defaults exclude interrupted runs, unresolved review, missing
/// required scores, and edited/manual results unless explicitly included.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Filter {
    pub shooter_id: String,
    pub drill_id: Option<String>,
    /// `None` compares across all versions of the drill (explicitly chosen).
    pub drill_version: Option<u32>,
    pub mode: Option<String>,
    pub equipment_tag: Option<String>,
    /// Compare runs made with this equipment profile (all of its versions).
    pub equipment_id: Option<String>,
    pub from_utc_ms: Option<i64>,
    pub to_utc_ms: Option<i64>,
    pub include_edited: bool,
    pub include_manual: bool,
    pub include_needs_review: bool,
    /// Runs without a resolved start may contribute splits only.
    pub include_unresolved_start_splits: bool,
}

impl Filter {
    pub fn new(shooter_id: &str) -> Self {
        Self {
            shooter_id: shooter_id.into(),
            drill_id: None,
            drill_version: None,
            mode: None,
            equipment_tag: None,
            equipment_id: None,
            from_utc_ms: None,
            to_utc_ms: None,
            include_edited: true,
            include_manual: false,
            include_needs_review: false,
            include_unresolved_start_splits: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    pub n: usize,
    pub min: f64,
    pub q1: f64,
    pub median: f64,
    pub q3: f64,
    pub max: f64,
}

pub fn quantile(sorted: &[f64], q: f64) -> f64 {
    let h = (sorted.len() - 1) as f64 * q;
    let lo = h.floor() as usize;
    let hi = h.ceil() as usize;
    sorted[lo] + (h - lo as f64) * (sorted[hi] - sorted[lo])
}

pub fn stats(values: &[f64]) -> Option<Stats> {
    let mut v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    Some(Stats {
        n: v.len(),
        min: v[0],
        q1: quantile(&v, 0.25),
        median: quantile(&v, 0.5),
        q3: quantile(&v, 0.75),
        max: v[v.len() - 1],
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Exclusion {
    OtherShooter,
    OtherDrillOrVersion,
    OtherMode,
    OtherEquipment,
    OutsideDates,
    Interrupted,
    NotComplete,
    NeedsReview,
    Edited,
    Manual,
    MissingRequiredScore,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub calc_version: String,
    pub filter: Filter,
    pub considered: usize,
    pub included: usize,
    pub excluded: BTreeMap<Exclusion, usize>,
    /// Seconds.
    pub first_shot: Option<Stats>,
    pub splits: Option<Stats>,
    pub final_time: Option<Stats>,
    pub hit_factor: Option<Stats>,
    /// Fewer than `MIN_TREND_N` included runs: values shown, no trend claims.
    pub insufficient: bool,
    /// `(run_id, review revision, score revision)` used, for cache invalidation and audit.
    pub sources: Vec<(String, Option<u32>, Option<u32>)>,
    /// Mixed timing methods among included runs (quantization/quality differs).
    pub mixed_timing_methods: Vec<String>,
    /// Included runs in time order, speed and score side by side (doc 07 "Analytics").
    pub points: Vec<HistoryPoint>,
    /// Equal-width distribution of included final times (seconds) and hit factors.
    pub final_time_bins: Vec<Bin>,
    pub hit_factor_bins: Vec<Bin>,
}

/// One included run as plotted and listed: values only where they are valid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryPoint {
    pub run_id: String,
    pub created_utc_ms: i64,
    /// Final time in seconds; only for runs with a resolved start.
    pub final_s: Option<f64>,
    pub hit_factor: Option<f64>,
    pub score_complete: bool,
    pub edited: bool,
    pub manual: bool,
    pub timing_method: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bin {
    /// Inclusive lower and upper edges (the last bin includes its upper edge).
    pub lo: f64,
    pub hi: f64,
    pub count: u32,
}

/// Equal-width bins over `[min, max]`, about sqrt(n) of them (1 to 10). Every value
/// lands in exactly one bin; identical values give one bin.
pub fn histogram(values: &[f64]) -> Vec<Bin> {
    let v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return vec![];
    }
    let lo = v.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if hi - lo <= f64::EPSILON * hi.abs().max(1.0) {
        return vec![Bin { lo, hi, count: v.len() as u32 }];
    }
    let n = ((v.len() as f64).sqrt().ceil() as usize).clamp(1, 10);
    let w = (hi - lo) / n as f64;
    let mut bins: Vec<Bin> = (0..n).map(|i| Bin { lo: lo + w * i as f64, hi: lo + w * (i + 1) as f64, count: 0 }).collect();
    bins[n - 1].hi = hi;
    for x in v {
        let i = (((x - lo) / w).floor() as usize).min(n - 1);
        bins[i].count += 1;
    }
    bins
}

fn exclusion(r: &RunRecord, f: &Filter) -> Option<Exclusion> {
    if r.shooter_id != f.shooter_id {
        return Some(Exclusion::OtherShooter);
    }
    if f.drill_id.is_some() && r.drill_id != f.drill_id {
        return Some(Exclusion::OtherDrillOrVersion);
    }
    if f.drill_version.is_some() && r.drill_version != f.drill_version {
        return Some(Exclusion::OtherDrillOrVersion);
    }
    if f.mode.as_ref().is_some_and(|m| m != &r.mode) {
        return Some(Exclusion::OtherMode);
    }
    if f.equipment_tag.as_ref().is_some_and(|t| !r.equipment_tags.contains(t)) {
        return Some(Exclusion::OtherEquipment);
    }
    if f.equipment_id.is_some() && r.equipment_id != f.equipment_id {
        return Some(Exclusion::OtherEquipment);
    }
    if f.from_utc_ms.is_some_and(|t| r.created_utc_ms < t) || f.to_utc_ms.is_some_and(|t| r.created_utc_ms > t) {
        return Some(Exclusion::OutsideDates);
    }
    match r.outcome {
        Some(Outcome::Complete) => {}
        Some(Outcome::Interrupted) => return Some(Exclusion::Interrupted),
        _ => return Some(Exclusion::NotComplete),
    }
    if r.review_state == Some(ReviewState::NeedsReview) && !f.include_needs_review {
        return Some(Exclusion::NeedsReview);
    }
    if r.edited && !f.include_edited {
        return Some(Exclusion::Edited);
    }
    if r.manual && !f.include_manual {
        return Some(Exclusion::Manual);
    }
    if r.score_required && !r.score_complete {
        return Some(Exclusion::MissingRequiredScore);
    }
    None
}

pub fn summarize(records: &[RunRecord], f: &Filter) -> Summary {
    let mut excluded = BTreeMap::new();
    let mut inc: Vec<&RunRecord> = Vec::new();
    for r in records {
        match exclusion(r, f) {
            Some(e) => *excluded.entry(e).or_insert(0) += 1,
            None => inc.push(r),
        }
    }
    let s = |ns: i64| ns as f64 / 1e9;
    // Origin-relative values only from runs with a resolved start.
    let first: Vec<f64> = inc.iter().filter(|r| r.origin_resolved).filter_map(|r| r.first_ns.map(s)).collect();
    let fin: Vec<f64> = inc.iter().filter(|r| r.origin_resolved).filter_map(|r| r.final_ns.map(s)).collect();
    let splits: Vec<f64> = inc
        .iter()
        .filter(|r| r.origin_resolved || f.include_unresolved_start_splits)
        .flat_map(|r| r.splits_ns.iter().map(|x| s(*x)))
        .collect();
    let hf: Vec<f64> = inc.iter().filter_map(|r| r.hit_factor).collect();
    let mut methods: Vec<String> = inc.iter().map(|r| r.timing_method.clone()).collect();
    methods.sort();
    methods.dedup();
    Summary {
        calc_version: ANALYTICS_CALC_VERSION.into(),
        filter: f.clone(),
        considered: records.len(),
        included: inc.len(),
        excluded,
        first_shot: stats(&first),
        splits: stats(&splits),
        final_time: stats(&fin),
        hit_factor: stats(&hf),
        insufficient: inc.len() < MIN_TREND_N,
        sources: inc.iter().map(|r| (r.run_id.clone(), r.revision_number, r.score_revision)).collect(),
        mixed_timing_methods: if methods.len() > 1 { methods } else { vec![] },
        final_time_bins: histogram(&fin),
        hit_factor_bins: histogram(&hf),
        points: {
            let mut p: Vec<HistoryPoint> = inc
                .iter()
                .map(|r| HistoryPoint {
                    run_id: r.run_id.clone(),
                    created_utc_ms: r.created_utc_ms,
                    final_s: if r.origin_resolved { r.final_ns.map(s) } else { None },
                    hit_factor: r.hit_factor,
                    score_complete: r.score_complete,
                    edited: r.edited,
                    manual: r.manual,
                    timing_method: r.timing_method.clone(),
                })
                .collect();
            p.sort_by(|a, b| a.created_utc_ms.cmp(&b.created_utc_ms).then(a.run_id.cmp(&b.run_id)));
            p
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_bins_cover_every_value_once() {
        let v = [1.0, 1.5, 2.0, 2.0, 3.0, 4.0, 4.0, 5.0, 9.0];
        let b = histogram(&v);
        assert_eq!(b.len(), 3, "sqrt(9) bins");
        assert_eq!(b.iter().map(|x| x.count).sum::<u32>(), 9);
        assert_eq!((b[0].lo, b[2].hi), (1.0, 9.0), "spans min to max, max included");
        assert_eq!(b[2].count, 1);
        assert_eq!(histogram(&[2.5, 2.5]), vec![Bin { lo: 2.5, hi: 2.5, count: 2 }]);
        assert!(histogram(&[]).is_empty());
        assert_eq!(histogram(&[f64::NAN, 1.0]).iter().map(|x| x.count).sum::<u32>(), 1);
    }

    #[test]
    fn points_are_chronological_with_origin_values_only_when_resolved() {
        let mut late = rec("late", 2.0);
        late.created_utc_ms = 300;
        let mut early = rec("early", 3.0);
        early.created_utc_ms = 50;
        let mut unresolved = rec("u", 4.0);
        unresolved.created_utc_ms = 200;
        unresolved.origin_resolved = false;
        let mut f = Filter::new("me");
        f.include_manual = true;
        f.include_needs_review = true;
        let s = summarize(&[late, early, unresolved], &f);
        let ids: Vec<&str> = s.points.iter().map(|p| p.run_id.as_str()).collect();
        assert_eq!(ids, ["early", "u", "late"]);
        assert_eq!(s.points[1].final_s, None, "no final time without a resolved start");
        assert_eq!(s.final_time_bins.iter().map(|b| b.count).sum::<u32>(), 2);
    }

    #[test]
    fn equipment_profile_filter_excludes_other_profiles_with_a_reason() {
        let mut other = rec("b", 2.0);
        other.equipment_id = Some("e2".into());
        let mut none = rec("c", 2.0);
        none.equipment_id = None;
        let mut f = Filter::new("me");
        f.equipment_id = Some("e1".into());
        assert_eq!(exclusion(&rec("a", 2.0), &f), None);
        assert_eq!(exclusion(&other, &f), Some(Exclusion::OtherEquipment));
        assert_eq!(exclusion(&none, &f), Some(Exclusion::OtherEquipment));
    }

    fn rec(id: &str, final_s: f64) -> RunRecord {
        RunRecord {
            run_id: id.into(),
            created_utc_ms: 100,
            shooter_id: "me".into(),
            drill_id: Some("d".into()),
            drill_version: Some(1),
            mode: "phone_live".into(),
            scoring_profile: None,
            equipment_tags: vec!["pistol".into()],
            equipment_id: Some("e1".into()),
            timing_method: "acoustic_cue".into(),
            outcome: Some(Outcome::Complete),
            review_state: Some(ReviewState::Automatic),
            edited: false,
            manual: false,
            origin_resolved: true,
            revision_number: Some(1),
            score_revision: None,
            first_ns: Some(1_000_000_000),
            final_ns: Some((final_s * 1e9) as i64),
            splits_ns: vec![250_000_000, 300_000_000],
            hit_factor: None,
            score_complete: false,
            score_required: false,
        }
    }

    #[test]
    fn quantiles_type7() {
        let s = stats(&[1.0, 2.0, 3.0, 4.0]).unwrap();
        assert_eq!((s.q1, s.median, s.q3), (1.75, 2.5, 3.25));
        assert_eq!(stats(&[]), None);
        assert_eq!(stats(&[7.0]).unwrap().median, 7.0);
    }

    #[test]
    fn a16_filters_change_denominators_and_exclusions_are_reported() {
        let mut rs: Vec<RunRecord> = (0..6).map(|i| rec(&format!("r{i}"), 3.0 + i as f64)).collect();
        rs[0].outcome = Some(Outcome::Interrupted);
        rs[1].review_state = Some(ReviewState::NeedsReview);
        rs[2].manual = true;
        rs[3].drill_version = Some(2);
        let mut f = Filter::new("me");
        f.drill_id = Some("d".into());
        f.drill_version = Some(1);
        let s = summarize(&rs, &f);
        assert_eq!(s.considered, 6);
        assert_eq!(s.included, 2);
        assert_eq!(s.excluded[&Exclusion::Interrupted], 1);
        assert_eq!(s.excluded[&Exclusion::NeedsReview], 1);
        assert_eq!(s.excluded[&Exclusion::Manual], 1);
        assert_eq!(s.excluded[&Exclusion::OtherDrillOrVersion], 1);
        assert!(s.insufficient, "two runs is not a trend");
        assert_eq!(s.final_time.as_ref().unwrap().median, 7.5);
        // Explicit inclusion changes the denominator, visibly.
        f.include_manual = true;
        f.include_needs_review = true;
        f.drill_version = None;
        let s2 = summarize(&rs, &f);
        assert_eq!(s2.included, 5);
        assert!(!s2.insufficient);
        assert_eq!(s2.sources.len(), 5);
    }

    #[test]
    fn unresolved_start_contributes_splits_only_when_chosen() {
        let mut a = rec("a", 4.0);
        a.origin_resolved = false;
        a.first_ns = None;
        a.final_ns = None;
        let b = rec("b", 5.0);
        let mut f = Filter::new("me");
        let s = summarize(&[a.clone(), b.clone()], &f);
        assert_eq!(s.splits.as_ref().unwrap().n, 2, "only b's splits");
        assert_eq!(s.final_time.as_ref().unwrap().n, 1);
        f.include_unresolved_start_splits = true;
        let s = summarize(&[a, b], &f);
        assert_eq!(s.splits.unwrap().n, 4);
        assert_eq!(s.final_time.unwrap().n, 1, "never origin-relative values from unresolved starts");
    }

    #[test]
    fn missing_required_score_excluded_and_mixed_methods_flagged() {
        let mut a = rec("a", 4.0);
        a.score_required = true;
        let mut b = rec("b", 5.0);
        b.timing_method = "scheduled_render".into();
        let s = summarize(&[a, b], &Filter::new("me"));
        assert_eq!(s.excluded[&Exclusion::MissingRequiredScore], 1);
        assert!(s.mixed_timing_methods.is_empty());
        let mut c = rec("c", 6.0);
        c.timing_method = "manual".into();
        c.manual = true;
        let mut f = Filter::new("me");
        f.include_manual = true;
        let s = summarize(&[rec("d", 3.0), c], &f);
        assert_eq!(s.mixed_timing_methods, vec!["acoustic_cue".to_string(), "manual".to_string()]);
    }
}
