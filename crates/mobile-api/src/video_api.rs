//! M4 video proof of concept: register a clip recorded with a run and place the run's
//! actual event references on it. Clips are video only (no audio track), so the
//! microphone used for timing is never shared with the recorder.

use std::path::Path;

use squib_domain::{CueKind, EventOrigin};
use squib_storage::{AttachmentRecord, RunDetail, VideoClipRecord};
use squib_timing::video::{CameraClock, ClockObservations, MappingStatus, VIDEO_MAPPING_METHOD, map_clip};

use crate::engine::SquibEngine;
use crate::ffi::SquibError;

/// Longest clip the app records (keeps files under the backup entry limit).
pub const MAX_CLIP_MS: i64 = 120_000;

fn inval(e: impl ToString) -> SquibError {
    SquibError::Invalid(e.to_string())
}

/// What the platform measured while recording.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct VideoMeta {
    pub mime: String,
    pub width: u32,
    pub height: u32,
    pub frame_rate: f32,
    pub duration_ms: i64,
    /// Camera timestamp of the first frame written to the file (file time zero).
    pub first_frame_camera_ns: i64,
    /// `realtime` or `unknown` (Camera2 `SENSOR_INFO_TIMESTAMP_SOURCE`).
    pub camera_clock: String,
    /// A frame's camera timestamp and both host clocks read in its capture-started callback.
    pub probe_camera_ns: i64,
    pub probe_mono_ns: i64,
    pub probe_boot_ns: i64,
    /// `elapsedRealtimeNanos() - nanoTime()` when recording started and when it stopped.
    pub start_boot_minus_mono_ns: i64,
    pub end_boot_minus_mono_ns: i64,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct VideoMarker {
    /// Position in the file, ms from the first frame.
    pub file_ms: i64,
    /// Position on the run timeline, ms from the start reference.
    pub run_ms: i64,
    /// `start`, `par`, or `shot`.
    pub kind: String,
    pub label: String,
    /// The event's time was changed or entered by a person.
    pub edited: bool,
    /// The event's time is an estimate (manual entry, or a cue known only from playback).
    pub approximate: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct VideoClipView {
    pub attachment_id: String,
    pub relative_path: String,
    pub bytes: i64,
    pub duration_ms: i64,
    pub width: u32,
    pub height: u32,
    pub frame_rate: f32,
    pub mapping_method: String,
    /// `measured`, `assumed`, or `unavailable`.
    pub mapping: String,
    /// Plain-language explanation shown with the video.
    pub mapping_note: String,
    /// Markers are only this precise (at least one frame).
    pub uncertainty_ms: u32,
    pub markers: Vec<VideoMarker>,
    /// Events that fall outside the recorded clip.
    pub outside_clip: u32,
}

fn observations(c: &VideoClipRecord) -> Option<ClockObservations> {
    Some(ClockObservations {
        camera_clock: CameraClock::parse(&c.camera_clock)?,
        first_frame_camera_ns: c.first_frame_camera_ns,
        probe_camera_ns: c.probe_camera_ns,
        probe_mono_ns: c.probe_mono_ns,
        probe_boot_ns: c.probe_boot_ns,
        start_boot_minus_mono_ns: c.start_boot_minus_mono_ns,
        end_boot_minus_mono_ns: c.end_boot_minus_mono_ns,
        frame_period_ns: 1_000_000_000_000 / i64::from(c.frame_rate_milli.max(1)),
    })
}

/// Events on the monotonic clock: (mono_ns, run_ms, kind, label, edited, approximate).
fn run_events(d: &RunDetail, start_mono: i64) -> Vec<(i64, i64, &'static str, String, bool, bool)> {
    let mut out = vec![(start_mono, 0, "start", "Start".to_string(), false, false)];
    for c in d.cues.iter().filter(|c| c.kind == CueKind::Par) {
        let (mono, approx) = match (c.acoustic_onset_mono_ns, c.render_mono_ns) {
            (Some(a), _) => (a, false),
            (None, Some(r)) => (r, true),
            (None, None) => continue,
        };
        let n = c.par_index.map(|i| i + 1).unwrap_or(0);
        out.push((mono, (mono - start_mono) / 1_000_000, "par", format!("Par {n}"), false, approx));
    }
    if let Some(rev) = d.revisions.last().filter(|r| r.content.origin_resolved) {
        for (i, e) in rev.content.accepted.iter().enumerate() {
            let t = e.timeline_ns;
            out.push((
                start_mono + t,
                t / 1_000_000,
                "shot",
                format!("Shot {} · {:.2} s", i + 1, t as f64 / 1e9),
                e.origin != EventOrigin::Detected,
                e.origin == EventOrigin::Manual,
            ));
        }
    }
    out.sort_by_key(|e| e.0);
    out
}

/// `measured`, `assumed`, or `unavailable` for a stored clip.
pub(crate) fn mapping_label(c: &VideoClipRecord) -> &'static str {
    match observations(c).map(|o| map_clip(&o).status) {
        Some(MappingStatus::Measured) => "measured",
        Some(MappingStatus::Assumed) => "assumed",
        _ => "unavailable",
    }
}

