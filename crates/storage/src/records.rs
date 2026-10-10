//! Records crossing the repository boundary.

use serde::{Deserialize, Serialize};
use squib_domain::{
    Candidate, ClassifiedCandidate, CueKind, Outcome, PersistState, QualityEvent, ReviewState, RunConfig, RunRevision,
    StartReferenceMethod,
};
use squib_timing::envelope::EnvelopeChunk;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunIntent {
    pub run_id: String,
    pub session_id: String,
    pub created_utc_ms: i64,
    pub tz_offset_min: i32,
    pub config: RunConfig,
}

/// Actual audio route at capture start. Requested preferences are not evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouteInfo {
    pub input_device: String,
    pub output_device: String,
    pub audio_source: String,
    pub unprocessed_supported: bool,
    pub effects: Vec<String>,
    pub os_build: String,
    pub device_model: String,
    /// The camera was recording video during capture. A camera session can change the
    /// audio path, so it is a separate route for calibration and qualification.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub camera_recording: bool,
}

impl RouteInfo {
    /// Route signature scopes calibration profiles and qualification evidence.
    pub fn signature(&self, sample_rate_hz: u32) -> String {
        format!(
            "{}|{}|{}|{}|{}Hz|{}{}",
            self.device_model,
            self.input_device,
            self.output_device,
            self.audio_source,
            sample_rate_hz,
            self.os_build,
            if self.camera_recording { "|camera" } else { "" }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EpochRecord {
    pub epoch_id: String,
    pub run_id: String,
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub sample_format: String,
    pub route: RouteInfo,
    pub clock_domain: String,
    pub started_mono_ns: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CueObservation {
    pub cue_id: u32,
    pub kind: CueKind,
    pub par_index: Option<u32>,
    pub template_version: String,
    pub requested_mono_ns: i64,
    pub issued_mono_ns: i64,
    pub render_mono_ns: Option<i64>,
    pub epoch_id: Option<String>,
    pub acoustic_onset_frame: Option<i64>,
    pub acoustic_onset_mono_ns: Option<i64>,
    pub match_ncc: Option<f64>,
    pub span_start_frame: Option<i64>,
    pub span_end_frame: Option<i64>,
    pub span_exact: Option<bool>,
    pub missed: bool,
    pub heard: Option<bool>,
}

/// Run-level progress written with batches so recovery can classify committed data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunProgress {
    pub start_method: Option<StartReferenceMethod>,
    pub start_ref_frame: Option<i64>,
    pub start_ref_mono_ns: Option<i64>,
    pub armed_mono_ns: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ObservationBatch {
    /// Strictly increasing per run, starting at 1.
    pub batch_seq: u64,
    pub epoch: Option<EpochRecord>,
    pub progress: Option<RunProgress>,
    pub cues: Vec<CueObservation>,
    pub candidates: Vec<(String, Candidate)>,
    /// (run-local sequence, epoch id, event)
    pub quality: Vec<(u64, Option<String>, QualityEvent)>,
    pub envelope: Vec<(String, EnvelopeChunk)>,
}

impl ObservationBatch {
    pub fn is_empty(&self) -> bool {
        self.epoch.is_none()
            && self.progress.is_none()
            && self.cues.is_empty()
            && self.candidates.is_empty()
            && self.quality.is_empty()
            && self.envelope.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FinalRecord {
    pub run_id: String,
    pub outcome: Outcome,
    pub remaining: ObservationBatch,
    pub stop_mono_ns: Option<i64>,
    pub stop_frame: Option<i64>,
    pub interrupt_reason: Option<String>,
    pub epoch_summary: Option<(String, String)>,
    pub initial_revision: Option<RunRevision>,
    pub finalized_utc_ms: i64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunRow {
    pub run_id: String,
    pub session_id: String,
    pub created_utc_ms: i64,
    pub tz_offset_min: i32,
    pub source_mode: String,
    pub config_hash: String,
    pub outcome: Option<Outcome>,
    pub persist_state: PersistState,
    pub review_state: Option<ReviewState>,
    pub start_method: Option<StartReferenceMethod>,
    pub start_ref_frame: Option<i64>,
    pub stop_frame: Option<i64>,
    pub interrupt_reason: Option<String>,
    pub last_durable_seq: u64,
    pub uncommitted_tail_possible: bool,
    pub finalized_utc_ms: Option<i64>,
    pub app_build: String,
    pub detector_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunSummary {
    pub row: RunRow,
    pub shooter_id: String,
    pub latest_revision: Option<u32>,
    pub accepted_count: Option<u32>,
    pub first_ns: Option<i64>,
    pub last_ns: Option<i64>,
    pub expected_count: Option<u32>,
    pub quality_warnings: u32,
    pub edited: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EpochRow {
    pub record: EpochRecord,
    pub summary_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunDetail {
    pub row: RunRow,
    pub config: RunConfig,
    pub epoch: Option<EpochRow>,
    pub cues: Vec<CueObservation>,
    pub candidates: Vec<Candidate>,
    pub classified: Vec<ClassifiedCandidate>,
    pub quality: Vec<QualityEvent>,
    pub revisions: Vec<RunRevision>,
    pub envelope: Vec<EnvelopeChunk>,
    /// Conditions pinned when the run was armed, as stored (redacted unless the user
    /// opted into precise retention).
    pub environment: Option<squib_environment::EnvironmentSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalibrationRecord {
    pub id: String,
    pub route_signature: String,
    pub label: Option<String>,
    pub algorithm_version: String,
    pub detector_config: squib_domain::DetectorConfig,
    pub suggestion_json: String,
    pub verdict: String,
    pub impulse_count: u32,
    pub os_build: Option<String>,
    pub created_utc_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecoveredRun {
    pub run_id: String,
    pub last_durable_seq: u64,
    pub committed_candidates: u32,
}
