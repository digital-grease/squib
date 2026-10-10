//! Schema 4: day plans, agenda progress from linked runs, checklists, coach rotation,
//! deletion, and upgrade from schema 3.

use std::path::PathBuf;

use squib_domain::*;
use squib_storage::*;
use squib_training::drill::{DrillVersion, starter_recipes};
use squib_training::manual::ManualString;

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("squib-planstore-{}-{}", name, std::process::id()));
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
        capture_video: false,
        diagnostic_recording: false,
        app_build: "test".into(),
    }
}

fn intent(id: &str, session: &str, mode: SourceMode, drill: Option<String>) -> RunIntent {
    RunIntent { run_id: id.into(), session_id: session.into(), created_utc_ms: 10, tz_offset_min: 0, config: cfg(mode, drill) }
}

fn plan(id: &str) -> DayPlan {
    DayPlan {
        id: id.into(),
        title: "Saturday practice".into(),
        date_local: "2026-10-10".into(),
        kind: "practice".into(),
        notes: String::new(),
        created_utc_ms: 1,
    }
}

fn item(id: &str, plan: &str, kind: &str, title: &str) -> PlanItem {
    PlanItem {
        id: id.into(),
        plan_id: plan.into(),
        ordinal: 0,
        kind: kind.into(),
        title: title.into(),
        time_local: None,
        drill_id: None,
        drill_version: None,
        target_strings: None,
        notes: String::new(),
        skipped: false,
    }
}

fn setup() -> (Repository, String) {
    let mut repo = Repository::open_in_memory().unwrap();
    repo.ensure_default_shooter(1).unwrap();
    let s = repo.active_session("default", "s1", 1, 0).unwrap();
    let mut recipe = starter_recipes().remove(0);
    recipe.mode = SourceMode::ManualEntry;
    repo.save_drill_version(&DrillVersion::first("d1", recipe, 1).unwrap()).unwrap();
    (repo, s)
}

#[test]
fn plan_agenda_order_validation_and_progress() {
    let (mut repo, s) = setup();
    assert_eq!(repo.schema_version().unwrap(), SCHEMA_VERSION);
    repo.create_plan(&plan("p1"), false).unwrap();
    let mut bad = plan("p2");
    bad.date_local = "10/10/2026".into();
    assert!(repo.create_plan(&bad, false).is_err());

    let mut drill = item("i1", "p1", "drill", "Single par");
    drill.drill_id = Some("d1".into());
    drill.drill_version = Some(1);
    drill.target_strings = Some(3);
    drill.time_local = Some("09:30".into());
    repo.add_plan_item(&drill).unwrap();
    let mut stage = item("i2", "p1", "stage", "Stage 3 walkthrough");
    stage.notes = "Start seated, gun on table".into();
    repo.add_plan_item(&stage).unwrap();
    repo.add_plan_item(&item("i3", "p1", "event", "Shooters meeting")).unwrap();

    // A drill item must reference a stored version; times must be HH:MM.
    let mut missing = item("i4", "p1", "drill", "Ghost");
    missing.drill_id = Some("d1".into());
    missing.drill_version = Some(9);
    assert!(repo.add_plan_item(&missing).is_err());
    let mut bad_time = item("i5", "p1", "event", "Lunch");
    bad_time.time_local = Some("12.30".into());
    assert!(repo.add_plan_item(&bad_time).is_err());

    repo.move_item("i3", true).unwrap();
    let ids: Vec<String> = repo.plan_items("p1").unwrap().into_iter().map(|i| i.id).collect();
    assert_eq!(ids, ["i1", "i3", "i2"]);
    repo.move_item("i1", true).unwrap(); // already first: no change
    assert_eq!(repo.plan_items("p1").unwrap()[0].id, "i1");

    // Progress counts only completed runs; aborted and in-progress runs are separate.
    let m = ManualString::parse("Club timer", 10_000_000, "1.5").unwrap();
    repo.insert_manual_run(&intent("r1", &s, SourceMode::ManualEntry, Some(drill_ref("d1", 1))), &m).unwrap();
    repo.insert_run_intent(&intent("r2", &s, SourceMode::ParOnly, None)).unwrap();
    repo.link_run("i1", "r1").unwrap();
    repo.link_run("i1", "r1").unwrap(); // idempotent
    repo.link_run("i1", "r2").unwrap();
    assert_eq!(repo.item_progress("i1").unwrap(), ItemProgress { completed: 1, aborted: 0 });
    assert!(repo.link_run("i1", "nope").is_err(), "links need a real run");

    // Skip marks the item; it never fabricates results.
    repo.set_item_skipped("i2", true).unwrap();
    assert!(repo.plan_items("p1").unwrap().iter().find(|i| i.id == "i2").unwrap().skipped);
    assert_eq!(repo.item_progress("i2").unwrap(), ItemProgress::default());

    // Deleting an item keeps its runs.
    repo.delete_item("i1").unwrap();
    assert!(repo.load_run("r1").is_ok());
    // Deleting a run removes its link.
    repo.link_run("i3", "r1").unwrap();
    repo.delete_run("r1").unwrap();
    assert_eq!(repo.item_progress("i3").unwrap(), ItemProgress::default());

    repo.archive_plan("p1").unwrap();
    assert!(repo.list_plans().unwrap().is_empty());
}