pub(crate) fn clip_view(c: &VideoClipRecord, d: &RunDetail, start_mono: Option<i64>) -> VideoClipView {
    let mut view = VideoClipView {
        attachment_id: c.attachment.id.clone(),
        relative_path: c.attachment.relative_path.clone(),
        bytes: c.attachment.bytes,
        duration_ms: c.duration_ms,
        width: c.width,
        height: c.height,
        frame_rate: c.frame_rate_milli as f32 / 1000.0,
        mapping_method: VIDEO_MAPPING_METHOD.into(),
        mapping: "unavailable".into(),
        mapping_note: String::new(),
        uncertainty_ms: 0,
        markers: vec![],
        outside_clip: 0,
    };
    let Some(o) = observations(c) else {
        view.mapping_note = "Not aligned: unrecognized camera clock.".into();
        return view;
    };
    let m = map_clip(&o);
    view.uncertainty_ms = (m.uncertainty_ns / 1_000_000) as u32;
    let note = match &m.status {
        MappingStatus::Measured => {
            "Aligned from the camera's reported clock. Experimental: not yet checked against a real phone's audio.".to_string()
        }
        MappingStatus::Assumed => {
            "Aligned on an assumption: this camera does not say which clock it uses, and its timestamps only looked consistent with the phone's. Treat markers as approximate.".to_string()
        }
        MappingStatus::Unavailable { reason } => format!("Not aligned: {reason}. The video plays without markers."),
    };
    view.mapping = match m.status {
        MappingStatus::Measured => "measured",
        MappingStatus::Assumed => "assumed",
        MappingStatus::Unavailable { .. } => "unavailable",
    }
    .into();
    view.mapping_note = note;
    if m.first_frame_mono_ns.is_none() {
        return view;
    }
    let Some(start) = start_mono else {
        view.mapping_note.push_str(" This run has no resolved start reference, so there are no markers.");
        return view;
    };
    for (mono, run_ms, kind, label, edited, approximate) in run_events(d, start) {
        match m.file_ms(mono) {
            Some(f) if (0..=c.duration_ms).contains(&f) => {
                view.markers.push(VideoMarker { file_ms: f, run_ms, kind: kind.into(), label, edited, approximate })
            }
            _ => view.outside_clip += 1,
        }
    }
    view
}

#[uniffi::export]
impl SquibEngine {
    /// Attach a recorded clip to a run. The file must already be inside the app's
    /// attachment directory; it is hashed here and never modified.
    pub fn register_video(
        &self,
        run_id: String,
        attachment_root: String,
        relative_path: String,
        meta: VideoMeta,
        now_utc_ms: i64,
    ) -> Result<VideoClipView, SquibError> {
        if CameraClock::parse(&meta.camera_clock).is_none() {
            return Err(inval("camera clock must be realtime or unknown"));
        }
        if !(1.0..=240.0).contains(&meta.frame_rate) || meta.width == 0 || meta.height == 0 {
            return Err(inval("invalid video format"));
        }
        if !(0..=MAX_CLIP_MS + 5_000).contains(&meta.duration_ms) {
            return Err(inval("clip is longer than the recording limit"));
        }
        let (sha256, bytes) =
            squib_domain::hash::sha256_file(&Path::new(&attachment_root).join(&relative_path)).map_err(inval)?;
        let rec = VideoClipRecord {
            attachment: AttachmentRecord {
                id: uuid::Uuid::new_v4().to_string(),
                run_id: Some(run_id.clone()),
                kind: "video".into(),
                relative_path,
                sha256,
                bytes: bytes as i64,
                mime: meta.mime,
                // Written by the app's own muxer with no location or audio track.
                metadata_stripped: true,
                created_utc_ms: now_utc_ms,
            },
            run_id: run_id.clone(),
            width: meta.width,
            height: meta.height,
            frame_rate_milli: (meta.frame_rate * 1000.0).round() as u32,
            duration_ms: meta.duration_ms,
            first_frame_camera_ns: meta.first_frame_camera_ns,
            camera_clock: meta.camera_clock,
            probe_camera_ns: meta.probe_camera_ns,
            probe_mono_ns: meta.probe_mono_ns,
            probe_boot_ns: meta.probe_boot_ns,
            start_boot_minus_mono_ns: meta.start_boot_minus_mono_ns,
            end_boot_minus_mono_ns: meta.end_boot_minus_mono_ns,
            created_utc_ms: now_utc_ms,
        };
        let id = rec.attachment.id.clone();
        self.store_actor().exec(move |repo| repo.insert_video_clip(&rec))?;
        self.run_videos(run_id)?.into_iter().find(|v| v.attachment_id == id).ok_or(SquibError::NotFound(id))
    }

    /// Clips for a run with markers from the run's latest review revision.
    pub fn run_videos(&self, run_id: String) -> Result<Vec<VideoClipView>, SquibError> {
        let repo = self.read_repo();
        let clips = repo.video_clips(&run_id)?;
        if clips.is_empty() {
            return Ok(vec![]);
        }
        let detail = repo.load_run(&run_id)?;
        let start = repo.start_ref_mono_ns(&run_id)?;
        Ok(clips.iter().map(|c| clip_view(c, &detail, start)).collect())
    }
}
