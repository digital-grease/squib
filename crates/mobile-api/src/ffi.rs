//! UniFFI control-plane types. Timing values are `i64` nanoseconds (Kotlin `Long`);
//! nothing crosses as a floating-point timestamp.

use squib_domain::{CueKind, DelayPolicy, SourceMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Mode {
    ParOnly,
    PhoneLive,
}

impl From<Mode> for SourceMode {
    fn from(m: Mode) -> Self {
        match m {
            Mode::ParOnly => SourceMode::ParOnly,
            Mode::PhoneLive => SourceMode::PhoneLive,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum StartDelay {
    Instant,
    Fixed { ms: u32 },
    Random { min_ms: u32, max_ms: u32 },
}

impl From<StartDelay> for DelayPolicy {
    fn from(d: StartDelay) -> Self {
        match d {
            StartDelay::Instant => DelayPolicy::Instant,
            StartDelay::Fixed { ms } => DelayPolicy::Fixed { ms },
            StartDelay::Random { min_ms, max_ms } => DelayPolicy::Random { min_ms, max_ms },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CueType {
    Start,
    Par,
}

impl From<CueKind> for CueType {
    fn from(k: CueKind) -> Self {
        match k {
            CueKind::Start => CueType::Start,
            CueKind::Par => CueType::Par,
        }
    }
}

impl From<CueType> for CueKind {
    fn from(k: CueType) -> Self {
        match k {
            CueType::Start => CueKind::Start,
            CueType::Par => CueKind::Par,
        }
    }
}

/// Actual route observed by the platform adapter.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct RouteReport {
    /// e.g. `builtin_mic`, `bluetooth_sco`, `usb`, `wired_headset`, `unknown`.
    pub input_device: String,
    /// e.g. `builtin_speaker`, `bluetooth_a2dp`, `wired_headphones`, `unknown`.
    pub output_device: String,
    /// `unprocessed` or `voice_recognition`.
    pub audio_source: String,
    pub unprocessed_supported: bool,
    /// Inspectable platform effects active on the session.
    pub effects: Vec<String>,
    pub os_build: String,
    pub device_model: String,
    /// Media volume as a fraction of maximum.
    pub media_volume: f32,
    pub mic_permission: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RouteLevel {
    /// Field-qualified. No route has this status in M1.
    Qualified,
    /// Allowed; timing is experimental until field qualification.
    Experimental,
    /// Not usable for this mode.
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Preflight {
    pub can_arm: bool,
    pub level: RouteLevel,
    /// Blocking problems first, then warnings. Text, not color only.
    pub messages: Vec<String>,
    pub route_signature: Option<String>,
    pub calibration_id: Option<String>,
    pub calibration_threshold_db: Option<f32>,
    pub calibration_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ArmRequest {
    pub mode: Mode,
    pub delay: StartDelay,
    /// Platform randomness in [0, 1) used to select a random delay.
    pub unit_random: f64,
    pub pars_ms: Vec<u32>,
    /// `Some` enables labelled auto stop after the last par.
    pub auto_stop_grace_ms: Option<u32>,
    /// Review hint only.
    pub expected_count: Option<u32>,
    /// Manual sensitivity override (dB above floor). `None`: calibration or default.
    pub threshold_db: Option<f32>,
    pub calibration_id: Option<String>,
    pub route: Option<RouteReport>,
    pub now_utc_ms: i64,
    pub tz_offset_min: i32,
    pub app_build: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CaptureMeta {
    pub sample_rate_hz: u32,
    pub block_frames: u32,
    pub platform_buffer_frames: Option<u32>,
    pub route: RouteReport,
    pub clock_domain: String,
    pub started_mono_ns: i64,
}

#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum NativeEffect {
    StartCapture { preferred_rate_hz: u32 },
    StopCapture,
    PlayCue { cue_id: u32, cue: CueType },
    KeepScreenOn { on: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum LifecycleEvent {
    ForegroundLost,
    RouteChanged,
    MicSilenced,
    AudioFocusLost,
    PermissionRevoked,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct LiveEvent {
    pub timeline_ns: i64,
    pub accepted: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct EngineView {
    /// Phase name: ready, preparing, armed, awaiting_cue, running, completing,
    /// interrupted, saved, save_pending, cancelled, failed, calibrating.
    pub phase: String,
    pub run_id: Option<String>,
    pub mode: Option<Mode>,
    pub outcome: Option<String>,
    /// pending / saved / failed. `saved` only after durable acknowledgment.
    pub persist: Option<String>,
    pub start_method: Option<String>,
    pub timestamp_quality: Option<String>,
    pub error: Option<String>,
    pub interrupt_reason: Option<String>,
    pub live_events: Vec<LiveEvent>,
    pub accepted_count: u32,
    pub uncertain_count: u32,
    pub first_ns: Option<i64>,
    pub last_ns: Option<i64>,
    pub pars_played: u32,
    pub pars_total: u32,
    pub quality_labels: Vec<String>,
    /// For adapter scheduling only; never displayed as a countdown.
    pub next_deadline_ns: Option<i64>,
    pub durable_seq: u64,
    pub effects: Vec<NativeEffect>,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct EventRefFfi {
    /// `candidate` or `manual`.
    pub kind: String,
    pub sequence: Option<u64>,
    pub manual_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ReviewEvent {
    pub reference: EventRefFfi,
    pub timeline_ns: i64,
    /// accepted / uncertain / rejected.
    pub state: String,
    /// detected / moved / manual.
    pub origin: String,
    pub reasons: Vec<String>,
    /// Ranking score, not a probability. `None` for manual events.
    pub detector_score: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CueMarker {
    pub label: String,
    pub timeline_ns: Option<i64>,
    pub heard: Option<bool>,
    pub missed: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct QualityView {
    pub label: String,
    pub severity: String,
    pub detail: String,
    pub timeline_ns: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ReviewView {
    pub run_id: String,
    pub created_utc_ms: i64,
    pub mode: String,
    pub outcome: Option<String>,
    pub persist: String,
    pub review_state: Option<String>,
    pub start_method: Option<String>,
    pub timestamp_quality: Option<String>,
    pub origin_resolved: bool,
    pub revision_number: u32,
    pub edited: bool,
    pub uncommitted_tail_possible: bool,
    pub interrupt_reason: Option<String>,
    pub events: Vec<ReviewEvent>,
    pub cues: Vec<CueMarker>,
    pub quality: Vec<QualityView>,
    pub count: u32,
    pub first_ns: Option<i64>,
    pub last_ns: Option<i64>,
    pub splits_ns: Vec<i64>,
    pub zero_splits: u32,
    pub expected_count: Option<u32>,
    /// Coarse RMS energy (dBFS) per hop, labelled "Energy" in the UI.
    pub energy_db: Vec<f32>,
    pub energy_hop_ns: i64,
    /// Timeline position of the first energy value.
    pub energy_start_ns: i64,
    pub pars_ms: Vec<u32>,
    pub delay_ms: u32,
    pub detector_version: Option<String>,
    pub threshold_db: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum ReviewAction {
    Accept { sequence: u64 },
    Reject { sequence: u64 },
    AddManual { timeline_ns: i64 },
    Move { reference: EventRefFfi, timeline_ns: i64 },
    RemoveManual { manual_id: String },
    Note { text: String },
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct RunListItem {
    pub run_id: String,
    pub created_utc_ms: i64,
    pub tz_offset_min: i32,
    pub mode: String,
    pub outcome: Option<String>,
    pub persist: String,
    pub review_state: Option<String>,
    pub start_method: Option<String>,
    pub count: Option<u32>,
    pub first_ns: Option<i64>,
    pub last_ns: Option<i64>,
    pub expected_count: Option<u32>,
    pub quality_warnings: u32,
    pub edited: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CalibrationView {
    /// ambient / impulses / done / failed.
    pub stage: String,
    pub ambient_median_dbfs: Option<f32>,
    pub ambient_p99_dbfs: Option<f32>,
    pub clipped_fraction: Option<f64>,
    pub impulse_ratios_db: Vec<f32>,
    pub suggested_threshold_db: Option<f32>,
    /// separated / ambient_only / poor_separation / ambient_clipped.
    pub verdict: Option<String>,
    pub message: String,
    pub effects: Vec<NativeEffect>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CaptureDiagnostics {
    pub blocks: u64,
    /// Frames fully processed by the DSP worker.
    pub processed_frames: i64,
    pub dsp_p50_ns: u64,
    pub dsp_p99_ns: u64,
    pub dsp_max_ns: u64,
    pub block_duration_ns: u64,
    pub queue_capacity_blocks: u64,
    pub queue_high_water_blocks: u64,
    pub queue_overflows: u64,
    pub timestamp_quality: String,
    pub anchors_accepted: u32,
    pub anchors_rejected: u32,
    pub drift_ppm: Option<f64>,
    pub max_anchor_residual_ns: i64,
    pub delivery_mean_ns: Option<i64>,
    pub delivery_max_ns: i64,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct RecoveredRunView {
    pub run_id: String,
    pub last_durable_seq: u64,
    pub committed_candidates: u32,
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum SquibError {
    #[error("{0}")]
    Rejected(String),
    #[error("storage: {0}")]
    Storage(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid: {0}")]
    Invalid(String),
}

impl From<squib_storage::StorageError> for SquibError {
    fn from(e: squib_storage::StorageError) -> Self {
        match e {
            squib_storage::StorageError::NotFound(s) => SquibError::NotFound(s),
            other => SquibError::Storage(other.to_string()),
        }
    }
}
