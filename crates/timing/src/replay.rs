//! Deterministic replay of PCM through the production pipeline.
//!
//! Simulates the adapter: arbitrary block chunking, platform anchors from a synthetic
//! monotonic clock, injected delivery delays, dropped blocks, and late cue requests.
//! Used by the corpus tests and the `squib-replay` CLI.

use serde::{Deserialize, Serialize};
use squib_domain::{
    Candidate, ClassifiedCandidate, ClassifyContext, CueKind, CueSpan, DetectorConfig, DetectorSuggestion, QualityEvent,
    classify, frames_to_ns, ns_to_frames,
};

use crate::block::{BlockHeader, EpochFormat, SampleFormat};
use crate::clock::FrameAnchor;
use crate::cue::{CueMatch, CueRequest};
use crate::pipeline::{DspEvent, Pipeline, PipelineSummary};
use crate::synth::{Rng, SynthLabels};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Chunking {
    Fixed { frames: u32 },
    Pseudorandom { seed: u64, min: u32, max: u32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CueReference {
    pub cue_id: u32,
    pub kind: CueKind,
    /// Frame at which the simulated app learned the cue was rendered.
    pub reference_frame: i64,
    pub render_known: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayOptions {
    pub chunking: Chunking,
    pub detector: DetectorConfig,
    pub cues: Vec<CueReference>,
    /// Emit a platform anchor every N blocks (None: no anchors).
    pub anchor_every_blocks: Option<u32>,
    /// Monotonic time of frame 0 on the synthetic clock.
    pub clock_base_ns: i64,
    /// Added to delivery time every `delay_every` blocks (A03 injection).
    pub delivery_delay_ns: i64,
    pub delay_every: u32,
    /// Drop this block sequence (gap injection).
    pub drop_block: Option<u64>,
    pub platform_buffer_frames: Option<u32>,
    /// Stop cutoff frame for classification.
    pub stop_frame: Option<i64>,
}

impl ReplayOptions {
    pub fn new(detector: DetectorConfig) -> Self {
        Self {
            chunking: Chunking::Fixed { frames: 480 },
            detector,
            cues: vec![],
            anchor_every_blocks: Some(10),
            clock_base_ns: 1_000_000_000_000,
            delivery_delay_ns: 0,
            delay_every: 0,
            drop_block: None,
            platform_buffer_frames: None,
            stop_frame: None,
        }
    }

    /// Cue references derived from labels: the app learns of rendering
    /// `render_lag_ms` before the cue reaches the microphone.
    pub fn with_label_cues(mut self, labels: &SynthLabels, render_lag_ms: i64, fallback_start_ms: Option<i64>) -> Self {
        let rate = labels.sample_rate_hz;
        let lag = ns_to_frames(render_lag_ms * 1_000_000, rate);
        match (labels.start_cue_onset, fallback_start_ms) {
            (Some(on), _) => {
                self.cues.push(CueReference { cue_id: 0, kind: CueKind::Start, reference_frame: on - lag, render_known: true })
            }
            (None, Some(ms)) => self.cues.push(CueReference {
                cue_id: 0,
                kind: CueKind::Start,
                reference_frame: ns_to_frames(ms * 1_000_000, rate),
                render_known: true,
            }),
            _ => {}
        }
        for (i, &on) in labels.par_cue_onsets.iter().enumerate() {
            self.cues.push(CueReference {
                cue_id: i as u32 + 1,
                kind: CueKind::Par,
                reference_frame: on - lag,
                render_known: true,
            });
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayOutput {
    pub candidates: Vec<Candidate>,
    pub cues: Vec<CueMatch>,
    pub quality: Vec<QualityEvent>,
    pub classified: Vec<ClassifiedCandidate>,
    pub summary: PipelineSummary,
    pub envelope_bytes: usize,
    /// Per-block processing time (ns) when measured by the caller; empty in tests.
    pub block_sizes: Vec<u32>,
}

impl ReplayOutput {
    pub fn start_onset(&self) -> Option<i64> {
        self.cues.iter().find(|c| c.kind == CueKind::Start).and_then(|c| c.onset_frame)
    }
}

/// Quantize to PCM16 as a device capture would.
pub fn to_pcm16(samples: &[f32]) -> Vec<i16> {
    samples.iter().map(|&x| (x.clamp(-1.0, 1.0) * 32768.0).round().clamp(-32768.0, 32767.0) as i16).collect()
}

pub fn replay(pcm: &[i16], sample_rate_hz: u32, opts: &ReplayOptions) -> ReplayOutput {
    replay_with(pcm, sample_rate_hz, opts, |_, _| {})
}

/// Replay with a per-block hook (used for timing measurements).
pub fn replay_with(
    pcm: &[i16],
    sample_rate_hz: u32,
    opts: &ReplayOptions,
    mut on_block: impl FnMut(&BlockHeader, std::time::Duration),
) -> ReplayOutput {
    let fmt = EpochFormat {
        sample_rate_hz,
        channels: 1,
        format: SampleFormat::Pcm16,
        platform_buffer_frames: opts.platform_buffer_frames,
    };
    let mut p = Pipeline::new(fmt, Some(opts.detector.clone()));
    let mut out_c = Vec::new();
    let mut out_cue = Vec::new();
    let mut out_q = Vec::new();
    let mut env_bytes = 0usize;
    let mut sink = |e: DspEvent| match e {
        DspEvent::Candidate(c) => out_c.push(c),
        DspEvent::Cue(m) => out_cue.push(m),
        DspEvent::Quality(q) => out_q.push(q),
        DspEvent::Envelope(c) => env_bytes += c.data.len(),
    };
    let mut rng = match opts.chunking {
        Chunking::Pseudorandom { seed, .. } => Some(Rng::new(seed)),
        Chunking::Fixed { .. } => None,
    };
    let mut pending_cues: Vec<&CueReference> = opts.cues.iter().collect();
    let mono = |f: i64| opts.clock_base_ns + frames_to_ns(f, sample_rate_hz);
    let mut pos = 0usize;
    let mut seq = 0u64;
    let mut sizes = Vec::new();
    while pos < pcm.len() {
        let n = match (&opts.chunking, rng.as_mut()) {
            (Chunking::Fixed { frames }, _) => *frames as usize,
            (Chunking::Pseudorandom { min, max, .. }, Some(r)) => {
                (*min as usize) + (r.next_u64() % u64::from(max - min + 1)) as usize
            }
            _ => unreachable!(),
        }
        .max(1)
        .min(pcm.len() - pos);
        let first = pos as i64;
        let end = first + n as i64;
        let this_seq = seq;
        seq += 1;
        pos += n;
        if opts.drop_block == Some(this_seq) {
            continue; // adapter lost this block entirely
        }
        let anchor = opts
            .anchor_every_blocks
            .filter(|&k| k > 0 && this_seq.is_multiple_of(u64::from(k)))
            .map(|_| FrameAnchor { frame: end, mono_ns: mono(end) });
        let delayed = opts.delay_every > 0 && this_seq.is_multiple_of(u64::from(opts.delay_every));
        let h = BlockHeader {
            sequence: this_seq,
            first_frame: first,
            frame_count: n as u32,
            sample_rate_hz,
            channels: 1,
            format: SampleFormat::Pcm16,
            anchor,
            delivered_ns: Some(mono(end) + 1_000_000 + if delayed { opts.delivery_delay_ns } else { 0 }),
            flags: 0,
        };
        let t0 = std::time::Instant::now();
        p.process_i16(&h, &pcm[first as usize..end as usize], &mut sink);
        on_block(&h, t0.elapsed());
        sizes.push(n as u32);
        // Cue requests arrive late, after rendering is reported (exercises history).
        pending_cues.retain(|c| {
            let issue_at = c.reference_frame + ns_to_frames(100_000_000, sample_rate_hz);
            if end >= issue_at {
                let req = CueRequest::around(c.cue_id, c.kind, c.reference_frame, c.render_known, sample_rate_hz);
                p.request_cue(req, &mut sink);
                false
            } else {
                true
            }
        });
    }
    p.finish(&mut sink);
    let summary = p.summary();

    let start_frame = out_cue.iter().find(|c| c.kind == CueKind::Start).and_then(|c| c.onset_frame);
    let cue_spans = out_cue
        .iter()
        .filter_map(|c| {
            Some(CueSpan {
                start_frame: c.onset_frame?,
                end_frame: c.end_frame?,
                is_start_cue: c.kind == CueKind::Start,
                exact: true,
            })
        })
        .collect();
    let ctx = ClassifyContext { sample_rate_hz, start_frame, cue_spans, stop_frame: opts.stop_frame };
    let classified = classify(&out_c, &ctx);
    ReplayOutput {
        candidates: out_c,
        cues: out_cue,
        quality: out_q,
        classified,
        summary,
        envelope_bytes: env_bytes,
        block_sizes: sizes,
    }
}

/// Comparison against labels using a predeclared association tolerance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    pub tolerance_frames: i64,
    pub labeled_events: usize,
    pub matched: usize,
    pub missed_onsets: Vec<i64>,
    /// Non-rejected candidates matching no labeled direct impulse.
    pub unmatched_candidates: Vec<u64>,
    /// Detected − labeled onset, frames, for matched events.
    pub onset_errors_frames: Vec<i64>,
    pub start_cue_error_frames: Option<i64>,
    pub start_cue_expected: bool,
    pub start_cue_found: bool,
    /// Matched events whose classification is SuggestedAccepted.
    pub matched_suggested_accepted: usize,
}

pub fn evaluate(out: &ReplayOutput, labels: &SynthLabels, tolerance_frames: i64) -> Metrics {
    let live: Vec<&ClassifiedCandidate> =
        out.classified.iter().filter(|c| c.classification != DetectorSuggestion::Rejected).collect();
    let mut used = vec![false; live.len()];
    let mut matched = 0;
    let mut missed = Vec::new();
    let mut errors = Vec::new();
    let mut acc = 0;
    for &on in &labels.shot_onsets {
        let best = live
            .iter()
            .enumerate()
            .filter(|(i, c)| !used[*i] && (c.onset_frame - on).abs() <= tolerance_frames)
            .min_by_key(|(_, c)| (c.onset_frame - on).abs());
        match best {
            Some((i, c)) => {
                used[i] = true;
                matched += 1;
                errors.push(c.onset_frame - on);
                if c.classification == DetectorSuggestion::SuggestedAccepted {
                    acc += 1;
                }
            }
            None => missed.push(on),
        }
    }
    let unmatched = live.iter().zip(used.iter()).filter(|(_, u)| !**u).map(|(c, _)| c.sequence).collect();
    Metrics {
        tolerance_frames,
        labeled_events: labels.shot_onsets.len(),
        matched,
        missed_onsets: missed,
        unmatched_candidates: unmatched,
        onset_errors_frames: errors,
        start_cue_error_frames: match (out.start_onset(), labels.start_cue_onset) {
            (Some(a), Some(b)) => Some(a - b),
            _ => None,
        },
        start_cue_expected: labels.start_cue_onset.is_some(),
        start_cue_found: out.start_onset().is_some(),
        matched_suggested_accepted: acc,
    }
}
