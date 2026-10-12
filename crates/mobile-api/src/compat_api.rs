//! Shareable compatibility report (plan phase 4.3, ADR-054; docs/squib/01 capability
//! registry, docs/squib/09: "Compatibility entries name hardware model, OS/build, audio
//! route/source/format, app/detector versions, placement, environment cohort, and test
//! date").
//!
//! Built from what the journal already holds about live runs, grouped by audio route. It
//! carries only categories and counts: no audio, location, notes, run ids, station names,
//! or times other than the report date. The user previews the text before sharing it.

use std::collections::BTreeMap;

use squib_domain::{Outcome, Severity, StartReferenceMethod};
use squib_timing::pipeline::PipelineSummary;

use crate::engine::SquibEngine;

/// How many recent runs are examined, and how many routes are reported.
const RUNS_EXAMINED: u32 = 200;
const ROUTES_REPORTED: usize = 3;
const MAX_COMMENT: usize = 500;

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CompatInput {
    pub device: String,
    pub android_version: String,
    pub app_version: String,
    /// Where the phone was: e.g. "On a bench or table". User's choice.
    pub placement: String,
    /// e.g. "Outdoor open range". User's choice.
    pub environment: String,
    /// The user's own verdict, e.g. "Works with corrections".
    pub verdict: String,
    pub comment: String,
    pub today_utc_ms: i64,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CompatReportView {
    pub text: String,
    pub routes: u32,
    pub live_runs: u32,
}

#[derive(Default)]
struct RouteStats {
    header: String,
    signature: Option<String>,
    runs: u32,
    complete: u32,
    cue_heard: u32,
    edited: u32,
    accepted_shots: u32,
    integrity: BTreeMap<String, u32>,
    clock_quality: BTreeMap<String, u32>,
    drift_ppm: Vec<f64>,
    anchors_rejected: u32,
    discontinuities: u32,
    clipped_runs: u32,
    floor_dbfs: Vec<f32>,
}

fn clip_text(s: &str, n: usize) -> String {
    s.trim().chars().filter(|c| !c.is_control() || *c == '\n').take(n).collect()
}

fn median(v: &mut [f64]) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    Some(v[v.len() / 2])
}

