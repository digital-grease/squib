//! Transparent streaming impulse-onset detector (`squib-onset-v2`; `squib-onset-v1`
//! for configurations that name it).
//!
//! Per sample:
//! 1. One-pole high-pass (`highpass_hz`) removes rumble/handling.
//! 2. Fast mean-square envelope (τ 0.25 ms), medium envelope (τ 15 ms), and a gated
//!    asymmetric noise floor (rise τ 1 s, fall τ 50 ms) that freezes during events.
//!    In v2 it also holds while the shot-timer tone ([`CUE_TONE_HZ`]) carries at least
//!    half the medium energy (ADR-067): a 300 ms start or 500 ms par beep, or its echo,
//!    is the app's own sound, not background, and learning it would cost the next shot
//!    its margin. Broadband sound (shots, echoes, wind, chatter) never meets that test,
//!    so everything else is as in v1.
//! 3. Trigger when fast energy exceeds the floor by `threshold_db` **and** exceeds the
//!    medium envelope from 2 ms earlier by `attack_min_db`. The attack criterion
//!    rejects slow onsets such as the ramped app cue sustain and steady noise.
//! 4. After a 2 ms peak window, the onset is the first sample in
//!    [trigger − 3 ms, peak] whose high-passed magnitude reaches
//!    max(15% of peak, 6σ of the floor).
//!    Floor ratio and attack features are the maxima over the peak window.
//! 5. Features are measured over a fixed 35 ms window after onset and the candidate is
//!    emitted at onset + 35 ms (fixed latency).
//!
//! Because all state is carried per sample, block boundaries cannot change any onset:
//! the chunk-invariance gate (≤ 1 sample) holds exactly. Classification relative to
//! cues/start happens later (`squib_domain::classify`); acceptance is a review action.
//! Duplicate protection is a short `min_gap_ms` after a peak, not an echo blackout:
//! quieter impulses shortly after a louder one are kept as `PossibleEcho` uncertain.

use squib_domain::{
    Candidate, CandidateFeatures, CandidateReason, DETECTOR_ALGORITHM_VERSION_V1, DetectorConfig, DetectorSuggestion,
};

use crate::cue::CUE_TONE_HZ;

const FAST_TAU_S: f64 = 0.000_25;
/// Smoothing of the demodulated cue tone (v2 floor hold): twice the medium envelope's,
/// because it smooths amplitude, not energy, so a dying tone keeps its share of the
/// medium energy and the hold lasts through the beep's echo.
const TONE_TAU_S: f64 = 2.0 * MED_TAU_S;
/// The floor holds while the cue tone carries at least this share of the medium energy.
const TONE_HOLD_SHARE: f64 = 0.5;
const MED_TAU_S: f64 = 0.015;
const FLOOR_UP_TAU_S: f64 = 1.0;
const FLOOR_DOWN_TAU_S: f64 = 0.05;
const ATTACK_DELAY_S: f64 = 0.002;
const PEAK_WINDOW_S: f64 = 0.002;
const LOOKBACK_S: f64 = 0.003;
const FEATURE_WINDOW_S: f64 = 0.035;
const FLOOR_FREEZE_S: f64 = 0.1;
const ONSET_FRACTION: f32 = 0.15;
const ONSET_SIGMAS: f64 = 6.0;
const CLIP_LEVEL: f32 = 0.999;
const MIN_FLOOR_MS: f64 = 1e-12;
const MAX_PENDING: usize = 8;
const SCORE_SPAN_DB: f32 = 24.0;

fn alpha(tau_s: f64, rate: f64) -> f64 {
    1.0 - (-1.0 / (tau_s * rate)).exp()
}

fn db(ratio: f64) -> f32 {
    (10.0 * ratio.max(1e-30).log10()) as f32
}

#[derive(Debug, Clone, Copy)]
struct Collecting {
    trigger: i64,
    peak_frame: i64,
    peak_hp: f32,
    peak_raw: f32,
    floor: f64,
    ratio_db: f32,
    attack_db: f32,
}

#[derive(Debug, Clone, Copy)]
struct Pending {
    onset: i64,
    emit_at: i64,
    c: Collecting,
}

