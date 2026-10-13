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
        diagnostic_recording: false,
        cue_channel: squib_domain::CueChannel::Sound,
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
        dataset: None,
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
            session_id: s.clone(),
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
    // An opt-in diagnostic recording.
    let audio = b"RIFF-fake-wav";
    std::fs::create_dir_all(att_root.join("diagnostics")).unwrap();
    std::fs::write(att_root.join("diagnostics/live-1.wav"), audio).unwrap();
    repo.insert_diagnostic(&DiagnosticRecord {
        attachment: AttachmentRecord {
            id: "diag-1".into(),
            run_id: Some("live-1".into()),
            kind: DIAGNOSTIC_KIND.into(),
            relative_path: "diagnostics/live-1.wav".into(),
            sha256: Sha256::digest(audio).iter().map(|b| format!("{b:02x}")).collect(),
            bytes: audio.len() as i64,
            mime: "audio/wav".into(),
            metadata_stripped: true,
            created_utc_ms: 18,
        },
        run_id: "live-1".into(),
        epoch_id: None,
        sample_rate_hz: 48_000,
        first_epoch_frame: 0,
        frames: 6,
        gaps: vec![(2, 3)],
        truncated: false,
        dropped_frames: 1,
        created_utc_ms: 18,
    })
    .unwrap();
    // A rifle with a load in two versions, and a range card made with version 2.
    repo.save_equipment_version(&EquipmentVersion {
        equipment_id: "rifle-1".into(),
        version: 1,
        name: "Bolt gun".into(),
        category: "rifle".into(),
        notes: String::new(),
        created_utc_ms: 19,
    })
    .unwrap();
    for (v, mv) in [(1u32, 790.0), (2, 795.5)] {
        repo.save_load_version(&test_load(v, mv)).unwrap();
    }
    let card_snap = repo.insert_snapshot(&station_snapshot("env-card"), LocationRetention::Precise).unwrap();
    repo.save_ballistic_card(&test_card("card-1", &test_load(2, 795.5), Some(card_snap.id), Some(s.clone()))).unwrap();
    repo
}

fn test_load(version: u32, mv: f64) -> LoadVersion {
    LoadVersion {
        load_id: "load-1".into(),
        version,
        name: "175 SMK".into(),
        equipment_id: Some("rifle-1".into()),
        load: squib_ballistics::Load {
            model: squib_ballistics::DragModel::G7,
            bc_lb_in2: 0.243,
            mass_kg: 0.011_34,
            muzzle_velocity_mps: mv,
            sight_height_m: 0.038,
            zero_range_m: 100.0,
        },
        bullet_diameter_m: Some(0.007_82),
        bullet_length_m: None,
        twist_m: Some(0.254),
        notes: "=cmd|' /C calc'!A0".into(),
        created_utc_ms: 20 + version as i64,
    }
}

fn test_card(id: &str, lv: &LoadVersion, snapshot: Option<String>, session: Option<String>) -> BallisticCard {
    let shot = squib_ballistics::Shot {
        load: lv.load.clone(),
        air: squib_ballistics::Atmosphere::from_station(95_000.0, 12.0, 0.4),
        zero_air: None,
        wind: Some(squib_ballistics::Wind { speed_mps: 3.0, from_deg: 90.0 }),
        look_angle_deg: 0.0,
        spin: None,
        // A backup keeps the optional terms a card used.
        coriolis: Some(squib_ballistics::Earth { latitude_deg: 45.0, azimuth_deg: 90.0 }),
    };
    let ranges = vec![100.0, 300.0, 500.0];
    let result = squib_ballistics::solve(&shot, &ranges).unwrap();
    BallisticCard {
        id: id.into(),
        load_id: lv.load_id.clone(),
        load_version: lv.version,
        environment_snapshot_id: snapshot,
        session_id: session,
        title: "Club range".into(),
        inputs: CardInputs { shot, ranges_m: ranges },
        result,
        created_utc_ms: 30,
    }
}

