//! Manual target-photo annotations (plan phase 4.2, ADR-053; docs/squib/07: "manual
//! annotations can reference target areas and result revisions"; no automatic hit
//! detection). A mark is a position on the photo, as a fraction of its width and height
//! (so it survives any display size), with a kind and an optional short label.

use serde::{Deserialize, Serialize};

pub const MAX_MARKS: usize = 200;
pub const MAX_LABEL: usize = 40;
pub const MAX_NOTE: usize = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkKind {
    Hit,
    Miss,
    Note,
}

impl MarkKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MarkKind::Hit => "hit",
            MarkKind::Miss => "miss",
            MarkKind::Note => "note",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "hit" => MarkKind::Hit,
            "miss" => MarkKind::Miss,
            "note" => MarkKind::Note,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mark {
    /// 0 (left) to 1 (right) of the photo as stored (already upright).
    pub x: f64,
    /// 0 (top) to 1 (bottom).
    pub y: f64,
    pub kind: MarkKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Annotation {
    pub marks: Vec<Mark>,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AnnotationError {
    #[error("at most {MAX_MARKS} marks")]
    TooManyMarks,
    #[error("a mark is outside the photo")]
    OutsidePhoto,
    #[error("mark labels are at most {MAX_LABEL} characters")]
    LabelTooLong,
    #[error("the note is at most {MAX_NOTE} characters")]
    NoteTooLong,
}

impl Annotation {
    /// Trim labels and the note, drop empty labels, and check the bounds.
    pub fn normalized(mut self) -> Result<Self, AnnotationError> {
        if self.marks.len() > MAX_MARKS {
            return Err(AnnotationError::TooManyMarks);
        }
        for m in &mut self.marks {
            if !(m.x.is_finite() && m.y.is_finite() && (0.0..=1.0).contains(&m.x) && (0.0..=1.0).contains(&m.y)) {
                return Err(AnnotationError::OutsidePhoto);
            }
            m.label = m.label.take().map(|l| l.trim().to_string()).filter(|l| !l.is_empty());
            if m.label.as_ref().is_some_and(|l| l.chars().count() > MAX_LABEL) {
                return Err(AnnotationError::LabelTooLong);
            }
        }
        self.note = self.note.trim().to_string();
        if self.note.chars().count() > MAX_NOTE {
            return Err(AnnotationError::NoteTooLong);
        }
        Ok(self)
    }

    pub fn count(&self, kind: MarkKind) -> usize {
        self.marks.iter().filter(|m| m.kind == kind).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(x: f64, y: f64, kind: MarkKind) -> Mark {
        Mark { x, y, kind, label: None }
    }

    #[test]
    fn marks_stay_on_the_photo_and_text_is_bounded() {
        let a = Annotation {
            marks: vec![mark(0.0, 1.0, MarkKind::Hit), mark(0.5, 0.5, MarkKind::Miss)],
            note: "  low left  ".into(),
        };
        let n = a.normalized().unwrap();
        assert_eq!(n.note, "low left");
        assert_eq!((n.count(MarkKind::Hit), n.count(MarkKind::Miss)), (1, 1));
        let bad = |m: Mark| Annotation { marks: vec![m], note: String::new() }.normalized();
        assert_eq!(bad(mark(1.01, 0.5, MarkKind::Hit)), Err(AnnotationError::OutsidePhoto));
        assert_eq!(bad(mark(f64::NAN, 0.5, MarkKind::Hit)), Err(AnnotationError::OutsidePhoto));
        let long = Mark { label: Some("x".repeat(41)), ..mark(0.1, 0.1, MarkKind::Note) };
        assert_eq!(bad(long), Err(AnnotationError::LabelTooLong));
        let blank = Mark { label: Some("   ".into()), ..mark(0.1, 0.1, MarkKind::Note) };
        assert_eq!(bad(blank).unwrap().marks[0].label, None);
        let many = Annotation { marks: vec![mark(0.1, 0.1, MarkKind::Hit); MAX_MARKS + 1], note: String::new() };
        assert_eq!(many.normalized(), Err(AnnotationError::TooManyMarks));
        let note = Annotation { marks: vec![], note: "n".repeat(MAX_NOTE + 1) };
        assert_eq!(note.normalized(), Err(AnnotationError::NoteTooLong));
        // Stored form omits absent labels.
        let json =
            serde_json::to_string(&Annotation { marks: vec![mark(0.25, 0.75, MarkKind::Hit)], note: String::new() }).unwrap();
        assert_eq!(json, r#"{"marks":[{"x":0.25,"y":0.75,"kind":"hit"}],"note":""}"#);
    }
}
