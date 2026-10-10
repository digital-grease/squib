//! Quality events: gaps, interruptions, degraded cues, and diagnostics.
//!
//! Severity `Integrity` means the capture can no longer support a complete run: the
//! run controller interrupts and the run can never be labelled Complete.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityKind {
    /// Frames missing between consecutive blocks.
    CaptureGap,
    /// Block sequence skipped, repeated, or reordered.
    SequenceDiscontinuity,
    /// Bounded capture queue full; producer could not enqueue a block.
    QueueOverflow,
    /// DSP output/control queue full.
    ControlQueueOverflow,
    /// Sample rate, channel count, or sample format changed mid-epoch.
    FormatChanged,
    /// Input or output device changed during the run.
    RouteChanged,
    /// Non-finite float samples were replaced by zero before DSP.
    NonfiniteInput,
    /// Some samples reached full scale.
    ClippedInput,
    /// A platform timestamp anchor failed validation and was not used.
    AnchorRejected,
    /// No platform timestamp anchor could be obtained.
    AnchorUnavailable,
    /// Anchors imply a jump in the frame/clock relationship.
    ClockDiscontinuity,
    /// Platform reported frames captured beyond what the reader consumed.
    CaptureOverrun,
    /// A cue was played later than its deadline (within tolerance).
    CueLate,
    /// A par cue was skipped because it could no longer be played in time.
    CueMissed,
    /// Start cue template not found in its search window.
    CueUnresolved,
    /// A par cue was played but not found acoustically. Informational.
    ParCueNotHeard,
    /// Playback render timestamp unavailable; only the request time is known.
    CueRenderUnavailable,
    /// App left the foreground during capture.
    ForegroundLost,
    /// The OS silenced this app's microphone (call, another app).
    MicSilenced,
    AudioFocusLost,
    PermissionRevoked,
    /// Platform capture API returned an error.
    CaptureError,
    /// DSP fell behind the capture stream beyond its budget.
    ProcessingOverrun,
    /// Stop arrived before DSP processed the full requested tail.
    TailUnprocessed,
    /// Delivery delay of captured blocks (diagnostic only; never changes timing).
    DeliveryDelay,
    /// A durable write failed.
    StorageWriteFailed,
    /// Recovered after process termination; observations after the last durable
    /// sequence may be missing.
    ProcessTerminated,
    /// Route not qualified for verified acoustic timing.
    RouteUnqualified,
    /// The opt-in diagnostic recording could not be started; timing is unaffected.
    DiagnosticRecordingFailed,
}

impl QualityKind {
    pub fn as_str(self) -> &'static str {
        // serde names are the canonical persisted tags.
        match self {
            QualityKind::CaptureGap => "capture_gap",
            QualityKind::SequenceDiscontinuity => "sequence_discontinuity",
            QualityKind::QueueOverflow => "queue_overflow",
            QualityKind::ControlQueueOverflow => "control_queue_overflow",
            QualityKind::FormatChanged => "format_changed",
            QualityKind::RouteChanged => "route_changed",
            QualityKind::NonfiniteInput => "nonfinite_input",
            QualityKind::ClippedInput => "clipped_input",
            QualityKind::AnchorRejected => "anchor_rejected",
            QualityKind::AnchorUnavailable => "anchor_unavailable",
            QualityKind::ClockDiscontinuity => "clock_discontinuity",
            QualityKind::CaptureOverrun => "capture_overrun",
            QualityKind::CueLate => "cue_late",
            QualityKind::CueMissed => "cue_missed",
            QualityKind::CueUnresolved => "cue_unresolved",
            QualityKind::ParCueNotHeard => "par_cue_not_heard",
            QualityKind::CueRenderUnavailable => "cue_render_unavailable",
            QualityKind::ForegroundLost => "foreground_lost",
            QualityKind::MicSilenced => "mic_silenced",
            QualityKind::AudioFocusLost => "audio_focus_lost",
            QualityKind::PermissionRevoked => "permission_revoked",
            QualityKind::CaptureError => "capture_error",
            QualityKind::ProcessingOverrun => "processing_overrun",
            QualityKind::TailUnprocessed => "tail_unprocessed",
            QualityKind::DeliveryDelay => "delivery_delay",
            QualityKind::StorageWriteFailed => "storage_write_failed",
            QualityKind::ProcessTerminated => "process_terminated",
            QualityKind::RouteUnqualified => "route_unqualified",
            QualityKind::DiagnosticRecordingFailed => "diagnostic_recording_failed",
        }
    }

    /// Human-readable label for review screens (text, not color only).
    pub fn label(self) -> &'static str {
        match self {
            QualityKind::CaptureGap => "Capture gap",
            QualityKind::SequenceDiscontinuity => "Audio blocks out of order",
            QualityKind::QueueOverflow => "Audio queue overflow",
            QualityKind::ControlQueueOverflow => "Event queue overflow",
            QualityKind::FormatChanged => "Audio format changed",
            QualityKind::RouteChanged => "Audio route changed",
            QualityKind::NonfiniteInput => "Invalid samples",
            QualityKind::ClippedInput => "Input clipped",
            QualityKind::AnchorRejected => "Clock anchor rejected",
            QualityKind::AnchorUnavailable => "Clock anchor unavailable",
            QualityKind::ClockDiscontinuity => "Clock discontinuity",
            QualityKind::CaptureOverrun => "Capture overrun",
            QualityKind::CueLate => "Cue played late",
            QualityKind::CueMissed => "Par cue missed",
            QualityKind::CueUnresolved => "Start cue not heard",
            QualityKind::ParCueNotHeard => "Par cue not heard",
            QualityKind::CueRenderUnavailable => "Cue playback time unknown",
            QualityKind::ForegroundLost => "App left foreground",
            QualityKind::MicSilenced => "Microphone taken by system",
            QualityKind::AudioFocusLost => "Audio focus lost",
            QualityKind::PermissionRevoked => "Permission revoked",
            QualityKind::CaptureError => "Capture error",
            QualityKind::ProcessingOverrun => "Processing fell behind",
            QualityKind::TailUnprocessed => "End of run not fully processed",
            QualityKind::DeliveryDelay => "Delayed audio delivery",
            QualityKind::StorageWriteFailed => "Save failed",
            QualityKind::ProcessTerminated => "App closed during run",
            QualityKind::RouteUnqualified => "Route not qualified",
            QualityKind::DiagnosticRecordingFailed => "Diagnostic recording failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    /// Capture integrity lost: interrupts the run.
    Integrity,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Integrity => "integrity",
        }
    }
}

/// A quality observation. Frame bounds are in the epoch's frame domain when known;
/// `at_ns` is a monotonic-clock reference when known. Unknown is `None`, never zero.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualityEvent {
    pub kind: QualityKind,
    pub severity: Severity,
    pub start_frame: Option<i64>,
    pub end_frame: Option<i64>,
    pub at_ns: Option<i64>,
    pub detail: String,
}

impl QualityEvent {
    pub fn new(kind: QualityKind, severity: Severity, detail: impl Into<String>) -> Self {
        Self { kind, severity, start_frame: None, end_frame: None, at_ns: None, detail: detail.into() }
    }

    pub fn frames(mut self, start: i64, end: i64) -> Self {
        self.start_frame = Some(start);
        self.end_frame = Some(end);
        self
    }

    pub fn at(mut self, ns: i64) -> Self {
        self.at_ns = Some(ns);
        self
    }

    pub fn interrupts(&self) -> bool {
        self.severity == Severity::Integrity
    }
}
