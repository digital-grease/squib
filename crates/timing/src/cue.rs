//! Deterministic app cue templates and template-matched cue onset detection.
//!
//! The played cue and the matcher's template come from the same function, so the
//! Android player and the detector cannot diverge. The matcher correlates the template
//! aligned at its **first sample**, so the best lag is the estimated onset frame of the
//! cue in the capture stream, not a correlation peak needing later alignment.
//!
//! An arbitrary loud transient has low normalized correlation with a chirp and is not
//! accepted as a cue (A04).

use std::f64::consts::PI;

use rustfft::FftPlanner;
use rustfft::num_complex::Complex;
use serde::{Deserialize, Serialize};
use squib_domain::{CueKind, ns_to_frames};

/// Raised-cosine ramp length at both ends of a cue (ms).
pub const CUE_RAMP_MS: f64 = 3.0;
/// Default minimum normalized correlation to accept a cue match.
pub const DEFAULT_MIN_NCC: f64 = 0.5;

fn chirp_params(kind: CueKind) -> (f64, f64) {
    match kind {
        CueKind::Start => (2000.0, 4000.0),
        CueKind::Par => (3500.0, 1800.0),
    }
}

/// Unit-amplitude cue template at `sample_rate_hz`.
pub fn cue_template(kind: CueKind, sample_rate_hz: u32) -> Vec<f32> {
    let rate = f64::from(sample_rate_hz);
    let dur = f64::from(kind.duration_ms()) / 1000.0;
    let n = (dur * rate).round() as usize;
    let ramp = ((CUE_RAMP_MS / 1000.0) * rate).round() as usize;
    let (f0, f1) = chirp_params(kind);
    (0..n)
        .map(|i| {
            let t = i as f64 / rate;
            let phase = 2.0 * PI * (f0 * t + (f1 - f0) * t * t / (2.0 * dur));
            let env = if i < ramp {
                0.5 - 0.5 * (PI * i as f64 / ramp as f64).cos()
            } else if i >= n - ramp {
                0.5 - 0.5 * (PI * (n - 1 - i) as f64 / ramp as f64).cos()
            } else {
                1.0
            };
            (phase.sin() * env) as f32
        })
        .collect()
}

