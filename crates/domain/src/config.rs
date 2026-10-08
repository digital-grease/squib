//! Immutable run configuration snapshot (docs/squib/06) and its validation.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::hash::content_hash;
use crate::state::SourceMode;

/// Documented upper bound for start delays, shown in the UI and enforced here.
pub const DELAY_MAX_MS: u32 = 30_000;
/// Maximum number of par cues in one run.
pub const MAX_PARS: usize = 10;
/// Maximum par time relative to the start reference.
pub const PAR_MAX_MS: u32 = 600_000;
/// Silence required between the end of one app cue and the start of the next.
pub const CUE_GAP_MS: u32 = 100;
/// Grace after the last par before an `AfterLastPar` auto stop.
pub const AUTO_STOP_GRACE_MAX_MS: u32 = 10_000;

/// App cue identities. Template generation lives in `squib-timing`; durations live here
/// because configuration validation depends on them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CueKind {
    Start,
    Par,
}

impl CueKind {
    /// Template version tag persisted with every cue observation.
    pub fn template_version(self) -> &'static str {
        match self {
            CueKind::Start => "start-chirp-v1",
            CueKind::Par => "par-chirp-v1",
        }
    }

    /// Template duration in milliseconds.
    pub fn duration_ms(self) -> u32 {
        match self {
            CueKind::Start => 150,
            CueKind::Par => 120,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CueKind::Start => "start",
            CueKind::Par => "par",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "start" => Some(CueKind::Start),
            "par" => Some(CueKind::Par),
            _ => None,
        }
    }
}

/// Start delay policy. The concrete delay is chosen by the platform on the control
/// path and stored in [`RunConfig::selected_delay_ms`]; replay reuses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DelayPolicy {
    Instant,
    Fixed { ms: u32 },
    Random { min_ms: u32, max_ms: u32 },
}

impl DelayPolicy {
    pub fn validate(&self) -> Result<(), ConfigError> {
        match *self {
            DelayPolicy::Instant => Ok(()),
            DelayPolicy::Fixed { ms } if ms > DELAY_MAX_MS => Err(ConfigError::DelayTooLong(ms)),
            DelayPolicy::Fixed { .. } => Ok(()),
            DelayPolicy::Random { min_ms, max_ms } => {
                if min_ms > max_ms {
                    Err(ConfigError::DelayRangeInverted { min_ms, max_ms })
                } else if max_ms > DELAY_MAX_MS {
                    Err(ConfigError::DelayTooLong(max_ms))
                } else {
                    Ok(())
                }
            }
        }
    }

    /// Whether `selected_ms` is a value this policy could have produced.
    pub fn admits(&self, selected_ms: u32) -> bool {
        match *self {
            DelayPolicy::Instant => selected_ms == 0,
            DelayPolicy::Fixed { ms } => selected_ms == ms,
            DelayPolicy::Random { min_ms, max_ms } => (min_ms..=max_ms).contains(&selected_ms),
        }
    }

