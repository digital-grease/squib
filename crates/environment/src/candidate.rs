//! Normalized measurement candidates (docs/squib/05 "Candidate contract").
//!
//! Every value is finite SI or an explicit non-numeric state. Pressure kinds and
//! elevation datums are separate fields, so a sea-level or altimeter value can never
//! occupy the local actual-pressure slot (A10), and an ellipsoid height cannot be
//! silently compared with a mean-sea-level height (A11).

use serde::{Deserialize, Serialize};

/// A resolvable field. Each pressure kind and each elevation reference is its own field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    /// Air temperature, kelvin.
    Temperature,
    /// Relative humidity, fraction 0–1.
    RelativeHumidity,
    /// Sustained wind speed, m/s.
    WindSpeed,
    /// Gust speed, m/s. Distinct from sustained wind.
    WindGust,
    /// Direction the wind blows FROM, degrees true.
    WindDirection,
    /// Actual pressure at the user's location, Pa. Only local sensors or manual
    /// actual-pressure entries may fill this field.
    LocalPressure,
    /// Actual pressure at a remote station's own elevation, Pa. Never relabelled local.
    RemoteStationPressure,
    /// Altimeter setting (reduced quantity), Pa.
    AltimeterSetting,
    /// Mean sea-level pressure (reduced quantity), Pa.
    SeaLevelPressure,
    /// Height above mean sea level, m (datum per candidate).
    ElevationMsl,
    /// Height above the WGS84 ellipsoid, m. Not comparable with MSL without a geoid model.
    ElevationEllipsoid,
}

impl Field {
    pub const ALL: [Field; 11] = [
        Field::Temperature,
        Field::RelativeHumidity,
        Field::WindSpeed,
        Field::WindGust,
        Field::WindDirection,
        Field::LocalPressure,
        Field::RemoteStationPressure,
        Field::AltimeterSetting,
        Field::SeaLevelPressure,
        Field::ElevationMsl,
        Field::ElevationEllipsoid,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Field::Temperature => "temperature",
            Field::RelativeHumidity => "relative_humidity",
            Field::WindSpeed => "wind_speed",
            Field::WindGust => "wind_gust",
            Field::WindDirection => "wind_direction",
            Field::LocalPressure => "local_pressure",
            Field::RemoteStationPressure => "remote_station_pressure",
            Field::AltimeterSetting => "altimeter_setting",
            Field::SeaLevelPressure => "sea_level_pressure",
            Field::ElevationMsl => "elevation_msl",
            Field::ElevationEllipsoid => "elevation_ellipsoid",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Field::ALL.iter().copied().find(|f| f.as_str() == s)
    }

    /// SI unit label for this field.
    pub fn si_unit(self) -> &'static str {
        match self {
            Field::Temperature => "K",
            Field::RelativeHumidity => "fraction",
            Field::WindSpeed | Field::WindGust => "m/s",
            Field::WindDirection => "deg_true_from",
            Field::LocalPressure | Field::RemoteStationPressure | Field::AltimeterSetting | Field::SeaLevelPressure => "Pa",
            Field::ElevationMsl | Field::ElevationEllipsoid => "m",
        }
    }

    pub fn is_elevation(self) -> bool {
        matches!(self, Field::ElevationMsl | Field::ElevationEllipsoid)
    }
}

/// A value: finite SI number or an explicit non-numeric state. Unknown is `Missing`,
/// never zero; calm wind is not north.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "si", rename_all = "snake_case")]
pub enum Value {
    Si(f64),
    /// Wind direction with no wind.
    Calm,
    /// Wind direction reported as variable.
    Variable,
    Missing,
}

impl Value {
    pub fn si(self) -> Option<f64> {
        match self {
            Value::Si(v) => Some(v),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// A sensor on this phone (barometer, location).
    PhoneSensor,
    /// An external sensor the user connected (future).
    ExternalSensor,
    /// A weather station observation.
    StationObservation,
    /// A model estimate at a defined height/terrain (future providers).
    ModelEstimate,
    /// Entered by the user.
    Manual,
}

/// Concise origin category for display (docs/squib/02 "Conditions screen").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Here,
    NearbyObservation,
    ModelEstimate,
    Manual,
    Unavailable,
}

impl SourceKind {
    pub fn origin(self) -> Origin {
        match self {
            SourceKind::PhoneSensor | SourceKind::ExternalSensor => Origin::Here,
            SourceKind::StationObservation => Origin::NearbyObservation,
            SourceKind::ModelEstimate => Origin::ModelEstimate,
            SourceKind::Manual => Origin::Manual,
        }
    }
}

/// Vertical reference of an elevation value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerticalDatum {
    /// WGS84 ellipsoid (Android `Location.getAltitude`).
    Wgs84Ellipsoid,
    /// Mean sea level as reported by the platform (geoid model unspecified).
    MslPlatform,
    /// Mean sea level as stated by a provider's station metadata.
    MslProvider,
    /// Mean sea level stated by the user.
    MslManual,
    Unknown,
}

impl VerticalDatum {
    pub fn is_msl(self) -> bool {
        matches!(self, VerticalDatum::MslPlatform | VerticalDatum::MslProvider | VerticalDatum::MslManual)
    }
}

/// Where a station is relative to the lookup place (precise provenance; redactable).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StationRef {
    pub station_id: String,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    /// Station elevation above MSL, m, when the provider states it.
    pub elevation_m: Option<f64>,
    /// Great-circle distance from the lookup place, m.
    pub distance_m: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeasurementCandidate {
    /// Stable within a refresh: `{provider}:{station or sensor}:{field}:{observed}`.
    pub id: String,
    pub field: Field,
    pub value: Value,
    /// Value and unit exactly as received, before normalization.
    pub original_value: Option<f64>,
    pub original_unit: Option<String>,
    pub source: SourceKind,
    /// e.g. `nws`, `android_barometer`, `android_location`, `manual`.
    pub provider: String,
    pub station: Option<StationRef>,
    /// Field observation time (UTC ms). `None` means unknown, never fetch time.
    pub observed_utc_ms: Option<i64>,
    /// When this app obtained the value (UTC ms).
    pub fetched_utc_ms: i64,
    /// Provider QC code kept verbatim even when not understood.
    pub qc: Option<String>,
    pub datum: Option<VerticalDatum>,
    /// Accuracy only when the source actually provides it, with its meaning.
    pub accuracy: Option<f64>,
    pub accuracy_meaning: Option<String>,
    /// Transformations applied (e.g. `km_h-1->m_s-1`).
    pub transforms: Vec<String>,
    pub adapter_version: String,
}

impl MeasurementCandidate {
    /// Age of the observation; unknown observation time has unknown age.
    pub fn age_ms(&self, now_utc_ms: i64) -> Option<i64> {
        self.observed_utc_ms.map(|t| now_utc_ms - t)
    }

    pub fn origin(&self) -> Origin {
        self.source.origin()
    }
}
