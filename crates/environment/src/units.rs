//! Unit normalization by declared unit code (never assumed JSON units) and validation.

use serde::{Deserialize, Serialize};

use crate::candidate::Field;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum UnitError {
    /// The unit code is not one this adapter version understands.
    UnknownUnit(String),
    /// The unit does not measure this field's quantity.
    WrongDimension {
        field: Field,
        unit: String,
    },
    NonFinite,
}

/// Strip a namespace prefix such as `wmoUnit:` or `unit:`.
fn bare(unit: &str) -> &str {
    unit.rsplit(':').next().unwrap_or(unit)
}

/// Convert `value` in `unit` to the SI unit of `field`. Returns the SI value and a
/// transform label (empty when the unit was already SI).
pub fn to_si(field: Field, value: f64, unit: &str) -> Result<(f64, Option<String>), UnitError> {
    if !value.is_finite() {
        return Err(UnitError::NonFinite);
    }
    let u = bare(unit);
    let wrong = || UnitError::WrongDimension { field, unit: unit.to_string() };
    let t = |label: &str| Some(format!("{u}->{label}"));
    let r = match field {
        Field::Temperature => match u {
            "degC" => (value + 273.15, t("K")),
            "degF" => ((value - 32.0) * 5.0 / 9.0 + 273.15, t("K")),
            "K" => (value, None),
            _ if is_known(u) => return Err(wrong()),
            _ => return Err(UnitError::UnknownUnit(unit.into())),
        },
        Field::RelativeHumidity => match u {
            "percent" => (value / 100.0, t("fraction")),
            "fraction" => (value, None),
            _ if is_known(u) => return Err(wrong()),
            _ => return Err(UnitError::UnknownUnit(unit.into())),
        },
        Field::WindSpeed | Field::WindGust => match u {
            "m_s-1" => (value, None),
            "km_h-1" => (value / 3.6, t("m_s-1")),
            "kt" | "knot" => (value * 1852.0 / 3600.0, t("m_s-1")),
            "mi_h-1" | "mph" => (value * 0.44704, t("m_s-1")),
            _ if is_known(u) => return Err(wrong()),
            _ => return Err(UnitError::UnknownUnit(unit.into())),
        },
        Field::WindDirection => match u {
            "degree_(angle)" | "deg" => (value, None),
            _ if is_known(u) => return Err(wrong()),
            _ => return Err(UnitError::UnknownUnit(unit.into())),
        },
        Field::LocalPressure | Field::RemoteStationPressure | Field::AltimeterSetting | Field::SeaLevelPressure => match u {
            "Pa" => (value, None),
            "hPa" | "mbar" => (value * 100.0, t("Pa")),
            "kPa" => (value * 1000.0, t("Pa")),
            "inHg" | "in_Hg" => (value * 3386.389, t("Pa")),
            _ if is_known(u) => return Err(wrong()),
            _ => return Err(UnitError::UnknownUnit(unit.into())),
        },
        Field::ElevationMsl | Field::ElevationEllipsoid => match u {
            "m" => (value, None),
            "ft" => (value * 0.3048, t("m")),
            _ if is_known(u) => return Err(wrong()),
            _ => return Err(UnitError::UnknownUnit(unit.into())),
        },
    };
    Ok(r)
}

fn is_known(u: &str) -> bool {
    matches!(
        u,
        "degC"
            | "degF"
            | "K"
            | "percent"
            | "fraction"
            | "m_s-1"
            | "km_h-1"
            | "kt"
            | "knot"
            | "mi_h-1"
            | "mph"
            | "degree_(angle)"
            | "deg"
            | "Pa"
            | "hPa"
            | "mbar"
            | "kPa"
            | "inHg"
            | "in_Hg"
            | "m"
            | "ft"
    )
}

/// Validation verdict for a normalized SI value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Plausibility {
    Ok,
    /// Physically possible but rare: kept, flagged.
    RareExtreme,
    /// Physically impossible: rejected.
    Invalid,
}

/// Hard and plausible bounds per field. Bounds are policy, fixture-tested, and
/// deliberately wide: a low pressure is not "broken" because it differs from a
/// sea-level report.
pub fn plausibility(field: Field, si: f64) -> Plausibility {
    let (hard, soft) = match field {
        Field::Temperature => ((173.15, 343.15), (213.15, 330.15)), // −100..70 °C; −60..57 °C
        Field::RelativeHumidity => ((0.0, 1.05), (0.0, 1.0)),
        Field::WindSpeed | Field::WindGust => ((0.0, 115.0), (0.0, 60.0)),
        Field::WindDirection => ((0.0, 360.0), (0.0, 360.0)),
        // Everest summit is about 33 kPa; extreme storms near 87 kPa at sea level.
        Field::LocalPressure | Field::RemoteStationPressure => ((25_000.0, 110_000.0), (50_000.0, 108_500.0)),
        Field::AltimeterSetting | Field::SeaLevelPressure => ((85_000.0, 110_000.0), (91_000.0, 108_500.0)),
        Field::ElevationMsl | Field::ElevationEllipsoid => ((-500.0, 9_000.0), (-450.0, 6_000.0)),
    };
    if !si.is_finite() || si < hard.0 || si > hard.1 {
        Plausibility::Invalid
    } else if si < soft.0 || si > soft.1 {
        Plausibility::RareExtreme
    } else {
        Plausibility::Ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn converts_by_declared_unit_code() {
        assert!(close(to_si(Field::Temperature, 26.0, "wmoUnit:degC").unwrap().0, 299.15));
        assert!(close(to_si(Field::Temperature, 32.0, "wmoUnit:degF").unwrap().0, 273.15));
        assert!(close(to_si(Field::RelativeHumidity, 19.5, "wmoUnit:percent").unwrap().0, 0.195));
        assert!(close(to_si(Field::WindSpeed, 36.0, "wmoUnit:km_h-1").unwrap().0, 10.0));
        assert!(close(to_si(Field::AltimeterSetting, 29.92, "inHg").unwrap().0, 101_320.758_88));
        assert!(close(to_si(Field::LocalPressure, 1013.25, "hPa").unwrap().0, 101_325.0));
        assert!(close(to_si(Field::ElevationMsl, 1000.0, "ft").unwrap().0, 304.8));
        assert_eq!(
            to_si(Field::Temperature, 5.0, "Pa").unwrap_err(),
            UnitError::WrongDimension { field: Field::Temperature, unit: "Pa".into() }
        );
        assert!(matches!(to_si(Field::Temperature, 5.0, "wmoUnit:degRe"), Err(UnitError::UnknownUnit(_))));
        assert_eq!(to_si(Field::Temperature, f64::NAN, "degC").unwrap_err(), UnitError::NonFinite);
    }

    #[test]
    fn plausibility_distinguishes_extreme_from_invalid() {
        assert_eq!(plausibility(Field::LocalPressure, 33_700.0), Plausibility::RareExtreme, "summit pressure is real");
        assert_eq!(plausibility(Field::LocalPressure, 83_000.0), Plausibility::Ok, "high-altitude range pressure is normal");
        assert_eq!(plausibility(Field::SeaLevelPressure, 83_000.0), Plausibility::Invalid);
        assert_eq!(plausibility(Field::Temperature, 400.0), Plausibility::Invalid);
        assert_eq!(plausibility(Field::RelativeHumidity, 1.2), Plausibility::Invalid);
        assert_eq!(plausibility(Field::WindSpeed, -1.0), Plausibility::Invalid);
    }
}
