//! Schema 7: sessions as range visits (idle-gap rollover, explicit end, notes, saved
//! cost, summaries, per-session deletion) and upgrade from schema 6.

use std::path::PathBuf;

use squib_domain::*;
use squib_storage::*;
use squib_training::manual::ManualString;

const H: i64 = 3_600_000;

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("squib-sessionstore-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn cfg() -> RunConfig {
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
        equipment_version_id: None,
        environment_snapshot_id: None,
        timestamp_mapping_method: TIMESTAMP_MAPPING_METHOD.into(),
        capture_video: false,
        diagnostic_recording: false,
        cue_channel: squib_domain::CueChannel::Sound,
        app_build: "test".into(),
    }
}

fn run(repo: &mut Repository, id: &str, at: i64) -> String {
    let s = repo.active_session("default", &format!("s-{id}"), at, 0).unwrap();
    let m = ManualString::parse("Club timer", 10_000_000, "1.5").unwrap();
    repo.insert_manual_run(
        &RunIntent { run_id: id.into(), session_id: s.clone(), created_utc_ms: at, tz_offset_min: 0, config: cfg() },
        &m,
    )
    .unwrap();
    s
}

#[test]
fn sessions_roll_over_after_idle_gap_and_end_on_request() {
    let mut repo = Repository::open_in_memory().unwrap();
    repo.ensure_default_shooter(1).unwrap();
    repo.set_setting(SETTING_ROUND_COST, r#"{"per_round_minor":30,"currency":"USD"}"#).unwrap();
    let a = run(&mut repo, "r1", 10 * H);
    assert_eq!(run(&mut repo, "r2", 13 * H), a, "within the idle gap: same visit");
    let b = run(&mut repo, "r3", 13 * H + SESSION_IDLE_MS + 1);
    assert_ne!(a, b, "a long gap starts a new visit");
    // The earlier session ended at its last run, and saved the cost from when it began.
    let all = repo.session_summaries(10).unwrap();
    let first = all.iter().find(|s| s.id == a).unwrap();
    assert_eq!((first.active, first.ended_utc_ms, first.run_count), (false, Some(13 * H), 2));
    assert!(first.cost_json.as_deref().unwrap().contains("\"per_round_minor\":30"));
    assert_eq!(all[0].id, b, "newest first");

    // Price change applies to new sessions only.
    repo.set_setting(SETTING_ROUND_COST, r#"{"per_round_minor":45,"currency":"USD"}"#).unwrap();
    repo.end_session(&b, 20 * H).unwrap();
    let c = run(&mut repo, "r4", 20 * H + 1);
    assert_ne!(b, c, "an ended session is not reused");
    let all = repo.session_summaries(10).unwrap();
    assert!(all.iter().find(|s| s.id == c).unwrap().cost_json.as_deref().unwrap().contains("45"));
    assert!(all.iter().find(|s| s.id == a).unwrap().cost_json.as_deref().unwrap().contains("30"));

    repo.set_session_notes(&a, "Indoor range, bay 4").unwrap();
    assert_eq!(repo.session_summaries(10).unwrap().iter().find(|s| s.id == a).unwrap().notes, "Indoor range, bay 4");
    assert!(repo.set_session_notes(&a, &"x".repeat(MAX_SESSION_NOTES + 1)).is_err());
    assert_eq!(repo.session_run_ids(&a).unwrap(), vec!["r2".to_string(), "r1".to_string()]);
    assert!(repo.end_session("nope", 1).is_err());

    // Per-session deletion removes that visit only.
    let d = repo.delete_session(&a).unwrap();
    assert_eq!(d.runs, 2);
    assert!(repo.session_summaries(10).unwrap().iter().all(|s| s.id != a));
    assert!(repo.load_run("r3").is_ok());
}

#[test]
fn upgrade_from_schema_6_keeps_sessions_open_as_before() {
    let dir = tmpdir("upgrade7");
    let p = dir.join("journal.db");
    {
        let c = rusqlite::Connection::open(&p).unwrap();
        for (_, sql) in &MIGRATIONS[..6] {
            c.execute_batch(sql).unwrap();
        }
        c.pragma_update(None, "user_version", 6).unwrap();
        c.execute_batch(
            "INSERT INTO shooter_profile(id, name, created_utc_ms) VALUES ('default', 'Me', 1);
             INSERT INTO session(id, shooter_id, started_utc_ms, tz_offset_min, notes, active) VALUES ('s1', 'default', 5, 0, 'old', 1);",
        )
        .unwrap();
    }
    let repo = Repository::open(&p).unwrap();
    assert_eq!(repo.schema_version().unwrap(), SCHEMA_VERSION);
    let s = &repo.session_summaries(5).unwrap()[0];
    assert_eq!((s.id.as_str(), s.notes.as_str(), s.active, s.ended_utc_ms, s.cost_json.clone()), ("s1", "old", true, None, None));
    // An old open session with no runs is idle-ended by the next run far in the future.
    assert_ne!(repo.active_session("default", "s2", 5 + SESSION_IDLE_MS + 10, 0).unwrap(), "s1");
}
