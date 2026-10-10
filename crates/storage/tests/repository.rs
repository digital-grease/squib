//! Repository behaviour: transactions, idempotence, immutability, recovery from a real
//! process abort, disk-full, and schema guards (docs/squib/09 "Repository").

use std::path::PathBuf;

use squib_domain::*;
use squib_storage::*;
use squib_timing::envelope::EnvelopeChunk;

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("squib-storage-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn live_config() -> RunConfig {
    RunConfig {
        schema_version: 1,
        source_mode: SourceMode::PhoneLive,
        delay_policy: DelayPolicy::Random { min_ms: 1000, max_ms: 3000 },
        selected_delay_ms: 1800,
        pars_ms: vec![],
        stop_policy: StopPolicy::Manual,
        expected_count: Some(2),
        start_cue_template: CueKind::Start.template_version().into(),
        par_cue_template: CueKind::Par.template_version().into(),
        detector: Some(DetectorConfig::default()),
        calibration_profile_id: None,
        route_signature: Some("sig".into()),
        shooter_id: "default".into(),
        drill_version_id: None,
        equipment_version_id: None,
        environment_snapshot_id: None,
        timestamp_mapping_method: TIMESTAMP_MAPPING_METHOD.into(),
        capture_video: false,
        app_build: "test".into(),
    }
}

fn setup(repo: &mut Repository, run_id: &str) {
    repo.ensure_default_shooter(1).unwrap();
    let s = repo.active_session("default", "sess1", 1, 0).unwrap();
    repo.insert_run_intent(&RunIntent {
        run_id: run_id.into(),
        session_id: s,
        created_utc_ms: 10,
        tz_offset_min: -300,
        config: live_config(),
    })
    .unwrap();
}

fn epoch(run_id: &str) -> EpochRecord {
    EpochRecord {
        epoch_id: "ep1".into(),
        run_id: run_id.into(),
        sample_rate_hz: 48_000,
        channels: 1,
        sample_format: "pcm16".into(),
        route: RouteInfo {
            input_device: "builtin_mic".into(),
            output_device: "builtin_speaker".into(),
            audio_source: "unprocessed".into(),
            unprocessed_supported: true,
            effects: vec![],
            os_build: "test".into(),
            device_model: "desktop".into(),
            camera_recording: false,
        },
        clock_domain: "CLOCK_MONOTONIC".into(),
        started_mono_ns: Some(1),
    }
}

fn cand(seq: u64, onset: i64) -> Candidate {
    Candidate {
        sequence: seq,
        onset_frame: onset,
        peak_frame: onset + 20,
        features: CandidateFeatures {
            peak_dbfs: -2.0,
            floor_dbfs: -60.0,
            floor_ratio_db: 45.0,
            attack_db: 30.0,
            rise_frames: 20,
            decay_db: 15.0,
            clipped_samples: 0,
        },
        suggestion: DetectorSuggestion::SuggestedAccepted,
        reasons: vec![],
        detector_score: 0.9,
        algorithm_version: DETECTOR_ALGORITHM_VERSION.into(),
        config_hash: "h".into(),
    }
}

fn start_cue() -> CueObservation {
    CueObservation {
        cue_id: 0,
        kind: CueKind::Start,
        par_index: None,
        template_version: CueKind::Start.template_version().into(),
        requested_mono_ns: 100,
        issued_mono_ns: 101,
        render_mono_ns: Some(120),
        epoch_id: Some("ep1".into()),
        acoustic_onset_frame: Some(48_000),
        acoustic_onset_mono_ns: Some(130),
        match_ncc: Some(0.93),
        span_start_frame: Some(48_000),
        span_end_frame: Some(55_200),
        span_exact: Some(true),
        missed: false,
        heard: Some(true),
    }
}

fn first_batch(run_id: &str) -> ObservationBatch {
    ObservationBatch {
        batch_seq: 1,
        epoch: Some(epoch(run_id)),
        progress: Some(RunProgress {
            start_method: Some(StartReferenceMethod::AcousticCue),
            start_ref_frame: Some(48_000),
            start_ref_mono_ns: Some(130),
            armed_mono_ns: Some(50),
        }),
        cues: vec![start_cue()],
        candidates: vec![("ep1".into(), cand(0, 30_000)), ("ep1".into(), cand(1, 80_000))],
        quality: vec![(0, Some("ep1".into()), QualityEvent::new(QualityKind::ClippedInput, Severity::Warning, "x"))],
        envelope: vec![("ep1".into(), EnvelopeChunk { first_frame: 0, hop_frames: 480, data: vec![10, 20, 30, 40] })],
    }
}

