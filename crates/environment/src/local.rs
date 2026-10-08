//! Candidates from phone sensors and manual entry.

use crate::candidate::{Field, MeasurementCandidate, SourceKind, Value, VerticalDatum};
use crate::units::{Plausibility, plausibility};

pub const LOCAL_ADAPTER_VERSION: &str = "android-local-v1";

fn base(field: Field, provider: &str, observed: Option<i64>, fetched: i64, source: SourceKind) -> MeasurementCandidate {
    MeasurementCandidate {
        id: format!("{provider}:{}:{}", field.as_str(), observed.map(|t| t.to_string()).unwrap_or_else(|| "unknown".into())),
        field,
        value: Value::Missing,
        original_value: None,
        original_unit: None,
        source,
        provider: provider.into(),
        station: None,
        observed_utc_ms: observed,
        fetched_utc_ms: fetched,
        qc: None,
        datum: None,
        accuracy: None,
        accuracy_meaning: None,
        transforms: vec![],
        adapter_version: LOCAL_ADAPTER_VERSION.into(),
    }
}

/// Phone barometer reading (hPa as reported by Android) as local actual pressure.
/// Sensor presence is not accuracy certification: no accuracy is attached.
pub fn barometer(hpa: f64, observed_utc_ms: i64) -> Option<MeasurementCandidate> {
    let pa = hpa * 100.0;
    if plausibility(Field::LocalPressure, pa) == Plausibility::Invalid {
        return None;
    }
    let mut c = base(Field::LocalPressure, "android_barometer", Some(observed_utc_ms), observed_utc_ms, SourceKind::PhoneSensor);
    c.value = Value::Si(pa);
    c.original_value = Some(hpa);
    c.original_unit = Some("hPa".into());
    c.transforms = vec!["hPa->Pa".into()];
    Some(c)
}

/// Android location heights. `ellipsoid_m` is `Location.getAltitude()` (WGS84);
/// `msl_m` is `getMslAltitudeMeters()` when the platform supplies it. Vertical accuracy
/// is attached only when reported, with its documented meaning.
pub fn location_heights(
    ellipsoid_m: Option<f64>,
    ellipsoid_accuracy_m: Option<f64>,
    msl_m: Option<f64>,
    msl_accuracy_m: Option<f64>,
    observed_utc_ms: i64,
) -> Vec<MeasurementCandidate> {
    let mut out = Vec::new();
    let mut push = |field: Field, v: Option<f64>, acc: Option<f64>, datum: VerticalDatum| {
        let Some(v) = v.filter(|v| plausibility(field, *v) != Plausibility::Invalid) else { return };
        let mut c = base(field, "android_location", Some(observed_utc_ms), observed_utc_ms, SourceKind::PhoneSensor);
        c.value = Value::Si(v);
        c.original_value = Some(v);
        c.original_unit = Some("m".into());
        c.datum = Some(datum);
        if let Some(a) = acc.filter(|a| a.is_finite() && *a > 0.0) {
            c.accuracy = Some(a);
            c.accuracy_meaning = Some("platform vertical accuracy, 68% confidence".into());
        }
        out.push(c);
    };
    push(Field::ElevationEllipsoid, ellipsoid_m, ellipsoid_accuracy_m, VerticalDatum::Wgs84Ellipsoid);
    push(Field::ElevationMsl, msl_m, msl_accuracy_m, VerticalDatum::MslPlatform);
    out
}

/// A manual entry in SI units. The field itself carries the pressure kind, so a
/// sea-level entry and an actual-pressure entry cannot be confused.
pub fn manual(field: Field, si: f64, entered_value: f64, entered_unit: &str, now_utc_ms: i64) -> Option<MeasurementCandidate> {
    if plausibility(field, si) == Plausibility::Invalid {
        return None;
    }
    let mut c = base(field, "manual", Some(now_utc_ms), now_utc_ms, SourceKind::Manual);
    c.value = Value::Si(si);
    c.original_value = Some(entered_value);
    c.original_unit = Some(entered_unit.into());
    if field == Field::ElevationMsl {
        c.datum = Some(VerticalDatum::MslManual);
    }
    if field == Field::ElevationEllipsoid {
        c.datum = Some(VerticalDatum::Wgs84Ellipsoid);
    }
    Some(c)
}

/// Manual calm/variable wind direction.
pub fn manual_direction_state(calm: bool, now_utc_ms: i64) -> MeasurementCandidate {
    let mut c = base(Field::WindDirection, "manual", Some(now_utc_ms), now_utc_ms, SourceKind::Manual);
    c.value = if calm { Value::Calm } else { Value::Variable };
    c
}
