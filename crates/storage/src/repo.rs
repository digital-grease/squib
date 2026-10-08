use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, params};
use squib_domain::{
    Candidate, ClassifyContext, CueKind, CueSpan, DOMAIN_SCHEMA_VERSION, Outcome, PersistState, QualityEvent, QualityKind,
    ReviewState, RunConfig, RunRevision, Severity, StartReferenceMethod, classify, compute_results, initial_revision,
};
use squib_timing::envelope::{ENVELOPE_VERSION, EnvelopeChunk};

use crate::records::*;
use crate::{MIGRATIONS, Result, SCHEMA_VERSION, StorageError};

pub const DEFAULT_SHOOTER_ID: &str = "default";

pub struct Repository {
    pub(crate) conn: Connection,
    path: Option<PathBuf>,
}

fn configure(conn: &Connection, read_only: bool) -> Result<()> {
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    if !read_only {
        let mode: String = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
        if !mode.eq_ignore_ascii_case("wal") && !mode.eq_ignore_ascii_case("memory") {
            return Err(StorageError::Write(format!("journal mode {mode} not supported")));
        }
        conn.pragma_update(None, "synchronous", "FULL")?;
    }
    Ok(())
}

fn user_version(conn: &Connection) -> Result<u32> {
    Ok(conn.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))?)
}

fn opt_enum<T>(s: Option<String>, parse: impl Fn(&str) -> Option<T>, what: &str) -> Result<Option<T>> {
    match s {
        None => Ok(None),
        Some(v) => parse(&v).map(Some).ok_or_else(|| StorageError::Corrupt(format!("bad {what}: {v}"))),
    }
}

impl Repository {
    /// Open (creating if needed) and migrate. A pre-migration backup is written next
    /// to an existing database before any schema change.
    pub fn open(path: &Path) -> Result<Self> {
        let existed = path.exists();
        let conn = Connection::open(path)?;
        configure(&conn, false)?;
        let mut repo = Self { conn, path: Some(path.to_path_buf()) };
        repo.migrate(existed)?;
        Ok(repo)
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        configure(&conn, false)?;
        let mut repo = Self { conn, path: None };
        repo.migrate(false)?;
        Ok(repo)
    }

    /// Separate read-only connection for projections (WAL snapshot reads).
    pub fn open_read_only(path: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        configure(&conn, true)?;
        let v = user_version(&conn)?;
        if v != SCHEMA_VERSION {
            return Err(StorageError::Migration { version: SCHEMA_VERSION, message: format!("read-only open found schema {v}") });
        }
        Ok(Self { conn, path: Some(path.to_path_buf()) })
    }

    /// Raw connection for the archive module (generic table export/import). Other
    /// code goes through typed methods.
    pub fn connection(&mut self) -> &mut Connection {
        &mut self.conn
    }

    /// Test hook: limit database growth to simulate a full disk.
    pub fn set_max_page_count(&self, pages: u32) -> Result<()> {
        self.conn.query_row(&format!("PRAGMA max_page_count = {pages}"), [], |_| Ok(()))?;
        Ok(())
    }

    pub fn schema_version(&self) -> Result<u32> {
        user_version(&self.conn)
    }

