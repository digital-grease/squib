//! Schema 2: upgrade from schema 1 with data, snapshot immutability and redaction,
//! run pinning, overrides, places, cache separability.

use std::path::PathBuf;

use squib_domain::*;
use squib_environment::nws::CachedDoc;
use squib_environment::privacy::LocationRetention;
use squib_environment::*;
use squib_storage::*;

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("squib-envstore-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn config(snapshot: Option<String>) -> RunConfig {
    RunConfig {
        schema_version: 1,
        source_mode: SourceMode::ParOnly,
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
        equipment_version_id: None,
        environment_snapshot_id: snapshot,
        timestamp_mapping_method: TIMESTAMP_MAPPING_METHOD.into(),
        capture_video: false,
        diagnostic_recording: false,
        app_build: "test".into(),
    }
}

fn snapshot(id: &str) -> EnvironmentSnapshot {
    let mut c = local::location_heights(None, None, Some(1621.0), Some(3.0), 1000);
    c.push(local::barometer(835.0, 1000).unwrap());
    let place = ReferencePlace {
        lat: 40.02,
        lon: -105.27,
        label: Some("Secret range".into()),
        source: "gps".into(),
        elevation_msl_m: None,
    };
    resolve(id, &c, &[], Some(place), vec![], &ResolverPolicy::default(), 1000)
}

#[test]
fn upgrade_from_schema_1_keeps_runs_and_backs_up() {
    let dir = tmpdir("upgrade");
    let p = dir.join("journal.db");
    {
        // Build a genuine schema-1 database containing a run.
        let c = rusqlite::Connection::open(&p).unwrap();
        c.execute_batch(MIGRATIONS[0].1).unwrap();
        c.pragma_update(None, "user_version", 1).unwrap();
        c.execute_batch(
            "INSERT INTO shooter_profile(id, name, created_utc_ms) VALUES ('default', 'Me', 1);
             INSERT INTO session(id, shooter_id, started_utc_ms, tz_offset_min, active) VALUES ('s1', 'default', 1, 0, 1);",
        )
        .unwrap();
        let cfg = serde_json::to_string(&config(None)).unwrap();
        c.execute(
            "INSERT INTO run(id, session_id, shooter_id, created_utc_ms, tz_offset_min, source_mode, config_json, config_hash,
               app_build, domain_schema_version, persist_state, outcome) VALUES ('r1','s1','default',5,0,'par_only',?1,'h','t',1,'saved','complete')",
            [cfg],
        )
        .unwrap();
    }
    let repo = Repository::open(&p).unwrap();
    assert_eq!(repo.schema_version().unwrap(), SCHEMA_VERSION);
    assert!(dir.join("journal.pre-v2.bak").exists(), "pre-migration backup written");
    let d = repo.load_run("r1").unwrap();
    assert_eq!(d.row.outcome, Some(Outcome::Complete));
    assert!(d.environment.is_none());
    // The backup is a complete schema-1 database.
    let b = rusqlite::Connection::open(dir.join("journal.pre-v2.bak")).unwrap();
    let v: u32 = b.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
    assert_eq!(v, 1);
}

