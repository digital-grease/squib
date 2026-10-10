//! Repeated-string set results (plan 1.3): a drill version's set rule applied to the
//! strings of one visit, or of one day-plan drill item.

use std::collections::BTreeMap;

use squib_storage::Repository;
use squib_training::aggregate::{Metric, SetResult, aggregate, metric_for};
use squib_training::analytics::RunRecord;
use squib_training::scoring::{ScoreStatus, builtin};

use crate::engine::SquibEngine;
use crate::ffi::SquibError;

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SetStringView {
    pub run_id: String,
    pub created_utc_ms: i64,
    pub value: Option<f64>,
    pub counted: bool,
    pub dropped: bool,
    /// `not_complete`, `score_incomplete`, or `extra`.
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct StringSetView {
    pub drill_id: String,
    pub drill_version: u32,
    pub drill_title: String,
    /// e.g. "Total of the best 4 of 5 strings".
    pub label: String,
    /// `seconds` (lower is better) or `hit_factor` (higher is better).
    pub metric: String,
    pub required: u32,
    pub scored: u32,
    pub value: Option<f64>,
    /// `complete` or `incomplete`.
    pub status: String,
    pub reason: Option<String>,
    pub calc_version: String,
    pub strings: Vec<SetStringView>,
}

fn view(drill_id: &str, version: u32, title: String, r: SetResult) -> StringSetView {
    let (status, reason) = match r.status {
        ScoreStatus::Complete => ("complete", None),
        ScoreStatus::Incomplete(x) | ScoreStatus::Invalid(x) => ("incomplete", Some(x)),
    };
    StringSetView {
        drill_id: drill_id.into(),
        drill_version: version,
        drill_title: title,
        label: r.label,
        metric: match r.metric {
            Metric::Seconds => "seconds",
            Metric::HitFactor => "hit_factor",
        }
        .into(),
        required: r.required,
        scored: r.scored,
        value: r.value,
        status: status.into(),
        reason,
        calc_version: r.calc_version,
        strings: r
            .strings
            .into_iter()
            .map(|s| SetStringView {
                run_id: s.run_id,
                created_utc_ms: s.created_utc_ms,
                value: s.value,
                counted: s.counted,
                dropped: s.dropped,
                note: s.note,
            })
            .collect(),
    }
}

/// Set results for the given runs, one per drill version that defines a set rule.
fn sets_for(repo: &Repository, run_ids: &[String]) -> Result<Vec<StringSetView>, SquibError> {
    let records = repo.analytics_records()?;
    let mut groups: BTreeMap<(String, u32), Vec<&RunRecord>> = BTreeMap::new();
    for r in records.iter().filter(|r| run_ids.contains(&r.run_id)) {
        if let (Some(d), Some(v)) = (&r.drill_id, r.drill_version) {
            groups.entry((d.clone(), v)).or_default().push(r);
        }
    }
    let mut out = Vec::new();
    for ((d, v), mut runs) in groups {
        let Some(dv) = repo.load_drill_version(&d, v)? else { continue };
        let Some(policy) = dv.recipe.aggregate.as_ref() else { continue };
        let metric = builtin(&dv.recipe.scoring_profile_id, dv.recipe.scoring_profile_version)
            .as_ref()
            .map(metric_for)
            .unwrap_or(Metric::Seconds);
        runs.sort_by(|a, b| a.created_utc_ms.cmp(&b.created_utc_ms).then(a.run_id.cmp(&b.run_id)));
        out.push(view(&d, v, dv.recipe.title.clone(), aggregate(&runs, policy, dv.recipe.repeats, metric)));
    }
    Ok(out)
}

#[uniffi::export]
impl SquibEngine {
    /// Set results for drills run during one visit.
    pub fn session_sets(&self, session_id: String) -> Result<Vec<StringSetView>, SquibError> {
        let repo = self.read_repo();
        let ids = repo.session_run_ids(&session_id)?;
        sets_for(&repo, &ids)
    }

    /// The set result of one day-plan drill item, from the runs linked to it.
    pub fn plan_item_set(&self, item_id: String) -> Result<Option<StringSetView>, SquibError> {
        let repo = self.read_repo();
        let ids = repo.item_run_ids(&item_id)?;
        Ok(sets_for(&repo, &ids)?.into_iter().next())
    }
}