    fn migrate(&mut self, existed: bool) -> Result<()> {
        let current = user_version(&self.conn)?;
        if current > SCHEMA_VERSION {
            return Err(StorageError::NewerSchema { found: current, supported: SCHEMA_VERSION });
        }
        if current == SCHEMA_VERSION {
            return Ok(());
        }
        if existed
            && current > 0
            && let Some(p) = &self.path
        {
            let bak = p.with_extension(format!("pre-v{}.bak", current + 1));
            let mut dst = Connection::open(&bak)?;
            let backup = rusqlite::backup::Backup::new(&self.conn, &mut dst)?;
            backup.run_to_completion(256, std::time::Duration::from_millis(0), None)?;
        }
        for &(version, sql) in MIGRATIONS {
            if version <= current {
                continue;
            }
            let tx = self.conn.transaction()?;
            tx.execute_batch(sql).map_err(|e| StorageError::Migration { version, message: e.to_string() })?;
            tx.pragma_update(None, "user_version", version)
                .map_err(|e| StorageError::Migration { version, message: e.to_string() })?;
            tx.execute(
                "INSERT OR REPLACE INTO app_meta(key, value) VALUES ('schema_migrated_to', ?1)",
                params![version.to_string()],
            )
            .map_err(|e| StorageError::Migration { version, message: e.to_string() })?;
            tx.commit().map_err(|e| StorageError::Migration { version, message: e.to_string() })?;
        }
        let reached = user_version(&self.conn)?;
        if reached != SCHEMA_VERSION {
            return Err(StorageError::Migration {
                version: SCHEMA_VERSION,
                message: format!("database reached schema {reached}"),
            });
        }
        let fk: Vec<String> = {
            let mut st = self.conn.prepare("PRAGMA foreign_key_check")?;
            st.query_map([], |r| r.get::<_, String>(0))?.collect::<std::result::Result<_, _>>()?
        };
        if !fk.is_empty() {
            return Err(StorageError::Integrity(format!("foreign key violations in {fk:?}")));
        }
        Ok(())
    }

    /// Run `PRAGMA quick_check`.
    pub fn quick_check(&self) -> Result<()> {
        let r: String = self.conn.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
        if r == "ok" { Ok(()) } else { Err(StorageError::Integrity(r)) }
    }

    pub fn ensure_default_shooter(&self, now_utc_ms: i64) -> Result<String> {
        self.conn.execute(
            "INSERT OR IGNORE INTO shooter_profile(id, name, created_utc_ms) VALUES (?1, 'Me', ?2)",
            params![DEFAULT_SHOOTER_ID, now_utc_ms],
        )?;
        Ok(DEFAULT_SHOOTER_ID.to_string())
    }

    /// Return the active session for the shooter, creating one with `new_id` if none.
    pub fn active_session(&self, shooter_id: &str, new_id: &str, now_utc_ms: i64, tz_offset_min: i32) -> Result<String> {
        let existing: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM session WHERE shooter_id = ?1 AND active = 1 ORDER BY started_utc_ms DESC LIMIT 1",
                params![shooter_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            return Ok(id);
        }
        self.conn.execute(
            "INSERT INTO session(id, shooter_id, started_utc_ms, tz_offset_min, active) VALUES (?1, ?2, ?3, ?4, 1)",
            params![new_id, shooter_id, now_utc_ms, tz_offset_min],
        )?;
        Ok(new_id.to_string())
    }

