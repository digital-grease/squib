//! Deterministic synthetic fixtures with known cue/impulse frame indices.
//!
//! Synthetic audio probes algorithm invariants (onset placement, overlap handling,
//! chunking). It cannot prove firearm acoustics in the field (docs/squib/09).

use serde::{Deserialize, Serialize};
use squib_domain::CueKind;

use crate::cue::cue_template;

/// xorshift64* PRNG: deterministic across platforms; not for security.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    pub fn gaussian(&mut self) -> f64 {
        let u1 = self.unit().max(1e-300);
        let u2 = self.unit();
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShotSpec {
    pub onset_frame: i64,
    /// Peak linear amplitude (may exceed 1.0 to force clipping).
    pub peak: f32,
    pub decay_ms: f32,
    /// (delay ms, level relative to direct in dB, negative).
    pub echoes: Vec<(f32, f32)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CueSpec {
    pub kind: CueKind,
    pub onset_frame: i64,
    pub gain: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SynthSpec {
    pub name: String,
    pub description: String,
    pub sample_rate_hz: u32,
    pub duration_frames: i64,
    /// White noise RMS level (dBFS).
    pub noise_dbfs: f32,
    pub seed: u64,
    pub cues: Vec<CueSpec>,
    pub shots: Vec<ShotSpec>,
}

/// Ground-truth labels written next to a fixture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SynthLabels {
    pub name: String,
    pub description: String,
    pub sample_rate_hz: u32,
    pub frames: i64,
    pub start_cue_onset: Option<i64>,
    pub par_cue_onsets: Vec<i64>,
    /// Direct (non-echo) impulse onsets, in order.
    pub shot_onsets: Vec<i64>,
    pub echo_onsets: Vec<i64>,
    pub clipped: bool,
}

/// Rise of a synthetic impulse to its peak (ms).
pub const SHOT_RISE_MS: f64 = 0.1;

fn add_shot(out: &mut [f32], rate: u32, onset: i64, peak: f32, decay_ms: f32, rng: &mut Rng) {
    let r = f64::from(rate);
    let rise = (SHOT_RISE_MS / 1000.0 * r).max(1.0);
    let len = (f64::from(decay_ms) / 1000.0 * r * 8.0) as i64;
    for i in 0..len {
        let idx = onset + i;
        if idx < 0 || idx as usize >= out.len() {
            continue;
        }
        let t = i as f64;
        let env = if t < rise { (t + 1.0) / rise } else { (-(t - rise) / (f64::from(decay_ms) / 1000.0 * r)).exp() };
        let carrier = 2.0 * rng.unit() - 1.0;
        let sign = if carrier >= 0.0 { 1.0 } else { -1.0 };
        // Magnitude in [0.5, 1]: keeps the leading edge sharp and unambiguous.
        let v = sign * (0.5 + 0.5 * carrier.abs());
        out[idx as usize] += (f64::from(peak) * env.min(1.0) * v) as f32;
    }
}

pub fn render(spec: &SynthSpec) -> (Vec<f32>, SynthLabels) {
    let n = usize::try_from(spec.duration_frames).expect("non-negative duration");
    let mut rng = Rng::new(spec.seed);
    let sigma = 10f64.powf(f64::from(spec.noise_dbfs) / 20.0);
    let mut out: Vec<f32> = (0..n).map(|_| (rng.gaussian() * sigma) as f32).collect();
    let mut labels = SynthLabels {
        name: spec.name.clone(),
        description: spec.description.clone(),
        sample_rate_hz: spec.sample_rate_hz,
        frames: spec.duration_frames,
        start_cue_onset: None,
        par_cue_onsets: vec![],
        shot_onsets: vec![],
        echo_onsets: vec![],
        clipped: false,
    };
    for c in &spec.cues {
        let t = cue_template(c.kind, spec.sample_rate_hz);
        for (i, v) in t.iter().enumerate() {
            let idx = c.onset_frame as usize + i;
            if idx < n {
                out[idx] += c.gain * v;
            }
        }
        match c.kind {
            CueKind::Start => labels.start_cue_onset = Some(c.onset_frame),
            CueKind::Par => labels.par_cue_onsets.push(c.onset_frame),
        }
    }
    for s in &spec.shots {
        add_shot(&mut out, spec.sample_rate_hz, s.onset_frame, s.peak, s.decay_ms, &mut rng);
        labels.shot_onsets.push(s.onset_frame);
        for &(delay_ms, rel_db) in &s.echoes {
            let d = (f64::from(delay_ms) / 1000.0 * f64::from(spec.sample_rate_hz)).round() as i64;
            let g = s.peak * 10f32.powf(rel_db / 20.0);
            add_shot(&mut out, spec.sample_rate_hz, s.onset_frame + d, g, s.decay_ms, &mut rng);
            labels.echo_onsets.push(s.onset_frame + d);
        }
    }
    for x in out.iter_mut() {
        if x.abs() >= 1.0 {
            labels.clipped = true;
            *x = x.clamp(-1.0, 32767.0 / 32768.0);
        }
    }
    labels.shot_onsets.sort_unstable();
    labels.echo_onsets.sort_unstable();
    (out, labels)
}

fn ms(rate: u32, v: f64) -> i64 {
    (v / 1000.0 * f64::from(rate)).round() as i64
}

/// The golden synthetic corpus. Each fixture probes a named failure mode.
pub fn corpus() -> Vec<SynthSpec> {
    let base = |name: &str, desc: &str, rate: u32, secs: f64, seed: u64| SynthSpec {
        name: name.into(),
        description: desc.into(),
        sample_rate_hz: rate,
        duration_frames: (secs * f64::from(rate)) as i64,
        noise_dbfs: -55.0,
        seed,
        cues: vec![],
        shots: vec![],
    };
    let shot = |rate: u32, at_ms: f64| ShotSpec { onset_frame: ms(rate, at_ms), peak: 0.8, decay_ms: 8.0, echoes: vec![] };
    let cue = |rate: u32, kind: CueKind, at_ms: f64, gain: f32| CueSpec { kind, onset_frame: ms(rate, at_ms), gain };

    let mut v = Vec::new();

    let r = 48_000;
    let mut s = base("basic_5_shots_48k", "Start cue then five well separated impulses.", r, 4.0, 1);
    s.cues.push(cue(r, CueKind::Start, 1000.0, 0.1));
    for t in [1650.0, 1980.0, 2290.0, 2610.0, 2950.0] {
        s.shots.push(shot(r, t));
    }
    v.push(s);

    let r = 44_100;
    let mut s = base("basic_5_shots_44k1", "Same timeline at 44.1 kHz: durations derive from actual rate.", r, 4.0, 2);
    s.cues.push(cue(r, CueKind::Start, 1000.0, 0.1));
    for t in [1650.0, 1980.0, 2290.0, 2610.0, 2950.0] {
        s.shots.push(shot(r, t));
    }
    v.push(s);

    let r = 48_000;
    let mut s = base(
        "fast_shot_during_cue",
        "Impulse 90 ms after cue onset (inside the 150 ms cue) and one 30 ms after cue end.",
        r,
        3.0,
        3,
    );
    s.cues.push(cue(r, CueKind::Start, 1000.0, 0.1));
    s.shots.push(shot(r, 1090.0));
    s.shots.push(shot(r, 1180.0));
    v.push(s);

    let r = 48_000;
    let mut s = base("echoes_indoor", "Impulses with reflections at 18 ms (−10 dB) and 41 ms (−14 dB).", r, 3.5, 4);
    s.cues.push(cue(r, CueKind::Start, 800.0, 0.1));
    for t in [1500.0, 1900.0, 2400.0] {
        let mut sh = shot(r, t);
        sh.echoes = vec![(18.0, -10.0), (41.0, -14.0)];
        s.shots.push(sh);
    }
    v.push(s);

    let r = 48_000;
    let mut s = base("adjacent_impulses", "Impulse pairs 60 ms and 40 ms apart.", r, 3.0, 5);
    s.cues.push(cue(r, CueKind::Start, 700.0, 0.1));
    for t in [1300.0, 1360.0, 2000.0, 2040.0] {
        s.shots.push(shot(r, t));
    }
    v.push(s);

    let r = 48_000;
    let mut s = base("clipped_impulses", "Impulses driven past full scale.", r, 3.0, 6);
    s.cues.push(cue(r, CueKind::Start, 700.0, 0.1));
    for t in [1400.0, 1800.0] {
        let mut sh = shot(r, t);
        sh.peak = 2.5;
        s.shots.push(sh);
    }
    v.push(s);

    let r = 48_000;
    let mut s = base("par_cue_in_stream", "Par cue audio must not count as an impulse.", r, 4.0, 7);
    s.cues.push(cue(r, CueKind::Start, 600.0, 0.1));
    s.cues.push(cue(r, CueKind::Par, 2600.0, 0.1));
    s.shots.push(shot(r, 1500.0));
    s.shots.push(shot(r, 3200.0));
    v.push(s);

    let r = 48_000;
    let mut s =
        base("no_cue_loud_transient", "No cue: a loud impulse in the cue window must not become the start (A04).", r, 3.0, 8);
    s.shots.push(shot(r, 1050.0));
    s.shots.push(shot(r, 1600.0));
    v.push(s);

    let r = 48_000;
    let mut s = base("ambient_no_events", "Cue then ambient noise only: false positives per minute.", r, 6.0, 9);
    s.cues.push(cue(r, CueKind::Start, 500.0, 0.1));
    s.noise_dbfs = -45.0;
    v.push(s);

    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendering_is_deterministic_and_labels_match_spec() {
        let c = corpus();
        let (a, la) = render(&c[0]);
        let (b, lb) = render(&c[0]);
        assert_eq!(a, b);
        assert_eq!(la, lb);
        assert_eq!(la.shot_onsets.len(), 5);
        assert_eq!(la.start_cue_onset, Some(48_000));
        let (_, lc) = render(c.iter().find(|s| s.name == "clipped_impulses").unwrap());
        assert!(lc.clipped);
    }
}