    /// Select a delay using caller-supplied uniform randomness in `[0, 1)`.
    /// Randomness is injected so the domain stays deterministic.
    pub fn select(&self, unit_random: f64) -> u32 {
        match *self {
            DelayPolicy::Instant => 0,
            DelayPolicy::Fixed { ms } => ms,
            DelayPolicy::Random { min_ms, max_ms } => {
                let u = if unit_random.is_finite() { unit_random.clamp(0.0, 1.0) } else { 0.0 };
                let span = f64::from(max_ms - min_ms);
                let v = min_ms + (u * (span + 1.0)).floor() as u32;
                v.min(max_ms)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StopPolicy {
    /// Stop only on an explicit user command.
    Manual,
    /// Stop automatically `grace_ms` after the last par cue. Labelled in the UI.
    AfterLastPar { grace_ms: u32 },
}

/// Transparent detector configuration. Versioned and hashed; frozen for a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DetectorConfig {
    pub algorithm_version: String,
    /// Fast-energy level above the noise floor needed to open a candidate (dB).
    pub threshold_db: f32,
    /// Fast energy above the pre-onset medium energy needed to open a candidate (dB).
    pub attack_min_db: f32,
    /// Extra floor margin for an automatic accepted suggestion (dB).
    pub accept_margin_db: f32,
    /// Attack needed for an automatic accepted suggestion (dB).
    pub attack_accept_db: f32,
    pub highpass_hz: f32,
    /// Short duplicate protection after a candidate peak (ms). Not an echo blackout.
    pub min_gap_ms: f32,
    /// Window after a candidate in which a much quieter impulse is a possible echo (ms).
    pub echo_window_ms: f32,
    /// Level drop that marks a possible echo inside the echo window (dB).
    pub echo_drop_db: f32,
    /// Initial noise floor before ambient baseline (dBFS, mean-square).
    pub initial_floor_dbfs: f32,
}

pub const DETECTOR_ALGORITHM_VERSION: &str = "squib-onset-v1";

impl Default for DetectorConfig {
    fn default() -> Self {
        Self {
            algorithm_version: DETECTOR_ALGORITHM_VERSION.to_string(),
            threshold_db: 24.0,
            attack_min_db: 9.0,
            accept_margin_db: 6.0,
            attack_accept_db: 12.0,
            highpass_hz: 300.0,
            min_gap_ms: 15.0,
            echo_window_ms: 80.0,
            echo_drop_db: 6.0,
            initial_floor_dbfs: -60.0,
        }
    }
}

impl DetectorConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        let finite = [
            self.threshold_db,
            self.attack_min_db,
            self.accept_margin_db,
            self.attack_accept_db,
            self.highpass_hz,
            self.min_gap_ms,
            self.echo_window_ms,
            self.echo_drop_db,
            self.initial_floor_dbfs,
        ]
        .iter()
        .all(|v| v.is_finite());
        if !finite {
            return Err(ConfigError::DetectorInvalid("non-finite value"));
        }
        if !(6.0..=60.0).contains(&self.threshold_db) {
            return Err(ConfigError::DetectorInvalid("threshold_db outside 6..=60"));
        }
        if !(0.0..=40.0).contains(&self.attack_min_db) {
            return Err(ConfigError::DetectorInvalid("attack_min_db outside 0..=40"));
        }
        if !(20.0..=2000.0).contains(&self.highpass_hz) {
            return Err(ConfigError::DetectorInvalid("highpass_hz outside 20..=2000"));
        }
        if !(1.0..=100.0).contains(&self.min_gap_ms) {
            return Err(ConfigError::DetectorInvalid("min_gap_ms outside 1..=100"));
        }
        if !(-120.0..=0.0).contains(&self.initial_floor_dbfs) {
            return Err(ConfigError::DetectorInvalid("initial_floor_dbfs outside -120..=0"));
        }
        Ok(())
    }

    /// Short content hash identifying this exact configuration.
    pub fn config_hash(&self) -> String {
        content_hash(self)[..16].to_string()
    }
}

/// Immutable run configuration persisted before the run is armed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunConfig {
    pub schema_version: u32,
    pub source_mode: SourceMode,
    pub delay_policy: DelayPolicy,
    pub selected_delay_ms: u32,
    /// Par times relative to the start reference, strictly increasing.
    pub pars_ms: Vec<u32>,
    pub stop_policy: StopPolicy,
    /// Review hint only. Never forces the detector to return this many events.
    pub expected_count: Option<u32>,
    pub start_cue_template: String,
    pub par_cue_template: String,
    /// Present for microphone modes.
    pub detector: Option<DetectorConfig>,
    pub calibration_profile_id: Option<String>,
    pub route_signature: Option<String>,
    pub shooter_id: String,
    /// Extension points for M2/M3; always `None` in M1.
    pub drill_version_id: Option<String>,
    pub equipment_version_id: Option<String>,
    pub environment_snapshot_id: Option<String>,
    pub timestamp_mapping_method: String,
    pub app_build: String,
}

