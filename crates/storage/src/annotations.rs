//! Manual target-photo annotations (plan phase 4.2, ADR-053). Versions are append-only;
//! each records the run's review and score revisions at the time it was saved.

use rusqlite::{OptionalExtension, params};
use squib_training::annotation::Annotation;

use crate::{Repository, Result, StorageError};

#[derive(Debug, Clone, PartialEq)]
pub struct PhotoAnnotation {
    pub id: String,
    pub attachment_id: String,
    pub run_id: String,
    pub number: u32,
    pub run_revision: Option<u32>,
    pub score_revision: Option<u32>,
    pub annotation: Annotation,
    pub created_utc_ms: i64,
}

impl Repository {
    /// Latest annotation version of a photo, if any.
    pub fn latest_photo_annotation(&self, attachment_id: &str) -> Result<Option<PhotoAnnotation>> {
        let row = self
            .conn
            .query_row(
                "SELECT id, attachment_id, run_id, number, run_revision, score_revision, content_json, created_utc_ms
                 FROM photo_annotation WHERE attachment_id = ?1 ORDER BY number DESC LIMIT 1",
                params![attachment_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, u32>(3)?,
                        r.get::<_, Option<u32>>(4)?,
                        r.get::<_, Option<u32>>(5)?,
                        r.get::<_, String>(6)?,
                        r.get::<_, i64>(7)?,
                    ))
                },
            )
            .optional()?;
        let Some((id, attachment_id, run_id, number, run_revision, score_revision, json, created_utc_ms)) = row else {
            return Ok(None);
        };
        let annotation: Annotation = serde_json::from_str(&json)?;
        Ok(Some(PhotoAnnotation { id, attachment_id, run_id, number, run_revision, score_revision, annotation, created_utc_ms }))
    }

    /// Save a new version for a photo. `base_number` is the version the edit started from
    /// (`None` for the first); a different latest version means someone else saved first.
    pub fn save_photo_annotation(
        &mut self,
        id: &str,
        attachment_id: &str,
        base_number: Option<u32>,
        annotation: &Annotation,
        now_utc_ms: i64,
    ) -> Result<PhotoAnnotation> {
        let tx = self.conn.transaction()?;
        let (run_id, kind): (Option<String>, String) = tx
            .query_row("SELECT run_id, kind FROM attachment WHERE id = ?1", params![attachment_id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?
            .ok_or_else(|| StorageError::NotFound(format!("photo {attachment_id}")))?;
        if kind != "photo" {
            return Err(StorageError::Conflict("only photos can be annotated".into()));
        }
        let run_id = run_id.ok_or_else(|| StorageError::Conflict("the photo is not attached to a run".into()))?;
        let latest: Option<u32> =
            tx.query_row("SELECT MAX(number) FROM photo_annotation WHERE attachment_id = ?1", params![attachment_id], |r| {
                r.get(0)
            })?;
        if latest != base_number {
            return Err(StorageError::Conflict("the marks were changed since this edit started".into()));
        }
        let run_revision: Option<u32> =
            tx.query_row("SELECT MAX(number) FROM run_revision WHERE run_id = ?1", params![run_id], |r| r.get(0))?;
        let score_revision: Option<u32> =
            tx.query_row("SELECT MAX(number) FROM score_revision WHERE run_id = ?1", params![run_id], |r| r.get(0))?;
        let number = latest.unwrap_or(0) + 1;
        tx.execute(
            "INSERT INTO photo_annotation (id, attachment_id, run_id, number, run_revision, score_revision, content_json, created_utc_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![id, attachment_id, run_id, number, run_revision, score_revision, serde_json::to_string(annotation)?, now_utc_ms],
        )?;
        tx.commit()?;
        Ok(PhotoAnnotation {
            id: id.into(),
            attachment_id: attachment_id.into(),
            run_id,
            number,
            run_revision,
            score_revision,
            annotation: annotation.clone(),
            created_utc_ms: now_utc_ms,
        })
    }

    /// Current review and score revision numbers of a run (to show what changed since).
    pub fn run_revision_numbers(&self, run_id: &str) -> Result<(Option<u32>, Option<u32>)> {
        let r = self.conn.query_row("SELECT MAX(number) FROM run_revision WHERE run_id = ?1", params![run_id], |r| r.get(0))?;
        let s = self.conn.query_row("SELECT MAX(number) FROM score_revision WHERE run_id = ?1", params![run_id], |r| r.get(0))?;
        Ok((r, s))
    }
}
