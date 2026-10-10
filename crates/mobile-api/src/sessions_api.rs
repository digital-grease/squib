//! Sessions facade (plan phase 1.1): range visits with notes, saved cost, round totals,
//! the visit's latest pinned conditions, and per-session deletion.

use squib_training::rounds::{CostSetting, summarize};

use crate::conditions::{FieldView, conditions_view_of};
use crate::engine::SquibEngine;
use crate::ffi::{RunListItem, SquibError};
use crate::training_api::DeletionView;

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SessionView {
    pub id: String,
    pub shooter_name: String,
    pub started_utc_ms: i64,
    pub tz_offset_min: i32,
    pub ended_utc_ms: Option<i64>,
    pub active: bool,
    pub notes: String,
    pub run_count: u32,
    pub complete_runs: u32,
    pub last_run_utc_ms: Option<i64>,
    /// Confirmed rounds only; runs with a proposal but no confirmation are counted separately.
    pub confirmed_rounds: u64,
    pub unconfirmed_runs: u32,
    /// Cost of confirmed rounds at the price saved with this session.
    pub cost_minor: Option<i64>,
    pub currency: Option<String>,
    pub per_round_minor: Option<i64>,
    /// Temperature, humidity, wind, and pressure from the newest run's pinned snapshot,
    /// with their original sources and ages as of that run.
    pub conditions: Vec<FieldView>,
}

const SESSION_FIELDS: [&str; 5] = ["temperature", "relative_humidity", "wind_speed", "wind_direction", "local_pressure"];

#[uniffi::export]
impl SquibEngine {
    pub fn list_sessions(&self, limit: u32) -> Result<Vec<SessionView>, SquibError> {
        let repo = self.read_repo();
        let names: std::collections::BTreeMap<String, String> =
            repo.list_shooters()?.into_iter().map(|s| (s.id, s.name)).collect();
        let counts = repo.round_counts()?;
        let mut out = Vec::new();
        for s in repo.session_summaries(limit.min(500))? {
            let ids = repo.session_run_ids(&s.id)?;
            let mine: Vec<_> = counts.iter().filter(|c| ids.contains(&c.run_id)).cloned().collect();
            let cost: Option<CostSetting> = s.cost_json.as_deref().and_then(|j| serde_json::from_str(j).ok());
            let r = summarize(&mine, cost.as_ref());
            let conditions = match &s.latest_snapshot_id {
                Some(id) => repo
                    .load_snapshot(id)?
                    .map(|snap| {
                        conditions_view_of(&snap, snap.created_utc_ms)
                            .into_iter()
                            .filter(|f| SESSION_FIELDS.contains(&f.field.as_str()) && f.origin != "unavailable")
                            .collect()
                    })
                    .unwrap_or_default(),
                None => vec![],
            };
            out.push(SessionView {
                shooter_name: names.get(&s.shooter_id).cloned().unwrap_or_else(|| s.shooter_id.clone()),
                id: s.id,
                started_utc_ms: s.started_utc_ms,
                tz_offset_min: s.tz_offset_min,
                ended_utc_ms: s.ended_utc_ms,
                active: s.active,
                notes: s.notes,
                run_count: s.run_count,
                complete_runs: s.complete_runs,
                last_run_utc_ms: s.last_run_utc_ms,
                confirmed_rounds: r.confirmed_rounds,
                unconfirmed_runs: r.unconfirmed_runs,
                cost_minor: r.cost_minor,
                currency: r.currency,
                per_round_minor: cost.map(|c| c.per_round_minor),
                conditions,
            });
        }
        Ok(out)
    }

    /// The session's runs, newest first, in the same form as History.
    pub fn session_runs(&self, session_id: String) -> Result<Vec<RunListItem>, SquibError> {
        let ids = self.read_repo().session_run_ids(&session_id)?;
        Ok(self.list_runs(1000)?.into_iter().filter(|r| ids.contains(&r.run_id)).collect())
    }

    pub fn set_session_notes(&self, session_id: String, notes: String) -> Result<(), SquibError> {
        self.store_actor().exec(move |repo| repo.set_session_notes(&session_id, &notes))?;
        Ok(())
    }

    /// End the visit now; the next run starts a new session. Refused during a run.
    pub fn end_session(&self, session_id: String, now_utc_ms: i64) -> Result<(), SquibError> {
        if self.run_active() {
            return Err(SquibError::Rejected("finish or cancel the current run first".into()));
        }
        self.store_actor().exec(move |repo| repo.end_session(&session_id, now_utc_ms))?;
        Ok(())
    }

    /// Change the per-round cost saved with one session (`None` clears it).
    pub fn set_session_cost(
        &self,
        session_id: String,
        per_round_minor: Option<i64>,
        currency: Option<String>,
    ) -> Result<(), SquibError> {
        let json = match (per_round_minor, currency) {
            (Some(p), Some(c)) if p >= 0 && c.len() == 3 && c.chars().all(|x| x.is_ascii_alphabetic()) => Some(
                serde_json::to_string(&CostSetting { per_round_minor: p, currency: c.to_ascii_uppercase() })
                    .map_err(|e| SquibError::Invalid(e.to_string()))?,
            ),
            (None, _) => None,
            _ => return Err(SquibError::Invalid("cost needs a non-negative amount and a 3-letter currency code".into())),
        };
        self.store_actor().exec(move |repo| repo.set_session_cost(&session_id, json.as_deref()))?;
        Ok(())
    }

    /// Delete one visit: its runs and everything that belongs to them. Refused during a run.
    pub fn delete_session(&self, session_id: String) -> Result<DeletionView, SquibError> {
        if self.run_active() {
            return Err(SquibError::Rejected("finish or cancel the current run first".into()));
        }
        let d = self.store_actor().exec(move |repo| repo.delete_session(&session_id))?;
        Ok(DeletionView { runs: d.runs, attachment_paths: d.attachment_paths })
    }
}
