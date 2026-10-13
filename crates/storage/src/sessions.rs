//! Sessions as range visits (docs/squib/02 "History and practice", 06 `Session`).
//!
//! A run joins its shooter's active session; a session ends when the user ends it or
//! when the next run comes after a long idle gap. Notes and the per-round cost are
//! saved with the session.

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::{Repository, Result, StorageError};

/// A run more than this long after the session's last activity starts a new session.
pub const SESSION_IDLE_MS: i64 = 4 * 3_600_000;
pub const MAX_SESSION_NOTES: usize = 4000;
/// Setting holding the current per-round cost (JSON), copied into new sessions.
pub const SETTING_ROUND_COST: &str = "round_cost";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub shooter_id: String,
    pub started_utc_ms: i64,
    pub tz_offset_min: i32,
    pub ended_utc_ms: Option<i64>,
    pub active: bool,
    pub notes: String,
    pub run_count: u32,
    pub complete_runs: u32,
    pub last_run_utc_ms: Option<i64>,
    /// Environment snapshot pinned by the session's most recent run that has one.
    pub latest_snapshot_id: Option<String>,
    /// Per-round cost saved with the session (JSON), if any.
    pub cost_json: Option<String>,
}

impl Repository {
    /// The session a new run joins: the shooter's active session, unless it has been idle
    /// longer than `SESSION_IDLE_MS`, in which case it is ended at its last activity and
    /// a new one begins with the current cost setting.
    pub fn active_session(&self, shooter_id: &str, new_id: &str, now_utc_ms: i64, tz_offset_min: i32) -> Result<String> {
        let existing: Option<(String, i64)> = self
            .conn
            .query_row(
                "SELECT s.id, MAX(s.started_utc_ms, COALESCE((SELECT MAX(created_utc_ms) FROM run WHERE session_id = s.id), 0))
                 FROM session s WHERE s.shooter_id = ?1 AND s.active = 1 ORDER BY s.started_utc_ms DESC LIMIT 1",
                params![shooter_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((id, last)) = existing {
            if now_utc_ms - last <= SESSION_IDLE_MS {
                return Ok(id);
            }
            self.conn.execute("UPDATE session SET active = 0, ended_utc_ms = ?2 WHERE id = ?1", params![id, last])?;
        }
        let cost = self.get_setting(SETTING_ROUND_COST)?;
        self.conn.execute(
            "INSERT INTO session(id, shooter_id, started_utc_ms, tz_offset_min, active, cost_json) VALUES (?1, ?2, ?3, ?4, 1, ?5)",
            params![new_id, shooter_id, now_utc_ms, tz_offset_min, cost],
        )?;
        Ok(new_id.to_string())
    }

    /// The shooter's current visit, if one is active and not idle; never starts one.
    /// Used for records that belong to a visit only when one is under way (range cards).
    pub fn current_session(&self, shooter_id: &str, now_utc_ms: i64) -> Result<Option<String>> {
        let existing: Option<(String, i64)> = self
            .conn
            .query_row(
                "SELECT s.id, MAX(s.started_utc_ms, COALESCE((SELECT MAX(created_utc_ms) FROM run WHERE session_id = s.id), 0))
                 FROM session s WHERE s.shooter_id = ?1 AND s.active = 1 ORDER BY s.started_utc_ms DESC LIMIT 1",
                params![shooter_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(existing.filter(|(_, last)| now_utc_ms - last <= SESSION_IDLE_MS).map(|(id, _)| id))
    }

    /// End a session now; the next run starts a new one.
    pub fn end_session(&self, session_id: &str, now_utc_ms: i64) -> Result<()> {
        let n = self.conn.execute(
            "UPDATE session SET active = 0, ended_utc_ms = COALESCE(ended_utc_ms, ?2) WHERE id = ?1",
            params![session_id, now_utc_ms],
        )?;
        if n == 0 {
            return Err(StorageError::NotFound(format!("session {session_id}")));
        }
        Ok(())
    }

    pub fn set_session_notes(&self, session_id: &str, notes: &str) -> Result<()> {
        if notes.chars().count() > MAX_SESSION_NOTES {
            return Err(StorageError::Conflict(format!("notes must be at most {MAX_SESSION_NOTES} characters")));
        }
        let v = if notes.trim().is_empty() { None } else { Some(notes) };
        self.conn.execute("UPDATE session SET notes = ?2 WHERE id = ?1", params![session_id, v])?;
        Ok(())
    }

    /// Replace the cost saved with one session (`None` clears it).
    pub fn set_session_cost(&self, session_id: &str, cost_json: Option<&str>) -> Result<()> {
        self.conn.execute("UPDATE session SET cost_json = ?2 WHERE id = ?1", params![session_id, cost_json])?;
        Ok(())
    }

    pub fn session_summaries(&self, limit: u32) -> Result<Vec<SessionSummary>> {
        let mut st = self.conn.prepare(
            "SELECT s.id, s.shooter_id, s.started_utc_ms, s.tz_offset_min, s.ended_utc_ms, s.active, s.notes, s.cost_json,
                    (SELECT COUNT(*) FROM run r WHERE r.session_id = s.id),
                    (SELECT COUNT(*) FROM run r WHERE r.session_id = s.id AND r.outcome = 'complete'),
                    (SELECT MAX(created_utc_ms) FROM run r WHERE r.session_id = s.id),
                    (SELECT json_extract(r.config_json, '$.environment_snapshot_id') FROM run r
                       WHERE r.session_id = s.id AND json_extract(r.config_json, '$.environment_snapshot_id') IS NOT NULL
                       ORDER BY r.created_utc_ms DESC LIMIT 1)
             FROM session s
             WHERE EXISTS (SELECT 1 FROM run r WHERE r.session_id = s.id) OR s.active = 1
             ORDER BY COALESCE((SELECT MAX(created_utc_ms) FROM run r WHERE r.session_id = s.id), s.started_utc_ms) DESC
             LIMIT ?1",
        )?;
        let rows = st.query_map(params![limit], |r| {
            Ok(SessionSummary {
                id: r.get(0)?,
                shooter_id: r.get(1)?,
                started_utc_ms: r.get(2)?,
                tz_offset_min: r.get(3)?,
                ended_utc_ms: r.get(4)?,
                active: r.get(5)?,
                notes: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
                cost_json: r.get(7)?,
                run_count: r.get(8)?,
                complete_runs: r.get(9)?,
                last_run_utc_ms: r.get(10)?,
                latest_snapshot_id: r.get(11)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn session_run_ids(&self, session_id: &str) -> Result<Vec<String>> {
        let mut st = self.conn.prepare("SELECT id FROM run WHERE session_id = ?1 ORDER BY created_utc_ms DESC, id")?;
        let rows = st.query_map(params![session_id], |r| r.get(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
}
