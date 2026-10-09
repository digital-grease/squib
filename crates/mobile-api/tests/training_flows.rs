//! M3 engine flows: profiles (A24), drills pinned to runs (A14), manual entry, scores,
//! rounds, analytics, backup/restore, deletion, and the diagnostic report.

use std::sync::Arc;

use squib_mobile::*;

const NOW: i64 = 1_791_413_400_000;

fn engine(name: &str) -> (Arc<SquibEngine>, std::path::PathBuf) {
    let d = std::env::temp_dir().join(format!("squib-train-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    (SquibEngine::new(d.join("journal.db").to_string_lossy().into(), NOW).unwrap(), d)
}

fn arm_req(drill: Option<(String, u32)>) -> ArmRequest {
    ArmRequest {
        mode: Mode::ParOnly,
        delay: StartDelay::Fixed { ms: 5000 },
        unit_random: 0.0,
        pars_ms: vec![],
        auto_stop_grace_ms: None,
        expected_count: None,
        threshold_db: None,
        calibration_id: None,
        route: None,
        now_utc_ms: NOW,
        tz_offset_min: 0,
        app_build: "test".into(),
        expected_rate_hz: 48_000,
        drill_id: drill.clone().map(|d| d.0),
        drill_version: drill.map(|d| d.1),
        plan_item_id: None,
    }
}

#[test]
fn starters_seeded_once_and_a24_profile_switch_blocked_during_run() {
    let (e, d) = engine("profiles");
    assert_eq!(e.list_drills().len(), 3, "neutral starter drills");
    drop(e);
    let e = SquibEngine::new(d.join("journal.db").to_string_lossy().into(), NOW).unwrap();
    assert_eq!(e.list_drills().len(), 3, "not reseeded");
    let coach = e.add_shooter("Alex".into(), NOW).unwrap();
    e.arm("a1".into(), arm_req(None), 1_000_000_000).unwrap();
    assert!(e.set_active_shooter(coach.id.clone()).is_err(), "no profile switch while armed");
    e.cancel("c".into(), 1_100_000_000).unwrap();
    e.set_active_shooter(coach.id.clone()).unwrap();
    assert!(e.list_shooters().iter().any(|s| s.id == coach.id && s.active));
    // A run armed now belongs to the new shooter.
    let v = e.arm("a2".into(), arm_req(None), 2_000_000_000).unwrap();
    assert!(v.run_id.is_some());
}

#[test]
fn a14_editing_a_drill_does_not_change_earlier_runs() {
    let (e, _) = engine("drills");
    let mut d = e.list_drills().into_iter().find(|d| d.input.title == "Single par").unwrap();
    let v = e.arm("a1".into(), arm_req(Some((d.drill_id.clone(), d.version))), 1_000_000_000).unwrap();
    let run = v.run_id.unwrap();
    e.cancel("c".into(), 1_100_000_000).unwrap();
    d.input.pars_ms = vec![1500];
    let edited = e.edit_drill(d.drill_id.clone(), d.input.clone(), NOW + 1).unwrap();
    assert_eq!(edited.version, 2);
    // The cancelled attempt still references version 1.
    let review = e.load_review(run).unwrap();
    assert_eq!(review.pars_ms, Vec::<u32>::new(), "run kept its immutable configuration");
    let invalid = DrillInput { title: " ".into(), ..d.input };
    assert!(e.create_drill(invalid, NOW).is_err());
}

#[test]
fn manual_entry_score_rounds_analytics() {
    let (e, _) = engine("manual");
    let drill = e.list_drills().into_iter().find(|d| d.input.title == "Timed string with score").unwrap();
    let mut ids = vec![];
    for t in ["1.10 1.60 2.05", "1.00 1.45 1.95", "1.20 1.70 2.20"] {
        ids.push(
            e.add_manual_run(
                "Club timer".into(),
                10,
                t.into(),
                Some(drill.drill_id.clone()),
                Some(drill.version),
                NOW,
                0,
                "test".into(),
            )
            .unwrap(),
        );
    }
    let s = e.run_score(ids[0].clone(), None).unwrap();
    assert_eq!(s.profile_id, "generic-points-hf");
    assert_eq!(s.status, "incomplete", "no score yet is incomplete, not zero");
    let s = e
        .set_score(
            ids[0].clone(),
            "generic-points-hf".into(),
            vec![ScoreCount { name: "A".into(), count: 3 }],
            true,
            None,
            String::new(),
            NOW,
        )
        .unwrap();
    assert_eq!(s.status, "complete");
    assert!((s.hit_factor.unwrap() - 15.0 / 2.05).abs() < 1e-9);
    assert!(
        e.set_score(
            ids[0].clone(),
            "generic-points-hf".into(),
            vec![ScoreCount { name: "Bogus".into(), count: 1 }],
            true,
            None,
            String::new(),
            NOW
        )
        .is_err()
    );
    assert_eq!(e.run_rounds(ids[0].clone()).unwrap().proposed, 0, "manual entries propose no detected rounds");
    e.confirm_rounds(ids[0].clone(), 3, NOW).unwrap();
    e.set_round_cost(Some(30), Some("usd".into())).unwrap();
    let t = e.round_totals();
    assert_eq!((t.confirmed_rounds, t.cost_minor, t.currency.as_deref()), (3, Some(90), Some("USD")));
    let f = AnalyticsFilterFfi {
        drill_id: Some(drill.drill_id.clone()),
        drill_version: Some(drill.version),
        mode: None,
        include_edited: true,
        include_manual: false,
        include_needs_review: false,
        include_unresolved_start_splits: false,
    };
    let a = e.analytics(f.clone()).unwrap();
    assert_eq!(a.included, 0, "manual results excluded by default");
    let a = e.analytics(AnalyticsFilterFfi { include_manual: true, ..f }).unwrap();
    assert_eq!(a.included, 1, "two runs lack the required score");
    assert!(a.excluded.iter().any(|x| x.reason == "missing_required_score" && x.count == 2));
    assert!(a.insufficient);
}

#[test]
fn backup_restore_delete_and_report() {
    let (e, d) = engine("backup");
    let id = e.add_manual_run("Club timer".into(), 10, "1.0 1.5".into(), None, None, NOW, 0, "test".into()).unwrap();
    e.add_saved_place("Secret range".into(), 40.0, -105.0, NOW).unwrap();
    let zip = d.join("backup.zip").to_string_lossy().to_string();
    let b = e.export_backup(zip.clone(), None, "test".into(), NOW).unwrap();
    assert_eq!(b.runs, 1);
    assert!(b.contains_location && !b.encrypted);

    let (fresh, fd) = engine("restore");
    let p = fresh.preview_import(zip.clone(), fd.to_string_lossy().into()).unwrap();
    assert_eq!(p.runs_new, 1);
    assert!(fresh.list_runs(10).unwrap().is_empty(), "preview changes nothing");
    let r = fresh.import_backup(zip.clone(), fd.to_string_lossy().into(), None).unwrap();
    assert_eq!(r.runs_new, 1);
    assert_eq!(fresh.load_review(id.clone()).unwrap().splits_ns, vec![500_000_000]);
    let r = fresh.import_backup(zip, fd.to_string_lossy().into(), None).unwrap();
    assert_eq!((r.runs_new, r.runs_already_present), (0, 1));

    let csv = e.export_csv().unwrap();
    assert!(csv.contains("manual"));
    let shared = e.share_results(vec![id.clone()]).unwrap();
    assert!(!shared.contains(&id) && !shared.contains("Secret range"));

    let report = e.diagnostic_report("Pixel Test".into(), "15".into(), "0.1.0".into());
    assert!(report.contains("Device: Pixel Test") && report.contains("manual_entry"));
    for leak in [id.as_str(), "Secret range", "40.0", "-105.0", "Club timer"] {
        assert!(!report.contains(leak), "report leaks {leak}");
    }

    let del = e.delete_run(id.clone()).unwrap();
    assert_eq!(del.runs, 1);
    assert!(e.load_review(id).is_err());
    let all = fresh.delete_all_history().unwrap();
    assert_eq!(all.runs, 1);
    assert!(fresh.list_saved_places().is_empty());
    drop(fresh);
    let reopened = SquibEngine::new(fd.join("journal.db").to_string_lossy().into(), NOW).unwrap();
    assert_eq!(reopened.list_drills().len(), 3, "starter drills return after deleting everything");
}
