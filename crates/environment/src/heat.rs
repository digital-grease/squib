//! Heat index (plan phase 3.5, ADR-051): the NWS algorithm, as published by the Weather
//! Prediction Center ("The Heat Index Equation"): the simple formula first, averaged with
//! the temperature; when that is 80 °F or more, the Rothfusz regression (NWS Technical
//! Attachment SR 90-23, 1990) with the low-humidity adjustment subtracted (RH < 13 %,
//! 80 to 112 °F) and the high-humidity adjustment added (RH > 85 %, 80 to 87 °F).
//! The regression is not valid beyond Steadman's data; values are for shade and light
//! wind, and NWS notes full sunshine can add up to 15 °F.
//!
//! The NWS heat index chart (weather.gov/safety/heat-index) shows the raw regression in the
//! humid corner (RH above 85 %, 80 to 87 °F), without the high-humidity adjustment the
//! algorithm adds; there the chart reads up to about 2 °F lower than this function.

pub const METHOD: &str = "nws-heat-index-rothfusz";

/// Heat index, °F, from air temperature (°F) and relative humidity (percent, 0 to 100).
pub fn heat_index_f(t: f64, rh: f64) -> f64 {
    let simple = 0.5 * (t + 61.0 + (t - 68.0) * 1.2 + rh * 0.094);
    if (simple + t) / 2.0 < 80.0 {
        return simple;
    }
    let mut hi =
        -42.379 + 2.049_015_23 * t + 10.143_331_27 * rh - 0.224_755_41 * t * rh - 0.006_837_83 * t * t - 0.054_817_17 * rh * rh
            + 0.001_228_74 * t * t * rh
            + 0.000_852_82 * t * rh * rh
            - 0.000_001_99 * t * t * rh * rh;
    if rh < 13.0 && (80.0..=112.0).contains(&t) {
        hi -= ((13.0 - rh) / 4.0) * ((17.0 - (t - 95.0).abs()) / 17.0).sqrt();
    } else if rh > 85.0 && (80.0..=87.0).contains(&t) {
        hi += ((rh - 85.0) / 10.0) * ((87.0 - t) / 5.0);
    }
    hi
}

/// NWS heat index chart categories ("Likelihood of heat disorders with prolonged exposure
/// or strenuous activity"), by heat index in °F.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeatCategory {
    Caution,
    ExtremeCaution,
    Danger,
    ExtremeDanger,
}

impl HeatCategory {
    /// `None` below 80 °F, where the chart gives no category.
    pub fn of(hi_f: f64) -> Option<Self> {
        let r = hi_f.round();
        Some(if r >= 125.0 {
            HeatCategory::ExtremeDanger
        } else if r >= 103.0 {
            HeatCategory::Danger
        } else if r >= 91.0 {
            HeatCategory::ExtremeCaution
        } else if r >= 80.0 {
            HeatCategory::Caution
        } else {
            return None;
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            HeatCategory::Caution => "Caution",
            HeatCategory::ExtremeCaution => "Extreme caution",
            HeatCategory::Danger => "Danger",
            HeatCategory::ExtremeDanger => "Extreme danger",
        }
    }
}
