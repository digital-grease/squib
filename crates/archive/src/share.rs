//! Shareable results (A19): a redacted subset, separate from private backup.
//!
//! Included: date (day only), mode, drill title, shot count, first/final times,
//! splits, score result, timing-method and Edited/Manual flags, and conditions values
//! with origin category and age. Excluded by default: coordinates, station ids and
//! names, place labels, elevation, notes, attachments, run/device identifiers.

use serde::{Deserialize, Serialize};
use squib_environment::Field;
use squib_storage::Repository;

use crate::Result;

pub const SHARE_FORMAT: &str = "squib-shared-results";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SharedCondition {
    pub field: String,
    /// SI value as a string (K, fraction, m/s, degrees, Pa).
    pub value_si: Option<String>,
    pub state: String,
    pub origin: String,
    pub age_s: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SharedRun {
    pub date: String,
    pub mode: String,
    pub drill: Option<String>,
    pub shots: u32,
    pub first_shot_s: Option<String>,
    pub final_time_s: Option<String>,
    pub splits_s: Vec<String>,
    pub hit_factor: Option<String>,
    pub timing_method: String,
    pub edited: bool,
    pub manual: bool,
    pub outcome: Option<String>,
    pub conditions: Vec<SharedCondition>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SharedResults {
    pub format: String,
    pub version: u32,
    pub note: String,
    pub runs: Vec<SharedRun>,
}

/// Build the redacted share payload for the selected runs (preview before sending).
pub fn share_results(repo: &Repository, run_ids: &[String]) -> Result<SharedResults> {
    let records = repo.analytics_records()?;
    let mut runs = Vec::new();
    for r in records.iter().filter(|r| run_ids.contains(&r.run_id)) {
        let detail = repo.load_run(&r.run_id)?;
        let s3 = |ns: i64| format!("{:.2}", ns as f64 / 1e9);
        let drill = match (&r.drill_id, r.drill_version) {
            (Some(d), Some(v)) => repo.load_drill_version(d, v)?.map(|dv| dv.recipe.title),
            _ => None,
        };
        let conditions = detail
            .environment
            .as_ref()
            .map(|s| {
                [Field::Temperature, Field::RelativeHumidity, Field::WindSpeed, Field::WindDirection, Field::LocalPressure]
                    .iter()
                    .filter_map(|f| s.field(*f))
                    .filter_map(|rf| {
                        let c = rf.chosen.as_ref()?;
                        Some(SharedCondition {
                            field: rf.field.as_str().into(),
                            value_si: c.value.si().map(|v| format!("{v:.2}")),
                            state: serde_json::to_value(c.value).ok()?.get("kind")?.as_str()?.to_string(),
                            origin: serde_json::to_value(rf.origin).ok()?.as_str()?.to_string(),
                            age_s: c.age_ms(s.created_utc_ms).map(|a| a / 1000),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        runs.push(SharedRun {
            date: crate::iso_utc(r.created_utc_ms)[..10].to_string(),
            mode: r.mode.clone(),
            drill,
            shots: detail.revisions.last().map(|v| v.content.accepted.len() as u32).unwrap_or(0),
            first_shot_s: r.first_ns.map(s3),
            final_time_s: r.final_ns.map(s3),
            splits_s: r.splits_ns.iter().map(|x| s3(*x)).collect(),
            hit_factor: r.hit_factor.map(|h| format!("{h:.4}")),
            timing_method: r.timing_method.clone(),
            edited: r.edited,
            manual: r.manual,
            outcome: r.outcome.map(|o| o.as_str().into()),
            conditions,
        });
    }
    Ok(SharedResults {
        format: SHARE_FORMAT.into(),
        version: 1,
        note: "Practice results from Squib. Timing is experimental unless stated; values are not official scores.".into(),
        runs,
    })
}
