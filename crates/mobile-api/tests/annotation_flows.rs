//! Plan phase 4.2: manual marks on a run's target photo, versioned and linked to the run's
//! review and score revisions; kept in backups, left out of shared results.

use std::sync::Arc;

use squib_mobile::*;

const NOW: i64 = 1_791_694_800_000;

fn engine(name: &str) -> (Arc<SquibEngine>, std::path::PathBuf) {
    let d = std::env::temp_dir().join(format!("squib-annot-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("photos")).unwrap();
    (SquibEngine::new(d.join("journal.db").to_string_lossy().into(), NOW).unwrap(), d)
}

fn mark(x: f64, y: f64, kind: &str) -> MarkView {
    MarkView { x, y, kind: kind.into(), label: None }
}

#[test]
fn marks_are_versioned_linked_to_revisions_backed_up_and_never_shared() {
    let (e, d) = engine("flow");
    let run = e.add_manual_run("Club timer".into(), 10, "1.0 1.5 2.0".into(), None, None, NOW, 0, "t".into()).unwrap();
    std::fs::write(d.join("photos/t1.jpg"), b"jpeg bytes").unwrap();
    let root = d.to_string_lossy().to_string();
    let photo = e.register_attachment(run.clone(), root.clone(), "photos/t1.jpg".into(), "image/jpeg".into(), true, NOW).unwrap();

    let empty = e.photo_annotation(photo.id.clone()).unwrap();
    assert_eq!((empty.number, empty.marks.len()), (None, 0));
    assert_eq!(empty.current_run_revision, Some(1));

    let v1 = e
        .save_photo_annotation(photo.id.clone(), None, vec![mark(0.5, 0.5, "hit"), mark(0.9, 0.1, "miss")], "two in".into(), NOW)
        .unwrap();
    assert_eq!((v1.number, v1.hits, v1.misses, v1.run_revision), (Some(1), 1, 1, Some(1)));
    // Bad input is refused, and an edit based on an old version does not overwrite a newer one.
    assert!(e.save_photo_annotation(photo.id.clone(), Some(1), vec![mark(1.5, 0.5, "hit")], "".into(), NOW).is_err());
    assert!(e.save_photo_annotation(photo.id.clone(), Some(1), vec![mark(0.5, 0.5, "bullseye")], "".into(), NOW).is_err());
    let v2 = e.save_photo_annotation(photo.id.clone(), Some(1), vec![mark(0.5, 0.5, "hit")], "one".into(), NOW + 1).unwrap();
    assert_eq!(v2.number, Some(2));
    assert!(e.save_photo_annotation(photo.id.clone(), Some(1), vec![], "stale".into(), NOW + 2).is_err());

    // A later review change shows that the marks belong to an earlier revision.
    let r = e.load_review(run.clone()).unwrap();
    e.apply_review(run.clone(), r.revision_number, vec![ReviewAction::AddManual { timeline_ns: 2_500_000_000 }], None, NOW + 3)
        .unwrap();
    let now = e.photo_annotation(photo.id.clone()).unwrap();
    assert_eq!((now.run_revision, now.current_run_revision), (Some(1), Some(2)));

    // Backups keep the marks; shared results leave them out.
    let zip = d.join("b.zip").to_string_lossy().to_string();
    e.export_backup(zip.clone(), root.clone(), true, false, "t".into(), NOW).unwrap();
    let (fresh, fd) = engine("restore");
    fresh.import_backup(zip, fd.to_string_lossy().into(), None).unwrap();
    let restored = fresh.photo_annotation(photo.id.clone()).unwrap();
    assert_eq!((restored.number, restored.note.as_str(), restored.hits), (Some(2), "one", 1));
    // Import as a copy (here into the same phone): the copy's photo carries the marks under
    // new ids, and the original is untouched.
    let copied = e.import_backup_copy(d.join("b.zip").to_string_lossy().to_string(), root.clone(), None, NOW + 5).unwrap();
    assert_eq!(copied.runs_new, 1);
    let copy_run = e.list_runs(10).unwrap().into_iter().find(|x| x.run_id != run).unwrap().run_id;
    let copy_photo = e.run_attachments(copy_run).remove(0);
    assert_ne!(copy_photo.id, photo.id);
    let c = e.photo_annotation(copy_photo.id).unwrap();
    assert_eq!((c.number, c.note.as_str()), (Some(2), "one"));
    assert_eq!(e.photo_annotation(photo.id.clone()).unwrap().number, Some(2));
    let shared = e.share_results(vec![run]).unwrap();
    assert!(!shared.contains("\"marks\"") && !shared.contains("one\"") && !shared.contains("photo"), "{shared}");
}
