//! Plan phase 1.1 engine flows: range visits with totals, saved cost, notes, end, delete.

use std::sync::Arc;

use squib_mobile::*;

const NOW: i64 = 1_791_413_400_000;
const H: i64 = 3_600_000;

fn engine(name: &str) -> Arc<SquibEngine> {
    let d = std::env::temp_dir().join(format!("squib-sess-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    SquibEngine::new(d.join("journal.db").to_string_lossy().into(), NOW).unwrap()
}

fn manual(e: &SquibEngine, at: i64) -> String {
    e.add_manual_run("Club timer".into(), 10, "1.0 1.5 2.0".into(), None, None, at, 0, "t".into()).unwrap()
}

#[test]
fn visits_group_runs_with_totals_cost_notes_and_deletion() {
    let e = engine("visits");
    e.set_round_cost(Some(30), Some("usd".into())).unwrap();
    let r1 = manual(&e, NOW);
    let r2 = manual(&e, NOW + H);
    e.confirm_rounds(r1.clone(), 3, NOW).unwrap();
    e.confirm_rounds(r2.clone(), 4, NOW).unwrap();
    // A later visit, after the price changed.
    e.set_round_cost(Some(50), Some("usd".into())).unwrap();
    let r3 = manual(&e, NOW + 9 * H);
    e.confirm_rounds(r3.clone(), 10, NOW).unwrap();

    let s = e.list_sessions(10).unwrap();
    assert_eq!(s.len(), 2);
    let (newest, first) = (&s[0], &s[1]);
    assert_eq!((newest.run_count, newest.confirmed_rounds, newest.cost_minor), (1, 10, Some(500)));
    assert_eq!((first.run_count, first.confirmed_rounds, first.cost_minor), (2, 7, Some(210)), "old visit keeps its old price");
    assert!(!first.active && first.ended_utc_ms == Some(NOW + H));
    assert_eq!(first.shooter_name, "Me");
    let runs: Vec<String> = e.session_runs(first.id.clone()).unwrap().into_iter().map(|r| r.run_id).collect();
    assert_eq!(runs, vec![r2.clone(), r1.clone()]);

    e.set_session_notes(first.id.clone(), "Outdoor, windy".into()).unwrap();
    e.set_session_cost(first.id.clone(), Some(40), Some("usd".into())).unwrap();
    assert!(e.set_session_cost(first.id.clone(), Some(40), Some("dollars".into())).is_err());
    let s = e.list_sessions(10).unwrap();
    let first = s.iter().find(|x| x.id == first.id).unwrap();
    assert_eq!((first.notes.as_str(), first.cost_minor, first.per_round_minor), ("Outdoor, windy", Some(280), Some(40)));

    // Ending the current visit makes the next run start a new one.
    e.end_session(newest.id.clone(), NOW + 9 * H + 1).unwrap();
    manual(&e, NOW + 9 * H + 2);
    assert_eq!(e.list_sessions(10).unwrap().len(), 3);

    let d = e.delete_session(first.id.clone()).unwrap();
    assert_eq!(d.runs, 2);
    assert!(e.load_review(r1).is_err() && e.load_review(r3).is_ok());
    assert_eq!(e.list_sessions(10).unwrap().len(), 2);
}
