//! Schema 3: drills (A14), scores (A15), manual strings (A20), rounds (A24),
//! deliberate deletion, analytics projection, and upgrade from schema 2.

use std::path::PathBuf;

use squib_domain::*;
use squib_storage::*;
use squib_training::analytics::{Filter, summarize};
use squib_training::drill::{DrillVersion, starter_recipes};
use squib_training::manual::ManualString;
use squib_training::rounds::{RoundCount, confirm};
use squib_training::scoring::{ScoreEntry, ScoreRevision};

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("squib-trainstore-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn cfg(mode: SourceMode, drill: Option<String>) -> RunConfig {
    RunConfig {
        schema_version: 1,
        source_mode: mode,
        delay_policy: DelayPolicy::Instant,
        selected_delay_ms: 0,
        pars_ms: vec![],
        stop_policy: StopPolicy::Manual,
        expected_count: None,
        start_cue_template: CueKind::Start.template_version().into(),
        par_cue_template: CueKind::Par.template_version().into(),
        detector: None,
        calibration_profile_id: None,
        route_signature: None,
        shooter_id: "default".into(),
        drill_version_id: drill,
        equipment_version_id: None,
        environment_snapshot_id: None,
        timestamp_mapping_method: TIMESTAMP_MAPPING_METHOD.into(),
        app_build: "test".into(),
    }
}

fn setup(repo: &mut Repository) -> String {
    repo.ensure_default_shooter(1).unwrap();
    repo.active_session("default", "s1", 1, 0).unwrap()
}

fn manual_run(repo: &mut Repository, session: &str, id: &str, times: &str, drill: Option<String>) {
    let m = ManualString::parse("Club timer", 10_000_000, times).unwrap();
    repo.insert_manual_run(
        &RunIntent {
            run_id: id.into(),
            session_id: session.into(),
            created_utc_ms: 10,
            tz_offset_min: 0,
            config: cfg(SourceMode::ManualEntry, drill),
        },
        &m,
    )
    .unwrap();
}

#[test]
fn a14_drill_versions_and_runs_keep_their_version() {
    let mut repo = Repository::open_in_memory().unwrap();
    assert_eq!(repo.schema_version().unwrap(), 3);
    let s = setup(&mut repo);
    let mut recipe = starter_recipes().remove(0);
    recipe.mode = SourceMode::ManualEntry;
    let v1 = DrillVersion::first("d1", recipe.clone(), 1).unwrap();
    repo.save_drill_version(&v1).unwrap();
    manual_run(&mut repo, &s, "r1", "1.5 2.0", Some(drill_ref("d1", 1)));
    let mut edited = recipe.clone();
    edited.title = "Single par (faster)".into();
    let v2 = v1.edit(edited, 2).unwrap();
    repo.save_drill_version(&v2).unwrap();
    // Skipping a version or re-saving an old number is refused.
    assert!(repo.save_drill_version(&v1.edit(recipe.clone(), 3).unwrap()).is_err());
    assert_eq!(repo.list_drills().unwrap()[0].version, 2);
    let d = repo.load_run("r1").unwrap();
    assert_eq!(d.config.drill_version_id.as_deref(), Some("d1#1"), "run keeps the version it used");
    assert_eq!(repo.load_drill_version("d1", 1).unwrap().unwrap(), v1);
    // A run cannot point at a drill version that does not exist.
    let err = repo
        .insert_run_intent(&RunIntent {
            run_id: "r9".into(),
            session_id: s,
            created_utc_ms: 1,
            tz_offset_min: 0,
            config: cfg(SourceMode::ParOnly, Some("d1#7".into())),
        })
        .unwrap_err();
    assert!(matches!(err, StorageError::Conflict(_)));
}

#[test]
fn a20_manual_run_keeps_precision_and_label() {
    let mut repo = Repository::open_in_memory().unwrap();
    let s = setup(&mut repo);
    manual_run(&mut repo, &s, "r1", "1.42 1.83 2.25", None);
    let d = repo.load_run("r1").unwrap();
    assert_eq!(d.row.outcome, Some(Outcome::Complete));
    assert_eq!(d.row.persist_state, PersistState::Saved);
    let rev = d.revisions.last().unwrap();
    assert!(rev.content.accepted.iter().all(|e| e.origin == EventOrigin::Manual), "no fictional detector events");
    assert!(d.candidates.is_empty());
    let r = compute_results(rev, None);
    assert_eq!(r.first_ns, Some(1_420_000_000));
    assert_eq!(r.splits_ns, vec![410_000_000, 420_000_000]);
    let m = repo.load_manual_string("r1").unwrap().unwrap();
    assert_eq!((m.source_label.as_str(), m.precision_ns), ("Club timer", 10_000_000));
}

#[test]
fn a15_score_revisions_chain_and_never_touch_timestamps() {
    let mut repo = Repository::open_in_memory().unwrap();
    let s = setup(&mut repo);
    manual_run(&mut repo, &s, "r1", "1.0 2.0", None);
    let before = repo.load_run("r1").unwrap().revisions;
    let e = ScoreEntry {
        profile_id: "generic-points-hf".into(),
        profile_version: 1,
        counts: [("A".to_string(), 2u32)].into_iter().collect(),
        complete: true,
        manual_elapsed_ns: None,
        manual_precision_ns: None,
        manual_points: None,
        notes: String::new(),
    };
    let s1 = ScoreRevision::new("r1", None, e.clone(), "local", 5).unwrap();
    repo.append_score_revision(&s1).unwrap();
    let s2 = ScoreRevision::new("r1", Some(&s1), e.clone(), "local", 6).unwrap();
    repo.append_score_revision(&s2).unwrap();
    let stale = ScoreRevision::new("r1", Some(&s1), e, "other", 7).unwrap();
    assert!(repo.append_score_revision(&stale).is_err(), "stale parent refused");
    assert_eq!(repo.latest_score("r1").unwrap().unwrap().number, 2);
    assert_eq!(repo.load_run("r1").unwrap().revisions, before, "scores do not change timing revisions");
}

