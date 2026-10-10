//! Detector stress study (M5 experiment 2): synthetic cohorts that push
//! `squib-onset-v1` toward known failure modes, and a fixed evaluation.
//!
//! The protocol (cohorts, variants, metrics, and go/no-go rule) was written before the
//! first run; see docs/implementation/m5-report.md. Everything here is deterministic.
//! Synthetic impulses probe algorithm behaviour; they are not firearm recordings and
//! say nothing about a phone's microphone or a real range (docs/squib/09).

use serde::{Deserialize, Serialize};
use squib_domain::{CueKind, DetectorConfig, DetectorSuggestion};

use crate::replay::{ReplayOptions, evaluate, replay, to_pcm16};
use crate::synth::{CueSpec, Rng, ShotSpec, SynthLabels, SynthSpec, add_shot, render};

pub const RATE: u32 = 48_000;
pub const SECONDS: f64 = 5.0;
pub const SEEDS_PER_COHORT: u64 = 6;
/// Association tolerance for labelled shots (same as the golden corpus: 0.25 ms).
pub const SHOT_TOLERANCE_FRAMES: i64 = (RATE / 4000) as i64;
/// Distractor association tolerance (rings and far shots have softer onsets).
pub const DISTRACTOR_TOLERANCE_FRAMES: i64 = (RATE / 200) as i64;

pub const COHORTS: [&str; 11] = [
    "clean_reference",
    "low_snr_30",
    "low_snr_20",
    "low_snr_15",
    "fast_splits",
    "reverb",
    "neighbour_bays",
    "steel_ring",
    "wind",
    "clipping",
    "golden_corpus",
];

fn f(rate: u32, ms: f64) -> i64 {
    (ms / 1000.0 * f64::from(rate)).round() as i64
}

/// One generated recording with ground truth.
pub struct Case {
    pub pcm: Vec<i16>,
    pub labels: SynthLabels,
    /// Onsets of events that are not the shooter's shots (other bays, steel).
    pub distractors: Vec<i64>,
}

fn base(name: &str, seed: u64, noise_dbfs: f32) -> SynthSpec {
    SynthSpec {
        name: name.into(),
        description: String::new(),
        sample_rate_hz: RATE,
        duration_frames: (SECONDS * f64::from(RATE)) as i64,
        noise_dbfs,
        seed,
        cues: vec![CueSpec { kind: CueKind::Start, onset_frame: f(RATE, 600.0), gain: 0.1 }],
        shots: vec![],
    }
}

fn shot(at_ms: f64, peak: f32) -> ShotSpec {
    ShotSpec { onset_frame: f(RATE, at_ms), peak, decay_ms: 8.0, echoes: vec![] }
}

/// Shot times from 1.4 s with the given split range (ms).
fn times(rng: &mut Rng, n: usize, split: (f64, f64)) -> Vec<f64> {
    let mut t = 1400.0 + rng.unit() * 300.0;
    let mut v = Vec::new();
    for _ in 0..n {
        v.push(t);
        t += split.0 + rng.unit() * (split.1 - split.0);
    }
    v
}

fn clip(x: &mut [f32]) -> bool {
    let mut c = false;
    for v in x.iter_mut() {
        if v.abs() >= 1.0 {
            c = true;
            *v = v.clamp(-1.0, 32767.0 / 32768.0);
        }
    }
    c
}

/// Damped sinusoid with a 0.5 ms attack: a struck steel target.
fn add_ring(out: &mut [f32], onset: i64, peak: f32, hz: f64, decay_ms: f64) {
    let r = f64::from(RATE);
    let attack = 0.0005 * r;
    for i in 0..(decay_ms / 1000.0 * r * 6.0) as i64 {
        let idx = onset + i;
        if idx < 0 || idx as usize >= out.len() {
            continue;
        }
        let t = i as f64;
        let env = if t < attack { t / attack } else { (-(t - attack) / (decay_ms / 1000.0 * r)).exp() };
        out[idx as usize] += (f64::from(peak) * env * (2.0 * std::f64::consts::PI * hz * t / r).sin()) as f32;
    }
}

