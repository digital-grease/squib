//! Plan phase 1.3 engine flows: repeated-string set results for a visit and a plan item.

use std::sync::Arc;

use squib_mobile::*;

const NOW: i64 = 1_791_413_400_000;

fn engine(name: &str) -> Arc<SquibEngine> {
    let d = std::env::temp_dir().join(format!("squib-sets-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    SquibEngine::new(d.join("journal.db").to_string_lossy().into(), NOW).unwrap()
}

fn drill_input(set: Option<&str>, keep: Option<u32>, profile: &str) -> DrillInput {
    DrillInput {
        title: "Three strings".into(),
        description: String::new(),
        mode: Mode::ParOnly,
        manual: true,
        delay: StartDelay::Instant,
        expected_count: None,
        pars_ms: vec![],
        repeats: 3,
        rest_s: 0,
        scoring_profile_id: profile.into(),
        notes: String::new(),
        equipment_tags: vec![],
        set_result: set.map(String::from),
        keep_best: keep,
    }
}

#[test]
fn set_results_follow_the_drill_rule_and_never_total_short_sets() {
    let e = engine("sets");
    assert!(e.create_drill(drill_input(Some("sum"), None, "generic-points-hf"), NOW).is_err(), "hit factors are not totalled");
    assert!(e.create_drill(drill_input(Some("sum"), Some(5), "generic-time"), NOW).is_err(), "keep best above the string count");
    let d = e.create_drill(drill_input(Some("sum"), Some(2), "generic-time"), NOW).unwrap();
    assert_eq!((d.input.set_result.as_deref(), d.input.keep_best), (Some("sum"), Some(2)));

    let run = |t: &str, at: i64| {
        e.add_manual_run("Club timer".into(), 10, t.into(), Some(d.drill_id.clone()), Some(d.version), at, 0, "t".into()).unwrap()
    };
    let r1 = run("1.0 2.00", NOW);
    run("1.0 3.00", NOW + 1);
    let session = e.list_sessions(5).unwrap()[0].id.clone();
    let s = &e.session_sets(session.clone()).unwrap()[0];
    assert_eq!((s.status.as_str(), s.value, s.scored), ("incomplete", None, 2), "2 of 3 is not a set result");
    assert_eq!(s.reason.as_deref(), Some("2 of 3 strings completed and scored"));

    run("1.0 2.50", NOW + 2);
    let s = &e.session_sets(session).unwrap()[0];
    assert_eq!(s.status, "complete");
    assert_eq!(s.label, "Total of the best 2 of 3 strings");
    assert!((s.value.unwrap() - 4.5).abs() < 1e-9, "{s:?}");
    assert_eq!(s.strings.iter().filter(|x| x.dropped).count(), 1);
    assert_eq!(s.metric, "seconds");

    // The same rule applies to a day-plan drill item's linked runs.
    let plan = e.create_plan("Practice".into(), "2026-10-10".into(), "practice".into(), false, NOW).unwrap();
    let item = e
        .add_plan_item(
            plan,
            PlanItemInput {
                kind: "drill".into(),
                title: String::new(),
                time_local: None,
                drill_id: Some(d.drill_id.clone()),
                drill_version: Some(d.version),
                target_strings: Some(3),
                notes: String::new(),
            },
        )
        .unwrap();
    e.link_run_to_item(item.clone(), r1).unwrap();
    let p = e.plan_item_set(item).unwrap().unwrap();
    assert_eq!((p.status.as_str(), p.scored), ("incomplete", 1));

    // A drill without a rule has no set result.
    let plain = e.create_drill(drill_input(None, None, "generic-time"), NOW).unwrap();
    e.add_manual_run("Club timer".into(), 10, "1.5".into(), Some(plain.drill_id), Some(plain.version), NOW + 3, 0, "t".into())
        .unwrap();
    let all = e.session_sets(e.list_sessions(5).unwrap()[0].id.clone()).unwrap();
    assert_eq!(all.len(), 1);
}