#[test]
fn checklist_template_copy_and_coach_squad() {
    let (mut repo, _) = setup();
    // No template yet: new plans get the neutral default list.
    repo.create_plan(&plan("p1"), true).unwrap();
    assert_eq!(repo.checklist(Some("p1")).unwrap().len(), default_checklist().len());
    // Editing the template affects later plans only.
    repo.add_checklist_item("t1", None, "Belt and holster").unwrap();
    repo.add_checklist_item("t2", None, "Mags loaded").unwrap();
    repo.create_plan(&plan("p2"), true).unwrap();
    let c = repo.checklist(Some("p2")).unwrap();
    assert_eq!(c.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(), ["Belt and holster", "Mags loaded"]);
    repo.set_checked(&c[0].id, true).unwrap();
    assert!(repo.checklist(Some("p2")).unwrap()[0].checked);
    assert!(!repo.checklist(None).unwrap()[0].checked, "template is unaffected");
    assert!(repo.add_checklist_item("t3", None, "   ").is_err());

    repo.add_shooter("a", "Alex", 2).unwrap();
    repo.add_shooter("b", "Blair", 3).unwrap();
    repo.set_squad(&["b".into(), "default".into(), "a".into()]).unwrap();
    assert_eq!(repo.squad().unwrap(), ["b", "default", "a"]);
    assert!(repo.set_squad(&["zzz".into()]).is_err());

    // Delete-all removes plans and their checklists but keeps the template and shooters.
    repo.delete_all_history().unwrap();
    assert!(repo.list_plans().unwrap().is_empty());
    assert!(repo.checklist(Some("p2")).unwrap().is_empty());
    assert_eq!(repo.checklist(None).unwrap().len(), 2);
    assert_eq!(repo.squad().unwrap().len(), 3);
}

#[test]
fn upgrade_from_schema_3_with_data() {
    let dir = tmpdir("upgrade");
    let p = dir.join("journal.db");
    {
        let c = rusqlite::Connection::open(&p).unwrap();
        for (_, sql) in &MIGRATIONS[..3] {
            c.execute_batch(sql).unwrap();
        }
        c.pragma_update(None, "user_version", 3).unwrap();
        c.execute_batch("INSERT INTO shooter_profile(id, name, created_utc_ms) VALUES ('default', 'Me', 1);").unwrap();
    }
    let mut repo = Repository::open(&p).unwrap();
    assert_eq!(repo.schema_version().unwrap(), SCHEMA_VERSION);
    assert!(dir.join("journal.pre-v4.bak").exists());
    repo.create_plan(&plan("p1"), true).unwrap();
    assert_eq!(repo.list_plans().unwrap().len(), 1);
}