/// Gusty wind: leaky-integrated (brown-like) noise with a slow random gust envelope,
/// scaled to `rms_dbfs`. Energy sits mostly below a few hundred hertz.
fn add_wind(out: &mut [f32], rng: &mut Rng, rms_dbfs: f64) {
    let r = f64::from(RATE);
    let leak = 1.0 - 2.0 * std::f64::consts::PI * 40.0 / r;
    let mut y = 0.0f64;
    let mut gust = 0.5f64;
    let mut target = 0.5f64;
    let mut w = Vec::with_capacity(out.len());
    for i in 0..out.len() {
        if i % (RATE as usize / 4) == 0 {
            target = 0.2 + rng.unit() * 0.8;
        }
        gust += (target - gust) * (1.0 / (0.3 * r));
        y = leak * y + rng.gaussian();
        w.push(y * gust);
    }
    let rms = (w.iter().map(|v| v * v).sum::<f64>() / w.len().max(1) as f64).sqrt().max(1e-12);
    let g = 10f64.powf(rms_dbfs / 20.0) / rms;
    for (o, v) in out.iter_mut().zip(w) {
        *o += (v * g) as f32;
    }
}

/// Sound added on top of the rendered recording (distractors, wind).
type Overlay = Box<dyn Fn(&mut [f32], &mut Rng)>;

