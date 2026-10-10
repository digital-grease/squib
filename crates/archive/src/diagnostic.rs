//! Diagnostic export (M5 experiment 3, docs/squib/08 "Diagnostic corpus"): one run's
//! opt-in recording with the configuration, detector versions, the user's labels, and
//! quality events, for detector research. Local file only; there is no upload path.
//!
//! Excluded by construction: location, conditions, station data, notes and review
//! reasons, editor identity, run/session/shooter ids, and calibration ids. The export
//! gets a fresh random id so it cannot be joined back to the user's history.

use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};
use squib_domain::{
    Candidate, ClassifiedCandidate, CueKind, DelayPolicy, DetectorConfig, EventOrigin, EventRef, QualityEvent, SourceMode,
    StartReferenceMethod,
};
use squib_storage::{DiagnosticRecord, Repository, RouteInfo};

use crate::{ArchiveError, Result};

pub const DIAGNOSTIC_FORMAT: &str = "squib-diagnostic";
pub const DIAGNOSTIC_FORMAT_VERSION: u32 = 1;
pub const CONSENT_SCOPE: &str = "research_only_no_redistribution";
pub const CONSENT_STATEMENT: &str = "I made this recording. It may contain other people's voices or sounds. I am sharing it only \
for research on Squib's shot detection by the person or project I send it to, and not for redistribution or inclusion in a public \
dataset.";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Consent {
    pub scope: String,
    pub statement: String,
    /// UTC calendar day the exporter confirmed the statement (`YYYY-MM-DD`).
    pub confirmed_day: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticManifest {
    pub format: String,
    pub format_version: u32,
    pub export_id: String,
    pub app_version: String,
    pub core_versions: Vec<String>,
    /// UTC day only.
    pub created_day: String,
    pub consent: Consent,
    pub contains_raw_audio: bool,
    pub contains_location: bool,
    pub contains_notes: bool,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordingInfo {
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub sample_format: String,
    /// Epoch frame of file frame 0; every frame below is an epoch frame.
    pub first_epoch_frame: i64,
    pub frames: i64,
    pub gaps: Vec<(i64, i64)>,
    pub truncated: bool,
    pub dropped_frames: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptureConfig {
    pub source_mode: SourceMode,
    pub delay_policy: DelayPolicy,
    pub selected_delay_ms: u32,
    pub pars_ms: Vec<u32>,
    pub detector: Option<DetectorConfig>,
    pub route: Option<RouteInfo>,
    pub clock_domain: Option<String>,
    pub timestamp_mapping_method: String,
    pub app_build: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CueLabel {
    pub kind: CueKind,
    pub par_index: Option<u32>,
    pub acoustic_onset_frame: Option<i64>,
    pub span_start_frame: Option<i64>,
    pub span_end_frame: Option<i64>,
    pub missed: bool,
    pub heard: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventLabel {
    pub event: EventRef,
    pub origin: EventOrigin,
    pub timeline_ns: i64,
    /// Epoch frame of the event when the start reference is a frame.
    pub epoch_frame: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Labels {
    pub outcome: Option<String>,
    pub start_method: Option<StartReferenceMethod>,
    pub start_ref_frame: Option<i64>,
    pub stop_frame: Option<i64>,
    pub cues: Vec<CueLabel>,
    /// Detector output exactly as recorded at the time of the run.
    pub candidates: Vec<Candidate>,
    pub classified: Vec<ClassifiedCandidate>,
    /// The user's latest review: accepted events are the shots, rejected candidates are not.
    pub review_revision: Option<u32>,
    pub reviewed_by_person: bool,
    pub accepted: Vec<EventLabel>,
    pub rejected: Vec<u64>,
    pub unresolved: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticSummary {
    pub bytes: u64,
    pub duration_ms: i64,
    pub accepted_events: u32,
    pub candidates: u32,
    pub gaps: u32,
    pub truncated: bool,
}

fn frames_of_ns(ns: i64, rate: u32) -> i64 {
    squib_domain::ns_to_frames(ns, rate)
}

/// Build the labels document for a run (no free text, no identifiers).
pub fn labels_for(repo: &Repository, run_id: &str) -> Result<(Labels, Option<String>, CaptureConfig)> {
    let d = repo.load_run(run_id)?;
    let rate = d.epoch.as_ref().map(|e| e.record.sample_rate_hz).unwrap_or(48_000);
    let start = d.row.start_ref_frame;
    let latest = d.revisions.last();
    let accepted = latest
        .map(|r| {
            r.content
                .accepted
                .iter()
                .map(|e| EventLabel {
                    event: e.event.clone(),
                    origin: e.origin,
                    timeline_ns: e.timeline_ns,
                    epoch_frame: start.filter(|_| r.content.origin_resolved).map(|s| s + frames_of_ns(e.timeline_ns, rate)),
                })
                .collect()
        })
        .unwrap_or_default();
    let labels = Labels {
        outcome: d.row.outcome.map(|o| o.as_str().to_string()),
        start_method: d.row.start_method,
        start_ref_frame: start,
        stop_frame: d.row.stop_frame,
        cues: d
            .cues
            .iter()
            .map(|c| CueLabel {
                kind: c.kind,
                par_index: c.par_index,
                acoustic_onset_frame: c.acoustic_onset_frame,
                span_start_frame: c.span_start_frame,
                span_end_frame: c.span_end_frame,
                missed: c.missed,
                heard: c.heard,
            })
            .collect(),
        candidates: d.candidates.clone(),
        classified: d.classified.clone(),
        review_revision: latest.map(|r| r.content.number),
        reviewed_by_person: latest.is_some_and(|r| r.content.number > 1),
        accepted,
        rejected: latest.map(|r| r.content.rejected.clone()).unwrap_or_default(),
        unresolved: latest.map(|r| r.content.unresolved.clone()).unwrap_or_default(),
    };
    let cfg = CaptureConfig {
        source_mode: d.config.source_mode,
        delay_policy: d.config.delay_policy,
        selected_delay_ms: d.config.selected_delay_ms,
        pars_ms: d.config.pars_ms.clone(),
        detector: d.config.detector.clone(),
        route: d.epoch.as_ref().map(|e| e.record.route.clone()),
        clock_domain: d.epoch.as_ref().map(|e| e.record.clock_domain.clone()),
        timestamp_mapping_method: d.config.timestamp_mapping_method.clone(),
        app_build: d.config.app_build.clone(),
    };
    Ok((labels, d.epoch.map(|e| e.record.epoch_id), cfg))
}

/// Quality events without free-text detail (machine codes and frames only).
fn quality_for(repo: &Repository, run_id: &str) -> Result<Vec<QualityEvent>> {
    Ok(repo
        .load_run(run_id)?
        .quality
        .into_iter()
        .map(|mut q| {
            q.detail = String::new();
            q
        })
        .collect())
}

pub struct DiagnosticExportOptions<'a> {
    pub export_id: String,
    pub app_version: String,
    pub core_versions: Vec<String>,
    pub created_day: String,
    /// The exporter confirmed `CONSENT_STATEMENT`. Export is refused without it.
    pub consent_confirmed: bool,
    pub attachment_root: &'a Path,
}

pub fn export_diagnostic(repo: &Repository, run_id: &str, out: &Path, o: &DiagnosticExportOptions) -> Result<DiagnosticSummary> {
    if !o.consent_confirmed {
        return Err(ArchiveError::InvalidData {
            table: "consent".into(),
            detail: "the consent statement was not confirmed".into(),
        });
    }
    let rec: DiagnosticRecord = repo.diagnostic(run_id)?.ok_or_else(|| ArchiveError::InvalidData {
        table: "diagnostic_recording".into(),
        detail: "no recording for this run".into(),
    })?;
    if !squib_storage::valid_attachment_path(&rec.attachment.relative_path) {
        return Err(ArchiveError::InvalidData { table: "attachment".into(), detail: "unsafe path".into() });
    }
    let wav = std::fs::read(o.attachment_root.join(&rec.attachment.relative_path))?;
    let (labels, _epoch, cfg) = labels_for(repo, run_id)?;
    let info = RecordingInfo {
        sample_rate_hz: rec.sample_rate_hz,
        channels: 1,
        sample_format: "pcm16".into(),
        first_epoch_frame: rec.first_epoch_frame,
        frames: rec.frames,
        gaps: rec.gaps.clone(),
        truncated: rec.truncated,
        dropped_frames: rec.dropped_frames,
    };
    let quality = quality_for(repo, run_id)?;
    let files = ["recording.wav", "recording.json", "config.json", "labels.json", "quality.json"];
    let manifest = DiagnosticManifest {
        format: DIAGNOSTIC_FORMAT.into(),
        format_version: DIAGNOSTIC_FORMAT_VERSION,
        export_id: o.export_id.clone(),
        app_version: o.app_version.clone(),
        core_versions: o.core_versions.clone(),
        created_day: o.created_day.clone(),
        consent: Consent {
            scope: CONSENT_SCOPE.into(),
            statement: CONSENT_STATEMENT.into(),
            confirmed_day: o.created_day.clone(),
        },
        contains_raw_audio: true,
        contains_location: false,
        contains_notes: false,
        files: files.iter().map(|f| f.to_string()).collect(),
    };
    let file = std::fs::File::create(out)?;
    let mut zw = zip::ZipWriter::new(file);
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, body) in [
        ("manifest.json", to_json(&manifest)?),
        ("recording.json", to_json(&info)?),
        ("config.json", to_json(&cfg)?),
        ("labels.json", to_json(&labels)?),
        ("quality.json", to_json(&quality)?),
    ] {
        zw.start_file(name, stored)?;
        zw.write_all(&body)?;
    }
    zw.start_file("recording.wav", stored)?;
    zw.write_all(&wav)?;
    zw.finish()?;
    Ok(DiagnosticSummary {
        bytes: std::fs::metadata(out)?.len(),
        duration_ms: rec.frames * 1000 / i64::from(rec.sample_rate_hz.max(1)),
        accepted_events: labels.accepted.len() as u32,
        candidates: labels.candidates.len() as u32,
        gaps: rec.gaps.len() as u32,
        truncated: rec.truncated,
    })
}

fn to_json<T: Serialize>(v: &T) -> Result<Vec<u8>> {
    serde_json::to_vec_pretty(v).map_err(|e| ArchiveError::Io(e.to_string()))
}

/// Everything read back from an export, for replay and research tools.
pub struct DiagnosticBundle {
    pub manifest: DiagnosticManifest,
    pub recording: RecordingInfo,
    pub config: CaptureConfig,
    pub labels: Labels,
    pub quality: Vec<QualityEvent>,
    pub wav: Vec<u8>,
}

pub fn read_diagnostic(path: &Path) -> Result<DiagnosticBundle> {
    let mut z = zip::ZipArchive::new(std::fs::File::open(path)?)?;
    let mut read = |name: &str, limit: u64| -> Result<Vec<u8>> {
        let f = z.by_name(name).map_err(|_| ArchiveError::InvalidData { table: name.into(), detail: "missing".into() })?;
        if f.size() > limit {
            return Err(ArchiveError::InvalidData { table: name.into(), detail: "too large".into() });
        }
        let mut v = Vec::with_capacity(f.size() as usize);
        std::io::Read::read_to_end(&mut std::io::Read::take(f, limit), &mut v)?;
        Ok(v)
    };
    let parse = |name: &str, b: Vec<u8>| -> Result<serde_json::Value> {
        serde_json::from_slice(&b).map_err(|e| ArchiveError::InvalidData { table: name.into(), detail: e.to_string() })
    };
    let de = |name: &str, v: serde_json::Value| -> Result<_> { Ok((name.to_string(), v)) };
    let m = de("manifest.json", parse("manifest.json", read("manifest.json", 1 << 20)?)?)?;
    let r = de("recording.json", parse("recording.json", read("recording.json", 1 << 20)?)?)?;
    let c = de("config.json", parse("config.json", read("config.json", 1 << 20)?)?)?;
    let l = de("labels.json", parse("labels.json", read("labels.json", 64 << 20)?)?)?;
    let q = de("quality.json", parse("quality.json", read("quality.json", 16 << 20)?)?)?;
    let wav = read("recording.wav", 64 << 20)?;
    fn typed<T: serde::de::DeserializeOwned>((name, v): (String, serde_json::Value)) -> Result<T> {
        serde_json::from_value(v).map_err(|e| ArchiveError::InvalidData { table: name, detail: e.to_string() })
    }
    let manifest: DiagnosticManifest = typed(m)?;
    if manifest.format != DIAGNOSTIC_FORMAT || manifest.format_version > DIAGNOSTIC_FORMAT_VERSION {
        return Err(ArchiveError::InvalidData {
            table: "manifest.json".into(),
            detail: "not a supported diagnostic export".into(),
        });
    }
    Ok(DiagnosticBundle { manifest, recording: typed(r)?, config: typed(c)?, labels: typed(l)?, quality: typed(q)?, wav })
}