fn export(repo: &mut Repository, out: &Path, att: Option<&Path>) -> ExportSummary {
    export_private(
        repo,
        out,
        &ExportOptions {
            app_version: "test".into(),
            created_utc_ms: 99,
            attachment_root: att.map(Path::to_path_buf),
            include_media: true,
            include_diagnostic_audio: false,
        },
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
    assert_eq!(rep.attachments_restored, 2, "diagnostic audio is not in a default backup");
    assert!(!summary.contains.raw_audio);
    assert!(dst.diagnostic("live-1").unwrap().is_none());
    assert!(!att_b.join("diagnostics/live-1.wav").exists());
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
    assert_eq!(dst.load_history("load-1").unwrap(), src.load_history("load-1").unwrap(), "every load version");
    assert_eq!(dst.list_ballistic_cards(10).unwrap(), src.list_ballistic_cards(10).unwrap(), "range cards exactly");

    // Importing the same backup again changes nothing.
    let again = import(&mut dst, &zip, &d, Some(&att_b), &Limits::default()).unwrap();
    assert_eq!(again.inserted.values().sum::<u64>(), 0);
    assert!(again.skipped_identical["run"] == 2);
}

#[test]
fn diagnostic_audio_is_in_a_backup_only_when_chosen() {
    let d = dir("diag-backup");
    let att_a = d.join("att-a");
    let mut src = build(&d.join("a.db"), &att_a);
    let zip = d.join("with-audio.zip");
    let s = export_private(
        &mut src,
        &zip,
        &ExportOptions {
            app_version: "t".into(),
            created_utc_ms: 1,
            attachment_root: Some(att_a.clone()),
            include_media: false,
            include_diagnostic_audio: true,
        },
    )
    .unwrap();
    assert!(s.contains.raw_audio);
    assert_eq!(s.attachments, 1, "only the recording; photos and videos were not chosen");
    let att_b = d.join("att-b");
    let mut dst = Repository::open(&d.join("b.db")).unwrap();
    dst.ensure_default_shooter(1).unwrap();
    import(&mut dst, &zip, &d, Some(&att_b), &Limits::default()).unwrap();
    assert_eq!(dst.diagnostic("live-1").unwrap(), src.diagnostic("live-1").unwrap(), "gaps and frame offsets exact");
    assert_eq!(std::fs::read(att_b.join("diagnostics/live-1.wav")).unwrap(), b"RIFF-fake-wav");
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

#[test]
fn import_as_copy_keeps_both_with_new_ids_and_provenance() {
    let d = dir("copy");
    let att_a = d.join("att-a");
    let mut src = build(&d.join("a.db"), &att_a);
    let zip = d.join("b.zip");
    export_private(
        &mut src,
        &zip,
        &ExportOptions {
            app_version: "t".into(),
            created_utc_ms: 77,
            attachment_root: Some(att_a.clone()),
            include_media: true,
            include_diagnostic_audio: true,
        },
    )
    .unwrap();
    // A phone restored from this backup, then changed locally: a normal import conflicts.
    let att_b = d.join("att-b");
    let mut dst = Repository::open(&d.join("b.db")).unwrap();
    dst.ensure_default_shooter(1).unwrap();
    import(&mut dst, &zip, &d, Some(&att_b), &Limits::default()).unwrap();
    dst.connection().execute("UPDATE saved_place SET label = 'Renamed here' WHERE id = 'pl-1'", []).unwrap();
    assert!(matches!(import(&mut dst, &zip, &d, Some(&att_b), &Limits::default()), Err(ArchiveError::Conflicts(_))));

    let p = preview_copy(&mut dst, &zip, &d, &Limits::default(), 500).unwrap();
    assert!(p.conflicts.is_empty() && p.inserted["run"] == 2, "{:?} {:?}", p.conflicts, p.inserted);
    let rep = import_copy(&mut dst, &zip, &d, Some(&att_b), &Limits::default(), 500).unwrap();
    assert_eq!(rep.inserted["run"], 2, "both runs copied under new ids");
    assert_eq!(dst.list_runs(10).unwrap().len(), 4);
    assert_eq!(dst.list_places().unwrap().len(), 2, "local place kept, archive place copied");
    assert!(dst.list_places().unwrap().iter().any(|p| p.label == "Renamed here"));
    // Unchanged reference data is not duplicated.
    assert_eq!(dst.list_drills().unwrap().len(), src.list_drills().unwrap().len());

    // Every copied run loads (hash chains re-verified) and maps back to its original.
    let prov: Vec<(String, String, String)> = {
        let c = dst.connection();
        let mut st = c.prepare("SELECT entity, new_id, original_id FROM import_provenance").unwrap();
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().map(|x| x.unwrap()).collect()
    };
    let new_live = prov.iter().find(|p| p.0 == "run" && p.2 == "live-1").map(|p| p.1.clone()).expect("provenance for live-1");
    let a = src.load_run("live-1").unwrap();
    let b = dst.load_run(&new_live).unwrap();
    assert_eq!(a.candidates, b.candidates, "original observations preserved in the copy");
    assert_eq!(a.revisions.len(), b.revisions.len());
    assert!(b.revisions.iter().all(|r| r.content.run_id == new_live && r.verify_hash()));
    assert_ne!(b.config.environment_snapshot_id, a.config.environment_snapshot_id, "snapshot copied with a new id");
    assert!(b.environment.is_some(), "and still pinned to the copied run");
    assert_eq!(dst.latest_score(&new_live).unwrap().map(|s| s.entry), src.latest_score("live-1").unwrap().map(|s| s.entry));

    // Attachments are copied under new paths with identical bytes; video and recording follow.
    let att = dst.list_attachments(Some(&new_live)).unwrap();
    assert_eq!(att.len(), 3);
    for x in &att {
        let orig = src.list_attachments(Some("live-1")).unwrap().into_iter().find(|o| o.kind == x.kind).unwrap();
        assert_ne!(x.relative_path, orig.relative_path);
        assert_eq!(std::fs::read(att_b.join(&x.relative_path)).unwrap(), std::fs::read(att_a.join(&orig.relative_path)).unwrap());
    }
    assert_eq!(dst.video_clips(&new_live).unwrap().len(), 1);
    assert!(dst.diagnostic(&new_live).unwrap().is_some());

    // The copied plan links the copied manual run, not the original.
    let new_manual = prov.iter().find(|p| p.0 == "run" && p.2 == "manual-1").map(|p| p.1.clone()).unwrap();
    let new_item = prov.iter().find(|p| p.0 == "plan_item" && p.2 == "item-1").map(|p| p.1.clone()).unwrap();
    assert_eq!(dst.item_run_ids(&new_item).unwrap(), vec![new_manual]);

    // Range cards are journal records: copied with new ids, their session and snapshot
    // links following the copies. The unchanged load keeps its id and is not duplicated.
    let cards = dst.list_ballistic_cards(10).unwrap();
    assert_eq!(cards.len(), 2);
    let new_card = prov.iter().find(|p| p.0 == "ballistic_card" && p.2 == "card-1").map(|p| p.1.clone()).unwrap();
    let copied = dst.ballistic_card(&new_card).unwrap().unwrap();
    let orig = src.ballistic_card("card-1").unwrap().unwrap();
    assert_eq!((copied.inputs.clone(), copied.result.clone()), (orig.inputs.clone(), orig.result.clone()), "card content exact");
    assert_eq!((copied.load_id.as_str(), copied.load_version), ("load-1", 2));
    assert_ne!(copied.session_id, orig.session_id);
    assert_ne!(copied.environment_snapshot_id, orig.environment_snapshot_id);
    let session_exists: i64 = dst
        .connection()
        .query_row("SELECT count(*) FROM session WHERE id = ?1", [copied.session_id.clone().unwrap()], |r| r.get(0))
        .unwrap();
    assert_eq!(session_exists, 1, "points at the copied session");
    assert_eq!(dst.list_loads().unwrap().len(), 1);
    assert_eq!(dst.load_history("load-1").unwrap().len(), 2);
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

// ---- Plan 5.2: hostile-input robustness (see testing/mutate.rs for run control) --------

#[path = "../../../testing/mutate.rs"]
mod mutate;

use mutate::{JSON_TOKENS, Rng, fuzz, iterations, mutate};

/// Every table's rows, in a stable order: equal fingerprints mean an unchanged journal.
fn fingerprint(repo: &mut Repository) -> String {
    let c = repo.connection();
    let tables: Vec<String> = c
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    let mut out = String::new();
    for t in tables {
        let mut st = c.prepare(&format!("SELECT * FROM \"{t}\" ORDER BY 1")).unwrap();
        let n = st.column_count();
        let mut rows = st.query([]).unwrap();
        out.push_str(&t);
        while let Some(r) = rows.next().unwrap() {
            for i in 0..n {
                out.push_str(&format!("|{:?}", r.get_ref(i).unwrap()));
            }
            out.push('\n');
        }
    }
    out
}

fn files_under(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = vec![];
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push((p.clone(), std::fs::read(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

fn zip_entries(path: &Path) -> Vec<(String, Vec<u8>)> {
    let mut za = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    (0..za.len())
        .map(|i| {
            let mut f = za.by_index(i).unwrap();
            let mut b = vec![];
            std::io::Read::read_to_end(&mut f, &mut b).unwrap();
            (f.name().to_string(), b)
        })
        .collect()
}

fn write_zip(path: &Path, entries: &[(String, Vec<u8>)]) {
    let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (n, b) in entries {
        z.start_file(n.as_str(), o).unwrap();
        z.write_all(b).unwrap();
    }
    z.finish().unwrap();
}

/// A destination journal with data of its own, restorable between iterations.
struct Dest {
    dir: PathBuf,
    pristine_db: Vec<u8>,
    repo: Repository,
    before: String,
    att_before: Vec<(PathBuf, Vec<u8>)>,
    /// Outcome counts, printed with --nocapture to show how deep the inputs reach.
    outcomes: std::collections::BTreeMap<String, usize>,
}

impl Dest {
    fn new(dir: &Path) -> Self {
        let db = dir.join("dst.db");
        let repo = Repository::open(&db).unwrap();
        repo.ensure_default_shooter(1).unwrap();
        repo.add_place(&SavedPlace { id: "mine".into(), label: "My range".into(), lat: 1.0, lon: 2.0, created_utc_ms: 1 })
            .unwrap();
        std::fs::create_dir_all(dir.join("att/photos")).unwrap();
        std::fs::write(dir.join("att/photos/mine.jpg"), b"mine").unwrap();
        drop(repo);
        let pristine_db = std::fs::read(&db).unwrap();
        let mut repo = Repository::open(&db).unwrap();
        let before = fingerprint(&mut repo);
        let att_before = files_under(&dir.join("att"));
        Self { dir: dir.to_path_buf(), pristine_db, repo, before, att_before, outcomes: Default::default() }
    }

    /// Import, then check all-or-nothing: a refused archive leaves journal and files untouched.
    fn check(&mut self, archive: &Path, copy: bool) -> std::result::Result<(), String> {
        let att = self.dir.join("att");
        let r = if copy {
            import_copy(&mut self.repo, archive, &self.dir, Some(&att), &Limits::default(), 7)
        } else {
            import(&mut self.repo, archive, &self.dir, Some(&att), &Limits::default())
        };
        let kind = match &r {
            Ok(_) => "accepted".to_string(),
            Err(e) => format!("{e:?}").split(['(', ' ', '{']).next().unwrap_or("").to_string(),
        };
        *self.outcomes.entry(kind).or_default() += 1;
        match r {
            Err(_) => {
                if fingerprint(&mut self.repo) != self.before {
                    return Err("refused import changed the journal".into());
                }
                if files_under(&att) != self.att_before {
                    return Err("refused import changed attachment files".into());
                }
                Ok(())
            }
            Ok(_) => {
                // Accepted: the journal stays consistent and our own data survives.
                let c = self.repo.connection();
                let ok: String = c.query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap();
                let fk: i64 = c.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r.get(0)).unwrap();
                if ok != "ok" || fk != 0 {
                    return Err(format!("accepted import left the journal inconsistent: {ok}, {fk} foreign key errors"));
                }
                if !self.repo.list_places().unwrap().iter().any(|p| p.id == "mine" && p.label == "My range") {
                    return Err("accepted import changed local data".into());
                }
                if std::fs::read(att.join("photos/mine.jpg")).ok().as_deref() != Some(b"mine".as_slice()) {
                    return Err("accepted import overwrote a local file".into());
                }
                self.reset();
                Ok(())
            }
        }
    }

    fn reset(&mut self) {
        let db = self.dir.join("dst.db");
        let _ = std::fs::remove_dir_all(self.dir.join("att"));
        std::fs::create_dir_all(self.dir.join("att/photos")).unwrap();
        std::fs::write(self.dir.join("att/photos/mine.jpg"), b"mine").unwrap();
        // Swap in a throwaway handle so the destination file is closed before it is replaced.
        self.repo = Repository::open(&self.dir.join("unused.db")).unwrap();
        for ext in ["db-wal", "db-shm"] {
            let _ = std::fs::remove_file(db.with_extension(ext));
        }
        std::fs::write(&db, &self.pristine_db).unwrap();
        self.repo = Repository::open(&db).unwrap();
    }
}

fn exported(d: &Path) -> Vec<(String, Vec<u8>)> {
    let mut src = build(&d.join("src.db"), &d.join("src-att"));
    let good = d.join("good.zip");
    export_private(
        &mut src,
        &good,
        &ExportOptions {
            app_version: "test".into(),
            created_utc_ms: 99,
            attachment_root: Some(d.join("src-att")),
            include_media: true,
            include_diagnostic_audio: true,
        },
    )
    .unwrap();
    zip_entries(&good)
}

/// One entry's content is mutated and the archive rebuilt with matching hashes, so the
/// input gets past the inventory checks into row validation, staging and merge.
#[test]
fn robust_backup_import_with_consistent_inventory() {
    let d = dir("fuzz-structured");
    let entries = exported(&d);
    let mut dest = Dest::new(&d);
    let mut rng = Rng::new(mutate::base_seed());
    let data: Vec<usize> = (0..entries.len()).filter(|&i| entries[i].0 != "manifest.json").collect();
    let p = d.join("case.zip");
    // Seeds are indexes into `data`; the mutation is applied to the chosen entry.
    let seeds: Vec<Vec<u8>> = data.iter().map(|i| i.to_le_bytes().to_vec()).collect();
    let mut n = 0usize;
    fuzz("backup entry", &seeds, &[], iterations(80), std::time::Duration::from_secs(10), |_| {
        n += 1;
        let target = data[rng.below(data.len())];
        let mut e = entries.clone();
        e[target].1 = mutate(&mut rng, &e[target].1, JSON_TOKENS);
        let listed: Vec<(&str, Vec<u8>)> =
            e.iter().filter(|(n, _)| n != "manifest.json").map(|(n, b)| (n.as_str(), b.clone())).collect();
        craft(&p, &listed, SCHEMA_VERSION, None);
        dest.check(&p, n.is_multiple_of(4)).map_err(|m| format!("{m} (mutated {})", e[target].0))
    });
    eprintln!("backup entry outcomes: {:?}", dest.outcomes);
}

/// The manifest is mutated (everything else intact), and the raw zip bytes are mutated.
#[test]
fn robust_backup_import_manifest_and_container() {
    let d = dir("fuzz-raw");
    let entries = exported(&d);
    let mut dest = Dest::new(&d);
    let mi = entries.iter().position(|(n, _)| n == "manifest.json").unwrap();
    let p = d.join("case.zip");
    fuzz("backup manifest", &[entries[mi].1.clone()], JSON_TOKENS, iterations(60), std::time::Duration::from_secs(10), |m| {
        let mut e = entries.clone();
        e[mi].1 = m.to_vec();
        write_zip(&p, &e);
        dest.check(&p, false)
    });
    write_zip(&d.join("whole.zip"), &entries);
    let whole = std::fs::read(d.join("whole.zip")).unwrap();
    fuzz(
        "backup zip bytes",
        &[whole],
        &[b"PK\x03\x04", b"PK\x01\x02", b"PK\x05\x06", b"\xff\xff\xff\xff"],
        iterations(60),
        std::time::Duration::from_secs(10),
        |b| {
            std::fs::write(&p, b).unwrap();
            dest.check(&p, false)
        },
    );
    eprintln!("backup manifest and zip outcomes: {:?}", dest.outcomes);
}

#[test]
fn robust_diagnostic_export_reader() {
    let d = dir("fuzz-diag");
    let src = build(&d.join("src.db"), &d.join("att"));
    let out = d.join("diag.zip");
    squib_archive::diagnostic::export_diagnostic(
        &src,
        "live-1",
        &out,
        &squib_archive::diagnostic::DiagnosticExportOptions {
            export_id: "x".into(),
            app_version: "t".into(),
            core_versions: vec![],
            created_day: "2026-10-11".into(),
            consent_confirmed: true,
            attachment_root: &d.join("att"),
        },
    )
    .unwrap();
    squib_archive::diagnostic::read_diagnostic(&out).expect("seed reads back");
    let entries = zip_entries(&out);
    let p = d.join("case.zip");
    let mut rng = Rng::new(mutate::base_seed() ^ 0xd1a6);
    let seeds: Vec<Vec<u8>> = vec![vec![0]];
    fuzz("diagnostic entry", &seeds, &[], iterations(300), std::time::Duration::from_secs(2), |_| {
        let mut e = entries.clone();
        let i = rng.below(e.len());
        e[i].1 = mutate(&mut rng, &e[i].1, JSON_TOKENS);
        if rng.chance(8) {
            e.remove(rng.below(e.len()));
        }
        write_zip(&p, &e);
        let _ = squib_archive::diagnostic::read_diagnostic(&p);
        Ok(())
    });
    write_zip(&d.join("whole.zip"), &entries);
    let whole = std::fs::read(d.join("whole.zip")).unwrap();
    fuzz(
        "diagnostic zip bytes",
        &[whole],
        &[b"PK\x03\x04", b"\xff\xff\xff\xff"],
        iterations(300),
        std::time::Duration::from_secs(2),
        |b| {
            std::fs::write(&p, b).unwrap();
            let _ = squib_archive::diagnostic::read_diagnostic(&p);
            Ok(())
        },
    );
}

// ---- Plan 5.7: optional passphrase encryption (age) ----------------------------------

fn secret(s: &str) -> crypt::SecretString {
    crypt::SecretString::from(s.to_string())
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into()).collect();
    v.sort();
    v
}

#[test]
fn encrypted_backup_restores_exactly_and_leaves_no_plaintext() {
    let d = dir("encrypted");
    let att_a = d.join("att-a");
    let mut src = build(&d.join("a.db"), &att_a);
    let plain_zip = d.join("plain.zip");
    export(&mut src, &plain_zip, Some(&att_a));
    let scratch = d.join("scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    let enc = d.join("backup.zip.age");
    let opts = ExportOptions {
        app_version: "test".into(),
        created_utc_ms: 99,
        attachment_root: Some(att_a.clone()),
        include_media: true,
        include_diagnostic_audio: false,
    };
    assert!(matches!(
        crypt::export_encrypted(&mut src, &enc, &scratch, &opts, &secret("short")),
        Err(ArchiveError::Passphrase(_))
    ));
    assert!(!enc.exists(), "a refused passphrase writes nothing");
    let summary = crypt::export_encrypted(&mut src, &enc, &scratch, &opts, &secret("correct horse battery")).unwrap();
    assert_eq!(summary.runs, 2);
    assert_eq!(summary.bytes, std::fs::metadata(&enc).unwrap().len());
    assert!(names_in(&scratch).is_empty(), "plaintext zip removed after encrypting");
    assert_eq!(crypt::detect(&enc).unwrap(), crypt::Encryption::Age);
    assert!(zip::ZipArchive::new(std::fs::File::open(&enc).unwrap()).is_err(), "not readable as a zip");
    // Private content is not visible in the encrypted bytes.
    let raw = std::fs::read(&enc).unwrap();
    for needle in [b"live-1".as_slice(), b"fake-mp4", b"manifest.json", b"squib-private-backup"] {
        assert!(!raw.windows(needle.len()).any(|w| w == needle), "{}", String::from_utf8_lossy(needle));
    }

    // Read through the import path: wrong or missing passphrase changes nothing.
    let att_b = d.join("att-b");
    let mut dst = Repository::open(&d.join("b.db")).unwrap();
    dst.ensure_default_shooter(500).unwrap();
    let before = fingerprint(&mut dst);
    let max = Limits::default().max_archive_bytes;
    assert_eq!(crypt::open(&enc, &scratch, None, max).unwrap_err(), ArchiveError::PassphraseRequired);
    assert_eq!(
        crypt::open(&enc, &scratch, Some(&secret("wrong horse battery")), max).unwrap_err(),
        ArchiveError::WrongPassphrase
    );
    // An encrypted file handed straight to the zip importer is refused, not misread.
    assert!(matches!(import(&mut dst, &enc, &scratch, Some(&att_b), &Limits::default()), Err(ArchiveError::NotSquib(_))));
    assert_eq!(fingerprint(&mut dst), before);
    assert!(names_in(&scratch).is_empty());

    let plain = crypt::open(&enc, &scratch, Some(&secret("correct horse battery")), max).unwrap();
    // The decrypted archive has exactly the entries an unencrypted backup has.
    let mut a: Vec<String> = zip_entries(plain.path()).into_iter().map(|e| e.0).collect();
    let mut b: Vec<String> = zip_entries(&plain_zip).into_iter().map(|e| e.0).collect();
    a.sort();
    b.sort();
    assert_eq!(a, b);
    let rep = import(&mut dst, plain.path(), &scratch, Some(&att_b), &Limits::default()).unwrap();
    drop(plain);
    assert_eq!(rep.inserted["run"], 2);
    assert_eq!(rep.attachments_restored, 2);
    for id in ["live-1", "manual-1"] {
        let x = src.load_run(id).unwrap();
        let y = dst.load_run(id).unwrap();
        assert_eq!((x.config, x.candidates, x.revisions, x.environment), (y.config, y.candidates, y.revisions, y.environment));
    }
    assert_eq!(std::fs::read(att_b.join("videos/v1.mp4")).unwrap(), b"fake-mp4");
    assert!(names_in(&scratch).is_empty(), "decrypted copy removed after import");
}

/// Mutated encrypted backups (header, stanza, payload) never panic, never leave the
/// decrypted temporary behind, and never change the journal. Uses a low work factor so
/// each case is fast; the limit on hostile work factors is a unit test in `crypt`.
#[test]
fn robust_encrypted_backup_open() {
    let d = dir("fuzz-age");
    let entries = exported(&d);
    let zip_path = d.join("inner.zip");
    write_zip(&zip_path, &entries);
    let enc = d.join("good.age");
    crypt::encrypt_file(&zip_path, &enc, &secret("correct horse battery"), 4).unwrap();
    let good = std::fs::read(&enc).unwrap();
    let scratch = d.join("age-scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    let mut dest = Dest::new(&d);
    let p = d.join("case.age");
    let mut opened = 0usize;
    fuzz(
        "encrypted backup",
        &[good],
        &[b"age-encryption.org/v1\n", b"-> scrypt ", b"\n--- ", b" 18\n", b" 99\n", b"-> X25519 "],
        iterations(150),
        std::time::Duration::from_secs(10),
        |b| {
            std::fs::write(&p, b).unwrap();
            let r = crypt::open(&p, &scratch, Some(&secret("correct horse battery")), Limits::default().max_archive_bytes);
            let result = match r {
                Ok(plain) => {
                    opened += 1;
                    dest.check(plain.path(), false)
                }
                Err(_) => Ok(()),
            };
            if !names_in(&scratch).is_empty() {
                return Err(format!("decrypted temporary left behind: {:?}", names_in(&scratch)));
            }
            result
        },
    );
    eprintln!("encrypted backup: {opened} mutants decrypted; import outcomes {:?}", dest.outcomes);
}
