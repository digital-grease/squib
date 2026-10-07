//! Guided sensitivity calibration: ambient statistics plus optional test impulses.
//!
//! Output is a suggested `threshold_db` with the evidence used. Two impulses are not an
//! accuracy validation; the profile records counts and statistics so later evidence
//! can supersede it. Labels such as "Indoor / stand behind shooter" are context only.

use serde::{Deserialize, Serialize};

pub const CALIBRATION_ALGORITHM_VERSION: &str = "ambient-impulse-v1";
pub const THRESHOLD_MIN_DB: f32 = 12.0;
pub const THRESHOLD_MAX_DB: f32 = 42.0;
/// Ambient-only suggestion keeps this margin above observed ambient fluctuation.
const AMBIENT_MARGIN_DB: f32 = 12.0;
/// Suggested threshold stays this far below the quietest test impulse.
const IMPULSE_MARGIN_DB: f32 = 6.0;

/// Accumulates 10 ms high-passed mean-square windows during ambient capture.
#[derive(Debug, Clone)]
pub struct AmbientAnalyzer {
    hop: usize,
    hp_a: f32,
    x1: f32,
    y1: f32,
    acc: f64,
    n: usize,
    windows_db: Vec<f32>,
    total: u64,
    clipped: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AmbientStats {
    pub windows: u32,
    pub median_dbfs: f32,
    pub p95_dbfs: f32,
    pub p99_dbfs: f32,
    pub max_dbfs: f32,
    pub clipped_fraction: f64,
}

impl AmbientAnalyzer {
    pub fn new(sample_rate_hz: u32, highpass_hz: f32) -> Self {
        let rc = 1.0 / (2.0 * std::f64::consts::PI * f64::from(highpass_hz));
        let dt = 1.0 / f64::from(sample_rate_hz);
        Self {
            hop: (sample_rate_hz / 100) as usize,
            hp_a: (rc / (rc + dt)) as f32,
            x1: 0.0,
            y1: 0.0,
            acc: 0.0,
            n: 0,
            windows_db: Vec::with_capacity(1024),
            total: 0,
            clipped: 0,
        }
    }

    pub fn push(&mut self, samples: &[f32]) {
        for &x in samples {
            let y = self.hp_a * (self.y1 + x - self.x1);
            self.x1 = x;
            self.y1 = y;
            self.acc += f64::from(y) * f64::from(y);
            self.n += 1;
            self.total += 1;
            if x.abs() >= 0.999 {
                self.clipped += 1;
            }
            if self.n == self.hop {
                let ms = self.acc / self.hop as f64;
                self.windows_db.push((10.0 * ms.max(1e-30).log10()) as f32);
                self.acc = 0.0;
                self.n = 0;
            }
        }
    }

    pub fn stats(&self) -> Option<AmbientStats> {
        if self.windows_db.len() < 10 {
            return None;
        }
        let mut v = self.windows_db.clone();
        v.sort_by(|a, b| a.total_cmp(b));
        let q = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
        Some(AmbientStats {
            windows: u32::try_from(v.len()).unwrap_or(u32::MAX),
            median_dbfs: q(0.5),
            p95_dbfs: q(0.95),
            p99_dbfs: q(0.99),
            max_dbfs: *v.last().unwrap(),
            clipped_fraction: self.clipped as f64 / self.total.max(1) as f64,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalibrationVerdict {
    /// Suggested threshold separates ambient from the test impulses.
    Separated,
    /// Ambient-only suggestion; no test impulses were detected.
    AmbientOnly,
    /// Test impulses too close to ambient fluctuation; setup may be unsuitable.
    PoorSeparation,
    /// Ambient input clipped; level information is lost.
    AmbientClipped,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalibrationSuggestion {
    pub algorithm_version: String,
    pub threshold_db: f32,
    pub verdict: CalibrationVerdict,
    /// Floor-relative ratios of detected test impulses (dB).
    pub impulse_ratios_db: Vec<f32>,
    pub ambient: AmbientStats,
}

/// Suggest a detector threshold from ambient statistics and test-impulse ratios.
pub fn suggest_threshold(ambient: &AmbientStats, impulse_ratios_db: &[f32]) -> CalibrationSuggestion {
    let spread = (ambient.p99_dbfs - ambient.median_dbfs).max(0.0);
    let ambient_min = (spread + AMBIENT_MARGIN_DB).clamp(THRESHOLD_MIN_DB, THRESHOLD_MAX_DB);
    let default = squib_domain::DetectorConfig::default().threshold_db;
    let (threshold, verdict) = if ambient.clipped_fraction > 0.0001 {
        (default.max(ambient_min), CalibrationVerdict::AmbientClipped)
    } else if impulse_ratios_db.is_empty() {
        (default.max(ambient_min), CalibrationVerdict::AmbientOnly)
    } else {
        let quietest = impulse_ratios_db.iter().copied().fold(f32::INFINITY, f32::min);
        let from_impulses = quietest - IMPULSE_MARGIN_DB;
        if from_impulses < ambient_min {
            (ambient_min, CalibrationVerdict::PoorSeparation)
        } else {
            (from_impulses.min(default.max(ambient_min)), CalibrationVerdict::Separated)
        }
    };
    CalibrationSuggestion {
        algorithm_version: CALIBRATION_ALGORITHM_VERSION.into(),
        threshold_db: threshold.clamp(THRESHOLD_MIN_DB, THRESHOLD_MAX_DB),
        verdict,
        impulse_ratios_db: impulse_ratios_db.to_vec(),
        ambient: ambient.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::Rng;

    fn ambient(db: f32) -> AmbientStats {
        let mut a = AmbientAnalyzer::new(48_000, 300.0);
        let mut r = Rng::new(3);
        let sigma = 10f64.powf(f64::from(db) / 20.0);
        let s: Vec<f32> = (0..48_000).map(|_| (r.gaussian() * sigma) as f32).collect();
        a.push(&s);
        a.stats().unwrap()
    }

    #[test]
    fn ambient_stats_track_level() {
        let st = ambient(-50.0);
        assert!((st.median_dbfs + 50.0).abs() < 1.5, "{st:?}");
        assert_eq!(st.windows, 100);
        assert_eq!(st.clipped_fraction, 0.0);
    }

    #[test]
    fn suggestions_respect_separation() {
        let st = ambient(-50.0);
        let s = suggest_threshold(&st, &[]);
        assert_eq!(s.verdict, CalibrationVerdict::AmbientOnly);
        let s = suggest_threshold(&st, &[40.0, 35.0]);
        assert_eq!(s.verdict, CalibrationVerdict::Separated);
        assert!(s.threshold_db <= 29.0);
        let s = suggest_threshold(&st, &[13.0]);
        assert_eq!(s.verdict, CalibrationVerdict::PoorSeparation);
        assert!(s.threshold_db >= THRESHOLD_MIN_DB);
    }
}