/// Cue template as PCM16 at the given linear gain (0..=1) for platform playback.
pub fn cue_pcm16(kind: CueKind, sample_rate_hz: u32, gain: f32) -> Vec<i16> {
    let g = gain.clamp(0.0, 1.0);
    cue_template(kind, sample_rate_hz).into_iter().map(|s| (s * g * 32767.0).round() as i16).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CueRequest {
    pub cue_id: u32,
    pub kind: CueKind,
    /// Earliest frame at which the cue onset may lie.
    pub window_start_frame: i64,
    /// Latest frame at which the cue onset may lie.
    pub window_end_frame: i64,
    pub min_ncc: f64,
}

impl CueRequest {
    /// Search window around a reference frame (render time if known, else request time).
    /// Output latency is unknown per device; when the render time is unknown the window
    /// is wider.
    pub fn around(cue_id: u32, kind: CueKind, reference_frame: i64, render_known: bool, rate: u32) -> Self {
        let before = ns_to_frames(50_000_000, rate);
        let after = ns_to_frames(if render_known { 300_000_000 } else { 800_000_000 }, rate);
        Self {
            cue_id,
            kind,
            window_start_frame: reference_frame - before,
            window_end_frame: reference_frame + after,
            min_ncc: DEFAULT_MIN_NCC,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CueMatch {
    pub cue_id: u32,
    pub kind: CueKind,
    pub template_version: String,
    /// Onset (first template sample) frame when accepted.
    pub onset_frame: Option<i64>,
    /// End (exclusive) of the matched cue span.
    pub end_frame: Option<i64>,
    /// Best normalized correlation in the window. A match score, not a probability.
    pub best_ncc: f64,
    /// Best normalized correlation outside the main lobe (ambiguity evidence).
    pub second_ncc: f64,
    pub window_start_frame: i64,
    pub window_end_frame: i64,
    /// True if the window could not be fully observed (gap or end of capture).
    pub incomplete: bool,
}

impl CueMatch {
    pub fn accepted(&self) -> bool {
        self.onset_frame.is_some()
    }
}

/// Normalized cross-correlation of `template` against `signal` for all full-overlap
/// lags. Returns NCC per lag (length `signal.len() - template.len() + 1`).
pub fn ncc(signal: &[f32], template: &[f32]) -> Vec<f64> {
    let n = template.len();
    if n == 0 || signal.len() < n {
        return vec![];
    }
    let lags = signal.len() - n + 1;
    let size = (signal.len() + n).next_power_of_two();
    let mut planner = FftPlanner::<f64>::new();
    let fwd = planner.plan_fft_forward(size);
    let inv = planner.plan_fft_inverse(size);
    let mut a: Vec<Complex<f64>> = signal.iter().map(|&x| Complex::new(f64::from(x), 0.0)).collect();
    a.resize(size, Complex::new(0.0, 0.0));
    let mut b: Vec<Complex<f64>> = template.iter().map(|&x| Complex::new(f64::from(x), 0.0)).collect();
    b.resize(size, Complex::new(0.0, 0.0));
    fwd.process(&mut a);
    fwd.process(&mut b);
    for (x, y) in a.iter_mut().zip(b.iter()) {
        *x *= y.conj();
    }
    inv.process(&mut a);
    let scale = 1.0 / size as f64;
    let t_norm = template.iter().map(|&x| f64::from(x) * f64::from(x)).sum::<f64>().sqrt();
    let mut prefix = Vec::with_capacity(signal.len() + 1);
    prefix.push(0.0f64);
    for &x in signal {
        let last = *prefix.last().unwrap();
        prefix.push(last + f64::from(x) * f64::from(x));
    }
    (0..lags)
        .map(|l| {
            let e = (prefix[l + n] - prefix[l]).max(0.0);
            let denom = t_norm * e.sqrt();
            if denom <= 1e-12 { 0.0 } else { (a[l].re * scale / denom).clamp(-1.0, 1.0) }
        })
        .collect()
}

/// Search `samples` (which begin at `samples_first_frame`) for a cue request.
pub fn match_cue(req: &CueRequest, template: &[f32], samples: &[f32], samples_first_frame: i64, incomplete: bool) -> CueMatch {
    let scores = ncc(samples, template);
    let n = template.len() as i64;
    let mut best = (f64::MIN, 0usize);
    for (i, &s) in scores.iter().enumerate() {
        let onset = samples_first_frame + i as i64;
        if onset < req.window_start_frame || onset > req.window_end_frame {
            continue;
        }
        if s > best.0 {
            best = (s, i);
        }
    }
    let mut second = 0.0f64;
    if best.0 > f64::MIN {
        let lobe = (n / 2) as usize;
        for (i, &s) in scores.iter().enumerate() {
            if i.abs_diff(best.1) > lobe && s > second {
                second = s;
            }
        }
    }
    let best_ncc = if best.0 == f64::MIN { 0.0 } else { best.0 };
    let accepted = best_ncc >= req.min_ncc;
    let onset = samples_first_frame + best.1 as i64;
    CueMatch {
        cue_id: req.cue_id,
        kind: req.kind,
        template_version: req.kind.template_version().to_string(),
        onset_frame: accepted.then_some(onset),
        end_frame: accepted.then_some(onset + n),
        best_ncc,
        second_ncc: second,
        window_start_frame: req.window_start_frame,
        window_end_frame: req.window_end_frame,
        incomplete,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_are_deterministic_and_rate_scaled() {
        let a = cue_template(CueKind::Start, 48_000);
        assert_eq!(a.len(), 7200);
        assert_eq!(a, cue_template(CueKind::Start, 48_000));
        assert_eq!(cue_template(CueKind::Start, 44_100).len(), 6615);
        assert_eq!(cue_template(CueKind::Par, 48_000).len(), 5760);
        assert_eq!(a[0], 0.0, "ramp starts at silence");
    }

    #[test]
    fn exact_onset_found_in_noise_and_par_cue_not_mistaken_for_start() {
        let rate = 48_000;
        let t = cue_template(CueKind::Start, rate);
        let mut sig = vec![0.0f32; 40_000];
        let mut s = 12345u32;
        for x in sig.iter_mut() {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            *x = ((s as f32 / u32::MAX as f32) - 0.5) * 0.02;
        }
        let onset = 12_345usize;
        for (i, v) in t.iter().enumerate() {
            sig[onset + i] += 0.2 * v;
        }
        let req =
            CueRequest { cue_id: 0, kind: CueKind::Start, window_start_frame: 1000, window_end_frame: 30_000, min_ncc: 0.5 };
        let m = match_cue(&req, &t, &sig, 1000, false);
        assert_eq!(m.onset_frame, Some(1000 + onset as i64));
        assert!(m.best_ncc > 0.9);
        let par = cue_template(CueKind::Par, rate);
        let m2 = match_cue(&req, &par, &sig, 1000, false);
        assert!(!m2.accepted(), "par template must not match the start cue (ncc {})", m2.best_ncc);
    }

    #[test]
    fn loud_impulse_is_not_a_cue() {
        let t = cue_template(CueKind::Start, 48_000);
        let mut sig = vec![0.0f32; 30_000];
        for i in 0..2000 {
            sig[10_000 + i] = 0.9 * (-(i as f32) / 300.0).exp() * if i % 2 == 0 { 1.0 } else { -1.0 };
        }
        let req = CueRequest { cue_id: 0, kind: CueKind::Start, window_start_frame: 0, window_end_frame: 22_000, min_ncc: 0.5 };
        let m = match_cue(&req, &t, &sig, 0, false);
        assert!(!m.accepted(), "impulse ncc {}", m.best_ncc);
    }
}