#[test]
fn snapshots_are_redacted_at_write_immutable_and_pinned_to_runs() {
    let mut repo = Repository::open_in_memory().unwrap();
    let stored = repo.insert_snapshot(&snapshot("e1"), LocationRetention::Redacted).unwrap();
    assert!(stored.redacted);
    let loaded = repo.load_snapshot("e1").unwrap().unwrap();
    let json = serde_json::to_string(&loaded).unwrap();
    assert!(!json.contains("Secret range") && !json.contains("40.02") && !json.contains("1621"));
    assert_eq!(loaded.field(Field::LocalPressure).unwrap().chosen.as_ref().unwrap().value, Value::Si(83_500.0));

    let precise = repo.insert_snapshot(&snapshot("e2"), LocationRetention::Precise).unwrap();
    assert!(!precise.redacted && serde_json::to_string(&precise).unwrap().contains("Secret range"));

    repo.ensure_default_shooter(1).unwrap();
    let s = repo.active_session("default", "s1", 1, 0).unwrap();
    repo.insert_run_intent(&RunIntent {
        run_id: "r1".into(),
        session_id: s.clone(),
        created_utc_ms: 2,
        tz_offset_min: 0,
        config: config(Some("e1".into())),
    })
    .unwrap();
    assert_eq!(repo.load_run("r1").unwrap().environment.unwrap().id, "e1");
    // A run cannot reference a snapshot that does not exist.
    let err = repo
        .insert_run_intent(&RunIntent {
            run_id: "r2".into(),
            session_id: s,
            created_utc_ms: 3,
            tz_offset_min: 0,
            config: config(Some("nope".into())),
        })
        .unwrap_err();
    assert!(matches!(err, StorageError::Conflict(_)), "{err:?}");

    // Snapshots cannot be edited or deleted.
    let dir = tmpdir("immut");
    let path = dir.join("j.db");
    let r2 = Repository::open(&path).unwrap();
    r2.insert_snapshot(&snapshot("e9"), LocationRetention::Redacted).unwrap();
    drop(r2);
    let c = rusqlite::Connection::open(&path).unwrap();
    assert!(c.execute("UPDATE environment_snapshot SET content_json = '{}'", []).unwrap_err().to_string().contains("immutable"));
    assert!(c.execute("DELETE FROM environment_snapshot", []).unwrap_err().to_string().contains("immutable"));
}

#[test]
fn overrides_places_settings_and_separable_cache() {
    let repo = Repository::open_in_memory().unwrap();
    let o = Override {
        candidate: local::manual(Field::WindSpeed, 4.0, 9.0, "mph", 10).unwrap(),
        set_utc_ms: 10,
        expires_utc_ms: None,
    };
    repo.set_override(&o).unwrap();
    repo.set_override(&Override { set_utc_ms: 11, ..o.clone() }).unwrap();
    assert_eq!(repo.list_overrides().unwrap().len(), 1, "one override per field");
    repo.clear_override(Field::WindSpeed).unwrap();
    assert!(repo.list_overrides().unwrap().is_empty());

    repo.add_place(&SavedPlace { id: "p1".into(), label: "Club range".into(), lat: 40.0, lon: -105.0, created_utc_ms: 1 })
        .unwrap();
    assert!(
        repo.add_place(&SavedPlace { id: "p2".into(), label: "Bad".into(), lat: 95.0, lon: 0.0, created_utc_ms: 1 }).is_err()
    );
    assert!(repo.add_place(&SavedPlace { id: "p3".into(), label: " ".into(), lat: 1.0, lon: 0.0, created_utc_ms: 1 }).is_err());
    assert_eq!(repo.list_places().unwrap().len(), 1);
    repo.delete_place("p1").unwrap();

    assert_eq!(repo.get_setting(SETTING_WEATHER_ENABLED).unwrap(), None, "weather lookup off until enabled");
    repo.set_setting(SETTING_WEATHER_ENABLED, "true").unwrap();
    assert_eq!(repo.get_setting(SETTING_WEATHER_ENABLED).unwrap().as_deref(), Some("true"));

    let doc =
        CachedDoc { key: "obs:KBDU".into(), body: b"{}".to_vec(), etag: Some("e".into()), fetched_utc_ms: 1, expires_utc_ms: 2 };
    repo.cache_put("nws", &doc).unwrap();
    assert_eq!(repo.cache_get("nws", "obs:KBDU").unwrap(), Some(doc));
    repo.insert_snapshot(&snapshot("keep"), LocationRetention::Redacted).unwrap();
    assert_eq!(repo.cache_clear().unwrap(), 1);
    assert!(repo.load_snapshot("keep").unwrap().is_some(), "clearing cache never touches snapshots");
}
