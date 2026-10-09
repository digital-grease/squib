//! Day plans (agenda, session plan, stage notes), checklists, and coach rotation.

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::{Repository, Result, StorageError};

pub const SETTING_SQUAD: &str = "coach_squad";
pub const SETTING_COACH_MODE: &str = "coach_mode";
pub const MAX_TEXT: usize = 200;
pub const MAX_NOTES: usize = 4000;
pub const MAX_ITEMS: usize = 100;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DayPlan {
    pub id: String,
    pub title: String,
    /// Local calendar date `YYYY-MM-DD` (display only; never used for timing).
    pub date_local: String,
    /// `practice` or `match`.
    pub kind: String,
    pub notes: String,
    pub created_utc_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanItem {
    pub id: String,
    pub plan_id: String,
    pub ordinal: i64,
    /// `drill`, `stage`, or `event`.
    pub kind: String,
    pub title: String,
    /// Local wall-clock `HH:MM` for the schedule, if any.
    pub time_local: Option<String>,
    pub drill_id: Option<String>,
    pub drill_version: Option<u32>,
    pub target_strings: Option<u32>,
    pub notes: String,
    pub skipped: bool,
}

/// Progress of an agenda item, derived from linked runs' outcomes.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ItemProgress {
    pub completed: u32,
    /// Cancelled, interrupted, or failed attempts: shown, never counted as done.
    pub aborted: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChecklistItem {
    pub id: String,
    pub plan_id: Option<String>,
    pub ordinal: i64,
    pub text: String,
    pub checked: bool,
}

fn valid_date(d: &str) -> bool {
    let b = d.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && d[..4].parse::<u32>().is_ok()
        && d[5..7].parse::<u32>().is_ok_and(|m| (1..=12).contains(&m))
        && d[8..].parse::<u32>().is_ok_and(|x| (1..=31).contains(&x))
}

fn valid_time(t: &str) -> bool {
    let b = t.as_bytes();
    b.len() == 5 && b[2] == b':' && t[..2].parse::<u32>().is_ok_and(|h| h < 24) && t[3..].parse::<u32>().is_ok_and(|m| m < 60)
}

fn text(s: &str, max: usize, what: &str) -> Result<String> {
    let t = s.trim();
    if t.is_empty() || t.chars().count() > max {
        return Err(StorageError::Conflict(format!("{what} must be 1-{max} characters")));
    }
    Ok(t.to_string())
}

fn notes(s: &str) -> Result<String> {
    if s.chars().count() > MAX_NOTES {
        return Err(StorageError::Conflict(format!("notes must be at most {MAX_NOTES} characters")));
    }
    Ok(s.to_string())
}

/// Neutral range-day checklist used when no template exists yet.
pub fn default_checklist() -> Vec<&'static str> {
    vec![
        "Eye protection",
        "Hearing protection",
        "Targets, pasters, stapler",
        "Range rules and check-in details",
        "Water and sun protection",
        "First aid kit",
        "Phone charged; Squib conditions updated",
    ]
}

impl Repository {
    // ---- Plans ----------------------------------------------------------------------------