#[uniffi::export]
impl SquibEngine {
    /// Build the compatibility report text for the user to preview and share.
    pub fn compatibility_report(&self, input: CompatInput) -> CompatReportView {
        let repo = self.read_repo();
        let mut routes: BTreeMap<String, RouteStats> = BTreeMap::new();
        let mut live_runs = 0;
        for s in repo.list_runs(RUNS_EXAMINED).unwrap_or_default() {
            if s.row.source_mode != "phone_live" {
                continue;
            }
            let Ok(d) = repo.load_run(&s.row.run_id) else { continue };
            let Some(epoch) = d.epoch.as_ref() else { continue };
            live_runs += 1;
            let r = &epoch.record.route;
            let format =
                format!("{} Hz, {} channel, {}", epoch.record.sample_rate_hz, epoch.record.channels, epoch.record.sample_format);
            let key = format!("{}|{}|{}|{}|{}", r.input_device, r.output_device, r.audio_source, format, r.camera_recording);
            let st = routes.entry(key).or_default();
            if st.header.is_empty() {
                st.header = format!(
                    "Input {} | output {} | source {} (unprocessed {}) | {} | effects [{}]{}",
                    r.input_device,
                    r.output_device,
                    r.audio_source,
                    if r.unprocessed_supported { "supported" } else { "not supported" },
                    format,
                    r.effects.join(", "),
                    if r.camera_recording { " | with video recording" } else { "" },
                );
                st.signature = d.config.route_signature.clone();
            }
            st.runs += 1;
            if d.row.outcome == Some(Outcome::Complete) {
                st.complete += 1;
            }
            if d.row.start_method == Some(StartReferenceMethod::AcousticCue) {
                st.cue_heard += 1;
            }
            if d.revisions.len() > 1 {
                st.edited += 1;
            }
            st.accepted_shots += s.accepted_count.unwrap_or(0);
            for q in d.quality.iter().filter(|q| q.severity == Severity::Integrity) {
                *st.integrity.entry(q.kind.as_str().to_string()).or_default() += 1;
            }
            if let Some(sum) = epoch.summary_json.as_deref().and_then(|j| serde_json::from_str::<PipelineSummary>(j).ok()) {
                *st.clock_quality.entry(sum.clock.quality.as_str().to_string()).or_default() += 1;
                st.drift_ppm.extend(sum.clock.drift_ppm);
                st.anchors_rejected += sum.clock.rejected;
                st.discontinuities += sum.clock.discontinuities;
                if sum.clipped_samples > 0 {
                    st.clipped_runs += 1;
                }
                st.floor_dbfs.push(sum.detector_floor_dbfs);
            }
        }

        let mut out = String::new();
        out.push_str("Squib compatibility report\n\n");
        out.push_str(&format!("Date: {}\n", squib_archive::iso_utc(input.today_utc_ms).chars().take(10).collect::<String>()));
        out.push_str(&format!("Device: {}\n", clip_text(&input.device, 80)));
        out.push_str(&format!("Android: {}\n", clip_text(&input.android_version, 60)));
        out.push_str(&format!("App: {}\n", clip_text(&input.app_version, 40)));
        for v in crate::core_versions() {
            out.push_str(&format!("{v}\n"));
        }
        out.push_str(&format!("Placement: {}\n", clip_text(&input.placement, 60)));
        out.push_str(&format!("Environment: {}\n", clip_text(&input.environment, 60)));
        out.push_str(&format!("Verdict (user): {}\n", clip_text(&input.verdict, 60)));
        let comment = clip_text(&input.comment, MAX_COMMENT);
        if !comment.is_empty() {
            out.push_str(&format!("Comment: {comment}\n"));
        }
        let mut ordered: Vec<RouteStats> = routes.into_values().collect();
        ordered.sort_by_key(|s| std::cmp::Reverse(s.runs));
        if ordered.is_empty() {
            out.push_str("\nNo live runs yet: run shot timing on this phone first, then build the report.\n");
        }
        let reported = ordered.len().min(ROUTES_REPORTED);
        for (i, st) in ordered.iter_mut().take(ROUTES_REPORTED).enumerate() {
            out.push_str(&format!("\nRoute {}: {}\n", i + 1, st.header));
            out.push_str(&format!(
                "Live runs {} | complete {} | start cue heard {} | reviewed with edits {} | accepted shots {}\n",
                st.runs, st.complete, st.cue_heard, st.edited, st.accepted_shots
            ));
            if !st.integrity.is_empty() {
                let parts: Vec<String> = st.integrity.iter().map(|(k, n)| format!("{k} x{n}")).collect();
                out.push_str(&format!("Interruptions and integrity events: {}\n", parts.join(", ")));
            }
            let quality: Vec<String> = st.clock_quality.iter().map(|(k, n)| format!("{k} x{n}")).collect();
            let drift = median(&mut st.drift_ppm).map(|d| format!("{d:.1} ppm")).unwrap_or_else(|| "n/a".into());
            let mut floors: Vec<f64> = st.floor_dbfs.iter().map(|f| f64::from(*f)).collect();
            let floor = median(&mut floors).map(|f| format!("{f:.0} dBFS")).unwrap_or_else(|| "n/a".into());
            out.push_str(&format!(
                "Clock: [{}] | median drift {} | anchors rejected {} | discontinuities {} | runs with clipping {} | median noise floor {}\n",
                quality.join(", "),
                drift,
                st.anchors_rejected,
                st.discontinuities,
                st.clipped_runs,
                floor
            ));
            match st.signature.as_deref().and_then(|sig| repo.latest_calibration(sig).ok().flatten()) {
                Some(c) => out.push_str(&format!(
                    "Sensitivity setup: {} with {} test impulses, threshold {:.0} dB ({})\n",
                    c.verdict, c.impulse_count, c.detector_config.threshold_db, c.algorithm_version
                )),
                None => out.push_str("Sensitivity setup: none saved for this route\n"),
            }
        }
        if ordered.len() > ROUTES_REPORTED {
            out.push_str(&format!("\n{} more routes not shown.\n", ordered.len() - ROUTES_REPORTED));
        }
        out.push_str(
            "\nContains no audio, location, notes, run identifiers, station names, or times other than the date. \
             Counts come from the recent runs on this phone.\n",
        );
        CompatReportView { text: out, routes: reported as u32, live_runs }
    }
}