#[test]
fn guarded_deletion_removes_everything_for_a_run_only() {
    let dir = tmpdir("delete");
    let p = dir.join("j.db");
    let mut repo = Repository::open(&p).unwrap();
    let s = setup(&mut repo);
    manual_run(&mut repo, &s, "keep", "1.0", None);
    manual_run(&mut repo, &s, "gone", "1.0 2.0", None);
    repo.set_round_count(&RoundCount { run_id: "gone".into(), proposed: 2, confirmed: Some(2), confirmed_utc_ms: Some(1) })
        .unwrap();
    repo.insert_attachment(&AttachmentRecord {
        id: "a1".into(),
        run_id: Some("gone".into()),
        kind: "photo".into(),
        relative_path: "attachments/a1.jpg".into(),
        sha256: "x".into(),
        bytes: 3,
        mime: "image/jpeg".into(),
        metadata_stripped: true,
        created_utc_ms: 1,
    })
    .unwrap();
    let rep = repo.delete_run("gone").unwrap();
    assert_eq!(rep.runs, 1);
    assert_eq!(rep.attachment_paths, vec!["attachments/a1.jpg".to_string()]);
    assert!(matches!(repo.load_run("gone"), Err(StorageError::NotFound(_))));
    assert!(repo.load_run("keep").is_ok());
    assert!(repo.round_counts().unwrap().is_empty());
    drop(repo);
    // Outside the repository's deletion path, originals stay protected.
    let c = rusqlite::Connection::open(&p).unwrap();
    assert!(c.execute("DELETE FROM run_revision", []).unwrap_err().to_string().contains("immutable"));
    assert!(c.execute("UPDATE manual_string SET precision_ns = 1", []).unwrap_err().to_string().contains("immutable"));
    let mut repo = Repository::open(&p).unwrap();
    assert_eq!(repo.delete_session(&s).unwrap().runs, 1);
    assert!(repo.list_runs(10).unwrap().is_empty());
}

#[test]
fn a24_rounds_and_analytics_projection() {
    let mut repo = Repository::open_in_memory().unwrap();
    let s = setup(&mut repo);
    for (i, t) in ["1.0 1.5 2.1", "1.1 1.6", "0.9 1.4 1.9"].iter().enumerate() {
        manual_run(&mut repo, &s, &format!("r{i}"), t, None);
    }
    let rc = RoundCount { run_id: "r0".into(), proposed: 0, confirmed: None, confirmed_utc_ms: None };
    repo.set_round_count(&confirm(&rc, 3, 9).unwrap()).unwrap();
    assert_eq!(repo.round_counts().unwrap()[0].confirmed, Some(3));
    let recs = repo.analytics_records().unwrap();
    assert_eq!(recs.len(), 3);
    assert!(recs.iter().all(|r| r.manual && r.timing_method == "manual"));
    let mut f = Filter::new("default");
    assert_eq!(summarize(&recs, &f).included, 0, "manual results excluded unless chosen");
    f.include_manual = true;
    let sm = summarize(&recs, &f);
    assert_eq!(sm.included, 3);
    assert!(sm.insufficient);
    assert_eq!(sm.final_time.unwrap().median, 1.9);
}

#[test]
fn upgrade_from_schema_2_with_data() {
    let dir = tmpdir("upgrade");
    let p = dir.join("journal.db");
    {
        let c = rusqlite::Connection::open(&p).unwrap();
        c.execute_batch(MIGRATIONS[0].1).unwrap();
        c.execute_batch(MIGRATIONS[1].1).unwrap();
        c.pragma_update(None, "user_version", 2).unwrap();
        c.execute_batch(
            "INSERT INTO shooter_profile(id, name, created_utc_ms) VALUES ('default', 'Me', 1);
             INSERT INTO session(id, shooter_id, started_utc_ms, tz_offset_min, active) VALUES ('s1', 'default', 1, 0, 1);",
        )
        .unwrap();
        c.execute(
            "INSERT INTO run(id, session_id, shooter_id, created_utc_ms, tz_offset_min, source_mode, config_json, config_hash,
               app_build, domain_schema_version, persist_state, outcome) VALUES ('r1','s1','default',5,0,'par_only',?1,'h','t',1,'saved','complete')",
            [serde_json::to_string(&cfg(SourceMode::ParOnly, None)).unwrap()],
        )
        .unwrap();
    }
    let mut repo = Repository::open(&p).unwrap();
    assert_eq!(repo.schema_version().unwrap(), 3);
    assert!(dir.join("journal.pre-v3.bak").exists());
    assert_eq!(repo.load_run("r1").unwrap().row.outcome, Some(Outcome::Complete));
    assert_eq!(repo.delete_run("r1").unwrap().runs, 1, "pre-existing runs can be deleted deliberately");
}
