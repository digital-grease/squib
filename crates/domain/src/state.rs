//! Orthogonal run status axes (docs/squib/06 "Run configuration snapshot").
//!
//! Outcome, review state, and persistence state are independent: `Saved` does not
//! imply `Complete`, and `Complete` does not imply `Reviewed`.

use serde::{Deserialize, Serialize};

/// How the run's events were sourced. Existence of a variant is not availability:
/// see [`SourceMode::available_in_m1`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceMode {
    /// Phone microphone with acoustic cue origin. Experimental until field-qualified.
    PhoneLive,
    /// Timer cues only; no microphone or location permission.
    ParOnly,
    /// Deferred (M4): a documented hardware timer integration.
    HardwareLive,
    /// Deferred (M3): an imported string from another timer.
    ImportedString,
    /// Deferred (M3): manually entered elapsed values.
    ManualEntry,
}

impl SourceMode {
    /// Whether this build offers the mode as a working feature. Manual entry (M3)
    /// records another timer's string; it is never armed.
    pub fn available_in_m1(self) -> bool {
        matches!(self, SourceMode::PhoneLive | SourceMode::ParOnly | SourceMode::ManualEntry)
    }

    /// Whether the run state machine can arm this mode.
    pub fn armable(self) -> bool {
        matches!(self, SourceMode::PhoneLive | SourceMode::ParOnly)
    }

    /// Whether the mode needs microphone capture.
    pub fn needs_microphone(self) -> bool {
        matches!(self, SourceMode::PhoneLive)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            SourceMode::PhoneLive => "phone_live",
            SourceMode::ParOnly => "par_only",
            SourceMode::HardwareLive => "hardware_live",
            SourceMode::ImportedString => "imported_string",
            SourceMode::ManualEntry => "manual_entry",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "phone_live" => SourceMode::PhoneLive,
            "par_only" => SourceMode::ParOnly,
            "hardware_live" => SourceMode::HardwareLive,
            "imported_string" => SourceMode::ImportedString,
            "manual_entry" => SourceMode::ManualEntry,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Complete,
    Interrupted,
    Cancelled,
    FailedToStart,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Complete => "complete",
            Outcome::Interrupted => "interrupted",
            Outcome::Cancelled => "cancelled",
            Outcome::FailedToStart => "failed_to_start",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "complete" => Outcome::Complete,
            "interrupted" => Outcome::Interrupted,
            "cancelled" => Outcome::Cancelled,
            "failed_to_start" => Outcome::FailedToStart,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    /// Only automatic detector suggestions; nothing left unresolved.
    Automatic,
    /// At least one uncertain candidate is unresolved.
    NeedsReview,
    /// A person produced the current revision and nothing is unresolved.
    Reviewed,
}

impl ReviewState {
    pub fn as_str(self) -> &'static str {
        match self {
            ReviewState::Automatic => "automatic",
            ReviewState::NeedsReview => "needs_review",
            ReviewState::Reviewed => "reviewed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "automatic" => ReviewState::Automatic,
            "needs_review" => ReviewState::NeedsReview,
            "reviewed" => ReviewState::Reviewed,
            _ => return None,
        })
    }
}

/// Persistence status shown to the user. `Saved` is only reported after the
/// repository acknowledges a durable commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersistState {
    Pending,
    Saved,
    Failed,
}

impl PersistState {
    pub fn as_str(self) -> &'static str {
        match self {
            PersistState::Pending => "pending",
            PersistState::Saved => "saved",
            PersistState::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "pending" => PersistState::Pending,
            "saved" => PersistState::Saved,
            "failed" => PersistState::Failed,
            _ => return None,
        })
    }
}

/// Quality of the mapping between captured frames and the monotonic clock
/// (docs/squib/04 `timestamp_quality`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimestampQuality {
    /// At least two mutually consistent anchors in the same frame domain.
    VerifiedSampleClock,
    /// A single plausible anchor; rate not yet cross-checked.
    ProvisionalSampleClock,
    /// Derived from delivery/callback arrival. Never labelled verified.
    Approximate,
    Unavailable,
}

impl TimestampQuality {
    pub fn as_str(self) -> &'static str {
        match self {
            TimestampQuality::VerifiedSampleClock => "verified_sample_clock",
            TimestampQuality::ProvisionalSampleClock => "provisional_sample_clock",
            TimestampQuality::Approximate => "approximate",
            TimestampQuality::Unavailable => "unavailable",
        }
    }
}

/// How the run's zero time was established (docs/squib/04 "Start and par cues").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartReferenceMethod {
    /// Cue template onset found in the same capture epoch as the events.
    AcousticCue,
    /// Platform playback timestamp of the cue's first rendered frame (par-only).
    ScheduledRender,
    /// Only the time the app asked for playback is known. Degraded.
    RequestedOnly,
    /// No start origin; first-shot and total are unavailable.
    Unresolved,
}

impl StartReferenceMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            StartReferenceMethod::AcousticCue => "acoustic_cue",
            StartReferenceMethod::ScheduledRender => "scheduled_render",
            StartReferenceMethod::RequestedOnly => "requested_only",
            StartReferenceMethod::Unresolved => "unresolved",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "acoustic_cue" => StartReferenceMethod::AcousticCue,
            "scheduled_render" => StartReferenceMethod::ScheduledRender,
            "requested_only" => StartReferenceMethod::RequestedOnly,
            "unresolved" => StartReferenceMethod::Unresolved,
            _ => return None,
        })
    }

    /// Whether first-shot and total times may be derived from this origin.
    pub fn supports_origin_relative_times(self) -> bool {
        !matches!(self, StartReferenceMethod::Unresolved)
    }
}
