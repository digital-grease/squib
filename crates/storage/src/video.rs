//! Video clips recorded with runs (M4 proof of concept). A clip is an attachment of
//! kind `video` plus the raw clock observations needed to place it on the run timeline.

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::{AttachmentRecord, Repository, Result, StorageError, valid_attachment_path};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoClipRecord {
    pub attachment: AttachmentRecord,
    pub run_id: String,
    pub width: u32,
    pub height: u32,
    pub frame_rate_milli: u32,
    pub duration_ms: i64,
    pub first_frame_camera_ns: i64,
    /// `realtime` or `unknown` (Camera2 timestamp source).
    pub camera_clock: String,
    pub probe_camera_ns: i64,
    pub probe_mono_ns: i64,
    pub probe_boot_ns: i64,
    pub start_boot_minus_mono_ns: i64,
    pub end_boot_minus_mono_ns: i64,
    pub created_utc_ms: i64,
}

impl Repository {
    /// Store the clip's attachment row and clock observations together.
    pub fn insert_video_clip(&mut self, v: &VideoClipRecord) -> Result<()> {
        let a = &v.attachment;
        if a.kind != "video" || a.run_id.as_deref() != Some(v.run_id.as_str()) {
            return Err(StorageError::Conflict("video clip attachment must be a video of the same run".into()));
        }
        if !valid_attachment_path(&a.relative_path) {
            return Err(StorageError::Conflict("attachment path must be app-relative".into()));
        }
        if !matches!(v.camera_clock.as_str(), "realtime" | "unknown") {
            return Err(StorageError::Conflict("camera clock must be realtime or unknown".into()));
        }
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO attachment(id, run_id, kind, relative_path, sha256, bytes, mime, metadata_stripped, created_utc_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![a.id, a.run_id, a.kind, a.relative_path, a.sha256, a.bytes, a.mime, a.metadata_stripped, a.created_utc_ms],
        )?;
        tx.execute(
            "INSERT INTO video_clip(attachment_id, run_id, width, height, frame_rate_milli, duration_ms, first_frame_camera_ns,
               camera_clock, probe_camera_ns, probe_mono_ns, probe_boot_ns, start_boot_minus_mono_ns, end_boot_minus_mono_ns,
               created_utc_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                a.id,
                v.run_id,
                v.width,
                v.height,
                v.frame_rate_milli,
                v.duration_ms,
                v.first_frame_camera_ns,
                v.camera_clock,
                v.probe_camera_ns,
                v.probe_mono_ns,
                v.probe_boot_ns,
                v.start_boot_minus_mono_ns,
                v.end_boot_minus_mono_ns,
                v.created_utc_ms
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn video_clips(&self, run_id: &str) -> Result<Vec<VideoClipRecord>> {
        let mut st = self.conn.prepare(
            "SELECT a.id, a.run_id, a.kind, a.relative_path, a.sha256, a.bytes, a.mime, a.metadata_stripped, a.created_utc_ms,
                    v.run_id, v.width, v.height, v.frame_rate_milli, v.duration_ms, v.first_frame_camera_ns, v.camera_clock,
                    v.probe_camera_ns, v.probe_mono_ns, v.probe_boot_ns, v.start_boot_minus_mono_ns, v.end_boot_minus_mono_ns,
                    v.created_utc_ms
             FROM video_clip v JOIN attachment a ON a.id = v.attachment_id WHERE v.run_id = ?1 ORDER BY v.created_utc_ms",
        )?;
        let rows = st.query_map(params![run_id], |r| {
            Ok(VideoClipRecord {
                attachment: AttachmentRecord {
                    id: r.get(0)?,
                    run_id: r.get(1)?,
                    kind: r.get(2)?,
                    relative_path: r.get(3)?,
                    sha256: r.get(4)?,
                    bytes: r.get(5)?,
                    mime: r.get(6)?,
                    metadata_stripped: r.get(7)?,
                    created_utc_ms: r.get(8)?,
                },
                run_id: r.get(9)?,
                width: r.get(10)?,
                height: r.get(11)?,
                frame_rate_milli: r.get(12)?,
                duration_ms: r.get(13)?,
                first_frame_camera_ns: r.get(14)?,
                camera_clock: r.get(15)?,
                probe_camera_ns: r.get(16)?,
                probe_mono_ns: r.get(17)?,
                probe_boot_ns: r.get(18)?,
                start_boot_minus_mono_ns: r.get(19)?,
                end_boot_minus_mono_ns: r.get(20)?,
                created_utc_ms: r.get(21)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// The run's resolved start reference on the monotonic clock, if any.
    pub fn start_ref_mono_ns(&self, run_id: &str) -> Result<Option<i64>> {
        Ok(self
            .conn
            .query_row("SELECT start_ref_mono_ns FROM run WHERE id = ?1", params![run_id], |r| r.get::<_, Option<i64>>(0))
            .optional()?
            .flatten())
    }
}
