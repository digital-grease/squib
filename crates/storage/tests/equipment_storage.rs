//! Schema 8: versioned equipment profiles pinned by runs, and upgrade from schema 7.

use std::path::PathBuf;

use squib_domain::*;
use squib_storage::*;
use squib_training::manual::ManualString;

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("squib-equipstore-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn ev(id: &str, version: u32, name: &str) -> EquipmentVersion {
    EquipmentVersion {
        equipment_id: id.into(),
        version,
        name: name.into(),
        category: "pistol".into(),
        notes: String::new(),
        created_utc_ms: version as i64,
    }
}

fn cfg(equipment: Option<String>) -> RunConfig {
    RunConfig {
        schema_version: 1,
        source_mode: SourceMode::ManualEntry,
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
        drill_version_id: None,
        equipment_version_id: equipment,
        environment_snapshot_id: None,
        timestamp_mapping_method: TIMESTAMP_MAPPING_METHOD.into(),
        capture_video: false,
        diagnostic_recording: false,
        cue_channel: squib_domain::CueChannel::Sound,
        app_build: "test".into(),
    }
}

#[test]
fn versions_are_sequential_immutable_and_pinned_by_runs() {
    let mut repo = Repository::open_in_memory().unwrap();
    repo.ensure_default_shooter(1).unwrap();
    let s = repo.active_session("default", "s1", 1, 0).unwrap();
    repo.save_equipment_version(&ev("g", 1, "Carry pistol")).unwrap();
    assert!(repo.save_equipment_version(&ev("g", 3, "Skip")).is_err(), "no gaps");
    assert!(repo.save_equipment_version(&ev("g", 1, "Rewrite")).is_err(), "no rewriting");
    let mut bad = ev("h", 1, "  ");
    assert!(repo.save_equipment_version(&bad).is_err(), "name required");
    bad = ev("h", 1, "Rifle");
    bad.category = "cannon".into();
    assert!(repo.save_equipment_version(&bad).is_err(), "known categories only");

    let m = ManualString::parse("Club timer", 10_000_000, "1.5").unwrap();
    let intent = |id: &str, e: Option<String>| RunIntent {
        run_id: id.into(),
        session_id: s.clone(),
        created_utc_ms: 5,
        tz_offset_min: 0,
        config: cfg(e),
    };
    repo.insert_manual_run(&intent("r1", Some(equipment_ref("g", 1))), &m).unwrap();
    assert!(repo.insert_manual_run(&intent("r2", Some(equipment_ref("g", 9))), &m).is_err(), "unknown version refused");
    assert!(repo.insert_manual_run(&intent("r3", Some("nonsense".into())), &m).is_err());

    // Edit: version 2; the run still points at version 1.
    repo.save_equipment_version(&ev("g", 2, "Carry pistol (new optic)")).unwrap();
    assert_eq!(repo.list_equipment().unwrap()[0].version, 2);
    assert_eq!(repo.load_run("r1").unwrap().config.equipment_version_id.as_deref(), Some("g#1"));
    assert_eq!(repo.load_equipment_version("g", 1).unwrap().unwrap().name, "Carry pistol");
    assert!(
        repo.connection().execute("UPDATE equipment_version SET name = 'x' WHERE equipment_id = 'g'", []).is_err(),
        "versions are immutable in the database too"
    );
    let rec = repo.analytics_records().unwrap();
    assert_eq!(rec[0].equipment_id.as_deref(), Some("g"));

    // Archive hides it from selection but keeps history readable.
    repo.archive_equipment("g").unwrap();
    assert!(repo.list_equipment().unwrap().is_empty());
    assert!(repo.load_equipment_version("g", 1).unwrap().is_some());

    repo.delete_all_history().unwrap();
    assert!(repo.load_equipment_version("g", 1).unwrap().is_none(), "delete-all removes equipment");
}

#[test]
fn upgrade_from_schema_7() {
    let dir = tmpdir("upgrade8");
    let p = dir.join("journal.db");
    {
        let c = rusqlite::Connection::open(&p).unwrap();
        for (_, sql) in &MIGRATIONS[..7] {
            c.execute_batch(sql).unwrap();
        }
        c.pragma_update(None, "user_version", 7).unwrap();
    }
    let mut repo = Repository::open(&p).unwrap();
    assert_eq!(repo.schema_version().unwrap(), SCHEMA_VERSION);
    assert!(dir.join("journal.pre-v8.bak").exists());
    repo.save_equipment_version(&ev("a", 1, "Rimfire trainer")).unwrap();
    assert_eq!(repo.list_equipment().unwrap().len(), 1);
}
