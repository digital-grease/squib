//! Plan phase 1.5 engine flows. An older backup of the same history restores without
//! conflict (edits are appended). Two phones restored from one backup and then edited
//! separately do conflict (each made a different revision 2); that backup can be
//! imported as a copy without touching local data.

use std::sync::Arc;

use squib_mobile::*;

const NOW: i64 = 1_791_413_400_000;

fn engine(name: &str) -> (Arc<SquibEngine>, std::path::PathBuf) {
    let d = std::env::temp_dir().join(format!("squib-copy-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    (SquibEngine::new(d.join("journal.db").to_string_lossy().into(), NOW).unwrap(), d)
}

fn add_shot(e: &SquibEngine, run: &str, at_ns: i64) {
    let r = e.load_review(run.into()).unwrap();
    e.apply_review(run.into(), r.revision_number, vec![ReviewAction::AddManual { timeline_ns: at_ns }], None, NOW + 1).unwrap();
}

#[test]
fn older_backups_merge_and_diverged_ones_import_as_a_copy() {
    let (a, da) = engine("phone-a");
    let run = a.add_manual_run("Club timer".into(), 10, "1.0 1.5".into(), None, None, NOW, 0, "t".into()).unwrap();
    let zip = da.join("shared.zip").to_string_lossy().to_string();
    a.export_backup(zip.clone(), da.to_string_lossy().into(), true, false, "t".into(), NOW, None).unwrap();

    // Same lineage: a review made after the backup is appended, so restoring the older
    // backup has nothing to conflict with.
    add_shot(&a, &run, 1_800_000_000);
    assert!(a.preview_import(zip.clone(), da.to_string_lossy().into(), None).unwrap().conflicts.is_empty());

    // Phone B restores the same backup and reviews the same run differently.
    let (b, db) = engine("phone-b");
    b.import_backup(zip, db.to_string_lossy().into(), None, None).unwrap();
    add_shot(&b, &run, 2_400_000_000);
    let from_b = db.join("b.zip").to_string_lossy().to_string();
    b.export_backup(from_b.clone(), db.to_string_lossy().into(), true, false, "t".into(), NOW, None).unwrap();

    let root = da.to_string_lossy().to_string();
    let p = a.preview_import(from_b.clone(), root.clone(), None).unwrap();
    assert!(p.conflicts.iter().any(|c| c.starts_with("run_revision")), "{:?}", p.conflicts);
    assert!(a.import_backup(from_b.clone(), root.clone(), None, None).is_err());

    let pc = a.preview_import_copy(from_b.clone(), root.clone(), NOW + 2, None).unwrap();
    assert!(pc.conflicts.is_empty());
    let v = a.import_backup_copy(from_b, root, None, NOW + 2, None).unwrap();
    assert_eq!(v.runs_new, 1);
    // Phone A's own review is untouched; the copy carries phone B's.
    let runs = a.list_runs(10).unwrap();
    assert_eq!(runs.len(), 2);
    let mine = a.load_review(run.clone()).unwrap();
    assert!(mine.events.iter().any(|e| e.timeline_ns == 1_800_000_000));
    let copy = a.load_review(runs.iter().find(|x| x.run_id != run).unwrap().run_id.clone()).unwrap();
    assert!(copy.events.iter().any(|e| e.timeline_ns == 2_400_000_000));
    assert!(!copy.events.iter().any(|e| e.timeline_ns == 1_800_000_000));
}
