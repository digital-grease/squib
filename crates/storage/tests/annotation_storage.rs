//! Schema 11: manual target-photo annotations, append-only, deleted only with their run.

use squib_storage::*;
use squib_training::annotation::{Annotation, Mark, MarkKind};

fn repo(name: &str) -> Repository {
    let d = std::env::temp_dir().join(format!("squib-annot-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let mut r = Repository::open(&d.join("journal.db")).unwrap();
    r.connection()
        .execute_batch(
            "INSERT INTO shooter_profile(id, name, created_utc_ms) VALUES ('me', 'Me', 1);
             INSERT INTO session(id, shooter_id, started_utc_ms, tz_offset_min, active) VALUES ('s1', 'me', 1, 0, 1);
             INSERT INTO run(id, session_id, shooter_id, created_utc_ms, tz_offset_min, source_mode, config_json, config_hash,
               app_build, domain_schema_version, persist_state, outcome)
               VALUES ('r1','s1','me',5,0,'par_only','{}','h','t',1,'saved','complete');
             INSERT INTO attachment(id, run_id, kind, relative_path, sha256, bytes, mime, metadata_stripped, created_utc_ms)
               VALUES ('ph1', 'r1', 'photo', 'photos/a.jpg', 'aa', 3, 'image/jpeg', 1, 6),
                      ('v1', 'r1', 'video', 'videos/a.mp4', 'bb', 3, 'video/mp4', 1, 6);",
        )
        .unwrap();
    r
}

fn note(n: &str) -> Annotation {
    Annotation { marks: vec![Mark { x: 0.4, y: 0.6, kind: MarkKind::Hit, label: Some("A".into()) }], note: n.into() }
}

#[test]
fn versions_append_detect_concurrent_edits_and_are_immutable() {
    let mut r = repo("versions");
    assert!(r.latest_photo_annotation("ph1").unwrap().is_none());
    let a1 = r.save_photo_annotation("a1", "ph1", None, &note("first"), 10).unwrap();
    assert_eq!((a1.number, a1.run_id.as_str(), a1.run_revision, a1.score_revision), (1, "r1", None, None));
    // An edit based on version 1 makes version 2; a second edit still based on 1 conflicts.
    let a2 = r.save_photo_annotation("a2", "ph1", Some(1), &note("second"), 20).unwrap();
    assert_eq!(a2.number, 2);
    assert!(matches!(r.save_photo_annotation("a3", "ph1", Some(1), &note("stale"), 30), Err(StorageError::Conflict(_))));
    assert_eq!(r.latest_photo_annotation("ph1").unwrap().unwrap().annotation.note, "second");
    // Only photos, and nothing for unknown attachments.
    assert!(matches!(r.save_photo_annotation("x", "v1", None, &note(""), 1), Err(StorageError::Conflict(_))));
    assert!(matches!(r.save_photo_annotation("x", "nope", None, &note(""), 1), Err(StorageError::NotFound(_))));
    // Earlier versions cannot be changed or removed outside the guarded deletion path.
    assert!(r.connection().execute("UPDATE photo_annotation SET content_json = '{}' WHERE id = 'a1'", []).is_err());
    assert!(r.connection().execute("DELETE FROM photo_annotation WHERE id = 'a1'", []).is_err());
    // Deleting the run removes its photos and their annotations.
    r.delete_run("r1").unwrap();
    assert!(r.latest_photo_annotation("ph1").unwrap().is_none());
    assert_eq!(r.schema_version().unwrap(), SCHEMA_VERSION);
}