    /// Durably record the run intent and immutable configuration before arming.
    pub fn insert_run_intent(&mut self, intent: &RunIntent) -> Result<()> {
        intent.config.validate().map_err(|e| StorageError::Conflict(format!("invalid configuration: {e}")))?;
        let tx = self.conn.transaction()?;
        let drill = Self::check_drill_ref(&tx, &intent.config)?;
        tx.execute(
            "INSERT INTO run(id, session_id, shooter_id, created_utc_ms, tz_offset_min, source_mode, config_json, config_hash,
                calibration_profile_id, app_build, detector_version, domain_schema_version, persist_state,
                environment_snapshot_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'pending', ?13)",
            params![
                intent.run_id,
                intent.session_id,
                intent.config.shooter_id,
                intent.created_utc_ms,
                intent.tz_offset_min,
                intent.config.source_mode.as_str(),
                serde_json::to_string(&intent.config)?,
                intent.config.config_hash(),
                intent.config.calibration_profile_id,
                intent.config.app_build,
                intent.config.detector.as_ref().map(|d| d.algorithm_version.clone()),
                DOMAIN_SCHEMA_VERSION,
                intent.config.environment_snapshot_id,
            ],
        )?;
        if let Some((id, v)) = drill {
            tx.execute("UPDATE run SET drill_id = ?2, drill_version = ?3 WHERE id = ?1", params![intent.run_id, id, v])?;
        }
        tx.commit()?;
        Ok(())
    }

    fn write_batch(tx: &Transaction<'_>, run_id: &str, b: &ObservationBatch) -> Result<()> {
        if let Some(e) = &b.epoch {
            tx.execute(
                "INSERT OR IGNORE INTO capture_epoch(id, run_id, sample_rate_hz, channels, sample_format, route_json,
                    route_signature, clock_domain, started_mono_ns) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    e.epoch_id,
                    run_id,
                    e.sample_rate_hz,
                    e.channels,
                    e.sample_format,
                    serde_json::to_string(&e.route)?,
                    e.route.signature(e.sample_rate_hz),
                    e.clock_domain,
                    e.started_mono_ns,
                ],
            )?;
        }
        if let Some(p) = &b.progress {
            tx.execute(
                "UPDATE run SET start_method = ?2, start_ref_frame = ?3, start_ref_mono_ns = ?4, armed_mono_ns = ?5 WHERE id = ?1",
                params![run_id, p.start_method.map(|m| m.as_str()), p.start_ref_frame, p.start_ref_mono_ns, p.armed_mono_ns],
            )?;
        }
        for c in &b.cues {
            tx.execute(
                "INSERT OR REPLACE INTO cue_observation(run_id, cue_id, kind, par_index, template_version, requested_mono_ns,
                    issued_mono_ns, render_mono_ns, epoch_id, acoustic_onset_frame, acoustic_onset_mono_ns, match_ncc,
                    span_start_frame, span_end_frame, span_exact, missed, heard)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
                params![
                    run_id,
                    c.cue_id,
                    c.kind.as_str(),
                    c.par_index,
                    c.template_version,
                    c.requested_mono_ns,
                    c.issued_mono_ns,
                    c.render_mono_ns,
                    c.epoch_id,
                    c.acoustic_onset_frame,
                    c.acoustic_onset_mono_ns,
                    c.match_ncc,
                    c.span_start_frame,
                    c.span_end_frame,
                    c.span_exact,
                    c.missed,
                    c.heard,
                ],
            )?;
        }
        for (epoch, c) in &b.candidates {
            // Idempotent ingest: identical retry is skipped; divergent content conflicts.
            let existing: Option<String> = tx
                .query_row(
                    "SELECT features_json FROM detected_candidate WHERE run_id = ?1 AND epoch_id = ?2 AND sequence = ?3",
                    params![run_id, epoch, c.sequence as i64],
                    |r| r.get(0),
                )
                .optional()?;
            let features = serde_json::to_string(&c.features)?;
            match existing {
                Some(f) if f == features => continue,
                Some(_) => return Err(StorageError::Conflict(format!("candidate {} differs from committed copy", c.sequence))),
                None => {}
            }
            tx.execute(
                "INSERT INTO detected_candidate(run_id, epoch_id, sequence, onset_frame, peak_frame, features_json, suggestion,
                    reasons_json, detector_score, algorithm_version, config_hash, batch_seq)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    run_id,
                    epoch,
                    c.sequence as i64,
                    c.onset_frame,
                    c.peak_frame,
                    features,
                    c.suggestion.as_str(),
                    serde_json::to_string(&c.reasons)?,
                    f64::from(c.detector_score),
                    c.algorithm_version,
                    c.config_hash,
                    b.batch_seq as i64,
                ],
            )?;
        }
        for (seq, epoch, q) in &b.quality {
            tx.execute(
                "INSERT OR IGNORE INTO quality_event(run_id, seq, epoch_id, kind, severity, start_frame, end_frame, at_mono_ns,
                    detail, batch_seq) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    run_id,
                    *seq as i64,
                    epoch,
                    q.kind.as_str(),
                    q.severity.as_str(),
                    q.start_frame,
                    q.end_frame,
                    q.at_ns,
                    q.detail,
                    b.batch_seq as i64,
                ],
            )?;
        }
        for (epoch, e) in &b.envelope {
            tx.execute(
                "INSERT OR IGNORE INTO energy_envelope(run_id, epoch_id, first_frame, hop_frames, version, data)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![run_id, epoch, e.first_frame, e.hop_frames, ENVELOPE_VERSION, e.data],
            )?;
        }
        if b.batch_seq > 0 {
            let n = tx.execute(
                "UPDATE run SET last_durable_seq = ?2 WHERE id = ?1 AND last_durable_seq < ?2",
                params![run_id, b.batch_seq as i64],
            )?;
            let _ = n;
        }
        Ok(())
    }

    fn require_unfinished(tx: &Transaction<'_>, run_id: &str) -> Result<()> {
        let outcome: Option<Option<String>> =
            tx.query_row("SELECT outcome FROM run WHERE id = ?1", params![run_id], |r| r.get(0)).optional()?;
        match outcome {
            None => Err(StorageError::NotFound(format!("run {run_id}"))),
            Some(Some(o)) => Err(StorageError::Conflict(format!("run {run_id} already finalized as {o}"))),
            Some(None) => Ok(()),
        }
    }

    /// Append one observation batch atomically. Returns the durable sequence.
    pub fn append_batch(&mut self, run_id: &str, batch: &ObservationBatch) -> Result<u64> {
        let tx = self.conn.transaction()?;
        Self::require_unfinished(&tx, run_id)?;
        Self::write_batch(&tx, run_id, batch)?;
        let seq: i64 = tx.query_row("SELECT last_durable_seq FROM run WHERE id = ?1", params![run_id], |r| r.get(0))?;
        tx.commit()?;
        Ok(seq as u64)
    }

    /// Commit remaining observations, cue records, revision 1, and the final state in
    /// one transaction. Only after this returns `Ok` may the UI show Saved.
    pub fn finalize_run(&mut self, f: &FinalRecord) -> Result<()> {
        let tx = self.conn.transaction()?;
        Self::require_unfinished(&tx, &f.run_id)?;
        Self::write_batch(&tx, &f.run_id, &f.remaining)?;
        if let Some((epoch, summary)) = &f.epoch_summary {
            tx.execute("UPDATE capture_epoch SET summary_json = ?2 WHERE id = ?1", params![epoch, summary])?;
        }
        let review = match &f.initial_revision {
            Some(r) => {
                Self::insert_revision(&tx, r)?;
                Some(r.review_state())
            }
            None => None,
        };
        tx.execute(
            "UPDATE run SET outcome = ?2, persist_state = 'saved', review_state = ?3, stop_mono_ns = ?4, stop_frame = ?5,
                interrupt_reason = ?6, finalized_utc_ms = ?7, error = ?8 WHERE id = ?1",
            params![
                f.run_id,
                f.outcome.as_str(),
                review.map(|r| r.as_str()),
                f.stop_mono_ns,
                f.stop_frame,
                f.interrupt_reason,
                f.finalized_utc_ms,
                f.error,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn insert_revision(tx: &Transaction<'_>, r: &RunRevision) -> Result<()> {
        if !r.verify_hash() {
            return Err(StorageError::Corrupt("revision content hash mismatch".into()));
        }
        tx.execute(
            "INSERT INTO run_revision(run_id, number, parent_number, parent_hash, content_hash, editor, created_utc_ms, content_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                r.content.run_id,
                r.content.number,
                r.content.parent_number,
                r.content.parent_hash,
                r.content_hash,
                r.content.editor,
                r.content.created_utc_ms,
                serde_json::to_string(&r.content)?,
            ],
        )?;
        Ok(())
    }

    /// Append a review revision. Fails with `Conflict` unless its parent is the latest
    /// revision (optimistic concurrency; no silent overwrite).
    pub fn append_revision(&mut self, r: &RunRevision) -> Result<ReviewState> {
        let tx = self.conn.transaction()?;
        let latest: Option<(u32, String)> = tx
            .query_row(
                "SELECT number, content_hash FROM run_revision WHERE run_id = ?1 ORDER BY number DESC LIMIT 1",
                params![r.content.run_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        match (&latest, r.content.parent_number, &r.content.parent_hash) {
            (Some((n, h)), Some(pn), Some(ph)) if *n == pn && h == ph => {}
            _ => return Err(StorageError::Conflict("revision parent is not the latest revision".into())),
        }
        Self::insert_revision(&tx, r)?;
        let state = r.review_state();
        tx.execute("UPDATE run SET review_state = ?2 WHERE id = ?1", params![r.content.run_id, state.as_str()])?;
        tx.commit()?;
        Ok(state)
    }

    /// Recover unfinished runs after process termination: mark interrupted, record a
    /// `process_terminated` quality event, flag the possible uncommitted tail, and build
    /// revision 1 from committed candidates only. Nothing is reconstructed.
    pub fn recover_unfinished(&mut self, now_utc_ms: i64) -> Result<Vec<RecoveredRun>> {
        let ids: Vec<(String, i64)> = {
            let mut st =
                self.conn.prepare("SELECT id, last_durable_seq FROM run WHERE outcome IS NULL ORDER BY created_utc_ms")?;
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<std::result::Result<_, _>>()?
        };
        let mut out = Vec::new();
        for (run_id, seq) in ids {
            let detail = self.load_run(&run_id)?;
            let resolved = detail.row.start_method == Some(StartReferenceMethod::AcousticCue);
            let rev =
                (!detail.candidates.is_empty()).then(|| initial_revision(&run_id, resolved, &detail.classified, now_utc_ms));
            let tx = self.conn.transaction()?;
            let next_q: i64 =
                tx.query_row("SELECT COALESCE(MAX(seq), -1) + 1 FROM quality_event WHERE run_id = ?1", params![run_id], |r| {
                    r.get(0)
                })?;
            let q = QualityEvent::new(
                QualityKind::ProcessTerminated,
                Severity::Integrity,
                format!("recovered after termination; observations after durable batch {seq} may be missing"),
            );
            tx.execute(
                "INSERT INTO quality_event(run_id, seq, kind, severity, detail, batch_seq) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![run_id, next_q, q.kind.as_str(), q.severity.as_str(), q.detail, seq],
            )?;
            let review = match &rev {
                Some(r) => {
                    Self::insert_revision(&tx, r)?;
                    Some(r.review_state().as_str())
                }
                None => None,
            };
            tx.execute(
                "UPDATE run SET outcome = 'interrupted', persist_state = 'saved', review_state = ?2,
                    interrupt_reason = 'process_terminated', uncommitted_tail_possible = 1, finalized_utc_ms = ?3
                 WHERE id = ?1 AND outcome IS NULL",
                params![run_id, review, now_utc_ms],
            )?;
            tx.commit()?;
            out.push(RecoveredRun {
                run_id,
                last_durable_seq: seq as u64,
                committed_candidates: u32::try_from(detail.candidates.len()).unwrap_or(u32::MAX),
            });
        }
        Ok(out)
    }

    fn read_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<RawRow> {
        Ok(RawRow {
            run_id: r.get(0)?,
            session_id: r.get(1)?,
            created_utc_ms: r.get(2)?,
            tz_offset_min: r.get(3)?,
            source_mode: r.get(4)?,
            config_hash: r.get(5)?,
            outcome: r.get(6)?,
            persist_state: r.get(7)?,
            review_state: r.get(8)?,
            start_method: r.get(9)?,
            start_ref_frame: r.get(10)?,
            stop_frame: r.get(11)?,
            interrupt_reason: r.get(12)?,
            last_durable_seq: r.get(13)?,
            uncommitted_tail_possible: r.get(14)?,
            finalized_utc_ms: r.get(15)?,
            app_build: r.get(16)?,
            detector_version: r.get(17)?,
            config_json: r.get(18)?,
        })
    }

    const ROW_COLS: &'static str = "id, session_id, created_utc_ms, tz_offset_min, source_mode, config_hash, outcome,
        persist_state, review_state, start_method, start_ref_frame, stop_frame, interrupt_reason, last_durable_seq,
        uncommitted_tail_possible, finalized_utc_ms, app_build, detector_version, config_json";

    pub fn list_runs(&self, limit: u32) -> Result<Vec<RunSummary>> {
        let raws: Vec<RawRow> = {
            let mut st =
                self.conn.prepare(&format!("SELECT {} FROM run ORDER BY created_utc_ms DESC, id LIMIT ?1", Self::ROW_COLS))?;
            st.query_map(params![limit], Self::read_row)?.collect::<std::result::Result<_, _>>()?
        };
        let mut out = Vec::with_capacity(raws.len());
        for raw in raws {
            let config: RunConfig = serde_json::from_str(&raw.config_json)?;
            let row = raw.into_row()?;
            let latest = self.latest_revision(&row.run_id)?;
            let warnings: u32 = self.conn.query_row(
                "SELECT COUNT(*) FROM quality_event WHERE run_id = ?1 AND severity != 'info'",
                params![row.run_id],
                |r| r.get(0),
            )?;
            let results = latest.as_ref().map(|r| compute_results(r, config.expected_count));
            out.push(RunSummary {
                latest_revision: latest.as_ref().map(|r| r.content.number),
                accepted_count: results.as_ref().map(|r| r.count),
                first_ns: results.as_ref().and_then(|r| r.first_ns),
                last_ns: results.as_ref().and_then(|r| r.last_ns),
                expected_count: config.expected_count,
                quality_warnings: warnings,
                edited: latest.as_ref().is_some_and(|r| r.is_edited()),
                row,
            });
        }
        Ok(out)
    }

    pub fn latest_revision(&self, run_id: &str) -> Result<Option<RunRevision>> {
        let row: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT content_json, content_hash FROM run_revision WHERE run_id = ?1 ORDER BY number DESC LIMIT 1",
                params![run_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        row.map(|(json, hash)| {
            let rev = RunRevision { content: serde_json::from_str(&json)?, content_hash: hash };
            if rev.verify_hash() { Ok(rev) } else { Err(StorageError::Corrupt("revision hash mismatch".into())) }
        })
        .transpose()
    }

    pub fn load_run(&self, run_id: &str) -> Result<RunDetail> {
        let raw = self
            .conn
            .query_row(&format!("SELECT {} FROM run WHERE id = ?1", Self::ROW_COLS), params![run_id], Self::read_row)
            .optional()?
            .ok_or_else(|| StorageError::NotFound(format!("run {run_id}")))?;
        let config: RunConfig = serde_json::from_str(&raw.config_json)?;
        let row = raw.into_row()?;
        let epoch = self
            .conn
            .query_row(
                "SELECT id, sample_rate_hz, channels, sample_format, route_json, clock_domain, started_mono_ns, summary_json
                 FROM capture_epoch WHERE run_id = ?1 LIMIT 1",
                params![run_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, u32>(1)?,
                        r.get::<_, u16>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                        r.get::<_, Option<i64>>(6)?,
                        r.get::<_, Option<String>>(7)?,
                    ))
                },
            )
            .optional()?
            .map(|(id, rate, ch, fmt, route, clock, started, summary)| -> Result<EpochRow> {
                Ok(EpochRow {
                    record: EpochRecord {
                        epoch_id: id,
                        run_id: run_id.to_string(),
                        sample_rate_hz: rate,
                        channels: ch,
                        sample_format: fmt,
                        route: serde_json::from_str(&route)?,
                        clock_domain: clock,
                        started_mono_ns: started,
                    },
                    summary_json: summary,
                })
            })
            .transpose()?;

        let cues: Vec<CueObservation> = {
            let mut st = self.conn.prepare(
                "SELECT cue_id, kind, par_index, template_version, requested_mono_ns, issued_mono_ns, render_mono_ns, epoch_id,
                    acoustic_onset_frame, acoustic_onset_mono_ns, match_ncc, span_start_frame, span_end_frame, span_exact,
                    missed, heard FROM cue_observation WHERE run_id = ?1 ORDER BY cue_id",
            )?;
            let rows = st.query_map(params![run_id], |r| {
                Ok((
                    r.get::<_, u32>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<u32>>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, Option<i64>>(6)?,
                    r.get::<_, Option<String>>(7)?,
                    r.get::<_, Option<i64>>(8)?,
                    r.get::<_, Option<i64>>(9)?,
                    r.get::<_, Option<f64>>(10)?,
                    (r.get::<_, Option<i64>>(11)?, r.get::<_, Option<i64>>(12)?, r.get::<_, Option<bool>>(13)?),
                    r.get::<_, bool>(14)?,
                    r.get::<_, Option<bool>>(15)?,
                ))
            })?;
            let mut v = Vec::new();
            for row in rows {
                let (cue_id, kind, par_index, tv, req, iss, ren, ep, onf, onns, ncc, (ss, se, sx), missed, heard) = row?;
                v.push(CueObservation {
                    cue_id,
                    kind: CueKind::parse(&kind).ok_or_else(|| StorageError::Corrupt(format!("cue kind {kind}")))?,
                    par_index,
                    template_version: tv,
                    requested_mono_ns: req,
                    issued_mono_ns: iss,
                    render_mono_ns: ren,
                    epoch_id: ep,
                    acoustic_onset_frame: onf,
                    acoustic_onset_mono_ns: onns,
                    match_ncc: ncc,
                    span_start_frame: ss,
                    span_end_frame: se,
                    span_exact: sx,
                    missed,
                    heard,
                });
            }
            v
        };

        let candidates: Vec<Candidate> = {
            let mut st = self.conn.prepare(
                "SELECT sequence, onset_frame, peak_frame, features_json, suggestion, reasons_json, detector_score,
                    algorithm_version, config_hash FROM detected_candidate WHERE run_id = ?1 ORDER BY epoch_id, sequence",
            )?;
            let rows = st.query_map(params![run_id], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, f64>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, String>(8)?,
                ))
            })?;
            let mut v = Vec::new();
            for row in rows {
                let (seq, on, pk, feat, sug, reasons, score, alg, hash) = row?;
                v.push(Candidate {
                    sequence: seq as u64,
                    onset_frame: on,
                    peak_frame: pk,
                    features: serde_json::from_str(&feat)?,
                    suggestion: serde_json::from_value(serde_json::Value::String(sug))?,
                    reasons: serde_json::from_str(&reasons)?,
                    detector_score: score as f32,
                    algorithm_version: alg,
                    config_hash: hash,
                });
            }
            v
        };

        let quality: Vec<QualityEvent> = {
            let mut st = self.conn.prepare(
                "SELECT kind, severity, start_frame, end_frame, at_mono_ns, detail FROM quality_event WHERE run_id = ?1 ORDER BY seq",
            )?;
            let rows = st.query_map(params![run_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })?;
            let mut v = Vec::new();
            for row in rows {
                let (kind, sev, sf, ef, at, detail) = row?;
                v.push(QualityEvent {
                    kind: serde_json::from_value(serde_json::Value::String(kind))?,
                    severity: serde_json::from_value::<Severity>(serde_json::Value::String(sev))?,
                    start_frame: sf,
                    end_frame: ef,
                    at_ns: at,
                    detail,
                });
            }
            v
        };

        let revisions: Vec<RunRevision> = {
            let mut st =
                self.conn.prepare("SELECT content_json, content_hash FROM run_revision WHERE run_id = ?1 ORDER BY number")?;
            let rows = st.query_map(params![run_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            let mut v = Vec::new();
            for row in rows {
                let (json, hash) = row?;
                let rev = RunRevision { content: serde_json::from_str(&json)?, content_hash: hash };
                if !rev.verify_hash() {
                    return Err(StorageError::Corrupt(format!("revision {} hash mismatch", rev.content.number)));
                }
                v.push(rev);
            }
            v
        };

        let envelope: Vec<EnvelopeChunk> = {
            let mut st = self.conn.prepare(
                "SELECT first_frame, hop_frames, data FROM energy_envelope WHERE run_id = ?1 ORDER BY epoch_id, first_frame",
            )?;
            st.query_map(params![run_id], |r| {
                Ok(EnvelopeChunk { first_frame: r.get(0)?, hop_frames: r.get(1)?, data: r.get(2)? })
            })?
            .collect::<std::result::Result<_, _>>()?
        };

        let classified = match &epoch {
            Some(e) => {
                classify(&candidates, &classify_context(e.record.sample_rate_hz, row.start_ref_frame, &cues, row.stop_frame))
            }
            None => vec![],
        };
        let environment = match &config.environment_snapshot_id {
            Some(id) => self.load_snapshot(id)?,
            None => None,
        };
        Ok(RunDetail { row, config, epoch, cues, candidates, classified, quality, revisions, envelope, environment })
    }

    pub fn insert_calibration(&self, c: &CalibrationRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO calibration_profile_version(id, route_signature, label, algorithm_version, detector_config_json,
                suggestion_json, verdict, impulse_count, os_build, created_utc_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                c.id,
                c.route_signature,
                c.label,
                c.algorithm_version,
                serde_json::to_string(&c.detector_config)?,
                c.suggestion_json,
                c.verdict,
                c.impulse_count,
                c.os_build,
                c.created_utc_ms,
            ],
        )?;
        Ok(())
    }

    /// Latest calibration for exactly this route signature. A different route, rate,
    /// or OS build never reuses a profile.
    pub fn latest_calibration(&self, route_signature: &str) -> Result<Option<CalibrationRecord>> {
        self.conn
            .query_row(
                "SELECT id, route_signature, label, algorithm_version, detector_config_json, suggestion_json, verdict,
                    impulse_count, os_build, created_utc_ms FROM calibration_profile_version
                 WHERE route_signature = ?1 ORDER BY created_utc_ms DESC LIMIT 1",
                params![route_signature],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                        r.get::<_, String>(6)?,
                        r.get::<_, u32>(7)?,
                        r.get::<_, Option<String>>(8)?,
                        r.get::<_, i64>(9)?,
                    ))
                },
            )
            .optional()?
            .map(|(id, sig, label, alg, cfg, sugg, verdict, n, os, at)| {
                Ok(CalibrationRecord {
                    id,
                    route_signature: sig,
                    label,
                    algorithm_version: alg,
                    detector_config: serde_json::from_str(&cfg)?,
                    suggestion_json: sugg,
                    verdict,
                    impulse_count: n,
                    os_build: os,
                    created_utc_ms: at,
                })
            })
            .transpose()
    }

    /// Mark a run's persistence as failed (in-memory payload still held by the engine).
    pub fn note_persist_failed(&self, run_id: &str, error: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE run SET persist_state = 'failed', error = ?2 WHERE id = ?1 AND outcome IS NULL",
            params![run_id, error],
        )?;
        Ok(())
    }
}