/// Fixed-size ring of recent samples indexed by absolute frame.
#[derive(Debug, Clone)]
struct Ring {
    hp: Vec<f32>,
    raw: Vec<f32>,
    mask: usize,
}

impl Ring {
    fn new(min_len: usize) -> Self {
        let n = min_len.next_power_of_two();
        Self { hp: vec![0.0; n], raw: vec![0.0; n], mask: n - 1 }
    }
    fn put(&mut self, frame: i64, hp: f32, raw: f32) {
        let i = (frame as usize) & self.mask;
        self.hp[i] = hp;
        self.raw[i] = raw;
    }
    fn hp(&self, frame: i64) -> f32 {
        self.hp[(frame as usize) & self.mask]
    }
    fn raw(&self, frame: i64) -> f32 {
        self.raw[(frame as usize) & self.mask]
    }
}

#[derive(Debug, Clone)]
pub struct OnsetDetector {
    cfg: DetectorConfig,
    config_hash: String,
    rate: u32,
    hp_a: f32,
    hp_x1: f32,
    hp_y1: f32,
    a_fast: f64,
    a_med: f64,
    a_up: f64,
    a_down: f64,
    fast: f64,
    med: f64,
    floor: f64,
    med_hist: Vec<f64>,
    ring: Ring,
    attack_delay: i64,
    peak_window: i64,
    lookback: i64,
    feature_window: i64,
    min_gap: i64,
    echo_window: i64,
    floor_freeze: i64,
    frame: i64,
    first_frame: Option<i64>,
    collecting: Option<Collecting>,
    pending: Vec<Pending>,
    rearm_after: i64,
    freeze_until: i64,
    recent: Vec<(i64, f32)>,
    next_seq: u64,
    /// v2: hold the floor while the cue tone dominates.
    tone_hold: bool,
    tone_w: f64,
    a_tone: f64,
    tone: (f64, f64),
}

impl OnsetDetector {
    pub fn new(cfg: DetectorConfig, sample_rate_hz: u32) -> Self {
        assert!(sample_rate_hz >= 8000, "unsupported sample rate");
        let r = f64::from(sample_rate_hz);
        let frames = |s: f64| (s * r).round() as i64;
        let rc = 1.0 / (2.0 * std::f64::consts::PI * f64::from(cfg.highpass_hz));
        let dt = 1.0 / r;
        let hp_a = (rc / (rc + dt)) as f32;
        let floor = 10f64.powf(f64::from(cfg.initial_floor_dbfs) / 10.0).max(MIN_FLOOR_MS);
        let attack_delay = frames(ATTACK_DELAY_S).max(1);
        let ring_len = (frames(FEATURE_WINDOW_S + LOOKBACK_S + PEAK_WINDOW_S) + 64) as usize * 2;
        Self {
            config_hash: cfg.config_hash(),
            rate: sample_rate_hz,
            hp_a,
            hp_x1: 0.0,
            hp_y1: 0.0,
            a_fast: alpha(FAST_TAU_S, r),
            a_med: alpha(MED_TAU_S, r),
            a_up: alpha(FLOOR_UP_TAU_S, r),
            a_down: alpha(FLOOR_DOWN_TAU_S, r),
            fast: floor,
            med: floor,
            floor,
            med_hist: vec![floor; attack_delay as usize],
            ring: Ring::new(ring_len),
            attack_delay,
            peak_window: frames(PEAK_WINDOW_S),
            lookback: frames(LOOKBACK_S),
            feature_window: frames(FEATURE_WINDOW_S),
            min_gap: frames(f64::from(cfg.min_gap_ms) / 1000.0),
            echo_window: frames(f64::from(cfg.echo_window_ms) / 1000.0),
            floor_freeze: frames(FLOOR_FREEZE_S),
            frame: 0,
            first_frame: None,
            collecting: None,
            pending: Vec::with_capacity(MAX_PENDING),
            rearm_after: 0,
            freeze_until: 0,
            recent: Vec::with_capacity(8),
            next_seq: 0,
            tone_hold: cfg.algorithm_version != DETECTOR_ALGORITHM_VERSION_V1,
            tone_w: 2.0 * std::f64::consts::PI * CUE_TONE_HZ / r,
            a_tone: alpha(TONE_TAU_S, r),
            tone: (0.0, 0.0),
            cfg,
        }
    }

