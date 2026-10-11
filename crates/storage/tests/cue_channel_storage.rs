//! Plan phase 2.3: schema 10 rebuilds the run table so it accepts the vibration and
//! screen start methods, keeping every row and every reference to it.

use squib_storage::*;

#[test]
fn upgrade_from_schema_9_rebuilds_run_and_keeps_references() {
    let dir = std::env::temp_dir().join(format!("squib-store-cue10-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("journal.db");
    {
        let c = rusqlite::Connection::open(&p).unwrap();
        c.pragma_update(None, "foreign_keys", "ON").unwrap();
        for (_, sql) in &MIGRATIONS[..9] {
            c.execute_batch(sql).unwrap();
        }
        c.pragma_update(None, "user_version", 9).unwrap();
        c.execute_batch(
            "INSERT INTO shooter_profile (id, name, created_utc_ms) VALUES ('s1', 'Me', 1);
             INSERT INTO session (id, shooter_id, started_utc_ms, tz_offset_min, active) VALUES ('v1', 's1', 1, 0, 1);
             INSERT INTO run (id, session_id, shooter_id, created_utc_ms, tz_offset_min, source_mode, config_json,
                              config_hash, app_build, domain_schema_version, outcome, persist_state, start_method,
                              start_ref_mono_ns, drill_version, equipment_version)
                 VALUES ('r1', 'v1', 's1', 5, 0, 'par_only', '{}', 'h', 't', 1, 'complete', 'saved', 'scheduled_render',
                         42, 3, 2);
             INSERT INTO cue_observation (run_id, cue_id, kind, template_version, requested_mono_ns, issued_mono_ns, missed)
                 VALUES ('r1', 0, 'start', 'start-chirp-v1', 40, 41, 0);",
        )
        .unwrap();
    }
    let mut repo = Repository::open(&p).unwrap();
    assert_eq!(repo.schema_version().unwrap(), SCHEMA_VERSION);
    assert!(dir.join("journal.pre-v10.bak").exists());
    let c = repo.connection();
    let fk: i64 = c.query_row("PRAGMA foreign_keys", [], |r| r.get(0)).unwrap();
    assert_eq!(fk, 1, "foreign keys are back on after the rebuild");
    let row: (String, i64, i64, i64) = c
        .query_row("SELECT start_method, start_ref_mono_ns, drill_version, equipment_version FROM run WHERE id = 'r1'", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .unwrap();
    assert_eq!(row, ("scheduled_render".into(), 42, 3, 2), "every column survives, including later additions");
    let indexes: i64 = c
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'index' AND tbl_name = 'run' AND name LIKE 'run_%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(indexes, 3);
    // References to run still hold: a cue for a run that does not exist is refused.
    assert!(
        c.execute(
            "INSERT INTO cue_observation (run_id, cue_id, kind, template_version, requested_mono_ns, issued_mono_ns, missed)
             VALUES ('nope', 0, 'start', 'x', 1, 1, 0)",
            [],
        )
        .is_err()
    );
    for m in ["vibration_request", "screen_frame"] {
        c.execute("UPDATE run SET start_method = ?1 WHERE id = 'r1'", [m]).unwrap();
    }
    assert!(c.execute("UPDATE run SET start_method = 'guess' WHERE id = 'r1'", []).is_err());
}