/// Classification context from stored cue observations.
pub fn classify_context(
    sample_rate_hz: u32,
    start_ref_frame: Option<i64>,
    cues: &[CueObservation],
    stop_frame: Option<i64>,
) -> ClassifyContext {
    let cue_spans = cues
        .iter()
        .filter_map(|c| {
            Some(CueSpan {
                start_frame: c.span_start_frame?,
                end_frame: c.span_end_frame?,
                is_start_cue: c.kind == CueKind::Start,
                exact: c.span_exact.unwrap_or(false),
            })
        })
        .collect();
    ClassifyContext { sample_rate_hz, start_frame: start_ref_frame, cue_spans, stop_frame }
}

struct RawRow {
    run_id: String,
    session_id: String,
    created_utc_ms: i64,
    tz_offset_min: i32,
    source_mode: String,
    config_hash: String,
    outcome: Option<String>,
    persist_state: String,
    review_state: Option<String>,
    start_method: Option<String>,
    start_ref_frame: Option<i64>,
    stop_frame: Option<i64>,
    interrupt_reason: Option<String>,
    last_durable_seq: i64,
    uncommitted_tail_possible: bool,
    finalized_utc_ms: Option<i64>,
    app_build: String,
    detector_version: Option<String>,
    config_json: String,
}

impl RawRow {
    fn into_row(self) -> Result<RunRow> {
        Ok(RunRow {
            run_id: self.run_id,
            session_id: self.session_id,
            created_utc_ms: self.created_utc_ms,
            tz_offset_min: self.tz_offset_min,
            source_mode: self.source_mode,
            config_hash: self.config_hash,
            outcome: opt_enum(self.outcome, Outcome::parse, "outcome")?,
            persist_state: PersistState::parse(&self.persist_state)
                .ok_or_else(|| StorageError::Corrupt(format!("persist_state {}", self.persist_state)))?,
            review_state: opt_enum(self.review_state, ReviewState::parse, "review_state")?,
            start_method: opt_enum(self.start_method, StartReferenceMethod::parse, "start_method")?,
            start_ref_frame: self.start_ref_frame,
            stop_frame: self.stop_frame,
            interrupt_reason: self.interrupt_reason,
            last_durable_seq: self.last_durable_seq as u64,
            uncommitted_tail_possible: self.uncommitted_tail_possible,
            finalized_utc_ms: self.finalized_utc_ms,
            app_build: self.app_build,
            detector_version: self.detector_version,
        })
    }
}