#[test]
fn intent_batches_finalize_and_revisions_round_trip() {
    let mut repo = Repository::open_in_memory().unwrap();
    assert_eq!(repo.schema_version().unwrap(), SCHEMA_VERSION);
    setup(&mut repo, "r1");
    let row = repo.load_run("r1").unwrap().row;
    assert_eq!(row.outcome, None);
    assert_eq!(row.persist_state, PersistState::Pending);

    assert_eq!(repo.append_batch("r1", &first_batch("r1")).unwrap(), 1);
    // Identical retry is idempotent.
    assert_eq!(repo.append_batch("r1", &first_batch("r1")).unwrap(), 1);
    // Divergent content for an existing candidate is a conflict, not an overwrite.
    let mut bad = first_batch("r1");
    bad.batch_seq = 2;
    bad.candidates[0].1.features.peak_dbfs = -9.0;
    assert!(matches!(repo.append_batch("r1", &bad), Err(StorageError::Conflict(_))));

    let d = repo.load_run("r1").unwrap();
    assert_eq!(d.candidates.len(), 2);
    // Pre-cue candidate (30_000 < 48_000) is a rejected diagnostic.
    assert_eq!(d.classified[0].classification, DetectorSuggestion::Rejected);
    let rev1 = initial_revision("r1", true, &d.classified, 20);
    repo.finalize_run(&FinalRecord {
        run_id: "r1".into(),
        outcome: Outcome::Complete,
        remaining: ObservationBatch { batch_seq: 2, ..Default::default() },
        stop_mono_ns: Some(5_000),
        stop_frame: Some(200_000),
        interrupt_reason: None,
        epoch_summary: Some(("ep1".into(), "{}".into())),
        initial_revision: Some(rev1.clone()),
        finalized_utc_ms: 30,
        error: None,
    })
    .unwrap();
    let d = repo.load_run("r1").unwrap();
    assert_eq!(d.row.outcome, Some(Outcome::Complete));
    assert_eq!(d.row.persist_state, PersistState::Saved);
    assert_eq!(d.row.last_durable_seq, 2);
    assert_eq!(d.revisions, vec![rev1.clone()]);
    assert_eq!(d.envelope.len(), 1);

    // Finalizing twice is refused.
    let again = FinalRecord {
        run_id: "r1".into(),
        outcome: Outcome::Interrupted,
        remaining: Default::default(),
        stop_mono_ns: None,
        stop_frame: None,
        interrupt_reason: None,
        epoch_summary: None,
        initial_revision: None,
        finalized_utc_ms: 31,
        error: None,
    };
    assert!(matches!(repo.finalize_run(&again), Err(StorageError::Conflict(_))));

    // A correction appends revision 2; originals are untouched.
    let rev2 = apply_revision(
        &rev1,
        &d.classified,
        vec![RevisionAction::AddManual { id: "m1".into(), timeline_ns: 900_000_000 }],
        "local",
        40,
        None,
    )
    .unwrap();
    assert_eq!(repo.append_revision(&rev2).unwrap(), ReviewState::Reviewed);
    // A stale edit based on revision 1 conflicts instead of overwriting revision 2.
    let stale = apply_revision(&rev1, &d.classified, vec![RevisionAction::Note { text: "x".into() }], "other", 41, None).unwrap();
    assert!(matches!(repo.append_revision(&stale), Err(StorageError::Conflict(_))));

    let summary = &repo.list_runs(10).unwrap()[0];
    assert_eq!(summary.latest_revision, Some(2));
    assert!(summary.edited);
    assert_eq!(summary.accepted_count, Some(2));
    assert_eq!(repo.load_run("r1").unwrap().candidates, d.candidates);
}

#[test]
fn originals_are_append_only_at_the_database_level() {
    let mut repo = Repository::open_in_memory().unwrap();
    setup(&mut repo, "r1");
    repo.append_batch("r1", &first_batch("r1")).unwrap();
    // Bypass the API: triggers must refuse mutation.
    let dir = tmpdir("immut");
    let p = dir.join("j.db");
    let mut file_repo = Repository::open(&p).unwrap();
    setup(&mut file_repo, "r1");
    file_repo.append_batch("r1", &first_batch("r1")).unwrap();
    drop(file_repo);
    let raw = rusqlite::Connection::open(&p).unwrap();
    let e = raw.execute("UPDATE detected_candidate SET onset_frame = 1", []).unwrap_err();
    assert!(e.to_string().contains("immutable"), "{e}");
    let e = raw.execute("DELETE FROM quality_event", []).unwrap_err();
    assert!(e.to_string().contains("immutable"), "{e}");
    let e = raw.execute("UPDATE detected_candidate SET score_is_probability = 1", []).unwrap_err();
    assert!(e.to_string().contains("immutable"));
}

#[test]
fn disk_full_is_reported_distinctly_and_rolls_back() {
    let mut repo = Repository::open_in_memory().unwrap();
    setup(&mut repo, "r1");
    let used: u32 = 64;
    repo.set_max_page_count(used).unwrap();
    let mut big = first_batch("r1");
    big.envelope = (0..400)
        .map(|i| ("ep1".to_string(), EnvelopeChunk { first_frame: i * 48_000, hop_frames: 480, data: vec![7u8; 4096] }))
        .collect();
    let err = repo.append_batch("r1", &big).unwrap_err();
    assert_eq!(err, StorageError::DiskFull);
    let d = repo.load_run("r1").unwrap();
    assert!(d.candidates.is_empty(), "failed batch left no partial rows");
    assert_eq!(d.row.last_durable_seq, 0);
    assert_eq!(d.row.outcome, None, "storage failure never claims a saved run");
}

