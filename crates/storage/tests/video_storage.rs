//! Schema 5: video clips as attachments with raw clock observations, upgrade from 4.

use std::path::PathBuf;

use squib_storage::*;

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("squib-videostore-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn clip(id: &str, run: &str, path: &str) -> VideoClipRecord {
    VideoClipRecord {
        attachment: AttachmentRecord {
            id: id.into(),
            run_id: Some(run.into()),
            kind: "video".into(),
            relative_path: path.into(),
            sha256: "00".repeat(32),
            bytes: 10,
            mime: "video/mp4".into(),
            metadata_stripped: true,
            created_utc_ms: 9,
        },
        run_id: run.into(),
        width: 1280,
        height: 720,
        frame_rate_milli: 30_000,
        duration_ms: 5_000,
        first_frame_camera_ns: 1,
        camera_clock: "realtime".into(),
        probe_camera_ns: 1,
        probe_mono_ns: 2,
        probe_boot_ns: 3,
        start_boot_minus_mono_ns: 1,
        end_boot_minus_mono_ns: 1,
        created_utc_ms: 9,
    }
}

#[test]
fn upgrade_from_schema_4_keeps_photos_and_accepts_videos() {
    let dir = tmpdir("upgrade");
    let p = dir.join("journal.db");
    {
        let c = rusqlite::Connection::open(&p).unwrap();
        for (_, sql) in &MIGRATIONS[..4] {
            c.execute_batch(sql).unwrap();
        }
        c.pragma_update(None, "user_version", 4).unwrap();
        c.execute_batch(
            "INSERT INTO shooter_profile(id, name, created_utc_ms) VALUES ('default', 'Me', 1);
             INSERT INTO session(id, shooter_id, started_utc_ms, tz_offset_min, active) VALUES ('s1', 'default', 1, 0, 1);
             INSERT INTO run(id, session_id, shooter_id, created_utc_ms, tz_offset_min, source_mode, config_json, config_hash,
               app_build, domain_schema_version, persist_state, outcome, start_ref_mono_ns)
               VALUES ('r1','s1','default',5,0,'par_only','{}','h','t',1,'saved','complete', 777);
             INSERT INTO attachment(id, run_id, kind, relative_path, sha256, bytes, mime, metadata_stripped, created_utc_ms)
               VALUES ('ph1', 'r1', 'photo', 'photos/a.jpg', 'aa', 3, 'image/jpeg', 1, 6);",
        )
        .unwrap();
        // Video kinds were not allowed before schema 5.
        assert!(
            c.execute_batch(
                "INSERT INTO attachment(id, run_id, kind, relative_path, sha256, bytes, mime, metadata_stripped, created_utc_ms)
                 VALUES ('v0', 'r1', 'video', 'videos/x.mp4', 'aa', 3, 'video/mp4', 1, 6);"
            )
            .is_err()
        );
    }
    let mut repo = Repository::open(&p).unwrap();
    assert_eq!(repo.schema_version().unwrap(), 5);
    assert!(dir.join("journal.pre-v5.bak").exists());
    let photos = repo.list_attachments(Some("r1")).unwrap();
    assert_eq!((photos.len(), photos[0].id.as_str(), photos[0].kind.as_str()), (1, "ph1", "photo"));
    assert_eq!(repo.start_ref_mono_ns("r1").unwrap(), Some(777));

    repo.insert_video_clip(&clip("v1", "r1", "videos/a.mp4")).unwrap();
    assert_eq!(repo.video_clips("r1").unwrap(), vec![clip("v1", "r1", "videos/a.mp4")]);
    assert!(repo.insert_video_clip(&clip("v2", "r1", "../escape.mp4")).is_err(), "paths stay app-relative");
    let mut wrong = clip("v3", "r1", "videos/c.mp4");
    wrong.attachment.kind = "photo".into();
    assert!(repo.insert_video_clip(&wrong).is_err());

    let d = repo.delete_run("r1").unwrap();
    assert_eq!(d.attachment_paths.len(), 2);
    assert!(repo.video_clips("r1").unwrap().is_empty());
}
