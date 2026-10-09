//! Repository operations for schema 3: drills, scores, manual strings, round counts,
//! attachments, shooter profiles, deliberate deletion, and analytics projection.

use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use squib_domain::{EventOrigin, EventRef, Outcome, RevisionContent, RunConfig, RunRevision, SourceMode, compute_results};
use squib_training::analytics::RunRecord;
use squib_training::drill::DrillVersion;
use squib_training::manual::ManualString;
use squib_training::rounds::RoundCount;
use squib_training::scoring::{ScoreRevision, ScoreStatus, builtin, compute};

use crate::{Repository, Result, RunIntent, StorageError};

/// `RunConfig::drill_version_id` encoding: `{drill_id}#{version}`.
pub fn drill_ref(drill_id: &str, version: u32) -> String {
    format!("{drill_id}#{version}")
}

pub fn parse_drill_ref(s: &str) -> Option<(String, u32)> {
    let (id, v) = s.rsplit_once('#')?;
    Some((id.to_string(), v.parse().ok()?))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShooterProfile {
    pub id: String,
    pub name: String,
    pub archived: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttachmentRecord {
    pub id: String,
    pub run_id: Option<String>,
    pub kind: String,
    pub relative_path: String,
    pub sha256: String,
    pub bytes: i64,
    pub mime: String,
    pub metadata_stripped: bool,
    pub created_utc_ms: i64,
}

/// What a deletion removed, so the app can remove app-owned files afterwards.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DeletionReport {
    pub runs: u32,
    pub attachment_paths: Vec<String>,
}

pub const MANUAL_EDITOR: &str = "manual-entry";

/// Attachment paths are relative to the app's attachment directory: no absolute
/// paths, drive or URL prefixes, backslashes, NUL, or empty/`.`/`..` components.
/// Enforced on every insert, on import, and before any file access.
pub fn valid_attachment_path(p: &str) -> bool {
    !p.is_empty()
        && p.len() <= 200
        && !p.starts_with('/')
        && !p.contains('\\')
        && !p.contains('\0')
        && !p.contains(':')
        && p.split('/').all(|c| !c.is_empty() && c != "." && c != "..")
}

/// (id, created, shooter, mode, config_json, outcome, review_state, start_method)
type RunRow8 = (String, i64, String, String, String, Option<String>, Option<String>, Option<String>);

impl Repository {
    // ---- Drills ---------------------------------------------------------------------

    pub fn save_drill_version(&mut self, v: &DrillVersion) -> Result<()> {
        v.recipe.validate().map_err(|e| StorageError::Conflict(e.to_string()))?;
        let tx = self.conn.transaction()?;
        if v.version == 1 {
            tx.execute("INSERT INTO drill(id, created_utc_ms) VALUES (?1, ?2)", params![v.drill_id, v.created_utc_ms])?;
        } else {
            let latest: Option<u32> =
                tx.query_row("SELECT MAX(version) FROM drill_version WHERE drill_id = ?1", params![v.drill_id], |r| r.get(0))?;
            if latest != Some(v.version - 1) {
                return Err(StorageError::Conflict("drill version must follow the latest version".into()));
            }
        }
        tx.execute(
            "INSERT INTO drill_version(drill_id, version, recipe_json, content_hash, created_utc_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![v.drill_id, v.version, serde_json::to_string(&v.recipe)?, v.content_hash, v.created_utc_ms],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn load_drill_version(&self, drill_id: &str, version: u32) -> Result<Option<DrillVersion>> {
        self.conn
            .query_row(
                "SELECT recipe_json, content_hash, created_utc_ms FROM drill_version WHERE drill_id = ?1 AND version = ?2",
                params![drill_id, version],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)),
            )
            .optional()?
            .map(|(j, h, t)| {
                Ok(DrillVersion {
                    drill_id: drill_id.into(),
                    version,
                    recipe: serde_json::from_str(&j)?,
                    content_hash: h,
                    created_utc_ms: t,
                })
            })
            .transpose()
    }

    /// Latest version of each non-archived drill.
    pub fn list_drills(&self) -> Result<Vec<DrillVersion>> {
        let ids: Vec<(String, u32)> = {
            let mut st = self.conn.prepare(
                "SELECT d.id, MAX(v.version) FROM drill d JOIN drill_version v ON v.drill_id = d.id
                 WHERE d.archived = 0 GROUP BY d.id ORDER BY d.created_utc_ms",
            )?;
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<std::result::Result<_, _>>()?
        };
        let mut out = Vec::new();
        for (id, v) in ids {
            if let Some(d) = self.load_drill_version(&id, v)? {
                out.push(d);
            }
        }
        Ok(out)
    }

    pub fn archive_drill(&self, drill_id: &str) -> Result<()> {
        self.conn.execute("UPDATE drill SET archived = 1 WHERE id = ?1", params![drill_id])?;
        Ok(())
    }

    // ---- Scores ---------------------------------------------------------------------

    pub fn append_score_revision(&mut self, r: &ScoreRevision) -> Result<()> {
        let tx = self.conn.transaction()?;
        let latest: Option<(u32, String)> = tx
            .query_row(
                "SELECT number, content_hash FROM score_revision WHERE run_id = ?1 ORDER BY number DESC LIMIT 1",
                params![r.run_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let ok = match (&latest, &r.parent_hash) {
            (None, None) => r.number == 1,
            (Some((n, h)), Some(ph)) => r.number == n + 1 && h == ph,
            _ => false,
        };
        if !ok {
            return Err(StorageError::Conflict("score revision parent is not the latest".into()));
        }
        tx.execute(
            "INSERT INTO score_revision(run_id, number, parent_hash, content_hash, entry_json, editor, created_utc_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                r.run_id,
                r.number,
                r.parent_hash,
                r.content_hash,
                serde_json::to_string(&r.entry)?,
                r.editor,
                r.created_utc_ms
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn latest_score(&self, run_id: &str) -> Result<Option<ScoreRevision>> {
        self.conn
            .query_row(
                "SELECT number, parent_hash, content_hash, entry_json, editor, created_utc_ms FROM score_revision
                 WHERE run_id = ?1 ORDER BY number DESC LIMIT 1",
                params![run_id],
                |r| {
                    Ok((
                        r.get::<_, u32>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, i64>(5)?,
                    ))
                },
            )
            .optional()?
            .map(|(n, ph, h, j, e, t)| {
                Ok(ScoreRevision {
                    run_id: run_id.into(),
                    number: n,
                    parent_hash: ph,
                    entry: serde_json::from_str(&j)?,
                    editor: e,
                    created_utc_ms: t,
                    content_hash: h,
                })
            })
            .transpose()
    }

    // ---- Manual runs (A20) ------------------------------------------------------------

    /// Record a string from another timer as a complete, saved run whose events are
    /// manual (no sample origin, no detector score) and whose precision is kept.
    pub fn insert_manual_run(&mut self, intent: &RunIntent, m: &ManualString) -> Result<()> {
        if intent.config.source_mode != SourceMode::ManualEntry {
            return Err(StorageError::Conflict("manual runs use manual_entry mode".into()));
        }
        m.validate().map_err(|e| StorageError::Conflict(e.to_string()))?;
        self.insert_run_intent(intent)?;
        let accepted = m
            .times_ns
            .iter()
            .enumerate()
            .map(|(i, t)| squib_domain::AcceptedEvent {
                event: EventRef::Manual { id: format!("m{i:04}") },
                timeline_ns: *t,
                origin: EventOrigin::Manual,
            })
            .collect();
        let content = RevisionContent {
            run_id: intent.run_id.clone(),
            number: 1,
            parent_number: None,
            parent_hash: None,
            // The source timer's own start is the reference for its times.
            origin_resolved: true,
            actions: vec![],
            accepted,
            unresolved: vec![],
            rejected: vec![],
            notes: vec![format!("Entered from {} at {} ms precision", m.source_label, m.precision_ns / 1_000_000)],
            editor: MANUAL_EDITOR.into(),
            created_utc_ms: intent.created_utc_ms,
            reason: None,
        };
        let rev = RunRevision { content_hash: squib_domain::hash::content_hash(&content), content };
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO manual_string(run_id, source_label, precision_ns, times_json) VALUES (?1, ?2, ?3, ?4)",
            params![intent.run_id, m.source_label, m.precision_ns, serde_json::to_string(&m.times_ns)?],
        )?;
        tx.execute(
            "INSERT INTO run_revision(run_id, number, parent_number, parent_hash, content_hash, editor, created_utc_ms, content_json)
             VALUES (?1, 1, NULL, NULL, ?2, ?3, ?4, ?5)",
            params![intent.run_id, rev.content_hash, MANUAL_EDITOR, intent.created_utc_ms, serde_json::to_string(&rev.content)?],
        )?;
        tx.execute(
            "UPDATE run SET outcome = 'complete', persist_state = 'saved', review_state = 'reviewed', finalized_utc_ms = ?2
             WHERE id = ?1",
            params![intent.run_id, intent.created_utc_ms],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn load_manual_string(&self, run_id: &str) -> Result<Option<ManualString>> {
        self.conn
            .query_row(
                "SELECT source_label, precision_ns, times_json FROM manual_string WHERE run_id = ?1",
                params![run_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?)),
            )
            .optional()?
            .map(|(l, p, j)| Ok(ManualString { source_label: l, precision_ns: p, times_ns: serde_json::from_str(&j)? }))
            .transpose()
    }

    // ---- Rounds (A24) -----------------------------------------------------------------

    pub fn set_round_count(&self, rc: &RoundCount) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO round_count(run_id, proposed, confirmed, confirmed_utc_ms) VALUES (?1, ?2, ?3, ?4)",
            params![rc.run_id, rc.proposed, rc.confirmed, rc.confirmed_utc_ms],
        )?;
        Ok(())
    }

    pub fn round_counts(&self) -> Result<Vec<RoundCount>> {
        let mut st = self.conn.prepare("SELECT run_id, proposed, confirmed, confirmed_utc_ms FROM round_count")?;
        let rows = st.query_map([], |r| {
            Ok(RoundCount { run_id: r.get(0)?, proposed: r.get(1)?, confirmed: r.get(2)?, confirmed_utc_ms: r.get(3)? })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    // ---- Attachments ------------------------------------------------------------------

    pub fn insert_attachment(&self, a: &AttachmentRecord) -> Result<()> {
        if !valid_attachment_path(&a.relative_path) {
            return Err(StorageError::Conflict("attachment path must be app-relative".into()));
        }
        self.conn.execute(
            "INSERT INTO attachment(id, run_id, kind, relative_path, sha256, bytes, mime, metadata_stripped, created_utc_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![a.id, a.run_id, a.kind, a.relative_path, a.sha256, a.bytes, a.mime, a.metadata_stripped, a.created_utc_ms],
        )?;
        Ok(())
    }

    pub fn list_attachments(&self, run_id: Option<&str>) -> Result<Vec<AttachmentRecord>> {
        let mut st = self.conn.prepare(
            "SELECT id, run_id, kind, relative_path, sha256, bytes, mime, metadata_stripped, created_utc_ms FROM attachment
             WHERE (?1 IS NULL OR run_id = ?1) ORDER BY created_utc_ms",
        )?;
        let rows = st.query_map(params![run_id], |r| {
            Ok(AttachmentRecord {
                id: r.get(0)?,
                run_id: r.get(1)?,
                kind: r.get(2)?,
                relative_path: r.get(3)?,
                sha256: r.get(4)?,
                bytes: r.get(5)?,
                mime: r.get(6)?,
                metadata_stripped: r.get(7)?,
                created_utc_ms: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    // ---- Shooter profiles ---------------------------------------------------------------

    pub fn add_shooter(&self, id: &str, name: &str, now: i64) -> Result<()> {
        let n = name.trim();
        if n.is_empty() || n.chars().count() > 40 {
            return Err(StorageError::Conflict("name must be 1-40 characters".into()));
        }
        self.conn.execute("INSERT INTO shooter_profile(id, name, created_utc_ms) VALUES (?1, ?2, ?3)", params![id, n, now])?;
        Ok(())
    }

    pub fn rename_shooter(&self, id: &str, name: &str) -> Result<()> {
        let n = name.trim();
        if n.is_empty() || n.chars().count() > 40 {
            return Err(StorageError::Conflict("name must be 1-40 characters".into()));
        }
        self.conn.execute("UPDATE shooter_profile SET name = ?2 WHERE id = ?1", params![id, n])?;
        Ok(())
    }

    pub fn list_shooters(&self) -> Result<Vec<ShooterProfile>> {
        let mut st = self.conn.prepare("SELECT id, name, archived FROM shooter_profile ORDER BY created_utc_ms")?;
        let rows = st.query_map([], |r| Ok(ShooterProfile { id: r.get(0)?, name: r.get(1)?, archived: r.get(2)? }))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    // ---- Deliberate deletion ------------------------------------------------------------

    fn delete_runs_tx(tx: &Transaction<'_>, run_ids: &[String], report: &mut DeletionReport) -> Result<()> {
        for id in run_ids {
            let mut st = tx.prepare("SELECT relative_path FROM attachment WHERE run_id = ?1")?;
            let paths: Vec<String> = st.query_map(params![id], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
            report.attachment_paths.extend(paths);
            let snapshot: Option<String> = tx
                .query_row("SELECT environment_snapshot_id FROM run WHERE id = ?1", params![id], |r| r.get(0))
                .optional()?
                .flatten();
            for t in [
                "plan_run",
                "attachment",
                "round_count",
                "manual_string",
                "score_revision",
                "energy_envelope",
                "quality_event",
                "detected_candidate",
                "cue_observation",
            ] {
                tx.execute(&format!("DELETE FROM {t} WHERE run_id = ?1"), params![id])?;
            }
            // Revisions reference their parents: delete newest first.
            tx.execute("DELETE FROM run_revision WHERE run_id = ?1 AND parent_number IS NOT NULL", params![id])?;
            tx.execute("DELETE FROM run_revision WHERE run_id = ?1", params![id])?;
            tx.execute("DELETE FROM capture_epoch WHERE run_id = ?1", params![id])?;
            report.runs += tx.execute("DELETE FROM run WHERE id = ?1", params![id])? as u32;
            if let Some(s) = snapshot {
                tx.execute(
                    "DELETE FROM environment_snapshot WHERE id = ?1 AND NOT EXISTS (SELECT 1 FROM run WHERE environment_snapshot_id = ?1)",
                    params![s],
                )?;
            }
        }
        Ok(())
    }

    fn with_guard(&mut self, f: impl FnOnce(&Transaction<'_>, &mut DeletionReport) -> Result<()>) -> Result<DeletionReport> {
        let tx = self.conn.transaction()?;
        tx.execute("INSERT INTO deletion_guard(token) VALUES ('active')", [])?;
        let mut report = DeletionReport::default();
        f(&tx, &mut report)?;
        tx.execute("DELETE FROM deletion_guard", [])?;
        tx.commit()?;
        Ok(report)
    }

    /// Delete one run and everything that belongs to it. Exported copies and OS
    /// backups are outside this action.
    pub fn delete_run(&mut self, run_id: &str) -> Result<DeletionReport> {
        let ids = vec![run_id.to_string()];
        self.with_guard(|tx, rep| Self::delete_runs_tx(tx, &ids, rep))
    }

    pub fn delete_session(&mut self, session_id: &str) -> Result<DeletionReport> {
        let ids: Vec<String> = {
            let mut st = self.conn.prepare("SELECT id FROM run WHERE session_id = ?1")?;
            st.query_map(params![session_id], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?
        };
        let sid = session_id.to_string();
        self.with_guard(move |tx, rep| {
            Self::delete_runs_tx(tx, &ids, rep)?;
            tx.execute("DELETE FROM session WHERE id = ?1", params![sid])?;
            Ok(())
        })
    }

    /// Delete every run, session, day plan, snapshot, drill, place, override, and cached document.
    pub fn delete_all_history(&mut self) -> Result<DeletionReport> {
        let ids: Vec<String> = {
            let mut st = self.conn.prepare("SELECT id FROM run")?;
            st.query_map([], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?
        };
        self.with_guard(move |tx, rep| {
            Self::delete_runs_tx(tx, &ids, rep)?;
            // Day plans reference drills; the checklist template (plan_id NULL) is kept.
            tx.execute("DELETE FROM checklist_item WHERE plan_id IS NOT NULL", [])?;
            tx.execute("DELETE FROM plan_item", [])?;
            tx.execute("DELETE FROM day_plan", [])?;
            for t in [
                "session",
                "environment_snapshot",
                "drill_version",
                "drill",
                "saved_place",
                "env_override",
                "provider_cache",
                "calibration_profile_version",
            ] {
                tx.execute(&format!("DELETE FROM {t}"), [])?;
            }
            // Starter drills are seeded again on next launch.
            tx.execute("DELETE FROM app_setting WHERE key IN ('last_place', 'starter_drills_seeded')", [])?;
            Ok(())
        })
    }

    // ---- Analytics projection ------------------------------------------------------------

    /// Project runs into analytics records from their latest review and score revisions.
    pub fn analytics_records(&self) -> Result<Vec<RunRecord>> {
        let rows: Vec<RunRow8> = {
            let mut st = self.conn.prepare(
                "SELECT id, created_utc_ms, shooter_id, source_mode, config_json, outcome, review_state, start_method FROM run
                 ORDER BY created_utc_ms",
            )?;
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)))?
                .collect::<std::result::Result<_, _>>()?
        };
        let mut out = Vec::new();
        for (id, created, shooter, mode, cfg, outcome, review, start) in rows {
            let config: RunConfig = serde_json::from_str(&cfg)?;
            let drill = config.drill_version_id.as_deref().and_then(parse_drill_ref);
            let recipe = match &drill {
                Some((d, v)) => self.load_drill_version(d, *v)?.map(|dv| dv.recipe),
                None => None,
            };
            let latest = self.latest_revision(&id)?;
            let res = latest.as_ref().map(|r| compute_results(r, config.expected_count));
            let score = self.latest_score(&id)?;
            let profile = recipe
                .as_ref()
                .and_then(|r| builtin(&r.scoring_profile_id, r.scoring_profile_version))
                .or_else(|| score.as_ref().and_then(|s| builtin(&s.entry.profile_id, s.entry.profile_version)));
            let manual = mode == SourceMode::ManualEntry.as_str();
            let elapsed = res.as_ref().and_then(|r| r.last_ns);
            let sr = profile.as_ref().map(|p| compute(score.as_ref().map(|s| &s.entry), p, elapsed));
            let score_required =
                profile.as_ref().is_some_and(|p| !matches!(p.kind, squib_training::scoring::ProfileKind::TimeOnly));
            out.push(RunRecord {
                run_id: id,
                created_utc_ms: created,
                shooter_id: shooter,
                drill_id: drill.as_ref().map(|d| d.0.clone()),
                drill_version: drill.as_ref().map(|d| d.1),
                mode,
                scoring_profile: profile.as_ref().map(|p| (p.id.clone(), p.version)),
                equipment_tags: recipe.as_ref().map(|r| r.equipment_tags.clone()).unwrap_or_default(),
                timing_method: if manual { "manual".into() } else { start.unwrap_or_else(|| "unresolved".into()) },
                outcome: outcome.as_deref().and_then(Outcome::parse),
                review_state: review.as_deref().and_then(squib_domain::ReviewState::parse),
                edited: latest.as_ref().is_some_and(|r| r.is_edited()),
                manual,
                origin_resolved: latest.as_ref().is_some_and(|r| r.content.origin_resolved),
                revision_number: latest.as_ref().map(|r| r.content.number),
                score_revision: score.as_ref().map(|s| s.number),
                first_ns: res.as_ref().and_then(|r| r.first_ns),
                final_ns: sr.as_ref().and_then(|s| s.final_time_ns).or(elapsed),
                splits_ns: res.map(|r| r.splits_ns).unwrap_or_default(),
                hit_factor: sr.as_ref().and_then(|s| s.hit_factor),
                score_complete: sr.as_ref().is_some_and(|s| s.status == ScoreStatus::Complete),
                score_required,
            });
        }
        Ok(out)
    }

    /// Validate that a drill version referenced by a run exists (called on intent).
    pub(crate) fn check_drill_ref(tx: &Transaction<'_>, cfg: &RunConfig) -> Result<Option<(String, u32)>> {
        let Some(r) = cfg.drill_version_id.as_deref() else { return Ok(None) };
        let (id, v) = parse_drill_ref(r).ok_or_else(|| StorageError::Conflict(format!("bad drill reference {r}")))?;
        let exists: bool = tx
            .query_row("SELECT 1 FROM drill_version WHERE drill_id = ?1 AND version = ?2", params![id, v], |_| Ok(true))
            .optional()?
            .unwrap_or(false);
        if !exists {
            return Err(StorageError::Conflict(format!("drill {id} version {v} does not exist")));
        }
        Ok(Some((id, v)))
    }
}