#[test]
fn newer_schema_is_rejected_and_file_kept() {
    let dir = tmpdir("newer");
    let p = dir.join("j.db");
    drop(Repository::open(&p).unwrap());
    let c = rusqlite::Connection::open(&p).unwrap();
    c.pragma_update(None, "user_version", 99).unwrap();
    drop(c);
    match Repository::open(&p) {
        Err(StorageError::NewerSchema { found: 99, supported }) => assert_eq!(supported, SCHEMA_VERSION),
        Err(e) => panic!("unexpected error {e:?}"),
        Ok(_) => panic!("opened a newer schema"),
    }
    assert!(p.exists());
}

#[test]
fn failed_migration_keeps_original_data() {
    // Simulate a pre-existing v0 database whose content collides with migration 1.
    let dir = tmpdir("migfail");
    let p = dir.join("j.db");
    let c = rusqlite::Connection::open(&p).unwrap();
    c.execute_batch("CREATE TABLE run(precious TEXT); INSERT INTO run VALUES ('keep me');").unwrap();
    drop(c);
    let err = Repository::open(&p).err().expect("migration must fail");
    assert!(matches!(err, StorageError::Migration { version: 1, .. }), "{err:?}");
    let c = rusqlite::Connection::open(&p).unwrap();
    let v: String = c.query_row("SELECT precious FROM run", [], |r| r.get(0)).unwrap();
    assert_eq!(v, "keep me");
    let uv: u32 = c.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
    assert_eq!(uv, 0);
}

#[test]
fn calibration_is_scoped_to_exact_route_signature() {
    let repo = Repository::open_in_memory().unwrap();
    let rec = CalibrationRecord {
        id: "c1".into(),
        route_signature: "phone|mic|spk|unprocessed|48000Hz|os1".into(),
        label: Some("Indoor".into()),
        algorithm_version: "ambient-impulse-v1".into(),
        detector_config: DetectorConfig { threshold_db: 20.0, ..Default::default() },
        suggestion_json: "{}".into(),
        verdict: "separated".into(),
        impulse_count: 3,
        os_build: Some("os1".into()),
        created_utc_ms: 5,
    };
    repo.insert_calibration(&rec).unwrap();
    assert_eq!(repo.latest_calibration(&rec.route_signature).unwrap().unwrap().detector_config.threshold_db, 20.0);
    assert!(repo.latest_calibration("phone|mic|spk|unprocessed|44100Hz|os1").unwrap().is_none());
}

const CHILD_ENV: &str = "SQUIB_STORAGE_ABORT_CHILD";

/// Child half of the process-termination test: writes intent + one durable batch,
/// starts but never commits a second, then aborts the process.
#[test]
fn abort_child() {
    let Ok(path) = std::env::var(CHILD_ENV) else { return };
    let mut repo = Repository::open(std::path::Path::new(&path)).unwrap();
    setup(&mut repo, "r1");
    repo.append_batch("r1", &first_batch("r1")).unwrap();
    // Uncommitted work in an open transaction dies with the process.
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute_batch(
        "BEGIN; INSERT INTO quality_event(run_id, seq, kind, severity, detail, batch_seq) VALUES ('r1', 99, 'capture_gap', 'integrity', 'lost', 2);",
    )
    .unwrap();
    std::process::abort();
}

#[test]
fn a08_process_abort_recovers_only_durable_records_as_interrupted() {
    let dir = tmpdir("abort");
    let p = dir.join("j.db");
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["abort_child", "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, &p)
        .status()
        .unwrap();
    assert!(!status.success(), "child must die abnormally");

    let mut repo = Repository::open(&p).unwrap();
    repo.quick_check().unwrap();
    let recovered = repo.recover_unfinished(1_000).unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].last_durable_seq, 1);
    assert_eq!(recovered[0].committed_candidates, 2);
    let d = repo.load_run("r1").unwrap();
    assert_eq!(d.row.outcome, Some(Outcome::Interrupted), "never Complete after termination");
    assert_eq!(d.row.persist_state, PersistState::Saved);
    assert!(d.row.uncommitted_tail_possible);
    assert!(d.quality.iter().any(|q| q.kind == QualityKind::ProcessTerminated));
    assert!(!d.quality.iter().any(|q| q.detail == "lost"), "uncommitted rows are not resurrected");
    assert_eq!(d.revisions.len(), 1, "revision 1 built from committed candidates only");
    // Recovery is idempotent.
    assert!(repo.recover_unfinished(2_000).unwrap().is_empty());
}
