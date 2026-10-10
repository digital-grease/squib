//! M5 experiment 3 facade: opt-in diagnostic recordings (review, delete, export).
//! Saving a recording (opt-in per run) and exporting it (separate, previewed, with a
//! consent statement) are distinct user decisions; there is no upload path.

use std::path::Path;

use squib_archive::diagnostic::{CONSENT_STATEMENT, DiagnosticExportOptions, export_diagnostic, labels_for};
use squib_domain::EventOrigin;

use crate::engine::SquibEngine;
use crate::ffi::SquibError;

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AudioMarker {
    /// Position in the recording, ms from its first sample.
    pub file_ms: i64,
    /// `shot` (the user's accepted events) or `cue`.
    pub kind: String,
    pub label: String,
    pub edited: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DiagnosticView {
    pub relative_path: String,
    pub bytes: i64,
    pub duration_ms: i64,
    pub sample_rate_hz: u32,
    pub gaps: u32,
    pub truncated: bool,
    pub dropped_frames: i64,
    pub markers: Vec<AudioMarker>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DiagnosticPreview {
    pub duration_ms: i64,
    pub recording_bytes: i64,
    pub accepted_events: u32,
    pub detector_candidates: u32,
    pub reviewed_by_person: bool,
    pub includes: Vec<String>,
    pub excludes: Vec<String>,
    pub consent_statement: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DiagnosticExportView {
    pub bytes: u64,
    pub duration_ms: i64,
    pub accepted_events: u32,
}

fn utc_day(ms: i64) -> String {
    squib_archive::iso_utc(ms).chars().take(10).collect()
}

#[uniffi::export]
impl SquibEngine {
    /// The run's recording with markers for its accepted events and heard cues.
    pub fn run_diagnostic(&self, run_id: String) -> Result<Option<DiagnosticView>, SquibError> {
        let repo = self.read_repo();
        let Some(d) = repo.diagnostic(&run_id)? else { return Ok(None) };
        let (labels, _, _) = labels_for(&repo, &run_id).map_err(|e| SquibError::Invalid(e.to_string()))?;
        let rate = i64::from(d.sample_rate_hz.max(1));
        let ms = |epoch_frame: i64| (epoch_frame - d.first_epoch_frame) * 1000 / rate;
        let mut markers: Vec<AudioMarker> = Vec::new();
        for c in &labels.cues {
            if let Some(f) = c.acoustic_onset_frame {
                let label = match c.par_index {
                    Some(i) => format!("Par {}", i + 1),
                    None => "Start cue".into(),
                };
                markers.push(AudioMarker { file_ms: ms(f), kind: "cue".into(), label, edited: false });
            }
        }
        for (i, e) in labels.accepted.iter().enumerate() {
            if let Some(f) = e.epoch_frame {
                markers.push(AudioMarker {
                    file_ms: ms(f),
                    kind: "shot".into(),
                    label: format!("Shot {} · {:.2} s", i + 1, e.timeline_ns as f64 / 1e9),
                    edited: e.origin != EventOrigin::Detected,
                });
            }
        }
        let duration_ms = d.frames * 1000 / rate;
        markers.retain(|m| (0..=duration_ms).contains(&m.file_ms));
        markers.sort_by_key(|m| m.file_ms);
        Ok(Some(DiagnosticView {
            relative_path: d.attachment.relative_path,
            bytes: d.attachment.bytes,
            duration_ms,
            sample_rate_hz: d.sample_rate_hz,
            gaps: d.gaps.len() as u32,
            truncated: d.truncated,
            dropped_frames: d.dropped_frames,
            markers,
        }))
    }

    /// Exactly what an export would contain, for the preview screen.
    pub fn diagnostic_preview(&self, run_id: String) -> Result<DiagnosticPreview, SquibError> {
        let repo = self.read_repo();
        let d = repo.diagnostic(&run_id)?.ok_or_else(|| SquibError::NotFound("no diagnostic recording for this run".into()))?;
        let (labels, _, _) = labels_for(&repo, &run_id).map_err(|e| SquibError::Invalid(e.to_string()))?;
        Ok(DiagnosticPreview {
            duration_ms: d.frames * 1000 / i64::from(d.sample_rate_hz.max(1)),
            recording_bytes: d.attachment.bytes,
            accepted_events: labels.accepted.len() as u32,
            detector_candidates: labels.candidates.len() as u32,
            reviewed_by_person: labels.reviewed_by_person,
            includes: vec![
                "The audio recording of this run (it may contain voices and other sounds nearby)".into(),
                "Your review: which events are shots, which are not".into(),
                "Detector settings and versions, and the phone's audio setup (model, Android version, microphone and speaker route)"
                    .into(),
                "Cue timing and recording quality notes (codes only)".into(),
                "The export date (day only)".into(),
            ],
            excludes: vec![
                "Location, saved places, weather stations, and conditions".into(),
                "Notes, review reasons, names, and shooter profiles".into(),
                "Any link to your history: the export gets a new random id".into(),
                "Photos, videos, and other runs".into(),
            ],
            consent_statement: CONSENT_STATEMENT.into(),
        })
    }

    /// Write the export to `out_path` (a private cache file the app then hands to the
    /// system file picker). Refused unless the consent statement was confirmed.
    pub fn export_diagnostic(
        &self,
        run_id: String,
        attachment_root: String,
        out_path: String,
        consent_confirmed: bool,
        app_version: String,
        now_utc_ms: i64,
    ) -> Result<DiagnosticExportView, SquibError> {
        let repo = self.read_repo();
        let o = DiagnosticExportOptions {
            export_id: uuid::Uuid::new_v4().to_string(),
            app_version,
            core_versions: crate::core_versions(),
            created_day: utc_day(now_utc_ms),
            consent_confirmed,
            attachment_root: Path::new(&attachment_root),
        };
        let s = export_diagnostic(&repo, &run_id, Path::new(&out_path), &o).map_err(|e| SquibError::Invalid(e.to_string()))?;
        Ok(DiagnosticExportView { bytes: s.bytes, duration_ms: s.duration_ms, accepted_events: s.accepted_events })
    }

    /// Remove a run's recording; the run and its timing stay. Returns the file path for
    /// the app to delete.
    pub fn delete_diagnostic(&self, run_id: String) -> Result<Option<String>, SquibError> {
        Ok(self.store_actor().exec(move |repo| repo.delete_diagnostic(&run_id))?)
    }

    /// At app start: remove recording files that no run owns (half-written when the
    /// app was closed mid-run, or finished but never registered). Returns how many.
    pub fn cleanup_diagnostic_partials(&self, attachment_root: String) -> u32 {
        let dir = Path::new(&attachment_root).join("diagnostics");
        let Ok(entries) = std::fs::read_dir(&dir) else { return 0 };
        let repo = self.read_repo();
        let mut n = 0;
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let owned = name.strip_suffix(".wav").is_some_and(|run| {
                repo.diagnostic(run).ok().flatten().is_some_and(|d| d.attachment.relative_path == format!("diagnostics/{name}"))
            });
            if !owned && std::fs::remove_file(e.path()).is_ok() {
                n += 1;
            }
        }
        n
    }
}
