//! Deterministic app cue sounds and template-matched cue onset detection (ADR-067).
//!
//! The cues copy a range shot timer. Measured from recordings of a PACT Club Timer III,
//! its beep is a 2327 Hz square-wave piezo tone (third harmonic about 12 dB and fifth
//! about 24 dB below the fundamental, no even harmonics to speak of) that switches on and
//! off in about 1 ms: 300 ms for the start, 500 ms for a par. Both cues here are that tone
//! at those lengths.
//!
//! The played cue and the matcher's template come from the same function, so the
//! Android player and the detector cannot diverge. The reported onset is the frame of the
//! template's **first sample** in the capture stream.
//!
//! A steady tone correlates almost as well a few cycles away from its true start, and in
//! a reverberant room tens of milliseconds late, so correlation alone cannot time it.
//! Matching therefore has two parts:
//! - the tone's envelope finds the first cue in the search window, and its **leading
//!   edge** times the onset; echoes cannot move the edge because they arrive after it. An
//!   edge that cannot be told apart from sound already there (a previous cue's echo, say)
//!   is reported unresolved rather than guessed;
//! - normalized correlation of the whole template at that onset decides whether it is the
//!   cue at all. An arbitrary loud transient correlates poorly with a 300 ms tone and is
//!   not accepted as a cue (A04).

use std::collections::VecDeque;
use std::f64::consts::PI;
use std::ops::{Add, Mul, Sub};

use rustfft::FftPlanner;
use rustfft::num_complex::Complex;
use serde::{Deserialize, Serialize};
use squib_domain::{CueKind, ns_to_frames};

/// Fundamental of both cues (Hz), as measured from a PACT Club Timer III.
pub const CUE_TONE_HZ: f64 = 2327.0;
/// Harmonic number and level relative to the fundamental (dB): the piezo's odd harmonics.
const CUE_HARMONICS: [(u32, f64); 3] = [(1, 0.0), (3, -12.0), (5, -24.0)];
/// Harmonics at or above this fraction of the sample rate are left out (no aliasing).
const HARMONIC_LIMIT: f64 = 0.45;
/// Raised-cosine ramp length at both ends of a cue (ms).
pub const CUE_RAMP_MS: f64 = 1.0;
/// Default minimum normalized correlation to accept a cue match.
pub const DEFAULT_MIN_NCC: f64 = 0.5;

// Leading-edge search, fixed with the template (tuned on simulated phone speaker and
// room responses, ADR-067).
/// The first local maximum of the envelope correlation within this fraction of the
/// strongest is the first cue in the window.
const ANCHOR_LOBE: f64 = 0.8;
/// A local maximum is the largest value within this distance either side (ms).
const ANCHOR_NEIGHBOURHOOD_MS: f64 = 10.0;
/// The edge is sought back from this far past the anchor, safely inside the tone (ms).
const ANCHOR_STEP_IN_MS: f64 = 5.0;
/// The noise floor is the quietest stretch of this length in the search buffer (ms).
const NOISE_WINDOW_MS: f64 = 10.0;
/// The tone is present where its envelope is 12 dB above the noise floor...
const RUN_OVER_NOISE: f64 = 4.0;
/// ...and no more than 20 dB below the cue's body.
const RUN_UNDER_BODY: f64 = 0.1;
/// Level just before the edge is averaged over this span (ms).
const EDGE_BEFORE_MS: f64 = 2.0;
/// The edge's peak is sought within this span after the tone appears (ms).
const EDGE_PEAK_MS: f64 = 3.0;
/// A real edge rises at least 12 dB above what came just before it.
const EDGE_MIN_RISE: f64 = 4.0;