    pub fn config(&self) -> &DetectorConfig {
        &self.cfg
    }

    /// Current noise floor estimate (dBFS mean-square of the high-passed signal).
    pub fn floor_dbfs(&self) -> f32 {
        db(self.floor)
    }

    /// Seed the noise floor from an ambient baseline measurement.
    pub fn set_floor_dbfs(&mut self, dbfs: f32) {
        if dbfs.is_finite() {
            self.floor = 10f64.powf(f64::from(dbfs) / 10.0).max(MIN_FLOOR_MS);
        }
    }

    /// Next frame index the detector expects.
    pub fn next_frame(&self) -> i64 {
        self.frame
    }

    /// Process contiguous samples starting at `first_frame`. Emits completed candidates.
    pub fn process(&mut self, first_frame: i64, samples: &[f32], out: &mut impl FnMut(Candidate)) {
        if self.first_frame.is_none() {
            self.first_frame = Some(first_frame);
            self.frame = first_frame;
            self.rearm_after = first_frame + self.attack_delay;
        }
        debug_assert_eq!(first_frame, self.frame, "detector requires contiguous input");
        for &x in samples {
            self.step(x, out);
        }
    }

    fn step(&mut self, x: f32, out: &mut impl FnMut(Candidate)) {
        let f = self.frame;
        let y = self.hp_a * (self.hp_y1 + x - self.hp_x1);
        self.hp_x1 = x;
        self.hp_y1 = y;
        let e = f64::from(y) * f64::from(y);
        self.fast += self.a_fast * (e - self.fast);
        self.med += self.a_med * (e - self.med);
        let mh = (f as usize) % self.med_hist.len();
        let med_delayed = self.med_hist[mh];
        self.med_hist[mh] = self.med;
        self.ring.put(f, y, x);

        let tone_dominant = self.tone_hold && {
            // Demodulate at the cue tone; a sinusoid of mean square P gives |z|^2 = P/2.
            let (s, c) = (self.tone_w * (f - self.first_frame.unwrap_or(f)) as f64).sin_cos();
            let yd = f64::from(y);
            self.tone.0 += self.a_tone * (yd * c - self.tone.0);
            self.tone.1 += self.a_tone * (-yd * s - self.tone.1);
            2.0 * (self.tone.0 * self.tone.0 + self.tone.1 * self.tone.1) >= TONE_HOLD_SHARE * self.med
        };
        let idle = self.collecting.is_none();
        if idle && f >= self.freeze_until && !tone_dominant {
            let a = if self.med > self.floor { self.a_up } else { self.a_down };
            self.floor = (self.floor + a * (self.med - self.floor)).max(MIN_FLOOR_MS);
        }

        match self.collecting.as_mut() {
            None => {
                if f >= self.rearm_after {
                    let ratio_db = db(self.fast / self.floor);
                    let attack_db = db(self.fast / med_delayed.max(self.floor));
                    if ratio_db >= self.cfg.threshold_db && attack_db >= self.cfg.attack_min_db {
                        self.collecting = Some(Collecting {
                            trigger: f,
                            peak_frame: f,
                            peak_hp: y.abs(),
                            peak_raw: x.abs(),
                            floor: self.floor,
                            ratio_db,
                            attack_db,
                        });
                    }
                }
            }
            Some(c) => {
                if y.abs() > c.peak_hp {
                    c.peak_hp = y.abs();
                    c.peak_frame = f;
                }
                c.peak_raw = c.peak_raw.max(x.abs());
                let ratio_db = db(self.fast / c.floor);
                c.ratio_db = c.ratio_db.max(ratio_db);
                // med_delayed lags 2 ms, so within the 2 ms peak window it still
                // describes pre-onset energy.
                let attack_db = db(self.fast / med_delayed.max(c.floor));
                c.attack_db = c.attack_db.max(attack_db);
                if f >= c.trigger + self.peak_window {
                    let c = *c;
                    self.collecting = None;
                    self.finish_trigger(c);
                }
            }
        }

        while self.pending.first().is_some_and(|p| f >= p.emit_at) {
            let p = self.pending.remove(0);
            let cand = self.build(p);
            out(cand);
        }
        self.frame += 1;
    }