/// Render one cohort recording. `index` selects the seed and parameter draw.
pub fn case(cohort: &str, index: u64) -> Option<Case> {
    let seed = 1_000 + index * 37 + COHORTS.iter().position(|c| *c == cohort)? as u64 * 1_000_003;
    let mut rng = Rng::new(seed ^ 0x5bd1_e995);
    let mut distractors = Vec::new();
    let mut extra: Vec<Overlay> = Vec::new();
    let spec = match cohort {
        "clean_reference" => {
            let mut s = base(cohort, seed, -55.0);
            s.shots = times(&mut rng, 5, (250.0, 450.0)).into_iter().map(|t| shot(t, 0.8)).collect();
            s
        }
        "low_snr_30" | "low_snr_20" | "low_snr_15" => {
            let snr: f32 = cohort[8..].parse().ok()?;
            let noise = -40.0;
            let peak = 10f32.powf((noise + snr) / 20.0);
            let mut s = base(cohort, seed, noise);
            s.cues[0].gain = 0.3;
            s.shots = times(&mut rng, 5, (250.0, 450.0)).into_iter().map(|t| shot(t, peak)).collect();
            s
        }
        "fast_splits" => {
            let mut s = base(cohort, seed, -55.0);
            s.shots = times(&mut rng, 6, (70.0, 120.0)).into_iter().map(|t| shot(t, 0.8)).collect();
            s
        }
        "reverb" => {
            let mut s = base(cohort, seed, -55.0);
            for t in times(&mut rng, 4, (350.0, 600.0)) {
                let mut sh = shot(t, 0.8);
                for k in 0..12 {
                    let d = 8.0 + rng.unit() as f32 * 242.0;
                    let lvl = -8.0 - (k as f32) * 22.0 / 11.0 - rng.unit() as f32 * 2.0;
                    sh.echoes.push((d, lvl));
                }
                s.shots.push(sh);
            }
            s
        }
        "neighbour_bays" => {
            let mut s = base(cohort, seed, -55.0);
            s.shots = times(&mut rng, 5, (300.0, 500.0)).into_iter().map(|t| shot(t, 0.8)).collect();
            for _ in 0..6 {
                let at = f(RATE, 1300.0 + rng.unit() * 3400.0);
                let peak = 0.8 * 10f32.powf((-6.0 - rng.unit() as f32 * 12.0) / 20.0);
                distractors.push(at);
                extra.push(Box::new(move |out, rng| add_shot(out, RATE, at, peak, 10.0, rng)));
            }
            s
        }
        "steel_ring" => {
            let mut s = base(cohort, seed, -55.0);
            for t in times(&mut rng, 4, (500.0, 800.0)) {
                s.shots.push(shot(t, 0.8));
                let at = f(RATE, t + 120.0 + rng.unit() * 280.0);
                let peak = 0.8 * 10f32.powf((-14.0 - rng.unit() as f32 * 10.0) / 20.0);
                let hz = 900.0 + rng.unit() * 900.0;
                distractors.push(at);
                extra.push(Box::new(move |out, _| add_ring(out, at, peak, hz, 150.0)));
            }
            s
        }
        "wind" => {
            let mut s = base(cohort, seed, -60.0);
            s.shots = times(&mut rng, 5, (250.0, 450.0)).into_iter().map(|t| shot(t, 0.5)).collect();
            let level = -32.0 + rng.unit() * 10.0;
            extra.push(Box::new(move |out, rng| add_wind(out, rng, level)));
            s
        }
        "clipping" => {
            let mut s = base(cohort, seed, -55.0);
            s.shots = times(&mut rng, 5, (250.0, 450.0)).into_iter().map(|t| shot(t, 1.5 + rng.unit() as f32 * 2.5)).collect();
            s
        }
        _ => return None,
    };
    let (mut audio, mut labels) = render(&spec);
    for e in &extra {
        e(&mut audio, &mut rng);
    }
    labels.clipped |= clip(&mut audio);
    // Reflections are real sound events but not the shooter's shots.
    distractors.extend(labels.echo_onsets.iter().copied());
    distractors.sort_unstable();
    Some(Case { pcm: to_pcm16(&audio), labels, distractors })
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CohortResult {
    pub cohort: String,
    pub variant: String,
    pub recordings: u32,
    pub labeled: u32,
    pub matched: u32,
    pub recall: f64,
    pub onset_error_p95_ms: f64,
    pub start_cue_found: u32,
    pub false_candidates: u32,
    pub false_per_min: f64,
    pub distractors: u32,
    pub distractors_detected: u32,
    pub distractors_accepted: u32,
    /// Suggested-accepted events that are not the shooter's shots, per minute.
    pub accepted_false_per_min: f64,
}

/// Evaluate one recording into `acc`.
fn add_case(acc: &mut CohortResult, c: &Case, det: &DetectorConfig, errors: &mut Vec<f64>) {
    let opts = ReplayOptions::new(det.clone()).with_label_cues(&c.labels, 20, Some(1000));
    let out = replay(&c.pcm, c.labels.sample_rate_hz, &opts);
    let m = evaluate(&out, &c.labels, SHOT_TOLERANCE_FRAMES);
    acc.recordings += 1;
    acc.labeled += m.labeled_events as u32;
    acc.matched += m.matched as u32;
    acc.start_cue_found += u32::from(m.start_cue_found);
    let ms_per_frame = 1000.0 / f64::from(c.labels.sample_rate_hz);
    errors.extend(m.onset_errors_frames.iter().map(|e| e.abs() as f64 * ms_per_frame));
    acc.distractors += c.distractors.len() as u32;
    let mut hit = vec![false; c.distractors.len()];
    for seq in &m.unmatched_candidates {
        let Some(cand) = out.classified.iter().find(|x| x.sequence == *seq) else { continue };
        let accepted = cand.classification == DetectorSuggestion::SuggestedAccepted;
        let near = c
            .distractors
            .iter()
            .enumerate()
            .filter(|(i, d)| !hit[*i] && (cand.onset_frame - **d).abs() <= DISTRACTOR_TOLERANCE_FRAMES)
            .min_by_key(|(_, d)| (cand.onset_frame - **d).abs())
            .map(|(i, _)| i);
        match near {
            Some(i) => {
                hit[i] = true;
                acc.distractors_detected += 1;
                if accepted {
                    acc.distractors_accepted += 1;
                    acc.accepted_false_per_min += 1.0;
                }
            }
            None => {
                acc.false_candidates += 1;
                if accepted {
                    acc.accepted_false_per_min += 1.0;
                }
            }
        }
    }
}

fn p95(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v[((v.len() - 1) as f64 * 0.95).round() as usize]
}

fn finish(mut acc: CohortResult, errors: Vec<f64>, minutes: f64) -> CohortResult {
    acc.recall = if acc.labeled == 0 { 1.0 } else { f64::from(acc.matched) / f64::from(acc.labeled) };
    acc.onset_error_p95_ms = p95(errors);
    acc.false_per_min = f64::from(acc.false_candidates) / minutes;
    acc.accepted_false_per_min /= minutes;
    acc
}

pub fn run_cohort(cohort: &str, variant: &str, det: &DetectorConfig) -> CohortResult {
    let mut acc = CohortResult { cohort: cohort.into(), variant: variant.into(), ..Default::default() };
    let mut errors = Vec::new();
    let mut minutes = 0.0;
    if cohort == "golden_corpus" {
        for spec in crate::synth::corpus() {
            let (audio, labels) = render(&spec);
            minutes += labels.frames as f64 / f64::from(labels.sample_rate_hz) / 60.0;
            let distractors = labels.echo_onsets.clone();
            add_case(&mut acc, &Case { pcm: to_pcm16(&audio), labels, distractors }, det, &mut errors);
        }
    } else {
        for i in 0..SEEDS_PER_COHORT {
            let Some(c) = case(cohort, i) else { break };
            minutes += SECONDS / 60.0;
            add_case(&mut acc, &c, det, &mut errors);
        }
    }
    finish(acc, errors, minutes.max(1e-9))
}

/// The pre-declared variants: (name, configuration).
pub fn variants() -> Vec<(&'static str, DetectorConfig)> {
    let d = DetectorConfig::default;
    vec![
        ("baseline", d()),
        ("threshold_18", DetectorConfig { threshold_db: 18.0, ..d() }),
        ("threshold_30", DetectorConfig { threshold_db: 30.0, ..d() }),
        ("attack_6", DetectorConfig { attack_min_db: 6.0, ..d() }),
        ("attack_12", DetectorConfig { attack_min_db: 12.0, attack_accept_db: 15.0, ..d() }),
        ("highpass_150", DetectorConfig { highpass_hz: 150.0, ..d() }),
        ("highpass_800", DetectorConfig { highpass_hz: 800.0, ..d() }),
        ("echo_drop_10", DetectorConfig { echo_drop_db: 10.0, ..d() }),
    ]
}

/// Post-hoc explorations added after seeing baseline results. They inform hypotheses
/// for a later pre-registered study and never receive a go/no-go verdict.
pub fn exploratory() -> Vec<(&'static str, DetectorConfig)> {
    let d = DetectorConfig::default;
    vec![
        ("explore_echo_window_250", DetectorConfig { echo_window_ms: 250.0, ..d() }),
        ("explore_echo_window_250_drop_10", DetectorConfig { echo_window_ms: 250.0, echo_drop_db: 10.0, ..d() }),
    ]
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VariantVerdict {
    pub variant: String,
    pub go: bool,
    pub reasons: Vec<String>,
}

/// Apply the pre-declared go/no-go rule to one variant against the baseline.
pub fn verdict(variant: &str, rows: &[CohortResult]) -> VariantVerdict {
    let get = |v: &str, c: &str| rows.iter().find(|r| r.variant == v && r.cohort == c);
    let mut reasons = Vec::new();
    let mut improved = false;
    let mut broken = false;
    for c in COHORTS {
        let (Some(b), Some(x)) = (get("baseline", c), get(variant, c)) else { continue };
        if c == "clean_reference" || c == "golden_corpus" {
            if x.recall < 1.0 || x.false_candidates > 0 {
                broken = true;
                reasons
                    .push(format!("{c}: recall {:.2}, {} false candidates (must stay 1.00 and 0)", x.recall, x.false_candidates));
            }
            continue;
        }
        let recall_gain = x.recall - b.recall;
        let fa_cut = b.accepted_false_per_min > 0.0 && x.accepted_false_per_min <= b.accepted_false_per_min / 2.0;
        if recall_gain >= 0.10 || fa_cut {
            improved = true;
            reasons.push(format!(
                "{c}: recall {:.2} -> {:.2}, accepted false/min {:.1} -> {:.1}",
                b.recall, x.recall, b.accepted_false_per_min, x.accepted_false_per_min
            ));
        }
        if recall_gain < -0.05 {
            broken = true;
            reasons.push(format!("{c}: recall drops {:.2} -> {:.2}", b.recall, x.recall));
        }
        let worse_fa = if b.accepted_false_per_min == 0.0 {
            x.accepted_false_per_min > 0.0
        } else {
            x.accepted_false_per_min > b.accepted_false_per_min * 1.25
        };
        if worse_fa {
            broken = true;
            reasons.push(format!(
                "{c}: accepted false/min rises {:.1} -> {:.1}",
                b.accepted_false_per_min, x.accepted_false_per_min
            ));
        }
    }
    if !improved {
        reasons.push("no stress cohort improved enough".into());
    }
    VariantVerdict { variant: variant.into(), go: improved && !broken, reasons }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cohorts_are_deterministic_and_labelled() {
        for c in COHORTS.iter().filter(|c| **c != "golden_corpus") {
            let a = case(c, 0).unwrap();
            let b = case(c, 0).unwrap();
            assert_eq!(a.pcm, b.pcm, "{c}");
            assert!(!a.labels.shot_onsets.is_empty(), "{c}");
            assert_eq!(a.labels.start_cue_onset, Some(f(RATE, 600.0)));
        }
        assert_eq!(case("neighbour_bays", 1).unwrap().distractors.len(), 6);
        assert_eq!(case("steel_ring", 1).unwrap().distractors.len(), 4);
        assert_eq!(case("reverb", 0).unwrap().distractors.len(), 48, "12 reflections per shot");
        assert!(case("clipping", 0).unwrap().labels.clipped);
        assert!(case("nope", 0).is_none());
    }

    #[test]
    fn baseline_passes_the_clean_reference() {
        let r = run_cohort("clean_reference", "baseline", &DetectorConfig::default());
        assert_eq!((r.recall, r.false_candidates, r.start_cue_found), (1.0, 0, SEEDS_PER_COHORT as u32), "{r:?}");
    }
}
