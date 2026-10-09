//! M3 facade: profiles, drills, scores, manual strings, rounds, analytics,
//! attachments, backup/import, CSV/share, deletion, and the diagnostic report.

use std::path::{Path, PathBuf};

use squib_archive::{ExportOptions, Limits};
use squib_domain::{DOMAIN_SCHEMA_VERSION, DelayPolicy, RunConfig, SourceMode, TIMESTAMP_MAPPING_METHOD};
use squib_storage::{AttachmentRecord, DEFAULT_SHOOTER_ID, RunIntent, StorageError, drill_ref, parse_drill_ref};
use squib_training::analytics::{Filter, Stats, summarize};
use squib_training::drill::{DrillRecipe, DrillVersion, RECIPE_SCHEMA_VERSION, starter_recipes};
use squib_training::manual::ManualString;
use squib_training::rounds::{CostSetting, RoundCount, confirm, propose};
use squib_training::scoring::{ScoreEntry, ScoreRevision, ScoreStatus, builtin, builtin_profiles, compute};

use crate::engine::SquibEngine;
use crate::ffi::{Mode, SquibError, StartDelay};

pub const SETTING_ACTIVE_SHOOTER: &str = "active_shooter";
pub const SETTING_DRILLS_SEEDED: &str = "starter_drills_seeded";
pub const SETTING_COST: &str = "round_cost";

fn inval(e: impl ToString) -> SquibError {
    SquibError::Invalid(e.to_string())
}