    pub fn create_plan(&mut self, p: &DayPlan, copy_template: bool) -> Result<()> {
        let title = text(&p.title, MAX_TEXT, "title")?;
        if !valid_date(&p.date_local) {
            return Err(StorageError::Conflict("date must be YYYY-MM-DD".into()));
        }
        if !matches!(p.kind.as_str(), "practice" | "match") {
            return Err(StorageError::Conflict("kind must be practice or match".into()));
        }
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO day_plan(id, title, date_local, kind, notes, created_utc_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![p.id, title, p.date_local, p.kind, notes(&p.notes)?, p.created_utc_ms],
        )?;
        if copy_template {
            let template: Vec<String> = {
                let mut st = tx.prepare("SELECT text FROM checklist_item WHERE plan_id IS NULL ORDER BY ordinal")?;
                st.query_map([], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?
            };
            let items: Vec<String> =
                if template.is_empty() { default_checklist().into_iter().map(String::from).collect() } else { template };
            for (i, t) in items.iter().enumerate() {
                tx.execute(
                    "INSERT INTO checklist_item(id, plan_id, ordinal, text) VALUES (?1, ?2, ?3, ?4)",
                    params![format!("{}-c{i}", p.id), p.id, i as i64, t],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn list_plans(&self) -> Result<Vec<DayPlan>> {
        let mut st = self.conn.prepare(
            "SELECT id, title, date_local, kind, notes, created_utc_ms FROM day_plan WHERE archived = 0
             ORDER BY date_local DESC, created_utc_ms DESC",
        )?;
        let rows = st.query_map([], |r| {
            Ok(DayPlan {
                id: r.get(0)?,
                title: r.get(1)?,
                date_local: r.get(2)?,
                kind: r.get(3)?,
                notes: r.get(4)?,
                created_utc_ms: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn update_plan_notes(&self, plan_id: &str, n: &str) -> Result<()> {
        self.conn.execute("UPDATE day_plan SET notes = ?2 WHERE id = ?1", params![plan_id, notes(n)?])?;
        Ok(())
    }

    /// Archive hides a plan; its runs and their links stay in history.
    pub fn archive_plan(&self, plan_id: &str) -> Result<()> {
        self.conn.execute("UPDATE day_plan SET archived = 1 WHERE id = ?1", params![plan_id])?;
        Ok(())
    }

    // ---- Agenda items -----------------------------------------------------------------------

    pub fn add_plan_item(&mut self, it: &PlanItem) -> Result<()> {
        let title = text(&it.title, MAX_TEXT, "title")?;
        if !matches!(it.kind.as_str(), "drill" | "stage" | "event") {
            return Err(StorageError::Conflict("kind must be drill, stage, or event".into()));
        }
        if it.time_local.as_deref().is_some_and(|t| !valid_time(t)) {
            return Err(StorageError::Conflict("time must be HH:MM".into()));
        }
        if it.kind == "drill" {
            let (Some(d), Some(v)) = (&it.drill_id, it.drill_version) else {
                return Err(StorageError::Conflict("a drill item needs a drill version".into()));
            };
            if self.load_drill_version(d, v)?.is_none() {
                return Err(StorageError::Conflict(format!("drill {d} version {v} does not exist")));
            }
        }
        let tx = self.conn.transaction()?;
        let n: i64 = tx.query_row("SELECT COUNT(*) FROM plan_item WHERE plan_id = ?1", params![it.plan_id], |r| r.get(0))?;
        if n as usize >= MAX_ITEMS {
            return Err(StorageError::Conflict(format!("at most {MAX_ITEMS} agenda items")));
        }
        tx.execute(
            "INSERT INTO plan_item(id, plan_id, ordinal, kind, title, time_local, drill_id, drill_version, target_strings, notes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                it.id,
                it.plan_id,
                n,
                it.kind,
                title,
                it.time_local,
                it.drill_id,
                it.drill_version,
                it.target_strings,
                notes(&it.notes)?
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn plan_items(&self, plan_id: &str) -> Result<Vec<PlanItem>> {
        let mut st = self.conn.prepare(
            "SELECT id, plan_id, ordinal, kind, title, time_local, drill_id, drill_version, target_strings, notes, skipped
             FROM plan_item WHERE plan_id = ?1 ORDER BY ordinal",
        )?;
        let rows = st.query_map(params![plan_id], |r| {
            Ok(PlanItem {
                id: r.get(0)?,
                plan_id: r.get(1)?,
                ordinal: r.get(2)?,
                kind: r.get(3)?,
                title: r.get(4)?,
                time_local: r.get(5)?,
                drill_id: r.get(6)?,
                drill_version: r.get(7)?,
                target_strings: r.get(8)?,
                notes: r.get(9)?,
                skipped: r.get(10)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn plan_item_exists(&self, item_id: &str) -> Result<bool> {
        Ok(self.conn.query_row("SELECT 1 FROM plan_item WHERE id = ?1", params![item_id], |_| Ok(())).optional()?.is_some())
    }

    pub fn update_item_notes(&self, item_id: &str, n: &str) -> Result<()> {
        self.conn.execute("UPDATE plan_item SET notes = ?2 WHERE id = ?1", params![item_id, notes(n)?])?;
        Ok(())
    }

    pub fn set_item_skipped(&self, item_id: &str, skipped: bool) -> Result<()> {
        self.conn.execute("UPDATE plan_item SET skipped = ?2 WHERE id = ?1", params![item_id, skipped])?;
        Ok(())
    }

    /// Swap an item with its neighbour (`up` toward the start).
    pub fn move_item(&mut self, item_id: &str, up: bool) -> Result<()> {
        let tx = self.conn.transaction()?;
        let (plan, ord): (String, i64) =
            tx.query_row("SELECT plan_id, ordinal FROM plan_item WHERE id = ?1", params![item_id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
        let q = if up {
            "SELECT id, ordinal FROM plan_item WHERE plan_id = ?1 AND ordinal < ?2 ORDER BY ordinal DESC LIMIT 1"
        } else {
            "SELECT id, ordinal FROM plan_item WHERE plan_id = ?1 AND ordinal > ?2 ORDER BY ordinal ASC LIMIT 1"
        };
        let other: Option<(String, i64)> = tx.query_row(q, params![plan, ord], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        if let Some((oid, oord)) = other {
            tx.execute("UPDATE plan_item SET ordinal = ?2 WHERE id = ?1", params![item_id, oord])?;
            tx.execute("UPDATE plan_item SET ordinal = ?2 WHERE id = ?1", params![oid, ord])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Remove an agenda item. Its runs stay in history; only the link is removed.
    pub fn delete_item(&mut self, item_id: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM plan_run WHERE item_id = ?1", params![item_id])?;
        tx.execute("DELETE FROM plan_item WHERE id = ?1", params![item_id])?;
        tx.commit()?;
        Ok(())
    }

    /// Record that a run was made from an agenda item (idempotent).
    pub fn link_run(&self, item_id: &str, run_id: &str) -> Result<()> {
        self.conn.execute("INSERT OR IGNORE INTO plan_run(item_id, run_id) VALUES (?1, ?2)", params![item_id, run_id])?;
        Ok(())
    }

    pub fn item_progress(&self, item_id: &str) -> Result<ItemProgress> {
        let mut st = self.conn.prepare("SELECT r.outcome FROM plan_run p JOIN run r ON r.id = p.run_id WHERE p.item_id = ?1")?;
        let mut prog = ItemProgress::default();
        for o in st.query_map(params![item_id], |r| r.get::<_, Option<String>>(0))? {
            match o?.as_deref() {
                Some("complete") => prog.completed += 1,
                Some(_) => prog.aborted += 1,
                None => {} // still in progress: neither done nor aborted
            }
        }
        Ok(prog)
    }

    // ---- Checklists ---------------------------------------------------------------------------

    /// `plan_id` `None` edits the template used for new plans.
    pub fn checklist(&self, plan_id: Option<&str>) -> Result<Vec<ChecklistItem>> {
        let mut st = self
            .conn
            .prepare("SELECT id, plan_id, ordinal, text, checked FROM checklist_item WHERE plan_id IS ?1 ORDER BY ordinal")?;
        let rows = st.query_map(params![plan_id], |r| {
            Ok(ChecklistItem { id: r.get(0)?, plan_id: r.get(1)?, ordinal: r.get(2)?, text: r.get(3)?, checked: r.get(4)? })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn add_checklist_item(&self, id: &str, plan_id: Option<&str>, t: &str) -> Result<()> {
        let t = text(t, MAX_TEXT, "item")?;
        let n: i64 =
            self.conn.query_row("SELECT COUNT(*) FROM checklist_item WHERE plan_id IS ?1", params![plan_id], |r| r.get(0))?;
        if n as usize >= MAX_ITEMS {
            return Err(StorageError::Conflict(format!("at most {MAX_ITEMS} checklist items")));
        }
        self.conn.execute(
            "INSERT INTO checklist_item(id, plan_id, ordinal, text) VALUES (?1, ?2, ?3, ?4)",
            params![id, plan_id, n, t],
        )?;
        Ok(())
    }

    pub fn set_checked(&self, id: &str, checked: bool) -> Result<()> {
        self.conn.execute("UPDATE checklist_item SET checked = ?2 WHERE id = ?1", params![id, checked])?;
        Ok(())
    }

    pub fn delete_checklist_item(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM checklist_item WHERE id = ?1", params![id])?;
        Ok(())
    }

    // ---- Coach rotation ---------------------------------------------------------------------------

    /// Ordered shooter ids for coach rotation; unknown or archived ids are dropped.
    pub fn squad(&self) -> Result<Vec<String>> {
        let ids: Vec<String> = self.get_setting(SETTING_SQUAD)?.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default();
        let known: Vec<String> = self.list_shooters()?.into_iter().filter(|s| !s.archived).map(|s| s.id).collect();
        Ok(ids.into_iter().filter(|i| known.contains(i)).collect())
    }

    pub fn set_squad(&self, ids: &[String]) -> Result<()> {
        if ids.len() > 50 {
            return Err(StorageError::Conflict("at most 50 shooters in a rotation".into()));
        }
        let known: Vec<String> = self.list_shooters()?.into_iter().map(|s| s.id).collect();
        if let Some(bad) = ids.iter().find(|i| !known.contains(i)) {
            return Err(StorageError::NotFound(format!("shooter {bad}")));
        }
        self.set_setting(SETTING_SQUAD, &serde_json::to_string(ids)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_and_time_validation() {
        assert!(valid_date("2026-10-08"));
        assert!(!valid_date("2026-13-08") && !valid_date("2026-10-8") && !valid_date("20261008xx"));
        assert!(valid_time("09:05") && valid_time("23:59"));
        assert!(!valid_time("24:00") && !valid_time("9:05") && !valid_time("09-05"));
    }
}