pub const TIMESTAMP_MAPPING_METHOD: &str = "frame-anchor-v1";

impl RunConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if !self.source_mode.available_in_m1() {
            return Err(ConfigError::ModeUnavailable(self.source_mode));
        }
        self.delay_policy.validate()?;
        if !self.delay_policy.admits(self.selected_delay_ms) {
            return Err(ConfigError::SelectedDelayOutsidePolicy(self.selected_delay_ms));
        }
        validate_pars(&self.pars_ms)?;
        if let StopPolicy::AfterLastPar { grace_ms } = self.stop_policy {
            if self.pars_ms.is_empty() {
                return Err(ConfigError::AutoStopWithoutPar);
            }
            if grace_ms > AUTO_STOP_GRACE_MAX_MS {
                return Err(ConfigError::AutoStopGraceTooLong(grace_ms));
            }
        }
        if let Some(n) = self.expected_count
            && (n == 0 || n > 1000)
        {
            return Err(ConfigError::ExpectedCountInvalid(n));
        }
        match (self.source_mode.needs_microphone(), &self.detector) {
            (true, None) => return Err(ConfigError::DetectorMissing),
            (false, Some(_)) => return Err(ConfigError::DetectorUnexpected),
            (_, Some(d)) => d.validate()?,
            _ => {}
        }
        if self.shooter_id.is_empty() {
            return Err(ConfigError::ShooterMissing);
        }
        Ok(())
    }

    pub fn config_hash(&self) -> String {
        content_hash(self)
    }
}

