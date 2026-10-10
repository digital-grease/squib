//! A17 (backup/restore, hostile and conflicting imports) and A19 (redacted sharing).

use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use squib_archive::*;
use squib_domain::*;
use squib_environment::privacy::LocationRetention;
use squib_environment::{Field, MeasurementCandidate, SourceKind, StationRef, Value, local, resolve};
use squib_storage::*;
use squib_timing::envelope::EnvelopeChunk;
use squib_training::drill::{DrillVersion, starter_recipes};
use squib_training::manual::ManualString;
use squib_training::rounds::RoundCount;
use squib_training::scoring::{ScoreEntry, ScoreRevision};

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("squib-archive-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn cfg(mode: SourceMode, drill: Option<String>, snapshot: Option<String>) -> RunConfig {
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
        detector: (mode == SourceMode::PhoneLive).then(DetectorConfig::default),
        calibration_profile_id: None,
        route_signature: None,
        shooter_id: "default".into(),
        drill_version_id: drill,
        equipment_version_id: None,
        environment_snapshot_id: snapshot,
        timestamp_mapping_method: TIMESTAMP_MAPPING_METHOD.into(),
        capture_video: false,
        app_build: "test".into(),
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

fn station_snapshot(id: &str) -> squib_environment::EnvironmentSnapshot {
    let t = MeasurementCandidate {
        id: "nws:KXYZ:temperature:1".into(),
        field: Field::Temperature,
        value: Value::Si(290.0),
        original_value: Some(16.85),
        original_unit: Some("wmoUnit:degC".into()),
        source: SourceKind::StationObservation,
        provider: "nws".into(),
        station: Some(StationRef {
            station_id: "KXYZ".into(),
            name: "Secret Field".into(),
            lat: 40.1,
            lon: -105.2,
            elevation_m: Some(1600.0),
            distance_m: Some(3000.0),
        }),
        observed_utc_ms: Some(1_000),
        fetched_utc_ms: 2_000,
        qc: Some("V".into()),
        datum: None,
        accuracy: None,
        accuracy_meaning: None,
        transforms: vec![],
        adapter_version: "nws-adapter-v1".into(),
    };
    let mut c = vec![t];
    c.push(local::barometer(835.0, 2_000).unwrap());
    resolve(id, &c, &[], None, vec![], &squib_environment::ResolverPolicy::default(), 2_500)
}

/// A journal exercising every exported table.
fn build(path: &Path, att_root: &Path) -> Repository {
    let mut repo = Repository::open(path).unwrap();
    repo.ensure_default_shooter(1).unwrap();
    let s = repo.active_session("default", "sess-1", 1, -300).unwrap();
    let snap = repo.insert_snapshot(&station_snapshot("env-1"), LocationRetention::Precise).unwrap();
    let mut recipe = starter_recipes().remove(2);
    recipe.title = "=HYPERLINK(\"evil\")".into();
    let dv = DrillVersion::first("drill-1", recipe, 5).unwrap();
    repo.save_drill_version(&dv).unwrap();
    // A live-style run with detections, a correction revision, and conditions.
    repo.insert_run_intent(&RunIntent {
        run_id: "live-1".into(),
        session_id: s.clone(),
        created_utc_ms: 10,
        tz_offset_min: -300,
        config: cfg(SourceMode::PhoneLive, Some(drill_ref("drill-1", 1)), Some(snap.id.clone())),
    })
    .unwrap();
    let route = RouteInfo {
        input_device: "builtin_mic".into(),
        output_device: "builtin_speaker".into(),
        audio_source: "unprocessed".into(),
        unprocessed_supported: true,
        effects: vec![],
        os_build: "t".into(),
        device_model: "t".into(),
        camera_recording: false,
    };
    let batch = ObservationBatch {
        batch_seq: 1,
        epoch: Some(EpochRecord {
            epoch_id: "ep-1".into(),
            run_id: "live-1".into(),
            sample_rate_hz: 48_000,
            channels: 1,
            sample_format: "pcm16".into(),
            route,
            clock_domain: "CLOCK_MONOTONIC".into(),
            started_mono_ns: Some(9_007_199_254_740_993), // above 2^53: must survive exactly
        }),
        progress: Some(RunProgress {
            start_method: Some(StartReferenceMethod::AcousticCue),
            start_ref_frame: Some(48_000),
            start_ref_mono_ns: Some(1),
            armed_mono_ns: Some(1),
        }),
        cues: vec![],
        candidates: vec![("ep-1".into(), cand(0, 80_000)), ("ep-1".into(), cand(1, 100_000))],
        quality: vec![(0, Some("ep-1".into()), QualityEvent::new(QualityKind::ClippedInput, Severity::Warning, "x"))],
        envelope: vec![("ep-1".into(), EnvelopeChunk { first_frame: 0, hop_frames: 480, data: vec![1, 2, 3, 4] })],
    };
    repo.append_batch("live-1", &batch).unwrap();
    let d = repo.load_run("live-1").unwrap();
    let rev1 = initial_revision("live-1", true, &d.classified, 11);
    repo.finalize_run(&FinalRecord {
        run_id: "live-1".into(),
        outcome: Outcome::Complete,
        remaining: ObservationBatch { batch_seq: 2, ..Default::default() },
        stop_mono_ns: Some(5),
        stop_frame: Some(500_000),
        interrupt_reason: None,
        epoch_summary: None,
        initial_revision: Some(rev1.clone()),
        finalized_utc_ms: 12,
        error: None,
    })
    .unwrap();
    let rev2 = apply_revision(
        &rev1,
        &d.classified,
        vec![RevisionAction::AddManual { id: "m1".into(), timeline_ns: 2_000_000_000 }],
        "local",
        13,
        None,
    )
    .unwrap();
    repo.append_revision(&rev2).unwrap();
    let entry = ScoreEntry {
        profile_id: "generic-points-hf".into(),
        profile_version: 1,
        counts: [("A".to_string(), 3u32)].into_iter().collect(),
        complete: true,
        manual_elapsed_ns: None,
        manual_precision_ns: None,
        manual_points: None,
        notes: "private note".into(),
    };
    repo.append_score_revision(&ScoreRevision::new("live-1", None, entry, "local", 14).unwrap()).unwrap();
    repo.set_round_count(&RoundCount { run_id: "live-1".into(), proposed: 2, confirmed: Some(3), confirmed_utc_ms: Some(15) })
        .unwrap();
    // A manual run.
    let m = ManualString::parse("Club timer", 10_000_000, "1.20 1.70").unwrap();
    repo.insert_manual_run(
        &RunIntent {
            run_id: "manual-1".into(),
            session_id: s,
            created_utc_ms: 20,
            tz_offset_min: -300,
            config: cfg(SourceMode::ManualEntry, None, None),
        },
        &m,
    )
    .unwrap();
    // An attachment file owned by the app.
    let photo = b"\xff\xd8fake-jpeg-without-exif";
    std::fs::create_dir_all(att_root.join("photos")).unwrap();
    std::fs::write(att_root.join("photos/a1.jpg"), photo).unwrap();
    repo.insert_attachment(&AttachmentRecord {
        id: "att-1".into(),
        run_id: Some("live-1".into()),
        kind: "photo".into(),
        relative_path: "photos/a1.jpg".into(),
        sha256: Sha256::digest(photo).iter().map(|b| format!("{b:02x}")).collect(),
        bytes: photo.len() as i64,
        mime: "image/jpeg".into(),
        metadata_stripped: true,
        created_utc_ms: 16,
    })
    .unwrap();
    repo.add_place(&SavedPlace { id: "pl-1".into(), label: "Club range".into(), lat: 40.0, lon: -105.0, created_utc_ms: 1 })
        .unwrap();
    // A day plan whose drill item produced the manual run.
    repo.create_plan(
        &DayPlan {
            id: "plan-1".into(),
            title: "Club match".into(),
            date_local: "2026-10-10".into(),
            kind: "match".into(),
            notes: "Squad 3".into(),
            created_utc_ms: 2,
        },
        true,
    )
    .unwrap();
    repo.add_plan_item(&PlanItem {
        id: "item-1".into(),
        plan_id: "plan-1".into(),
        ordinal: 0,
        kind: "stage".into(),
        title: "Stage 1".into(),
        time_local: Some("08:45".into()),
        drill_id: None,
        drill_version: None,
        target_strings: None,
        notes: "Low ready, 12 rounds".into(),
        skipped: false,
    })
    .unwrap();
    repo.link_run("item-1", "manual-1").unwrap();
    // A video clip with its clock observations.
    let clip = b"fake-mp4";
    std::fs::create_dir_all(att_root.join("videos")).unwrap();
    std::fs::write(att_root.join("videos/v1.mp4"), clip).unwrap();
    repo.insert_video_clip(&VideoClipRecord {
        attachment: AttachmentRecord {
            id: "vid-1".into(),
            run_id: Some("live-1".into()),
            kind: "video".into(),
            relative_path: "videos/v1.mp4".into(),
            sha256: Sha256::digest(clip).iter().map(|b| format!("{b:02x}")).collect(),
            bytes: clip.len() as i64,
            mime: "video/mp4".into(),
            metadata_stripped: true,
            created_utc_ms: 17,
        },
        run_id: "live-1".into(),
        width: 1280,
        height: 720,
        frame_rate_milli: 29_970,
        duration_ms: 4_000,
        first_frame_camera_ns: 9_007_199_254_740_000,
        camera_clock: "realtime".into(),
        probe_camera_ns: 9_007_199_254_740_000,
        probe_mono_ns: 9_007_199_254_740_993,
        probe_boot_ns: 9_007_199_254_741_001,
        start_boot_minus_mono_ns: 8,
        end_boot_minus_mono_ns: 8,
        created_utc_ms: 17,
    })
    .unwrap();
    repo
}

fn export(repo: &mut Repository, out: &Path, att: Option<&Path>) -> ExportSummary {
    export_private(
        repo,
        out,
        &ExportOptions { app_version: "test".into(), created_utc_ms: 99, attachment_root: att.map(Path::to_path_buf) },
    )
    .unwrap()
}

#[test]
fn a17_full_round_trip_preserves_observations_edits_provenance_and_attachments() {
    let d = dir("roundtrip");
    let att_a = d.join("att-a");
    let mut src = build(&d.join("a.db"), &att_a);
    let zip = d.join("backup.zip");
    let summary = export(&mut src, &zip, Some(&att_a));
    assert_eq!(summary.runs, 2);
    assert_eq!(summary.attachments, 2);
    assert!(summary.contains.location, "saved place and precise snapshot are private location data");
    assert!(!summary.contains.raw_audio);

    // Clean install: a fresh journal with only its own default profile.
    let att_b = d.join("att-b");
    let mut dst = Repository::open(&d.join("b.db")).unwrap();
    dst.ensure_default_shooter(500).unwrap();
    let preview = preview(&mut dst, &zip, &d, &Limits::default()).unwrap();
    assert_eq!(preview.inserted["run"], 2);
    assert!(preview.conflicts.is_empty());
    assert!(dst.list_runs(10).unwrap().is_empty(), "preview writes nothing");
    let rep = import(&mut dst, &zip, &d, Some(&att_b), &Limits::default()).unwrap();
    assert_eq!(rep.inserted["run"], 2);
    assert_eq!(rep.attachments_restored, 2);
    assert_eq!(dst.video_clips("live-1").unwrap(), src.video_clips("live-1").unwrap(), "clock observations exact");
    assert_eq!(std::fs::read(att_b.join("videos/v1.mp4")).unwrap(), b"fake-mp4");
    assert_eq!(std::fs::read(att_b.join("photos/a1.jpg")).unwrap(), std::fs::read(att_a.join("photos/a1.jpg")).unwrap());

    for id in ["live-1", "manual-1"] {
        let a = src.load_run(id).unwrap();
        let b = dst.load_run(id).unwrap();
        assert_eq!(a.config, b.config);
        assert_eq!(a.candidates, b.candidates, "original observations");
        assert_eq!(a.revisions, b.revisions, "review history and hashes");
        assert_eq!(a.quality, b.quality);
        assert_eq!(a.envelope, b.envelope);
        assert_eq!(a.environment, b.environment, "pinned conditions");
        assert_eq!(a.epoch, b.epoch);
    }
    assert_eq!(dst.load_run("live-1").unwrap().epoch.unwrap().record.started_mono_ns, Some(9_007_199_254_740_993));
    assert_eq!(dst.latest_score("live-1").unwrap(), src.latest_score("live-1").unwrap());
    assert_eq!(dst.load_manual_string("manual-1").unwrap(), src.load_manual_string("manual-1").unwrap());
    assert_eq!(dst.round_counts().unwrap(), src.round_counts().unwrap());
    assert_eq!(dst.list_drills().unwrap(), src.list_drills().unwrap());
    assert_eq!(dst.list_places().unwrap(), src.list_places().unwrap());
    assert_eq!(dst.list_plans().unwrap(), src.list_plans().unwrap());
    assert_eq!(dst.plan_items("plan-1").unwrap(), src.plan_items("plan-1").unwrap());
    assert_eq!(dst.item_progress("item-1").unwrap().completed, 1);
    assert_eq!(dst.checklist(Some("plan-1")).unwrap(), src.checklist(Some("plan-1")).unwrap());

    // Importing the same backup again changes nothing.
    let again = import(&mut dst, &zip, &d, Some(&att_b), &Limits::default()).unwrap();
    assert_eq!(again.inserted.values().sum::<u64>(), 0);
    assert!(again.skipped_identical["run"] == 2);
}

#[test]
fn a17_conflicting_records_abort_without_changes() {
    let d = dir("conflict");
    let mut src = build(&d.join("a.db"), &d.join("att"));
    let zip = d.join("b.zip");
    export(&mut src, &zip, None);
    let mut dst = Repository::open(&d.join("b.db")).unwrap();
    dst.add_place(&SavedPlace { id: "pl-1".into(), label: "Different label".into(), lat: 1.0, lon: 1.0, created_utc_ms: 1 })
        .unwrap();
    let err = import(&mut dst, &zip, &d, None, &Limits::default()).unwrap_err();
    assert!(matches!(err, ArchiveError::Conflicts(1)), "{err:?}");
    assert!(dst.list_runs(10).unwrap().is_empty(), "nothing imported");
    assert_eq!(dst.list_places().unwrap()[0].label, "Different label", "local data untouched");
}

/// Write a zip from (name, bytes) pairs plus a manifest listing `listed`.
fn craft(path: &Path, entries: &[(&str, Vec<u8>)], schema: u32, listed: Option<Vec<FileEntry>>) {
    let f = std::fs::File::create(path).unwrap();
    let mut z = zip::ZipWriter::new(f);
    let o = zip::write::SimpleFileOptions::default();
    let mut files = vec![];
    for (n, b) in entries {
        z.start_file(*n, o).unwrap();
        z.write_all(b).unwrap();
        files.push(FileEntry {
            path: n.to_string(),
            sha256: Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect(),
            bytes: b.len() as u64,
        });
    }
    let m = Manifest {
        format: FORMAT.into(),
        format_version: FORMAT_VERSION,
        schema_version: schema,
        app_version: "t".into(),
        created_utc_ms: 1,
        private: true,
        encrypted: false,
        contains: Contains { location: false, attachments: false, raw_audio: false, notes: false },
        row_counts: Default::default(),
        files: listed.unwrap_or(files),
    };
    z.start_file("manifest.json", o).unwrap();
    z.write_all(&serde_json::to_vec(&m).unwrap()).unwrap();
    z.finish().unwrap();
}

#[test]
fn a17_hostile_archives_are_rejected_before_mutation() {
    let d = dir("hostile");
    let mut dst = Repository::open(&d.join("t.db")).unwrap();
    let lim = Limits::default();
    let try_import = |dst: &mut Repository, p: &Path| import(dst, p, &d, None, &lim).unwrap_err();

    let p = d.join("traversal.zip");
    craft(&p, &[("attachments/../../evil.sh", b"x".to_vec())], 3, None);
    assert!(matches!(try_import(&mut dst, &p), ArchiveError::BadEntry(_)));

    let p = d.join("unlisted.zip");
    craft(&p, &[("data/session.json", b"[]".to_vec())], 3, Some(vec![]));
    assert!(matches!(try_import(&mut dst, &p), ArchiveError::Inventory(_)));

    let p = d.join("hash.zip");
    craft(
        &p,
        &[("data/session.json", b"[]".to_vec())],
        3,
        Some(vec![FileEntry { path: "data/session.json".into(), sha256: "00".into(), bytes: 2 }]),
    );
    assert!(matches!(try_import(&mut dst, &p), ArchiveError::HashMismatch(_)));

    let p = d.join("newer.zip");
    craft(&p, &[], SCHEMA_VERSION + 1, None);
    assert!(matches!(try_import(&mut dst, &p), ArchiveError::NewerSchema { .. }));

    let p = d.join("injection.zip");
    let rows = br#"[{"id\" TEXT); DROP TABLE run; --":{"s":"x"}}]"#.to_vec();
    craft(&p, &[("data/saved_place.json", rows)], 3, None);
    assert!(matches!(try_import(&mut dst, &p), ArchiveError::InvalidData { .. }));

    let p = d.join("unknown_table.zip");
    craft(&p, &[("data/sqlite_master.json", b"[]".to_vec())], 3, None);
    assert!(matches!(try_import(&mut dst, &p), ArchiveError::InvalidData { .. }));

    let p = d.join("bomb.zip");
    {
        let f = std::fs::File::create(&p).unwrap();
        let mut z = zip::ZipWriter::new(f);
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        z.start_file("data/session.json", o).unwrap();
        z.write_all(&vec![b' '; 5_000_000]).unwrap();
        z.finish().unwrap();
    }
    assert!(matches!(try_import(&mut dst, &p), ArchiveError::CompressionRatio(_)));

    // A tampered review revision fails its content hash during staging.
    let mut src = build(&d.join("src.db"), &d.join("att"));
    let good = d.join("good.zip");
    export(&mut src, &good, None);
    let mut za = zip::ZipArchive::new(std::fs::File::open(&good).unwrap()).unwrap();
    let mut entries = vec![];
    for i in 0..za.len() {
        let mut f = za.by_index(i).unwrap();
        let mut b = vec![];
        std::io::Read::read_to_end(&mut f, &mut b).unwrap();
        let name = f.name().to_string();
        if name == "manifest.json" {
            continue;
        }
        if name == "data/run_revision.json" {
            b = String::from_utf8(b).unwrap().replace("2000000000", "1000000000").into_bytes();
        }
        entries.push((name, b));
    }
    let refs: Vec<(&str, Vec<u8>)> = entries.iter().map(|(n, b)| (n.as_str(), b.clone())).collect();
    let p = d.join("tampered.zip");
    craft(&p, &refs, SCHEMA_VERSION, None);
    let e = try_import(&mut dst, &p);
    assert!(matches!(e, ArchiveError::InvalidData { .. }), "{e:?}");
    assert!(dst.list_runs(10).unwrap().is_empty());
}

#[test]
fn older_schema_backup_imports_through_migrations() {
    let d = dir("older");
    let cell = |s: &str| format!("{{\"s\":\"{s}\"}}");
    let int = |i: i64| format!("{{\"i\":\"{i}\"}}");
    let shooter = format!(r#"[{{"id":{},"name":{},"notes":null,"created_utc_ms":{}}}]"#, cell("default"), cell("Me"), int(1));
    let session = format!(
        r#"[{{"id":{},"shooter_id":{},"started_utc_ms":{},"tz_offset_min":{},"notes":null,"active":{}}}]"#,
        cell("s-old"),
        cell("default"),
        int(1),
        int(0),
        int(1)
    );
    let p = d.join("v1.zip");
    craft(&p, &[("data/shooter_profile.json", shooter.into_bytes()), ("data/session.json", session.into_bytes())], 1, None);
    let mut dst = Repository::open(&d.join("t.db")).unwrap();
    let rep = import(&mut dst, &p, &d, None, &Limits::default()).unwrap();
    assert_eq!(rep.inserted["session"], 1);
    let leftovers: Vec<_> = std::fs::read_dir(&d)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("squib-import"))
        .collect();
    assert!(leftovers.is_empty(), "staging files cleaned up: {leftovers:?}");
    assert_eq!(rep.inserted["shooter_profile"], 1);
}

#[test]
fn csv_escapes_formulas_and_a19_share_is_redacted() {
    let d = dir("csv");
    let repo = build(&d.join("a.db"), &d.join("att"));
    let csv = csv_export::runs_csv(&repo).unwrap();
    assert!(csv.contains("'=HYPERLINK"), "{csv}");
    assert!(!csv.contains(",=HYPERLINK"));
    assert!(csv.lines().next().unwrap().contains("first_shot_s"));
    let shared = share::share_results(&repo, &["live-1".into(), "manual-1".into()]).unwrap();
    let json = serde_json::to_string(&shared).unwrap();
    for leak in ["KXYZ", "Secret Field", "40.1", "-105.2", "Club range", "live-1", "private note", "photos/a1.jpg", "sess-1"] {
        assert!(!json.contains(leak), "share leaks {leak}: {json}");
    }
    assert_eq!(shared.runs.len(), 2);
    let live = shared.runs.iter().find(|r| r.mode == "phone_live").unwrap();
    assert!(live.edited);
    assert_eq!(live.conditions.iter().find(|c| c.field == "temperature").unwrap().origin, "nearby_observation");
    assert!(shared.runs.iter().any(|r| r.manual && r.timing_method == "manual"));
}

#[test]
fn attachment_rows_with_traversal_paths_are_rejected() {
    let d = dir("attpath");
    let mut dst = Repository::open(&d.join("t.db")).unwrap();
    for bad in ["../../evil", "/etc/passwd", "photos/../../x", "C:/x", "a\\\\b"] {
        let row = format!(
            r#"[{{"id":{{"s":"a1"}},"run_id":null,"kind":{{"s":"photo"}},"relative_path":{{"s":"{bad}"}},"sha256":{{"s":"00"}},"bytes":{{"i":"1"}},"mime":{{"s":"image/jpeg"}},"metadata_stripped":{{"i":"1"}},"created_utc_ms":{{"i":"1"}}}}]"#
        );
        let p = d.join("bad.zip");
        craft(&p, &[("data/attachment.json", row.into_bytes())], SCHEMA_VERSION, None);
        let e = import(&mut dst, &p, &d, Some(&d.join("att")), &Limits::default()).unwrap_err();
        assert!(matches!(e, ArchiveError::InvalidData { .. }), "{bad}: {e:?}");
    }
    assert!(dst.list_attachments(None).unwrap().is_empty());
    assert!(!squib_storage::valid_attachment_path("../x") && squib_storage::valid_attachment_path("photos/a.jpg"));
}