    fn finish_trigger(&mut self, c: Collecting) {
        let sigma = c.floor.sqrt();
        let level = (ONSET_FRACTION * c.peak_hp).max((ONSET_SIGMAS * sigma) as f32);
        let earliest = (c.trigger - self.lookback).max(self.first_frame.unwrap_or(0));
        let mut onset = c.peak_frame;
        let mut i = earliest;
        while i <= c.peak_frame {
            if self.ring.hp(i).abs() >= level {
                onset = i;
                break;
            }
            i += 1;
        }
        self.rearm_after = c.peak_frame + self.min_gap;
        self.freeze_until = onset + self.floor_freeze;
        if self.pending.len() < MAX_PENDING {
            self.pending.push(Pending { onset, emit_at: onset + self.feature_window, c });
        }
    }

    fn build(&mut self, p: Pending) -> Candidate {
        let ms = |v: f64| (v * f64::from(self.rate) / 1000.0).round() as i64;
        let energy = |a: i64, b: i64| -> f64 {
            let mut s = 0.0f64;
            let mut i = a;
            while i < b {
                let v = f64::from(self.ring.hp(i));
                s += v * v;
                i += 1;
            }
            s / (b - a).max(1) as f64
        };
        let early = energy(p.onset, p.onset + ms(5.0));
        let late = energy(p.onset + ms(15.0), p.onset + ms(35.0));
        let mut clipped = 0u32;
        let mut i = p.onset;
        while i < p.onset + self.feature_window {
            if self.ring.raw(i).abs() >= CLIP_LEVEL {
                clipped += 1;
            }
            i += 1;
        }
        let peak_dbfs = (20.0 * f64::from(p.c.peak_raw.max(1e-9)).log10()) as f32;
        let features = CandidateFeatures {
            peak_dbfs,
            floor_dbfs: db(p.c.floor),
            floor_ratio_db: p.c.ratio_db,
            attack_db: p.c.attack_db,
            rise_frames: u32::try_from(p.c.peak_frame - p.onset).unwrap_or(0),
            decay_db: db(early / late.max(1e-30)),
            clipped_samples: clipped,
        };

        let mut reasons = Vec::new();
        let echo = self
            .recent
            .iter()
            .any(|&(pf, pdb)| p.c.peak_frame - pf <= self.echo_window && peak_dbfs <= pdb - self.cfg.echo_drop_db);
        if echo {
            reasons.push(CandidateReason::PossibleEcho);
        }
        let strong =
            p.c.ratio_db >= self.cfg.threshold_db + self.cfg.accept_margin_db && p.c.attack_db >= self.cfg.attack_accept_db;
        if !strong {
            reasons.push(CandidateReason::LowMargin);
        }
        if clipped > 0 {
            reasons.push(CandidateReason::Clipped);
        }
        let suggestion = if strong && !echo { DetectorSuggestion::SuggestedAccepted } else { DetectorSuggestion::Uncertain };
        let clamp01 = |v: f32| v.clamp(0.0, 1.0);
        let score = 0.5 * clamp01((p.c.ratio_db - self.cfg.threshold_db) / SCORE_SPAN_DB)
            + 0.5 * clamp01((p.c.attack_db - self.cfg.attack_min_db) / SCORE_SPAN_DB);

        self.recent.retain(|&(pf, _)| p.c.peak_frame - pf <= self.echo_window);
        if self.recent.len() == self.recent.capacity() {
            self.recent.remove(0);
        }
        self.recent.push((p.c.peak_frame, peak_dbfs));

        let seq = self.next_seq;
        self.next_seq += 1;
        Candidate {
            sequence: seq,
            onset_frame: p.onset,
            peak_frame: p.c.peak_frame,
            features,
            suggestion,
            reasons,
            detector_score: score,
            algorithm_version: self.cfg.algorithm_version.clone(),
            config_hash: self.config_hash.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cue::cue_template;
    use squib_domain::{CueKind, DETECTOR_ALGORITHM_VERSION};

    const RATE: u32 = 48_000;

    fn noise(len: usize, rms: f32, seed: u32) -> Vec<f32> {
        let mut s = seed;
        (0..len)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                (s as f32 / u32::MAX as f32 - 0.5) * rms * 3.46
            })
            .collect()
    }

