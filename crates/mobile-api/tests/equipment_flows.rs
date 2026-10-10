//! Plan phase 1.2 engine flows: equipment pinned on runs, locked during runs, edits as
//! versions, analytics filter, CSV column, backup round trip.

use std::sync::Arc;

use squib_mobile::*;

const NOW: i64 = 1_791_413_400_000;

fn engine(name: &str) -> (Arc<SquibEngine>, std::path::PathBuf) {
    let d = std::env::temp_dir().join(format!("squib-equip-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    (SquibEngine::new(d.join("journal.db").to_string_lossy().into(), NOW).unwrap(), d)
}

fn arm_req() -> ArmRequest {
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
        drill_id: None,
        drill_version: None,
        plan_item_id: None,
        video: false,
        diagnostic_root: None,
    }
}

fn manual(e: &SquibEngine) -> String {
    e.add_manual_run("Club timer".into(), 10, "1.0 1.5 2.0".into(), None, None, NOW, 0, "t".into()).unwrap()
}

#[test]
fn equipment_is_pinned_versioned_filtered_exported_and_backed_up() {
    let (e, d) = engine("flow");
    assert_eq!(e.equipment_categories().len(), 6);
    let g = e.add_equipment("=HYPERLINK(\"x\")".into(), "pistol".into(), "".into(), NOW).unwrap();
    let r = e.add_equipment("Rifle".into(), "rifle".into(), "308".into(), NOW).unwrap();
    assert!(e.add_equipment("Bad".into(), "cannon".into(), "".into(), NOW).is_err());

    let none = manual(&e);
    e.set_active_equipment(Some(g.id.clone())).unwrap();
    let with_g1 = manual(&e);
    // Locked during a run.
    e.arm("a".into(), arm_req(), 1_000_000_000).unwrap();
    assert!(e.set_active_equipment(Some(r.id.clone())).is_err());
    e.cancel("c".into(), 1_100_000_000).unwrap();

    // Editing makes version 2; the earlier run keeps version 1's name.
    e.edit_equipment(g.id.clone(), "Carry pistol".into(), "pistol".into(), "new optic".into(), NOW).unwrap();
    let with_g2 = manual(&e);
    let label = |id: &str| e.load_review(id.into()).unwrap().equipment;
    assert_eq!(label(&none), None);
    assert_eq!(label(&with_g1).as_deref(), Some("=HYPERLINK(\"x\") (pistol)"));
    assert_eq!(label(&with_g2).as_deref(), Some("Carry pistol (pistol)"));
    assert!(e.list_runs(10).unwrap().iter().any(|x| x.equipment.as_deref() == Some("Carry pistol (pistol)")));

    // Analytics filter: all versions of one profile, with the exclusion reason stated.
    let f = AnalyticsFilterFfi {
        drill_id: None,
        drill_version: None,
        mode: None,
        include_edited: true,
        include_manual: true,
        include_needs_review: true,
        include_unresolved_start_splits: false,
        equipment_id: Some(g.id.clone()),
    };
    let a = e.analytics(f).unwrap();
    assert_eq!(a.included, 2);
    assert!(a.excluded.iter().any(|x| x.reason == "other_equipment" && x.count == 1));

    // CSV carries the version and escapes formula-like names.
    let csv = e.export_csv().unwrap();
    assert!(csv.lines().next().unwrap().contains(",equipment,"));
    assert!(csv.contains("'=HYPERLINK") && csv.contains("Carry pistol (pistol) v2"));

    // Archiving clears the active choice; history still shows the old label.
    e.archive_equipment(g.id.clone()).unwrap();
    assert!(e.list_equipment().unwrap().iter().all(|x| x.id != g.id));
    assert_eq!(label(&with_g2).as_deref(), Some("Carry pistol (pistol)"));
    assert!(!manual(&e).is_empty(), "runs without equipment still work after archiving the active one");

    // Backup round trip keeps the versions the runs point at.
    let zip = d.join("b.zip").to_string_lossy().to_string();
    e.export_backup(zip.clone(), d.to_string_lossy().into(), false, false, "t".into(), NOW).unwrap();
    let (fresh, fd) = engine("restore");
    fresh.import_backup(zip, fd.to_string_lossy().into(), None).unwrap();
    assert_eq!(fresh.load_review(with_g1).unwrap().equipment.as_deref(), Some("=HYPERLINK(\"x\") (pistol)"));
}