/// Unit-peak cue template at `sample_rate_hz`.
pub fn cue_template(kind: CueKind, sample_rate_hz: u32) -> Vec<f32> {
    let rate = f64::from(sample_rate_hz);
    let n = (f64::from(kind.duration_ms()) / 1000.0 * rate).round() as usize;
    let ramp = ((CUE_RAMP_MS / 1000.0) * rate).round() as usize;
    let parts: Vec<(f64, f64)> = CUE_HARMONICS
        .iter()
        .map(|&(h, db)| (CUE_TONE_HZ * f64::from(h), 10f64.powf(db / 20.0)))
        .filter(|&(f, _)| f < HARMONIC_LIMIT * rate)
        .collect();
    let raw: Vec<f64> = (0..n)
        .map(|i| {
            let t = i as f64 / rate;
            parts.iter().map(|&(f, a)| a * (2.0 * PI * f * t).sin()).sum()
        })
        .collect();
    let peak = raw.iter().fold(0.0f64, |m, v| m.max(v.abs())).max(f64::MIN_POSITIVE);
    raw.iter()
        .enumerate()
        .map(|(i, &v)| {
            let env = if i < ramp {
                0.5 - 0.5 * (PI * i as f64 / ramp as f64).cos()
            } else if i >= n - ramp {
                0.5 - 0.5 * (PI * (n - 1 - i) as f64 / ramp as f64).cos()
            } else {
                1.0
            };
            (v / peak * env) as f32
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
    /// Normalized correlation at the accepted onset, or the best in the window when no
    /// onset was accepted. A match score, not a probability.
    pub best_ncc: f64,
    /// Best normalized correlation more than a template length from that lag (ambiguity
    /// evidence).
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

/// Cross-correlation of `template` against `signal` at every full-overlap lag.
fn xcorr(signal: &[f64], template: &[f64]) -> Vec<f64> {
    let n = template.len();
    if n == 0 || signal.len() < n {
        return vec![];
    }
    let lags = signal.len() - n + 1;
    let size = (signal.len() + n).next_power_of_two();
    let mut planner = FftPlanner::<f64>::new();
    let fwd = planner.plan_fft_forward(size);
    let inv = planner.plan_fft_inverse(size);
    let mut a: Vec<Complex<f64>> = signal.iter().map(|&x| Complex::new(x, 0.0)).collect();
    a.resize(size, Complex::new(0.0, 0.0));
    let mut b: Vec<Complex<f64>> = template.iter().map(|&x| Complex::new(x, 0.0)).collect();
    b.resize(size, Complex::new(0.0, 0.0));
    fwd.process(&mut a);
    fwd.process(&mut b);
    for (x, y) in a.iter_mut().zip(b.iter()) {
        *x *= y.conj();
    }
    inv.process(&mut a);
    let scale = 1.0 / size as f64;
    a[..lags].iter().map(|c| c.re * scale).collect()
}

/// Normalized cross-correlation of `template` against `signal` for all full-overlap
/// lags. Returns NCC per lag (length `signal.len() - template.len() + 1`).
pub fn ncc(signal: &[f32], template: &[f32]) -> Vec<f64> {
    let s: Vec<f64> = signal.iter().map(|&x| f64::from(x)).collect();
    let t: Vec<f64> = template.iter().map(|&x| f64::from(x)).collect();
    let corr = xcorr(&s, &t);
    let n = t.len();
    let t_norm = t.iter().map(|x| x * x).sum::<f64>().sqrt();
    let mut prefix = Vec::with_capacity(s.len() + 1);
    prefix.push(0.0f64);
    for &x in &s {
        let last = *prefix.last().unwrap();
        prefix.push(last + x * x);
    }
    corr.iter()
        .enumerate()
        .map(|(l, &c)| {
            let e = (prefix[l + n] - prefix[l]).max(0.0);
            let denom = t_norm * e.sqrt();
            if denom <= 1e-12 { 0.0 } else { (c / denom).clamp(-1.0, 1.0) }
        })
        .collect()
}

fn odd(m: usize) -> usize {
    m.max(1) | 1
}

fn ms_frames(ms: f64, rate: f64) -> usize {
    (ms / 1000.0 * rate).round() as usize
}

/// Centred moving average of odd length `m`, zero outside the slice.
fn moving_average<T>(x: &[T], m: usize) -> Vec<T>
where
    T: Copy + Default + Add<Output = T> + Sub<Output = T> + Mul<f64, Output = T>,
{
    let h = m / 2;
    let mut prefix = Vec::with_capacity(x.len() + 1);
    prefix.push(T::default());
    for &v in x {
        let last = *prefix.last().unwrap();
        prefix.push(last + v);
    }
    let k = 1.0 / m as f64;
    (0..x.len()).map(|i| (prefix[(i + h + 1).min(x.len())] - prefix[i.saturating_sub(h)]) * k).collect()
}

/// Largest value within `d` either side of each index.
fn sliding_max(x: &[f64], d: usize) -> Vec<f64> {
    let mut out = Vec::with_capacity(x.len());
    let mut q: VecDeque<usize> = VecDeque::new();
    let mut next = 0;
    for i in 0..x.len() {
        while next < x.len() && next <= i + d {
            while q.back().is_some_and(|&b| x[b] <= x[next]) {
                q.pop_back();
            }
            q.push_back(next);
            next += 1;
        }
        while q.front().is_some_and(|&f| f + d < i) {
            q.pop_front();
        }
        out.push(x[*q.front().expect("window holds index i")]);
    }
    out
}

fn median(x: &[f64]) -> f64 {
    let mut v = x.to_vec();
    let mid = v.len() / 2;
    v.select_nth_unstable_by(mid, f64::total_cmp);
    v[mid]
}

/// Level of the cue tone in `x`: demodulated at [`CUE_TONE_HZ`] and smoothed by a 1 ms
/// triangular low-pass (two centred 0.5 ms moving averages), which also rejects the
/// harmonics.
fn tone_envelope(x: &[f32], rate: f64) -> Vec<f64> {
    let m = odd(ms_frames(0.5, rate));
    let w = 2.0 * PI * CUE_TONE_HZ / rate;
    let z: Vec<Complex<f64>> = x.iter().enumerate().map(|(i, &v)| Complex::from_polar(f64::from(v), -w * i as f64)).collect();
    moving_average(&moving_average(&z, m), m).iter().map(|c| 2.0 * c.norm()).collect()
}

/// Fractional index of the cue's leading edge in `env`, searched from `anchor`, a lag in
/// the first cue's correlation lobe. `None` when there is no clear edge: the tone does
/// not stand out, or it was already sounding where the buffer begins, or it rises too
/// little over what came before (an echo of an earlier cue, say).
fn edge_time(env: &[f64], anchor: usize, n: usize, rate: f64) -> Option<f64> {
    let len = env.len();
    if anchor >= len {
        return None;
    }
    let nw = odd(ms_frames(NOISE_WINDOW_MS, rate));
    let avg = moving_average(env, nw);
    let skip = nw / 2;
    let inner = if len > 2 * skip { &avg[skip..len - skip] } else { &avg[..] };
    let noise = inner.iter().fold(f64::INFINITY, |m, &v| m.min(v)).max(1e-12);
    let (b0, b1) = (anchor + n / 4, (anchor + 3 * n / 4).min(len));
    if b0 >= b1 {
        return None;
    }
    let thr = (RUN_OVER_NOISE * noise).max(RUN_UNDER_BODY * median(&env[b0..b1]));
    let mut i = (anchor + ms_frames(ANCHOR_STEP_IN_MS, rate)).min(len - 1);
    if env[i] < thr {
        let lim = (anchor + n / 4).min(len - 1);
        while i < lim && env[i] < thr {
            i += 1;
        }
        if env[i] < thr {
            return None;
        }
    }
    while i > 0 && env[i - 1] >= thr {
        i -= 1;
    }
    let k = ms_frames(EDGE_BEFORE_MS, rate).max(1);
    if i < k {
        return None;
    }
    let before = env[i - k..i].iter().sum::<f64>() / k as f64;
    let end = (i + ms_frames(EDGE_PEAK_MS, rate).max(1)).min(len);
    let peak = env[i..end].iter().fold(0.0f64, |m, &v| m.max(v));
    if peak < EDGE_MIN_RISE * before {
        return None;
    }
    // Halfway up the rise, interpolated between samples.
    let c = before + 0.5 * (peak - before);
    let j = (i..end).find(|&j| env[j] >= c)?;
    let frac = if env[j] > env[j - 1] { ((c - env[j - 1]) / (env[j] - env[j - 1])).clamp(0.0, 1.0) } else { 1.0 };
    Some((j - 1) as f64 + frac)
}

/// Fractional sample index of the leading edge of the first cue in `samples` whose lag
/// lies in `lo..=hi`.
fn locate_edge(samples: &[f32], template: &[f32], lo: usize, hi: usize, rate: f64) -> Option<f64> {
    let env = tone_envelope(samples, rate);
    let ex = xcorr(&env, &tone_envelope(template, rate));
    let hi = hi.min(ex.len().checked_sub(1)?);
    if lo > hi {
        return None;
    }
    let w = &ex[lo..=hi];
    let top = ANCHOR_LOBE * w.iter().fold(0.0f64, |m, &v| m.max(v));
    if top <= 0.0 {
        return None;
    }
    let local = sliding_max(w, ms_frames(ANCHOR_NEIGHBOURHOOD_MS, rate));
    let first = (0..w.len()).find(|&l| w[l] >= top && w[l] >= local[l])?;
    edge_time(&env, lo + first, template.len(), rate)
}

/// Where [`edge_time`] puts the clean template's own edge, relative to its first sample.
fn template_edge_offset(template: &[f32], rate: f64) -> Option<f64> {
    let pad = ms_frames(20.0, rate);
    let mut x = vec![0.0f32; pad];
    x.extend_from_slice(template);
    x.resize(x.len() + pad, 0.0);
    edge_time(&tone_envelope(&x, rate), pad, template.len(), rate).map(|e| e - pad as f64)
}

/// Search `samples` (which begin at `samples_first_frame`) for a cue request.
pub fn match_cue(
    req: &CueRequest,
    template: &[f32],
    samples: &[f32],
    samples_first_frame: i64,
    incomplete: bool,
    sample_rate_hz: u32,
) -> CueMatch {
    let rate = f64::from(sample_rate_hz);
    let scores = ncc(samples, template);
    let n = template.len() as i64;
    let last = scores.len() as i64 - 1;
    let lo = (req.window_start_frame - samples_first_frame).max(0);
    let hi = (req.window_end_frame - samples_first_frame).min(last);
    let window_best =
        (lo <= hi).then(|| (lo..=hi).map(|l| (scores[l as usize], l)).fold((f64::MIN, lo), |b, s| if s.0 > b.0 { s } else { b }));
    let onset = window_best
        .and_then(|_| locate_edge(samples, template, lo as usize, hi as usize, rate))
        .zip(template_edge_offset(template, rate))
        .map(|(edge, off)| (edge - off).round() as i64)
        .filter(|l| (lo..=hi).contains(l));
    // Score at the onset: the best within one tone period of it, so the carrier phase the
    // speaker and microphone add does not matter.
    let period = (rate / CUE_TONE_HZ).ceil() as i64;
    let scored = onset.map(|l| {
        let s = ((l - period).max(0)..=(l + period).min(last)).map(|i| scores[i as usize]).fold(f64::MIN, f64::max);
        (l, s)
    });
    let accepted = scored.filter(|&(_, s)| s >= req.min_ncc);
    let (best_ncc, ref_lag) = match (accepted, window_best) {
        (Some((l, s)), _) => (s, Some(l)),
        (None, Some((s, l))) => (s, Some(l)),
        (None, None) => (0.0, None),
    };
    let second = ref_lag
        .map(|r| scores.iter().enumerate().filter(|(i, _)| (*i as i64 - r).abs() > n).map(|(_, &s)| s).fold(0.0f64, f64::max))
        .unwrap_or(0.0);
    let onset_frame = accepted.map(|(l, _)| samples_first_frame + l);
    CueMatch {
        cue_id: req.cue_id,
        kind: req.kind,
        template_version: req.kind.template_version().to_string(),
        onset_frame,
        end_frame: onset_frame.map(|o| o + n),
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

    const RATE: u32 = 48_000;

    struct Rng(u32);
    impl Rng {
        fn next(&mut self) -> f32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 17;
            self.0 ^= self.0 << 5;
            self.0 as f32 / u32::MAX as f32 - 0.5
        }
    }

    fn noise(len: usize, level: f32, seed: u32) -> Vec<f32> {
        let mut r = Rng(seed);
        (0..len).map(|_| r.next() * level).collect()
    }

    fn add(sig: &mut [f32], at: usize, x: &[f32], gain: f32) {
        for (i, v) in x.iter().enumerate() {
            if let Some(s) = sig.get_mut(at + i) {
                *s += gain * v;
            }
        }
    }

    /// Linear convolution through the FFT.
    fn convolve(x: &[f32], h: &[f32]) -> Vec<f32> {
        let size = (x.len() + h.len()).next_power_of_two();
        let mut planner = FftPlanner::<f64>::new();
        let fwd = planner.plan_fft_forward(size);
        let inv = planner.plan_fft_inverse(size);
        let mut a: Vec<Complex<f64>> = x.iter().map(|&v| Complex::new(f64::from(v), 0.0)).collect();
        a.resize(size, Complex::new(0.0, 0.0));
        let mut b: Vec<Complex<f64>> = h.iter().map(|&v| Complex::new(f64::from(v), 0.0)).collect();
        b.resize(size, Complex::new(0.0, 0.0));
        fwd.process(&mut a);
        fwd.process(&mut b);
        for (p, q) in a.iter_mut().zip(b.iter()) {
            *p *= q;
        }
        inv.process(&mut a);
        a[..x.len() + h.len() - 1].iter().map(|c| (c.re / size as f64) as f32).collect()
    }

    /// A reverberant room as the phone hears itself: the direct path, discrete early
    /// reflections, and an exponentially decaying diffuse tail `drr_db` below the direct
    /// sound. Returns the impulse response; its direct path is at index 0.
    fn room(reflections: &[(f64, f32)], rt60_s: f64, drr_db: f32, seed: u32) -> Vec<f32> {
        let len = (rt60_s * f64::from(RATE)) as usize;
        let mut h = vec![0.0f32; len.max(1)];
        h[0] = 1.0;
        for &(ms, db) in reflections {
            h[ms_frames(ms, f64::from(RATE))] += 10f32.powf(db / 20.0) * if (ms as u32).is_multiple_of(2) { 1.0 } else { -1.0 };
        }
        let mut r = Rng(seed);
        let start = ms_frames(3.0, f64::from(RATE));
        let tail: Vec<f32> = (start..len).map(|k| r.next() * (-6.9 * (k as f64) / len as f64).exp() as f32).collect();
        let e = tail.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-9);
        let g = 10f32.powf(-drr_db / 20.0) / e;
        for (k, v) in tail.iter().enumerate() {
            h[start + k] += g * v;
        }
        h
    }

    fn request(kind: CueKind, start: i64, end: i64) -> CueRequest {
        CueRequest { cue_id: 0, kind, window_start_frame: start, window_end_frame: end, min_ncc: DEFAULT_MIN_NCC }
    }

    /// Magnitude of `x` at `f` (single-bin DFT).
    fn level(x: &[f32], f: f64) -> f64 {
        let w = 2.0 * PI * f / f64::from(RATE);
        let (re, im) = x.iter().enumerate().fold((0.0, 0.0), |(re, im), (i, &v)| {
            (re + f64::from(v) * (w * i as f64).cos(), im - f64::from(v) * (w * i as f64).sin())
        });
        (re * re + im * im).sqrt()
    }

    #[test]
    fn templates_are_deterministic_and_rate_scaled() {
        let a = cue_template(CueKind::Start, RATE);
        assert_eq!(a.len(), 14_400);
        assert_eq!(a, cue_template(CueKind::Start, RATE));
        assert_eq!(cue_template(CueKind::Start, 44_100).len(), 13_230);
        assert_eq!(cue_template(CueKind::Par, RATE).len(), 24_000);
        assert_eq!(a[0], 0.0, "ramp starts at silence");
        assert_eq!(*a.last().unwrap(), 0.0, "and ends at silence");
        let peak = a.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((0.99..=1.0).contains(&peak), "unit peak, {peak}");
    }

    #[test]
    fn the_tone_has_the_measured_shot_timer_spectrum() {
        // One second of whole cycles is not available in 300 ms; 300 ms holds 698.1 of
        // them, so measure on a whole number of fundamental periods from the middle.
        let t = cue_template(CueKind::Start, RATE);
        let periods = 600.0;
        let len = (periods * f64::from(RATE) / CUE_TONE_HZ).round() as usize;
        let mid = &t[(t.len() - len) / 2..(t.len() + len) / 2];
        let f1 = level(mid, CUE_TONE_HZ);
        let db = |f: f64| 20.0 * (level(mid, f) / f1).log10();
        assert!((db(3.0 * CUE_TONE_HZ) + 12.0).abs() < 0.3, "H3 {:.2} dB", db(3.0 * CUE_TONE_HZ));
        assert!((db(5.0 * CUE_TONE_HZ) + 24.0).abs() < 0.3, "H5 {:.2} dB", db(5.0 * CUE_TONE_HZ));
        for h in [2.0, 4.0, 7.0] {
            assert!(db(h * CUE_TONE_HZ) < -45.0, "harmonic {h}: {:.1} dB", db(h * CUE_TONE_HZ));
        }
        // Off-pitch energy is small: nothing sweeps.
        assert!(db(2000.0) < -30.0 && db(2700.0) < -30.0);
        // At an 8 kHz output rate the harmonics above 3.6 kHz are left out, not aliased.
        let low = cue_template(CueKind::Start, 8_000);
        assert_eq!(low.len(), 2_400);
        assert!(low.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn exact_onset_found_in_noise_at_both_rates() {
        for rate in [48_000u32, 44_100] {
            let t = cue_template(CueKind::Start, rate);
            let mut sig = noise(60_000, 0.02, 12_345);
            let onset = 12_345usize;
            add(&mut sig, onset, &t, 0.2);
            let req = request(CueKind::Start, 1000, 30_000);
            let m = match_cue(&req, &t, &sig, 1000, false, rate);
            assert_eq!(m.onset_frame, Some(1000 + onset as i64), "{rate} Hz");
            assert_eq!(m.end_frame, Some(1000 + (onset + t.len()) as i64));
            assert!(m.best_ncc > 0.9, "{}", m.best_ncc);
        }
    }

    #[test]
    fn loud_impulse_is_not_a_cue() {
        let t = cue_template(CueKind::Start, RATE);
        let mut sig = vec![0.0f32; 40_000];
        for i in 0..2000 {
            sig[10_000 + i] = 0.9 * (-(i as f32) / 300.0).exp() * if i % 2 == 0 { 1.0 } else { -1.0 };
        }
        let req = request(CueKind::Start, 0, 22_000);
        let m = match_cue(&req, &t, &sig, 0, false, RATE);
        assert!(!m.accepted(), "impulse ncc {}", m.best_ncc);
        // Nor a gunshot-like burst of noise.
        let mut sig = noise(40_000, 0.0005, 3);
        let burst: Vec<f32> = noise(6000, 2.0, 9).iter().enumerate().map(|(i, v)| v * (-(i as f32) / 400.0).exp()).collect();
        add(&mut sig, 10_000, &burst, 1.0);
        let m = match_cue(&req, &t, &sig, 0, false, RATE);
        assert!(!m.accepted(), "burst ncc {}", m.best_ncc);
    }

    #[test]
    fn a_reverberant_room_does_not_move_the_onset() {
        // Plain correlation picks a lag well inside the tone here; the edge does not move.
        let rooms = [
            room(&[(0.8, -4.0)], 0.05, 40.0, 1),
            room(&[(0.8, -4.0), (6.0, -16.0), (11.0, -18.0)], 0.6, 15.0, 2),
            room(&[(1.2, -3.0), (3.0, -6.0), (7.0, -9.0)], 0.8, 3.0, 3),
            room(&[(0.8, -4.0), (4.0, -10.0), (9.0, -12.0)], 1.2, 6.0, 4),
        ];
        let t = cue_template(CueKind::Start, RATE);
        let tol = ms_frames(1.0, f64::from(RATE)) as i64;
        for (k, h) in rooms.iter().enumerate() {
            let onset = 4_800usize;
            let mut dry = vec![0.0f32; 60_000];
            add(&mut dry, onset, &t, 0.3);
            let mut sig = convolve(&dry, h);
            sig.truncate(60_000);
            for (s, n) in sig.iter_mut().zip(noise(60_000, 0.002, 77 + k as u32)) {
                *s += n;
            }
            let req = request(CueKind::Start, 2_400, 4_800 + 14_400);
            let m = match_cue(&req, &t, &sig, 0, false, RATE);
            let err = m.onset_frame.map(|o| o - onset as i64);
            assert!(err.is_some_and(|e| e.abs() <= tol), "room {k}: onset error {err:?} frames, ncc {}", m.best_ncc);
        }
    }

    #[test]
    fn a_shot_just_before_or_during_the_cue_does_not_move_the_onset() {
        let t = cue_template(CueKind::Start, RATE);
        let h = room(&[(0.8, -4.0), (6.0, -16.0)], 0.6, 15.0, 5);
        for shot_ms in [-40.0f64, 120.0] {
            let onset = 9_600usize;
            let mut dry = vec![0.0f32; 60_000];
            add(&mut dry, onset, &t, 0.3);
            let at = (onset as f64 + shot_ms / 1000.0 * f64::from(RATE)) as usize;
            let burst: Vec<f32> = noise(6000, 4.0, 21).iter().enumerate().map(|(i, v)| v * (-(i as f32) / 384.0).exp()).collect();
            add(&mut dry, at, &burst, 1.0);
            let mut sig = convolve(&dry, &h);
            sig.truncate(60_000);
            let req = request(CueKind::Start, onset as i64 - 2_400, onset as i64 + 14_400);
            let m = match_cue(&req, &t, &sig, 0, false, RATE);
            let err = m.onset_frame.map(|o| o - onset as i64);
            assert!(err.is_some_and(|e| e.abs() <= 48), "shot at {shot_ms} ms: error {err:?}, ncc {}", m.best_ncc);
        }
    }

    #[test]
    fn the_start_search_finds_the_start_not_a_later_par_of_the_same_pitch() {
        // Render time unknown: the window reaches 800 ms, past the first par at 400 ms.
        let start = cue_template(CueKind::Start, RATE);
        let par = cue_template(CueKind::Par, RATE);
        let onset = 4_800usize;
        let mut sig = noise(120_000, 0.002, 31);
        add(&mut sig, onset, &start, 0.3);
        add(&mut sig, onset + 19_200, &par, 0.3);
        let req = CueRequest::around(0, CueKind::Start, onset as i64, false, RATE);
        let m = match_cue(&req, &start, &sig, 0, false, RATE);
        assert_eq!(m.onset_frame, Some(onset as i64));
        // And a par search finds its own par, not the start before it or a par after it.
        let next = onset + 19_200 + 28_800;
        add(&mut sig, next, &par, 0.3);
        let req = CueRequest::around(1, CueKind::Par, (onset + 19_200) as i64, false, RATE);
        let m = match_cue(&req, &par, &sig, 0, false, RATE);
        assert_eq!(m.onset_frame, Some((onset + 19_200) as i64));
    }

    #[test]
    fn an_edge_lost_in_a_previous_cues_echo_is_unresolved_not_guessed() {
        // A par 100 ms after the previous par ends, in a very reverberant room: the
        // earlier tone's echo is still loud when the next one starts.
        let par = cue_template(CueKind::Par, RATE);
        let h = room(&[(1.2, -3.0), (3.0, -6.0)], 1.5, 0.0, 6);
        let first = 4_800usize;
        let second = first + 28_800;
        let mut dry = vec![0.0f32; 100_000];
        add(&mut dry, first, &par, 0.3);
        add(&mut dry, second, &par, 0.3);
        let mut sig = convolve(&dry, &h);
        sig.truncate(100_000);
        for (s, n) in sig.iter_mut().zip(noise(100_000, 0.0005, 41)) {
            *s += n;
        }
        let req = CueRequest::around(2, CueKind::Par, second as i64, true, RATE);
        let m = match_cue(&req, &par, &sig, 0, false, RATE);
        if let Some(o) = m.onset_frame {
            assert!((o - second as i64).abs() <= 48, "a resolved onset must be right: error {}", o - second as i64);
        }
    }

    #[test]
    fn the_template_edge_offset_is_where_the_rise_is_half_done() {
        let t = cue_template(CueKind::Start, RATE);
        let off = template_edge_offset(&t, f64::from(RATE)).unwrap();
        // The 1 ms ramp is half way at 0.5 ms; the symmetric low-pass keeps it there.
        assert!((off - 24.0).abs() < 2.0, "offset {off} frames");
        let par = template_edge_offset(&cue_template(CueKind::Par, RATE), f64::from(RATE)).unwrap();
        assert!((par - off).abs() < 1e-6, "start {off}, par {par}");
    }
}