    fn detector(version: &str) -> OnsetDetector {
        OnsetDetector::new(DetectorConfig { algorithm_version: version.into(), ..DetectorConfig::default() }, RATE)
    }

    /// Run `x` through the detector and return its candidates and the floor at each of
    /// `probes` (frame indices).
    fn run(d: &mut OnsetDetector, x: &[f32], probes: &[usize]) -> (Vec<Candidate>, Vec<f32>) {
        let mut out = Vec::new();
        let mut floors = Vec::new();
        let mut at = 0;
        for &p in probes {
            d.process(at as i64, &x[at..p], &mut |c| out.push(c));
            floors.push(d.floor_dbfs());
            at = p;
        }
        d.process(at as i64, &x[at..], &mut |c| out.push(c));
        (out, floors)
    }

    #[test]
    fn v2_does_not_learn_the_app_beep_as_background_and_v1_did() {
        // Quiet range, then a 500 ms par beep as loud as the phone hears its own speaker.
        let par = cue_template(CueKind::Par, RATE);
        let mut x = noise(96_000, 0.0015, 7);
        let onset = 24_000;
        for (i, v) in par.iter().enumerate() {
            x[onset + i] += 0.1 * v;
        }
        let end = onset + par.len();
        let probes = [onset - 1, end + 960];
        let (_, v1) = run(&mut detector(DETECTOR_ALGORITHM_VERSION_V1), &x, &probes);
        let (_, v2) = run(&mut detector(DETECTOR_ALGORITHM_VERSION), &x, &probes);
        // 20 ms after the beep ends: v1's floor has climbed most of the way to the beep,
        // v2's is where it was before the beep.
        assert!(v1[1] - v1[0] > 15.0, "v1 floor {:.1} -> {:.1} dBFS", v1[0], v1[1]);
        assert!((v2[1] - v2[0]).abs() < 1.0, "v2 floor {:.1} -> {:.1} dBFS", v2[0], v2[1]);
    }

    #[test]
    fn v2_is_v1_for_everything_but_the_cue_tone() {
        // Broadband sound: noise that steps up 20 dB (the floor must still learn it),
        // impulses before and after the step, and an off-pitch tone.
        let mut x = noise(144_000, 0.001, 11);
        for (i, v) in noise(72_000, 0.01, 12).iter().enumerate() {
            x[72_000 + i] += v;
        }
        for (k, at) in [20_000usize, 50_000, 100_000, 130_000].iter().enumerate() {
            for i in 0..1200 {
                x[at + i] += 0.8 * (-(i as f32) / 200.0).exp() * if (i + k) % 2 == 0 { 1.0 } else { -1.0 };
            }
        }
        for i in 0..14_400 {
            x[30_000 + i] += 0.05 * (2.0 * std::f32::consts::PI * 1500.0 * i as f32 / RATE as f32).sin();
        }
        let probes = [71_999, 100_000, 143_000];
        let (c1, f1) = run(&mut detector(DETECTOR_ALGORITHM_VERSION_V1), &x, &probes);
        let (c2, f2) = run(&mut detector(DETECTOR_ALGORITHM_VERSION), &x, &probes);
        assert_eq!(f1, f2, "floors");
        assert!(f2[2] - f2[0] > 15.0, "the floor still follows a real change in background");
        let strip = |c: &[Candidate]| {
            c.iter()
                .map(|c| (c.onset_frame, c.peak_frame, c.features.clone(), c.suggestion, c.reasons.clone(), c.detector_score))
                .collect::<Vec<_>>()
        };
        assert_eq!(strip(&c1), strip(&c2));
        for at in [20_000i64, 50_000, 100_000, 130_000] {
            assert!(c2.iter().any(|c| (c.onset_frame - at).abs() <= 2), "impulse at {at}: {:?}", strip(&c2));
        }
    }
}
