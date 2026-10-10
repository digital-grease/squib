//! Opt-in diagnostic recordings (M5 experiment 3): one bounded WAV per run, written
//! only when the user turned it on for that run, plus the metadata that keeps every
//! file sample aligned with the run's epoch frames.

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::{AttachmentRecord, Repository, Result, StorageError, valid_attachment_path};

pub const DIAGNOSTIC_KIND: &str = "diagnostic_audio";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticRecord {
    pub attachment: AttachmentRecord,
    pub run_id: String,
    pub epoch_id: Option<String>,
    pub sample_rate_hz: u32,
    /// Epoch frame of file frame 0.
    pub first_epoch_frame: i64,
    pub frames: i64,
    /// `[start, end)` epoch frames written as silence because they were not available.
    pub gaps: Vec<(i64, i64)>,
    /// The recording reached its length limit before the run ended.
    pub truncated: bool,
    /// Frames the recorder dropped because its writer fell behind (also listed as gaps).
    pub dropped_frames: i64,
    pub created_utc_ms: i64,
}

impl Repository {
    pub fn insert_diagnostic(&mut self, d: &DiagnosticRecord) -> Result<()> {
        let a = &d.attachment;
        if a.kind != DIAGNOSTIC_KIND || a.run_id.as_deref() != Some(d.run_id.as_str()) {
            return Err(StorageError::Conflict("diagnostic attachment must be diagnostic audio of the same run".into()));
        }
        if !valid_attachment_path(&a.relative_path) {
            return Err(StorageError::Conflict("attachment path must be app-relative".into()));
        }
        let gaps = serde_json::to_string(&d.gaps)?;
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO attachment(id, run_id, kind, relative_path, sha256, bytes, mime, metadata_stripped, created_utc_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![a.id, a.run_id, a.kind, a.relative_path, a.sha256, a.bytes, a.mime, a.metadata_stripped, a.created_utc_ms],
        )?;
        tx.execute(
            "INSERT INTO diagnostic_recording(attachment_id, run_id, epoch_id, sample_rate_hz, first_epoch_frame, frames,
               gaps_json, truncated, dropped_frames, created_utc_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                a.id,
                d.run_id,
                d.epoch_id,
                d.sample_rate_hz,
                d.first_epoch_frame,
                d.frames,
                gaps,
                d.truncated,
                d.dropped_frames,
                d.created_utc_ms
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn diagnostic(&self, run_id: &str) -> Result<Option<DiagnosticRecord>> {
        let row = self
            .conn
            .query_row(
                "SELECT a.id, a.run_id, a.kind, a.relative_path, a.sha256, a.bytes, a.mime, a.metadata_stripped, a.created_utc_ms,
                        d.epoch_id, d.sample_rate_hz, d.first_epoch_frame, d.frames, d.gaps_json, d.truncated, d.dropped_frames,
                        d.created_utc_ms
                 FROM diagnostic_recording d JOIN attachment a ON a.id = d.attachment_id WHERE d.run_id = ?1",
                params![run_id],
                |r| {
                    Ok((
                        AttachmentRecord {
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
                        r.get::<_, Option<String>>(9)?,
                        r.get::<_, u32>(10)?,
                        r.get::<_, i64>(11)?,
                        r.get::<_, i64>(12)?,
                        r.get::<_, String>(13)?,
                        r.get::<_, bool>(14)?,
                        r.get::<_, i64>(15)?,
                        r.get::<_, i64>(16)?,
                    ))
                },
            )
            .optional()?;
        let Some((attachment, epoch_id, sample_rate_hz, first_epoch_frame, frames, gaps, truncated, dropped_frames, created)) =
            row
        else {
            return Ok(None);
        };
        Ok(Some(DiagnosticRecord {
            run_id: run_id.to_string(),
            attachment,
            epoch_id,
            sample_rate_hz,
            first_epoch_frame,
            frames,
            gaps: serde_json::from_str(&gaps)?,
            truncated,
            dropped_frames,
            created_utc_ms: created,
        }))
    }

    /// Delete a run's diagnostic recording rows; the run itself is kept. Returns the
    /// file path for the app to remove.
    pub fn delete_diagnostic(&mut self, run_id: &str) -> Result<Option<String>> {
        let Some(d) = self.diagnostic(run_id)? else { return Ok(None) };
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM diagnostic_recording WHERE run_id = ?1", params![run_id])?;
        tx.execute("DELETE FROM attachment WHERE id = ?1", params![d.attachment.id])?;
        tx.commit()?;
        Ok(Some(d.attachment.relative_path))
    }
}