// ---- FFI types ------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ShooterView {
    pub id: String,
    pub name: String,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DrillInput {
    pub title: String,
    pub description: String,
    pub mode: Mode,
    /// `manual_entry` drills record another timer's result.
    pub manual: bool,
    pub delay: StartDelay,
    pub expected_count: Option<u32>,
    pub pars_ms: Vec<u32>,
    pub repeats: u32,
    pub rest_s: u32,
    pub scoring_profile_id: String,
    pub notes: String,
    pub equipment_tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DrillView {
    pub drill_id: String,
    pub version: u32,
    pub input: DrillInput,
    pub scoring_title: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ScoringProfileView {
    pub id: String,
    pub title: String,
    pub discipline_label: String,
    /// Category or penalty names the user can count, with their value
    /// (points for hit-factor profiles, milliseconds for time-plus).
    pub inputs: Vec<ScoreInput>,
    pub manual: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ScoreInput {
    pub name: String,
    pub value: i32,
    /// `points`, `penalty_points`, or `penalty_ms`.
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ScoreCount {
    pub name: String,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ScoreView {
    pub profile_id: String,
    pub profile_title: String,
    pub revision: Option<u32>,
    pub counts: Vec<ScoreCount>,
    pub complete: bool,
    /// complete / incomplete / invalid.
    pub status: String,
    pub status_reason: Option<String>,
    pub display: Option<String>,
    pub hit_factor: Option<f64>,
    pub final_time_ns: Option<i64>,
    pub manual: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct RoundsView {
    pub proposed: u32,
    pub confirmed: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct RoundTotalsView {
    pub confirmed_rounds: u64,
    pub unconfirmed_runs: u32,
    pub cost_minor: Option<i64>,
    pub currency: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct StatsView {
    pub n: u32,
    pub min: f64,
    pub q1: f64,
    pub median: f64,
    pub q3: f64,
    pub max: f64,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AnalyticsFilterFfi {
    pub drill_id: Option<String>,
    pub drill_version: Option<u32>,
    pub mode: Option<String>,
    pub include_edited: bool,
    pub include_manual: bool,
    pub include_needs_review: bool,
    pub include_unresolved_start_splits: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ExclusionCount {
    pub reason: String,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AnalyticsView {
    pub considered: u32,
    pub included: u32,
    pub excluded: Vec<ExclusionCount>,
    pub first_shot_s: Option<StatsView>,
    pub splits_s: Option<StatsView>,
    pub final_time_s: Option<StatsView>,
    pub hit_factor: Option<StatsView>,
    pub insufficient: bool,
    pub mixed_timing_methods: Vec<String>,
    pub calc_version: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AttachmentView {
    pub id: String,
    pub relative_path: String,
    pub bytes: i64,
    pub metadata_stripped: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct BackupView {
    pub bytes: u64,
    pub runs: u64,
    pub attachments: u64,
    pub contains_location: bool,
    pub encrypted: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ImportView {
    pub runs_new: u64,
    pub runs_already_present: u64,
    pub records_new: u64,
    pub conflicts: Vec<String>,
    pub attachments_restored: u64,
    pub schema_version: u32,
    pub contains_location: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DeletionView {
    pub runs: u32,
    /// App-relative attachment paths for the platform to delete.
    pub attachment_paths: Vec<String>,
}

fn stats_view(s: Option<Stats>) -> Option<StatsView> {
    s.map(|s| StatsView { n: s.n as u32, min: s.min, q1: s.q1, median: s.median, q3: s.q3, max: s.max })
}

fn to_recipe(d: &DrillInput) -> DrillRecipe {
    DrillRecipe {
        schema_version: RECIPE_SCHEMA_VERSION,
        title: d.title.trim().into(),
        description: d.description.clone(),
        mode: if d.manual { SourceMode::ManualEntry } else { d.mode.into() },
        delay: d.delay.into(),
        expected_count: d.expected_count,
        pars_ms: d.pars_ms.clone(),
        repeats: d.repeats,
        rest_s: d.rest_s,
        scoring_profile_id: d.scoring_profile_id.clone(),
        scoring_profile_version: 1,
        notes: d.notes.clone(),
        equipment_tags: d.equipment_tags.iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect(),
    }
}

fn from_recipe(r: &DrillRecipe) -> DrillInput {
    DrillInput {
        title: r.title.clone(),
        description: r.description.clone(),
        mode: if r.mode == SourceMode::PhoneLive { Mode::PhoneLive } else { Mode::ParOnly },
        manual: r.mode == SourceMode::ManualEntry,
        delay: match r.delay {
            DelayPolicy::Instant => StartDelay::Instant,
            DelayPolicy::Fixed { ms } => StartDelay::Fixed { ms },
            DelayPolicy::Random { min_ms, max_ms } => StartDelay::Random { min_ms, max_ms },
        },
        expected_count: r.expected_count,
        pars_ms: r.pars_ms.clone(),
        repeats: r.repeats,
        rest_s: r.rest_s,
        scoring_profile_id: r.scoring_profile_id.clone(),
        notes: r.notes.clone(),
        equipment_tags: r.equipment_tags.clone(),
    }
}

pub(crate) fn drill_view(v: &DrillVersion) -> DrillView {
    DrillView {
        drill_id: v.drill_id.clone(),
        version: v.version,
        scoring_title: builtin(&v.recipe.scoring_profile_id, v.recipe.scoring_profile_version)
            .map(|p| p.title)
            .unwrap_or_default(),
        input: from_recipe(&v.recipe),
    }
}

fn profile_view(p: &squib_training::scoring::ScoringProfile) -> ScoringProfileView {
    use squib_training::scoring::ProfileKind::*;
    let inputs = match &p.kind {
        PointsHitFactor { categories, penalties } => categories
            .iter()
            .map(|(n, v)| ScoreInput { name: n.clone(), value: *v, kind: "points".into() })
            .chain(penalties.iter().map(|(n, v)| ScoreInput { name: n.clone(), value: *v, kind: "penalty_points".into() }))
            .collect(),
        TimePlus { penalties_ms } => penalties_ms
            .iter()
            .map(|(n, v)| ScoreInput { name: n.clone(), value: *v as i32, kind: "penalty_ms".into() })
            .collect(),
        _ => vec![],
    };
    ScoringProfileView {
        id: p.id.clone(),
        title: p.title.clone(),
        discipline_label: p.discipline_label.clone(),
        inputs,
        manual: matches!(p.kind, ManualResult),
    }
}

impl SquibEngine {
    pub(crate) fn active_shooter(&self) -> String {
        self.read_repo().get_setting(SETTING_ACTIVE_SHOOTER).ok().flatten().unwrap_or_else(|| DEFAULT_SHOOTER_ID.into())
    }

    /// Seed the neutral starter drills once, on first launch.
    pub(crate) fn seed_drills(&self, now_utc_ms: i64) {
        if self.read_repo().get_setting(SETTING_DRILLS_SEEDED).ok().flatten().is_some() {
            return;
        }
        let _ = self.store_actor().exec(move |repo| {
            for (i, r) in starter_recipes().into_iter().enumerate() {
                let v = DrillVersion::first(&format!("starter-{}", i + 1), r, now_utc_ms)
                    .map_err(|e| StorageError::Conflict(e.to_string()))?;
                repo.save_drill_version(&v)?;
            }
            repo.set_setting(SETTING_DRILLS_SEEDED, "1")
        });
    }

    /// Run elapsed time from the latest review revision (final accepted event).
    fn run_elapsed(&self, run_id: &str) -> Result<Option<i64>, SquibError> {
        let d = self.read_repo().load_run(run_id)?;
        Ok(d.revisions.last().and_then(|r| squib_domain::compute_results(r, None).last_ns))
    }

    fn scoring_profile_for(&self, run_id: &str) -> Result<Option<squib_training::scoring::ScoringProfile>, SquibError> {
        let repo = self.read_repo();
        let d = repo.load_run(run_id)?;
        if let Some((id, v)) = d.config.drill_version_id.as_deref().and_then(parse_drill_ref)
            && let Some(dv) = repo.load_drill_version(&id, v)?
        {
            return Ok(builtin(&dv.recipe.scoring_profile_id, dv.recipe.scoring_profile_version));
        }
        Ok(repo.latest_score(run_id)?.and_then(|s| builtin(&s.entry.profile_id, s.entry.profile_version)))
    }
}

#[uniffi::export]
impl SquibEngine {
    // ---- Profiles (A24: switching is refused while a run is active) ---------------------

    pub fn list_shooters(&self) -> Vec<ShooterView> {
        let active = self.active_shooter();
        self.read_repo()
            .list_shooters()
            .unwrap_or_default()
            .into_iter()
            .filter(|s| !s.archived)
            .map(|s| ShooterView { active: s.id == active, id: s.id, name: s.name })
            .collect()
    }

    pub fn add_shooter(&self, name: String, now_utc_ms: i64) -> Result<ShooterView, SquibError> {
        let id = uuid::Uuid::new_v4().to_string();
        let (i, n) = (id.clone(), name.clone());
        self.store_actor().exec(move |repo| repo.add_shooter(&i, &n, now_utc_ms))?;
        Ok(ShooterView { id, name: name.trim().into(), active: false })
    }

    pub fn rename_shooter(&self, id: String, name: String) -> Result<(), SquibError> {
        self.store_actor().exec(move |repo| repo.rename_shooter(&id, &name))?;
        Ok(())
    }

    pub fn set_active_shooter(&self, id: String) -> Result<(), SquibError> {
        if self.run_active() {
            return Err(SquibError::Rejected("finish or cancel the current run before switching shooter".into()));
        }
        if !self.list_shooters().iter().any(|s| s.id == id) {
            return Err(SquibError::NotFound(format!("shooter {id}")));
        }
        self.store_actor().exec(move |repo| repo.set_setting(SETTING_ACTIVE_SHOOTER, &id))?;
        Ok(())
    }

    // ---- Drills (A14) ------------------------------------------------------------------

    pub fn list_drills(&self) -> Vec<DrillView> {
        self.read_repo().list_drills().unwrap_or_default().iter().map(drill_view).collect()
    }

    pub fn scoring_profiles(&self) -> Vec<ScoringProfileView> {
        builtin_profiles().iter().map(profile_view).collect()
    }

    pub fn create_drill(&self, input: DrillInput, now_utc_ms: i64) -> Result<DrillView, SquibError> {
        let v = DrillVersion::first(&uuid::Uuid::new_v4().to_string(), to_recipe(&input), now_utc_ms).map_err(inval)?;
        let view = drill_view(&v);
        self.store_actor().exec(move |repo| repo.save_drill_version(&v))?;
        Ok(view)
    }

    /// Editing creates the next version; runs keep the version they used.
    pub fn edit_drill(&self, drill_id: String, input: DrillInput, now_utc_ms: i64) -> Result<DrillView, SquibError> {
        let latest = self
            .read_repo()
            .list_drills()?
            .into_iter()
            .find(|d| d.drill_id == drill_id)
            .ok_or_else(|| SquibError::NotFound(format!("drill {drill_id}")))?;
        let v = latest.edit(to_recipe(&input), now_utc_ms).map_err(inval)?;
        let view = drill_view(&v);
        self.store_actor().exec(move |repo| repo.save_drill_version(&v))?;
        Ok(view)
    }

    pub fn archive_drill(&self, drill_id: String) -> Result<(), SquibError> {
        self.store_actor().exec(move |repo| repo.archive_drill(&drill_id))?;
        Ok(())
    }

    /// Recipe as a small shareable file (data only).
    pub fn drill_file(&self, drill_id: String, version: u32) -> Result<Vec<u8>, SquibError> {
        let v = self.read_repo().load_drill_version(&drill_id, version)?.ok_or_else(|| SquibError::NotFound(drill_id.clone()))?;
        serde_json::to_vec_pretty(&v.recipe).map_err(inval)
    }

    /// Import a shared recipe file as a new drill (validated data, never executed).
    pub fn import_drill_file(&self, bytes: Vec<u8>, now_utc_ms: i64) -> Result<DrillView, SquibError> {
        let r = DrillRecipe::from_shared_bytes(&bytes).map_err(inval)?;
        let v = DrillVersion::first(&uuid::Uuid::new_v4().to_string(), r, now_utc_ms).map_err(inval)?;
        let view = drill_view(&v);
        self.store_actor().exec(move |repo| repo.save_drill_version(&v))?;
        Ok(view)
    }

    // ---- Scores (A15) -------------------------------------------------------------------

    pub fn run_score(&self, run_id: String, profile_id: Option<String>) -> Result<ScoreView, SquibError> {
        let latest = self.read_repo().latest_score(&run_id)?;
        let profile = match profile_id {
            Some(id) => builtin(&id, 1),
            None => self.scoring_profile_for(&run_id)?,
        }
        .or_else(|| builtin("generic-time", 1))
        .ok_or_else(|| inval("no scoring profile"))?;
        let entry = latest.as_ref().map(|s| &s.entry).filter(|e| e.profile_id == profile.id);
        let r = compute(entry, &profile, self.run_elapsed(&run_id)?);
        let (status, reason) = match &r.status {
            ScoreStatus::Complete => ("complete".to_string(), None),
            ScoreStatus::Incomplete(s) => ("incomplete".to_string(), Some(s.clone())),
            ScoreStatus::Invalid(s) => ("invalid".to_string(), Some(s.clone())),
        };
        Ok(ScoreView {
            profile_id: profile.id.clone(),
            profile_title: profile.title.clone(),
            revision: latest.as_ref().map(|s| s.number),
            counts: entry
                .map(|e| e.counts.iter().map(|(n, c)| ScoreCount { name: n.clone(), count: *c }).collect())
                .unwrap_or_default(),
            complete: entry.is_some_and(|e| e.complete),
            status,
            status_reason: reason,
            display: r.display,
            hit_factor: r.hit_factor,
            final_time_ns: r.final_time_ns,
            manual: r.manual,
        })
    }

    /// Save a score as a new score revision (never touches timing).
    #[allow(clippy::too_many_arguments)] // UniFFI methods take positional arguments.
    pub fn set_score(
        &self,
        run_id: String,
        profile_id: String,
        counts: Vec<ScoreCount>,
        complete: bool,
        manual_points: Option<i64>,
        notes: String,
        now_utc_ms: i64,
    ) -> Result<ScoreView, SquibError> {
        let entry = ScoreEntry {
            profile_id: profile_id.clone(),
            profile_version: 1,
            counts: counts.into_iter().filter(|c| c.count > 0).map(|c| (c.name, c.count)).collect(),
            complete,
            manual_elapsed_ns: None,
            manual_precision_ns: None,
            manual_points,
            notes,
        };
        let parent = self.read_repo().latest_score(&run_id)?;
        let rev = ScoreRevision::new(&run_id, parent.as_ref(), entry, "local-device", now_utc_ms).map_err(inval)?;
        self.store_actor().exec(move |repo| repo.append_score_revision(&rev))?;
        self.run_score(run_id, Some(profile_id))
    }

    // ---- Manual strings (A20) -----------------------------------------------------------

    #[allow(clippy::too_many_arguments)] // UniFFI methods take positional arguments.
    pub fn add_manual_run(
        &self,
        source_label: String,
        precision_ms: u32,
        times_text: String,
        drill_id: Option<String>,
        drill_version: Option<u32>,
        now_utc_ms: i64,
        tz_offset_min: i32,
        app_build: String,
    ) -> Result<String, SquibError> {
        if self.run_active() {
            return Err(SquibError::Rejected("a timed run is in progress".into()));
        }
        let m = ManualString::parse(&source_label, i64::from(precision_ms) * 1_000_000, &times_text).map_err(inval)?;
        let shooter = self.active_shooter();
        let config = RunConfig {
            schema_version: DOMAIN_SCHEMA_VERSION,
            source_mode: SourceMode::ManualEntry,
            delay_policy: DelayPolicy::Instant,
            selected_delay_ms: 0,
            pars_ms: vec![],
            stop_policy: squib_domain::StopPolicy::Manual,
            expected_count: None,
            start_cue_template: squib_domain::CueKind::Start.template_version().into(),
            par_cue_template: squib_domain::CueKind::Par.template_version().into(),
            detector: None,
            calibration_profile_id: None,
            route_signature: None,
            shooter_id: shooter.clone(),
            drill_version_id: drill_id.zip(drill_version).map(|(d, v)| drill_ref(&d, v)),
            equipment_version_id: None,
            environment_snapshot_id: self.pin_conditions(now_utc_ms)?,
            timestamp_mapping_method: TIMESTAMP_MAPPING_METHOD.into(),
            app_build,
        };
        config.validate().map_err(inval)?;
        let run_id = uuid::Uuid::new_v4().to_string();
        let rid = run_id.clone();
        let sid = uuid::Uuid::new_v4().to_string();
        self.store_actor().exec(move |repo| {
            let session_id = repo.active_session(&shooter, &sid, now_utc_ms, tz_offset_min)?;
            repo.insert_manual_run(&RunIntent { run_id: rid, session_id, created_utc_ms: now_utc_ms, tz_offset_min, config }, &m)
        })?;
        Ok(run_id)
    }

    // ---- Rounds (A24) -------------------------------------------------------------------

    pub fn run_rounds(&self, run_id: String) -> Result<RoundsView, SquibError> {
        let repo = self.read_repo();
        if let Some(rc) = repo.round_counts()?.into_iter().find(|r| r.run_id == run_id) {
            return Ok(RoundsView { proposed: rc.proposed, confirmed: rc.confirmed });
        }
        let d = repo.load_run(&run_id)?;
        let accepted = d.revisions.last().map(|r| r.content.accepted.len() as u32).unwrap_or(0);
        Ok(RoundsView { proposed: propose(&d.row.source_mode, accepted), confirmed: None })
    }

    pub fn confirm_rounds(&self, run_id: String, rounds: u32, now_utc_ms: i64) -> Result<RoundsView, SquibError> {
        let v = self.run_rounds(run_id.clone())?;
        let rc =
            confirm(&RoundCount { run_id, proposed: v.proposed, confirmed: None, confirmed_utc_ms: None }, rounds, now_utc_ms)
                .map_err(inval)?;
        let out = RoundsView { proposed: rc.proposed, confirmed: rc.confirmed };
        self.store_actor().exec(move |repo| repo.set_round_count(&rc))?;
        Ok(out)
    }

    pub fn set_round_cost(&self, per_round_minor: Option<i64>, currency: Option<String>) -> Result<(), SquibError> {
        let value = match (per_round_minor, currency) {
            (Some(c), Some(cur)) => {
                let s = CostSetting { per_round_minor: c, currency: cur.trim().to_uppercase() };
                s.validate().map_err(inval)?;
                serde_json::to_string(&s).map_err(inval)?
            }
            _ => String::new(),
        };
        self.store_actor().exec(move |repo| repo.set_setting(SETTING_COST, &value))?;
        Ok(())
    }

    pub fn round_totals(&self) -> RoundTotalsView {
        let repo = self.read_repo();
        let cost: Option<CostSetting> = repo.get_setting(SETTING_COST).ok().flatten().and_then(|j| serde_json::from_str(&j).ok());
        let s = squib_training::rounds::summarize(&repo.round_counts().unwrap_or_default(), cost.as_ref());
        RoundTotalsView {
            confirmed_rounds: s.confirmed_rounds,
            unconfirmed_runs: s.unconfirmed_runs,
            cost_minor: s.cost_minor,
            currency: s.currency,
        }
    }

    // ---- Analytics (A16) ----------------------------------------------------------------

    pub fn analytics(&self, f: AnalyticsFilterFfi) -> Result<AnalyticsView, SquibError> {
        let recs = self.read_repo().analytics_records()?;
        let mut filter = Filter::new(&self.active_shooter());
        filter.drill_id = f.drill_id;
        filter.drill_version = f.drill_version;
        filter.mode = f.mode;
        filter.include_edited = f.include_edited;
        filter.include_manual = f.include_manual;
        filter.include_needs_review = f.include_needs_review;
        filter.include_unresolved_start_splits = f.include_unresolved_start_splits;
        let s = summarize(&recs, &filter);
        Ok(AnalyticsView {
            considered: s.considered as u32,
            included: s.included as u32,
            excluded: s
                .excluded
                .iter()
                .map(|(k, v)| ExclusionCount {
                    reason: serde_json::to_value(k).ok().and_then(|x| x.as_str().map(String::from)).unwrap_or_default(),
                    count: *v as u32,
                })
                .collect(),
            first_shot_s: stats_view(s.first_shot),
            splits_s: stats_view(s.splits),
            final_time_s: stats_view(s.final_time),
            hit_factor: stats_view(s.hit_factor),
            insufficient: s.insufficient,
            mixed_timing_methods: s.mixed_timing_methods,
            calc_version: s.calc_version,
        })
    }

    // ---- Attachments --------------------------------------------------------------------

    /// Register a file the platform already wrote (metadata stripped) under the
    /// attachment root. The core hashes it and records the reference.
    pub fn register_attachment(
        &self,
        run_id: String,
        attachment_root: String,
        relative_path: String,
        mime: String,
        metadata_stripped: bool,
        now_utc_ms: i64,
    ) -> Result<AttachmentView, SquibError> {
        let bytes = std::fs::read(Path::new(&attachment_root).join(&relative_path)).map_err(inval)?;
        let rec = AttachmentRecord {
            id: uuid::Uuid::new_v4().to_string(),
            run_id: Some(run_id),
            kind: "photo".into(),
            relative_path,
            sha256: squib_domain::hash::sha256_hex(&bytes),
            bytes: bytes.len() as i64,
            mime,
            metadata_stripped,
            created_utc_ms: now_utc_ms,
        };
        let view =
            AttachmentView { id: rec.id.clone(), relative_path: rec.relative_path.clone(), bytes: rec.bytes, metadata_stripped };
        self.store_actor().exec(move |repo| repo.insert_attachment(&rec))?;
        Ok(view)
    }

    pub fn run_attachments(&self, run_id: String) -> Vec<AttachmentView> {
        self.read_repo()
            .list_attachments(Some(&run_id))
            .unwrap_or_default()
            .into_iter()
            .map(|a| AttachmentView {
                id: a.id,
                relative_path: a.relative_path,
                bytes: a.bytes,
                metadata_stripped: a.metadata_stripped,
            })
            .collect()
    }

    // ---- Portability (A17, A19) -----------------------------------------------------------

    pub fn export_backup(
        &self,
        out_path: String,
        attachment_root: Option<String>,
        app_version: String,
        now_utc_ms: i64,
    ) -> Result<BackupView, SquibError> {
        let opts = ExportOptions { app_version, created_utc_ms: now_utc_ms, attachment_root: attachment_root.map(PathBuf::from) };
        let s = self.store_actor().exec(move |repo| {
            squib_archive::export_private(repo, Path::new(&out_path), &opts).map_err(|e| StorageError::Write(e.to_string()))
        })?;
        Ok(BackupView {
            bytes: s.bytes,
            runs: s.runs,
            attachments: s.attachments,
            contains_location: s.contains.location,
            encrypted: false,
        })
    }

    pub fn preview_import(&self, archive_path: String, scratch_dir: String) -> Result<ImportView, SquibError> {
        let r = self.store_actor().exec(move |repo| {
            squib_archive::preview(repo, Path::new(&archive_path), Path::new(&scratch_dir), &Limits::default())
                .map_err(|e| StorageError::Conflict(e.to_string()))
        })?;
        Ok(import_view(&r))
    }

    pub fn import_backup(
        &self,
        archive_path: String,
        scratch_dir: String,
        attachment_root: Option<String>,
    ) -> Result<ImportView, SquibError> {
        if self.run_active() {
            return Err(SquibError::Rejected("finish the current run before importing".into()));
        }
        let r = self.store_actor().exec(move |repo| {
            squib_archive::import(
                repo,
                Path::new(&archive_path),
                Path::new(&scratch_dir),
                attachment_root.as_deref().map(Path::new),
                &Limits::default(),
            )
            .map_err(|e| StorageError::Conflict(e.to_string()))
        })?;
        Ok(import_view(&r))
    }

    pub fn export_csv(&self) -> Result<String, SquibError> {
        squib_archive::csv_export::runs_csv(&self.read_repo()).map_err(inval)
    }

    /// Redacted results for the selected runs, as JSON, for preview before sharing.
    pub fn share_results(&self, run_ids: Vec<String>) -> Result<String, SquibError> {
        let s = squib_archive::share::share_results(&self.read_repo(), &run_ids).map_err(inval)?;
        serde_json::to_string_pretty(&s).map_err(inval)
    }

    // ---- Deletion ----------------------------------------------------------------------------

    pub fn delete_run(&self, run_id: String) -> Result<DeletionView, SquibError> {
        if self.current_run_id().as_deref() == Some(run_id.as_str()) && self.run_active() {
            return Err(SquibError::Rejected("cannot delete the run in progress".into()));
        }
        let r = self.store_actor().exec(move |repo| repo.delete_run(&run_id))?;
        Ok(DeletionView { runs: r.runs, attachment_paths: r.attachment_paths })
    }

    pub fn delete_all_history(&self) -> Result<DeletionView, SquibError> {
        if self.run_active() {
            return Err(SquibError::Rejected("finish the current run first".into()));
        }
        let r = self.store_actor().exec(|repo| repo.delete_all_history())?;
        Ok(DeletionView { runs: r.runs, attachment_paths: r.attachment_paths })
    }

    // ---- Diagnostic report (GitHub issue export) ------------------------------------------------

    /// Plain-text diagnostic report for a bug report. Contains versions, device model,
    /// capture diagnostics, and recent run outcomes with quality labels. Never contains
    /// audio, coordinates, station ids or names, place labels, notes, or run ids.
    pub fn diagnostic_report(&self, device: String, android_version: String, app_version: String) -> String {
        let mut out = String::new();
        out.push_str(&format!("App: {app_version}\nDevice: {device}\nAndroid: {android_version}\n"));
        for v in crate::core_versions() {
            out.push_str(&format!("{v}\n"));
        }
        if let Some(d) = self.capture_diagnostics() {
            out.push_str(&format!(
                "\nLast capture: blocks {} | DSP p50 {} us p99 {} us max {} us | block {} us | queue {} of {} high water, {} overflows | clock {} ({} anchors ok, {} rejected, drift {}) | delivery mean {} ms max {} ms\n",
                d.blocks,
                d.dsp_p50_ns / 1000,
                d.dsp_p99_ns / 1000,
                d.dsp_max_ns / 1000,
                d.block_duration_ns / 1000,
                d.queue_high_water_blocks,
                d.queue_capacity_blocks,
                d.queue_overflows,
                d.timestamp_quality,
                d.anchors_accepted,
                d.anchors_rejected,
                d.drift_ppm.map(|p| format!("{p:.1} ppm")).unwrap_or_else(|| "n/a".into()),
                d.delivery_mean_ns.map(|v| (v / 1_000_000).to_string()).unwrap_or_else(|| "n/a".into()),
                d.delivery_max_ns / 1_000_000,
            ));
        }
        let repo = self.read_repo();
        out.push_str("\nRecent runs (newest first):\n");
        for s in repo.list_runs(10).unwrap_or_default() {
            let quality: Vec<String> = repo
                .load_run(&s.row.run_id)
                .map(|d| {
                    d.quality
                        .iter()
                        .filter(|q| q.severity != squib_domain::Severity::Info)
                        .map(|q| q.kind.as_str().to_string())
                        .collect()
                })
                .unwrap_or_default();
            out.push_str(&format!(
                "- {} | {} | {} | review {} | start {} | shots {} | quality [{}]\n",
                squib_archive::iso_utc(s.row.created_utc_ms),
                s.row.source_mode,
                s.row.outcome.map(|o| o.as_str()).unwrap_or("unfinished"),
                s.row.review_state.map(|r| r.as_str()).unwrap_or("n/a"),
                s.row.start_method.map(|m| m.as_str()).unwrap_or("n/a"),
                s.accepted_count.unwrap_or(0),
                quality.join(", "),
            ));
        }
        out
    }
}

fn import_view(r: &squib_archive::ImportReport) -> ImportView {
    ImportView {
        runs_new: r.inserted.get("run").copied().unwrap_or(0),
        runs_already_present: r.skipped_identical.get("run").copied().unwrap_or(0),
        records_new: r.inserted.values().sum(),
        conflicts: r.conflicts.clone(),
        attachments_restored: r.attachments_restored,
        schema_version: r.schema_version,
        contains_location: r.contains_location,
    }
}
