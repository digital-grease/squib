//! Versioned declarative drill recipes (docs/squib/07 "Drill recipe").
//!
//! A recipe is bounded data: no expressions, no code, no remote references. Editing a
//! drill creates a new immutable version; runs keep the version they were armed with
//! (A14). Instantiating a recipe yields the parameters for one run; each repeat is a
//! new run with a newly sampled delay.

use serde::{Deserialize, Serialize};
use squib_domain::{DelayPolicy, SourceMode, hash::content_hash, validate_pars};
use thiserror::Error;

pub const RECIPE_SCHEMA_VERSION: u32 = 1;
pub const MAX_TITLE: usize = 80;
pub const MAX_DESCRIPTION: usize = 1000;
pub const MAX_NOTES: usize = 2000;
pub const MAX_REPEATS: u32 = 50;
pub const MAX_REST_S: u32 = 600;
pub const MAX_TAGS: usize = 10;
pub const MAX_TAG_LEN: usize = 30;
/// Upper bound for a serialized recipe accepted from a file or another device.
pub const MAX_RECIPE_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrillRecipe {
    pub schema_version: u32,
    pub title: String,
    pub description: String,
    /// `par_only`, `phone_live`, or `manual_entry`.
    pub mode: SourceMode,
    pub delay: DelayPolicy,
    /// Review hint only; never forces detection.
    pub expected_count: Option<u32>,
    /// Par times within one string, relative to the start (validated like run pars).
    pub pars_ms: Vec<u32>,
    /// Strings per set. Each is a separate run.
    pub repeats: u32,
    /// Rest between strings (a separate timer, not part of a captured string).
    pub rest_s: u32,
    pub scoring_profile_id: String,
    pub scoring_profile_version: u32,
    pub notes: String,
    pub equipment_tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum DrillError {
    #[error("unsupported recipe schema {0}")]
    Schema(u32),
    #[error("title must be 1-{MAX_TITLE} characters")]
    Title,
    #[error("{0} is too long")]
    TooLong(&'static str),
    #[error("mode {0:?} cannot be used for drills")]
    Mode(SourceMode),
    #[error("repeats must be 1-{MAX_REPEATS}")]
    Repeats,
    #[error("rest must be at most {MAX_REST_S} s")]
    Rest,
    #[error("at most {MAX_TAGS} equipment tags of {MAX_TAG_LEN} characters")]
    Tags,
    #[error("invalid timing: {0}")]
    Timing(String),
    #[error("unknown scoring profile {0} v{1}")]
    Scoring(String, u32),
    #[error("recipe file is too large or malformed: {0}")]
    Malformed(String),
}

impl DrillRecipe {
    pub fn validate(&self) -> Result<(), DrillError> {
        if self.schema_version != RECIPE_SCHEMA_VERSION {
            return Err(DrillError::Schema(self.schema_version));
        }
        let t = self.title.trim();
        if t.is_empty() || t.chars().count() > MAX_TITLE {
            return Err(DrillError::Title);
        }
        if self.description.chars().count() > MAX_DESCRIPTION {
            return Err(DrillError::TooLong("description"));
        }
        if self.notes.chars().count() > MAX_NOTES {
            return Err(DrillError::TooLong("notes"));
        }
        if !matches!(self.mode, SourceMode::ParOnly | SourceMode::PhoneLive | SourceMode::ManualEntry) {
            return Err(DrillError::Mode(self.mode));
        }
        if !(1..=MAX_REPEATS).contains(&self.repeats) {
            return Err(DrillError::Repeats);
        }
        if self.rest_s > MAX_REST_S {
            return Err(DrillError::Rest);
        }
        if self.equipment_tags.len() > MAX_TAGS
            || self.equipment_tags.iter().any(|t| t.trim().is_empty() || t.chars().count() > MAX_TAG_LEN)
        {
            return Err(DrillError::Tags);
        }
        self.delay.validate().map_err(|e| DrillError::Timing(e.to_string()))?;
        validate_pars(&self.pars_ms).map_err(|e| DrillError::Timing(e.to_string()))?;
        if let Some(n) = self.expected_count
            && (n == 0 || n > 1000)
        {
            return Err(DrillError::Timing("expected count must be 1-1000".into()));
        }
        if crate::scoring::builtin(&self.scoring_profile_id, self.scoring_profile_version).is_none() {
            return Err(DrillError::Scoring(self.scoring_profile_id.clone(), self.scoring_profile_version));
        }
        Ok(())
    }

    pub fn content_hash(&self) -> String {
        content_hash(self)
    }

    /// Parse a recipe shared as a file: size-bounded, schema-checked, validated.
    /// Data only: nothing in a recipe can fetch or execute anything.
    pub fn from_shared_bytes(b: &[u8]) -> Result<Self, DrillError> {
        if b.len() > MAX_RECIPE_BYTES {
            return Err(DrillError::Malformed("larger than 16 KB".into()));
        }
        let r: DrillRecipe = serde_json::from_slice(b).map_err(|e| DrillError::Malformed(e.to_string()))?;
        r.validate()?;
        Ok(r)
    }
}

/// An immutable version of a drill.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrillVersion {
    pub drill_id: String,
    pub version: u32,
    pub recipe: DrillRecipe,
    pub content_hash: String,
    pub created_utc_ms: i64,
}

impl DrillVersion {
    pub fn first(drill_id: &str, recipe: DrillRecipe, now: i64) -> Result<Self, DrillError> {
        recipe.validate()?;
        Ok(Self { drill_id: drill_id.into(), version: 1, content_hash: recipe.content_hash(), recipe, created_utc_ms: now })
    }

    /// An edit produces the next version; the previous version is untouched.
    pub fn edit(&self, recipe: DrillRecipe, now: i64) -> Result<Self, DrillError> {
        recipe.validate()?;
        Ok(Self {
            drill_id: self.drill_id.clone(),
            version: self.version + 1,
            content_hash: recipe.content_hash(),
            recipe,
            created_utc_ms: now,
        })
    }
}

/// Neutral starter recipes (docs/squib/07): single par window, repeated par windows,
/// timed string with manually entered score.
pub fn starter_recipes() -> Vec<DrillRecipe> {
    let base = |title: &str, desc: &str| DrillRecipe {
        schema_version: RECIPE_SCHEMA_VERSION,
        title: title.into(),
        description: desc.into(),
        mode: SourceMode::ParOnly,
        delay: DelayPolicy::Random { min_ms: 2000, max_ms: 4000 },
        expected_count: None,
        pars_ms: vec![],
        repeats: 1,
        rest_s: 0,
        scoring_profile_id: "generic-time".into(),
        scoring_profile_version: 1,
        notes: String::new(),
        equipment_tags: vec![],
    };
    vec![
        DrillRecipe { pars_ms: vec![2000], ..base("Single par", "One par cue two seconds after the start.") },
        DrillRecipe {
            pars_ms: vec![1500],
            repeats: 5,
            rest_s: 20,
            ..base("Repeated par", "Five strings with a 1.5 s par and 20 s rest between them.")
        },
        DrillRecipe {
            mode: SourceMode::PhoneLive,
            scoring_profile_id: "generic-points-hf".into(),
            ..base("Timed string with score", "Live-timed string; enter target points afterwards.")
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starters_validate_and_bounds_are_enforced() {
        for r in starter_recipes() {
            r.validate().unwrap();
        }
        let ok = starter_recipes().remove(0);
        let bad = |f: fn(&mut DrillRecipe)| {
            let mut r = ok.clone();
            f(&mut r);
            r.validate().unwrap_err()
        };
        assert_eq!(bad(|r| r.title = "  ".into()), DrillError::Title);
        assert_eq!(bad(|r| r.repeats = 0), DrillError::Repeats);
        assert_eq!(bad(|r| r.rest_s = 601), DrillError::Rest);
        assert_eq!(bad(|r| r.mode = SourceMode::HardwareLive), DrillError::Mode(SourceMode::HardwareLive));
        assert!(matches!(bad(|r| r.pars_ms = vec![1000, 1050]), DrillError::Timing(_)));
        assert!(matches!(bad(|r| r.scoring_profile_id = "uspsa".into()), DrillError::Scoring(..)));
        assert_eq!(bad(|r| r.equipment_tags = vec!["x".repeat(31)]), DrillError::Tags);
    }

    #[test]
    fn a14_edits_create_versions_and_leave_the_old_one_intact() {
        let v1 = DrillVersion::first("d1", starter_recipes().remove(0), 1).unwrap();
        let snapshot = v1.clone();
        let mut r = v1.recipe.clone();
        r.pars_ms = vec![1800];
        let v2 = v1.edit(r, 2).unwrap();
        assert_eq!(v1, snapshot);
        assert_eq!(v2.version, 2);
        assert_ne!(v1.content_hash, v2.content_hash);
    }

    #[test]
    fn shared_recipe_files_are_bounded_data() {
        let json = serde_json::to_vec(&starter_recipes()[1]).unwrap();
        assert_eq!(DrillRecipe::from_shared_bytes(&json).unwrap(), starter_recipes()[1]);
        assert!(DrillRecipe::from_shared_bytes(&vec![b' '; MAX_RECIPE_BYTES + 1]).is_err());
        assert!(DrillRecipe::from_shared_bytes(br#"{"title":"x","script":"rm -rf"}"#).is_err());
        let mut evil = serde_json::to_value(&starter_recipes()[0]).unwrap();
        evil["fetch_url"] = "https://example.com/x".into();
        // Unknown fields are ignored data; they cannot cause any action.
        let r = DrillRecipe::from_shared_bytes(&serde_json::to_vec(&evil).unwrap()).unwrap();
        assert_eq!(r, starter_recipes()[0]);
    }
}