/// Validate par cue times so app cues never overlap each other or the start cue.
pub fn validate_pars(pars_ms: &[u32]) -> Result<(), ConfigError> {
    if pars_ms.len() > MAX_PARS {
        return Err(ConfigError::TooManyPars(pars_ms.len()));
    }
    let start_end = CueKind::Start.duration_ms() + CUE_GAP_MS;
    let par_span = CueKind::Par.duration_ms() + CUE_GAP_MS;
    let mut prev: Option<u32> = None;
    for &p in pars_ms {
        if p > PAR_MAX_MS {
            return Err(ConfigError::ParTooLong(p));
        }
        match prev {
            None if p < start_end => return Err(ConfigError::ParOverlapsStartCue(p)),
            Some(q) if p <= q => return Err(ConfigError::ParsNotIncreasing),
            Some(q) if p - q < par_span => return Err(ConfigError::ParCuesOverlap { first_ms: q, second_ms: p }),
            _ => {}
        }
        prev = Some(p);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum ConfigError {
    #[error("source mode {0:?} is not available in this build")]
    ModeUnavailable(SourceMode),
    #[error("start delay {0} ms exceeds the documented maximum")]
    DelayTooLong(u32),
    #[error("random delay minimum {min_ms} ms exceeds maximum {max_ms} ms")]
    DelayRangeInverted { min_ms: u32, max_ms: u32 },
    #[error("selected delay {0} ms is outside the delay policy")]
    SelectedDelayOutsidePolicy(u32),
    #[error("{0} par cues exceed the maximum")]
    TooManyPars(usize),
    #[error("par {0} ms exceeds the maximum par time")]
    ParTooLong(u32),
    #[error("par {0} ms would overlap the start cue")]
    ParOverlapsStartCue(u32),
    #[error("par times must be strictly increasing")]
    ParsNotIncreasing,
    #[error("par cues at {first_ms} ms and {second_ms} ms would overlap")]
    ParCuesOverlap { first_ms: u32, second_ms: u32 },
    #[error("auto stop after last par requires at least one par")]
    AutoStopWithoutPar,
    #[error("auto stop grace {0} ms exceeds the maximum")]
    AutoStopGraceTooLong(u32),
    #[error("expected count {0} is outside 1..=1000")]
    ExpectedCountInvalid(u32),
    #[error("microphone mode requires a detector configuration")]
    DetectorMissing,
    #[error("par-only mode must not carry a detector configuration")]
    DetectorUnexpected,
    #[error("detector configuration invalid: {0}")]
    DetectorInvalid(&'static str),
    #[error("shooter id missing")]
    ShooterMissing,
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn par_only_config() -> RunConfig {
        RunConfig {
            schema_version: 1,
            source_mode: SourceMode::ParOnly,
            delay_policy: DelayPolicy::Random { min_ms: 1000, max_ms: 4000 },
            selected_delay_ms: 2500,
            pars_ms: vec![2000, 4000],
            stop_policy: StopPolicy::Manual,
            expected_count: None,
            start_cue_template: CueKind::Start.template_version().into(),
            par_cue_template: CueKind::Par.template_version().into(),
            detector: None,
            calibration_profile_id: None,
            route_signature: None,
            shooter_id: "default".into(),
            drill_version_id: None,
            equipment_version_id: None,
            environment_snapshot_id: None,
            timestamp_mapping_method: TIMESTAMP_MAPPING_METHOD.into(),
            app_build: "test".into(),
        }
    }

    #[test]
    fn delay_bounds_are_validated() {
        assert!(DelayPolicy::Random { min_ms: 5, max_ms: 4 }.validate().is_err());
        assert!(DelayPolicy::Random { min_ms: 0, max_ms: DELAY_MAX_MS + 1 }.validate().is_err());
        assert!(DelayPolicy::Fixed { ms: DELAY_MAX_MS }.validate().is_ok());
        assert!(DelayPolicy::Random { min_ms: 3, max_ms: 3 }.validate().is_ok());
    }

    #[test]
    fn random_selection_covers_inclusive_range_and_is_injected() {
        let p = DelayPolicy::Random { min_ms: 1000, max_ms: 1002 };
        assert_eq!(p.select(0.0), 1000);
        assert_eq!(p.select(0.999_999), 1002);
        assert_eq!(p.select(f64::NAN), 1000);
        assert_eq!(p.select(1.0), 1002);
        assert!(p.admits(1001) && !p.admits(1003));
    }

    #[test]
    fn overlapping_and_unordered_pars_rejected() {
        assert_eq!(validate_pars(&[100]), Err(ConfigError::ParOverlapsStartCue(100)));
        assert!(matches!(validate_pars(&[1000, 1100]), Err(ConfigError::ParCuesOverlap { .. })));
        assert_eq!(validate_pars(&[1000, 900]), Err(ConfigError::ParsNotIncreasing));
        assert!(validate_pars(&[250, 470, 690]).is_ok());
    }

    #[test]
    fn par_only_rejects_detector_and_live_requires_one() {
        let mut c = par_only_config();
        assert!(c.validate().is_ok());
        c.detector = Some(DetectorConfig::default());
        assert_eq!(c.validate(), Err(ConfigError::DetectorUnexpected));
        c.source_mode = SourceMode::PhoneLive;
        assert!(c.validate().is_ok());
        c.detector = None;
        assert_eq!(c.validate(), Err(ConfigError::DetectorMissing));
    }

    #[test]
    fn deferred_modes_unavailable() {
        let mut c = par_only_config();
        c.source_mode = SourceMode::HardwareLive;
        assert!(matches!(c.validate(), Err(ConfigError::ModeUnavailable(_))));
        assert!(!SourceMode::ManualEntry.armable() && SourceMode::ManualEntry.available_in_m1());
    }

    #[test]
    fn selected_delay_must_match_policy() {
        let mut c = par_only_config();
        c.selected_delay_ms = 999;
        assert!(matches!(c.validate(), Err(ConfigError::SelectedDelayOutsidePolicy(999))));
    }

    #[test]
    fn config_hash_changes_with_content() {
        let a = par_only_config();
        let mut b = a.clone();
        b.selected_delay_ms = 2501;
        assert_ne!(a.config_hash(), b.config_hash());
        assert_eq!(a.config_hash(), par_only_config().config_hash());
    }
}
