//! M4 engine flows: day plans, agenda progress from runs, checklists, coach rotation.

use std::sync::Arc;
use std::time::Duration;

use squib_mobile::*;

const NOW: i64 = 1_791_413_400_000;

fn engine(name: &str) -> Arc<SquibEngine> {
    let d = std::env::temp_dir().join(format!("squib-plan-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    SquibEngine::new(d.join("journal.db").to_string_lossy().into(), NOW).unwrap()
}

fn arm_req(item: Option<String>) -> ArmRequest {
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
        plan_item_id: item,
        video: false,
        diagnostic_root: None,
    }
}

fn input(kind: &str, title: &str) -> PlanItemInput {
    PlanItemInput {
        kind: kind.into(),
        title: title.into(),
        time_local: None,
        drill_id: None,
        drill_version: None,
        target_strings: None,
        notes: String::new(),
    }
}

#[test]
fn agenda_progress_comes_from_linked_runs_only() {
    let e = engine("agenda");
    let plan = e.create_plan("Saturday practice".into(), "2026-10-10".into(), "practice".into(), true, NOW).unwrap();
    let drill = e.list_drills().into_iter().find(|d| d.input.title == "Timed string with score").unwrap();
    let item = e
        .add_plan_item(
            plan.clone(),
            PlanItemInput {
                drill_id: Some(drill.drill_id.clone()),
                drill_version: Some(drill.version),
                target_strings: Some(2),
                time_local: Some("09:30".into()),
                ..input("drill", "")
            },
        )
        .unwrap();
    let stage = e
        .add_plan_item(
            plan.clone(),
            PlanItemInput { time_local: Some("10:00".into()), notes: "Start seated".into(), ..input("stage", "Stage 2") },
        )
        .unwrap();

    let v = e.load_plan(plan.clone(), Some(9 * 60)).unwrap();
    assert_eq!(v.items[0].title, "Timed string with score", "drill title is the default");
    assert!(!v.checklist.is_empty(), "checklist copied from the default template");
    let next = v.next_up.unwrap();
    assert_eq!((next.item_id.as_str(), next.minutes_until), (item.as_str(), 30));

    // A cancelled timed attempt is shown as aborted, never as done.
    assert!(e.arm("a0".into(), arm_req(Some("missing".into())), 1_000_000_000).is_err(), "unknown item refused");
    e.arm("a1".into(), arm_req(Some(item.clone())), 1_000_000_000).unwrap();
    e.cancel("c1".into(), 1_100_000_000).unwrap();
    let mut aborted = 0;
    for _ in 0..500 {
        e.poll(1_200_000_000);
        aborted = e.load_plan(plan.clone(), None).unwrap().items[0].aborted;
        if aborted == 1 {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(aborted, 1);
    e.reset().ok();

    // Manual entries count once linked.
    for t in ["1.1 1.6 2.0", "1.0 1.5 1.9"] {
        let r = e
            .add_manual_run(
                "Club timer".into(),
                10,
                t.into(),
                Some(drill.drill_id.clone()),
                Some(drill.version),
                NOW,
                0,
                "t".into(),
            )
            .unwrap();
        e.link_run_to_item(item.clone(), r).unwrap();
    }
    let v = e.load_plan(plan.clone(), Some(9 * 60 + 40)).unwrap();
    assert_eq!((v.items[0].completed, v.items[0].aborted, v.items[0].done), (2, 1, true));
    assert_eq!(v.next_up.unwrap().item_id, stage, "finished items drop out of next up");
    assert_eq!(e.list_plans().unwrap()[0].done_items, 1);

    // Skipping fabricates nothing.
    e.set_item_skipped(stage.clone(), true).unwrap();
    let v = e.load_plan(plan.clone(), None).unwrap();
    assert!(v.items[1].skipped && v.items[1].completed == 0 && !v.items[1].done);

    e.move_plan_item(stage.clone(), true).unwrap();
    assert_eq!(e.load_plan(plan.clone(), None).unwrap().items[0].id, stage);
    e.add_checklist_item(Some(plan.clone()), "Spare batteries".into()).unwrap();
    let c = e.load_plan(plan.clone(), None).unwrap().checklist;
    e.set_checklist_checked(c.last().unwrap().id.clone(), true).unwrap();
    assert!(e.load_plan(plan.clone(), None).unwrap().checklist.last().unwrap().checked);
    assert!(e.checklist_template().unwrap().is_empty(), "plan edits do not touch the template");
    assert!(e.add_plan_item(plan, PlanItemInput { time_local: Some("25:00".into()), ..input("event", "Lunch") }).is_err());
}

#[test]
fn coach_rotation_advances_and_runs_show_shooter() {
    let e = engine("coach");
    let me = e.list_shooters()[0].clone();
    let a = e.add_shooter("Alex".into(), NOW).unwrap();
    let b = e.add_shooter("Blair".into(), NOW).unwrap();
    assert!(e.coach().next.is_none(), "rotation is off by default");
    let c = e.set_coach(true, vec![a.id.clone(), b.id.clone(), me.id.clone()]).unwrap();
    assert_eq!(c.squad.len(), 3);
    assert_eq!(c.next.unwrap().id, a.id);
    assert!(e.set_coach(true, vec!["ghost".into()]).is_err());

    e.arm("a1".into(), arm_req(None), 1_000_000_000).unwrap();
    assert!(e.advance_shooter().is_err(), "no switching while a run is active");
    e.cancel("c1".into(), 1_100_000_000).unwrap();
    for _ in 0..500 {
        if e.poll(1_200_000_000).persist.as_deref() == Some("saved") {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(e.list_runs(5).unwrap()[0].shooter_name, me.name);
    e.reset().ok();
    let c = e.advance_shooter().unwrap();
    assert!(e.list_shooters().iter().any(|s| s.id == a.id && s.active));
    assert_eq!(c.next.unwrap().id, b.id);
    e.advance_shooter().unwrap();
    assert_eq!(e.advance_shooter().unwrap().next.unwrap().id, a.id, "wraps around");
}
