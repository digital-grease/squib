//! Manually entered strings from another timer (A20).
//!
//! The values are elapsed times relative to that timer's own start, with the timer's
//! display precision kept. No fictional sample origin, detector score, or host-clock
//! synchronization is attached.

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAX_SHOTS: usize = 1000;
pub const MAX_ELAPSED_NS: i64 = 3_600_000_000_000; // one hour

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManualString {
    /// Free-text source label, e.g. "Club timer" (no serial numbers required).
    pub source_label: String,
    /// Display resolution of the source, e.g. 10 ms for a 0.01 s timer.
    pub precision_ns: i64,
    /// Cumulative shot times from the source timer's start, ns, non-decreasing.
    pub times_ns: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum ManualError {
    #[error("source label must be 1-80 characters")]
    Label,
    #[error("precision must be 1 ms, 10 ms, or 100 ms")]
    Precision,
    #[error("enter 1-{MAX_SHOTS} shot times")]
    Count,
    #[error("times must be positive, at most one hour, and not decreasing")]
    Times,
    #[error("time {0} is finer than the stated precision")]
    FinerThanPrecision(usize),
}

impl ManualString {
    pub fn validate(&self) -> Result<(), ManualError> {
        let l = self.source_label.trim();
        if l.is_empty() || l.chars().count() > 80 {
            return Err(ManualError::Label);
        }
        if ![1_000_000, 10_000_000, 100_000_000].contains(&self.precision_ns) {
            return Err(ManualError::Precision);
        }
        if self.times_ns.is_empty() || self.times_ns.len() > MAX_SHOTS {
            return Err(ManualError::Count);
        }
        if self.times_ns.iter().any(|t| *t <= 0 || *t > MAX_ELAPSED_NS) || self.times_ns.windows(2).any(|w| w[1] < w[0]) {
            return Err(ManualError::Times);
        }
        if let Some(i) = self.times_ns.iter().position(|t| t % self.precision_ns != 0) {
            return Err(ManualError::FinerThanPrecision(i));
        }
        Ok(())
    }

    /// Parse user text such as "1.42, 1.83 2.25" (seconds) at a given precision.
    pub fn parse(source_label: &str, precision_ns: i64, text: &str) -> Result<Self, ManualError> {
        let mut times = Vec::new();
        for tok in text.split(|c: char| c == ',' || c.is_whitespace()).filter(|t| !t.is_empty()) {
            let s: f64 = tok.parse().map_err(|_| ManualError::Times)?;
            if !s.is_finite() {
                return Err(ManualError::Times);
            }
            // Round to the stated precision so float parsing cannot invent resolution.
            let ns = ((s * 1e9) / precision_ns as f64).round() as i64 * precision_ns;
            times.push(ns);
        }
        let m = ManualString { source_label: source_label.trim().into(), precision_ns, times_ns: times };
        m.validate()?;
        Ok(m)
    }

    pub fn splits_ns(&self) -> Vec<i64> {
        self.times_ns.windows(2).map(|w| w[1] - w[0]).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a20_keeps_precision_and_label() {
        let m = ManualString::parse("Borrowed timer", 10_000_000, "1.42, 1.83 2.25").unwrap();
        assert_eq!(m.times_ns, vec![1_420_000_000, 1_830_000_000, 2_250_000_000]);
        assert_eq!(m.splits_ns(), vec![410_000_000, 420_000_000]);
        assert_eq!(m.precision_ns, 10_000_000);
        assert_eq!(m.source_label, "Borrowed timer");
        // Equal times are allowed (zero split), decreasing are not.
        assert!(ManualString::parse("t", 10_000_000, "1.00 1.00").is_ok());
        assert_eq!(ManualString::parse("t", 10_000_000, "2.0 1.0").unwrap_err(), ManualError::Times);
        assert_eq!(ManualString::parse("", 10_000_000, "1.0").unwrap_err(), ManualError::Label);
        assert_eq!(ManualString::parse("t", 7, "1.0").unwrap_err(), ManualError::Precision);
        assert_eq!(ManualString::parse("t", 10_000_000, "").unwrap_err(), ManualError::Count);
        assert_eq!(ManualString::parse("t", 10_000_000, "nan").unwrap_err(), ManualError::Times);
        let finer = ManualString { source_label: "t".into(), precision_ns: 10_000_000, times_ns: vec![1_234_000_000] };
        assert_eq!(finer.validate().unwrap_err(), ManualError::FinerThanPrecision(0));
    }
}
