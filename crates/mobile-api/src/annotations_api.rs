//! Manual target-photo annotations (plan phase 4.2, ADR-053): marks and a note on a run's
//! photo, saved as versions that record the run's review and score revisions at the time.
//! No automatic hit detection. Annotations stay out of shared results.

use squib_training::annotation::{Annotation, Mark, MarkKind};

use crate::engine::SquibEngine;
use crate::ffi::SquibError;

fn inval(e: impl ToString) -> SquibError {
    SquibError::Invalid(e.to_string())
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct MarkView {
    /// Fractions of the photo's width and height, 0 to 1, from the top left.
    pub x: f64,
    pub y: f64,
    /// `hit`, `miss`, or `note`.
    pub kind: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PhotoAnnotationView {
    pub attachment_id: String,
    /// Version number, `None` before the first save.
    pub number: Option<u32>,
    pub marks: Vec<MarkView>,
    pub note: String,
    pub created_utc_ms: Option<i64>,
    /// Review and score revisions when these marks were saved, and now.
    pub run_revision: Option<u32>,
    pub score_revision: Option<u32>,
    pub current_run_revision: Option<u32>,
    pub current_score_revision: Option<u32>,
    pub hits: u32,
    pub misses: u32,
}

fn to_view(m: &Mark) -> MarkView {
    MarkView { x: m.x, y: m.y, kind: m.kind.as_str().into(), label: m.label.clone() }
}

#[uniffi::export]
impl SquibEngine {
    /// The latest marks on a photo (empty before the first save).
    pub fn photo_annotation(&self, attachment_id: String) -> Result<PhotoAnnotationView, SquibError> {
        let repo = self.read_repo();
        let latest = repo.latest_photo_annotation(&attachment_id)?;
        let run_id = match &latest {
            Some(a) => Some(a.run_id.clone()),
            None => {
                repo.list_attachments(None)?
                    .into_iter()
                    .find(|a| a.id == attachment_id && a.kind == "photo")
                    .ok_or_else(|| SquibError::NotFound(format!("photo {attachment_id}")))?
                    .run_id
            }
        };
        let (current_run_revision, current_score_revision) = match &run_id {
            Some(r) => repo.run_revision_numbers(r)?,
            None => (None, None),
        };
        let a = latest.as_ref();
        let ann = a.map(|a| a.annotation.clone()).unwrap_or_default();
        Ok(PhotoAnnotationView {
            attachment_id,
            number: a.map(|a| a.number),
            marks: ann.marks.iter().map(to_view).collect(),
            note: ann.note.clone(),
            created_utc_ms: a.map(|a| a.created_utc_ms),
            run_revision: a.and_then(|a| a.run_revision),
            score_revision: a.and_then(|a| a.score_revision),
            current_run_revision,
            current_score_revision,
            hits: ann.count(MarkKind::Hit) as u32,
            misses: ann.count(MarkKind::Miss) as u32,
        })
    }

    /// Save the marks as a new version. `base_number` is the version the edit started
    /// from; if someone saved since, the save is refused instead of overwriting it.
    pub fn save_photo_annotation(
        &self,
        attachment_id: String,
        base_number: Option<u32>,
        marks: Vec<MarkView>,
        note: String,
        now_utc_ms: i64,
    ) -> Result<PhotoAnnotationView, SquibError> {
        let marks = marks
            .into_iter()
            .map(|m| {
                Ok(Mark {
                    x: m.x,
                    y: m.y,
                    kind: MarkKind::parse(&m.kind).ok_or_else(|| inval("unknown mark kind"))?,
                    label: m.label,
                })
            })
            .collect::<Result<Vec<_>, SquibError>>()?;
        let ann = Annotation { marks, note }.normalized().map_err(inval)?;
        let id = uuid::Uuid::new_v4().to_string();
        let aid = attachment_id.clone();
        self.store_actor().exec(move |repo| repo.save_photo_annotation(&id, &aid, base_number, &ann, now_utc_ms))?;
        self.photo_annotation(attachment_id)
    }
}
