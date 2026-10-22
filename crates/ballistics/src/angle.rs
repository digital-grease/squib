//! Angular units for corrections (plan 8.4, ADR-066). The solver reports its corrections
//! through these functions, and the in-app primers and the angle converter use the same
//! ones, so the text, the converter and the range card cannot disagree.
//!
//! An angle and the size it covers at a distance are related as the solver relates a
//! miss to its correction: size = distance x tan(angle).

/// One yard, metres (exact, international yard).
pub const YARD_M: f64 = 0.9144;
/// One inch, metres (exact).
pub const INCH_M: f64 = 0.0254;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AngleUnit {
    /// Milliradian: 1/1000 radian.
    Mil,
    /// Minute of angle: 1/60 degree.
    Moa,
    /// Inches per hundred yards ("shooter's MOA"): the angle that covers exactly 1 inch at 100 yards.
    Iphy,
}

impl AngleUnit {
    /// Parses `mil`, `moa` or `iphy`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "mil" => Some(Self::Mil),
            "moa" => Some(Self::Moa),
            "iphy" => Some(Self::Iphy),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mil => "mil",
            Self::Moa => "moa",
            Self::Iphy => "iphy",
        }
    }

    /// How the unit is written after a number.
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Mil => "mil",
            Self::Moa => "MOA",
            Self::Iphy => "IPHY",
        }
    }

    /// Radians to this unit. Mil and MOA are computed exactly as the solver always has,
    /// so range cards are unchanged bit for bit.
    pub fn from_rad(self, rad: f64) -> f64 {
        match self {
            Self::Mil => rad * 1000.0,
            Self::Moa => rad.to_degrees() * 60.0,
            Self::Iphy => rad / iphy_rad(),
        }
    }

    /// This unit to radians.
    pub fn to_rad(self, value: f64) -> f64 {
        match self {
            Self::Mil => value / 1000.0,
            Self::Moa => (value / 60.0).to_radians(),
            Self::Iphy => value * iphy_rad(),
        }
    }
}

/// One IPHY in radians.
fn iphy_rad() -> f64 {
    (INCH_M / (100.0 * YARD_M)).atan()
}

/// The size (m) an angle covers at a distance (m).
pub fn subtension_m(rad: f64, distance_m: f64) -> f64 {
    distance_m * rad.tan()
}

/// One unit of `from` expressed in `to`.
pub fn ratio(from: AngleUnit, to: AngleUnit) -> f64 {
    to.from_rad(from.to_rad(1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inches(m: f64) -> f64 {
        m / INCH_M
    }

    #[test]
    fn definitions_match_published_values() {
        // 1 MOA = pi / 10800 rad; at 100 yd it covers 1.0472 in (NIST SP 811 for the
        // minute of arc; the inch figure is the usual 1.047).
        assert!((AngleUnit::Moa.to_rad(1.0) - std::f64::consts::PI / 10800.0).abs() < 1e-18);
        assert_eq!((inches(subtension_m(AngleUnit::Moa.to_rad(1.0), 100.0 * YARD_M)) * 1e4).round(), 10_472.0);
        // 1 mil covers 10 cm at 100 m and 3.6 in at 100 yd.
        assert!((subtension_m(AngleUnit::Mil.to_rad(1.0), 100.0) - 0.1).abs() < 1e-7);
        assert!((inches(subtension_m(AngleUnit::Mil.to_rad(1.0), 100.0 * YARD_M)) - 3.6).abs() < 1e-5);
        // 1 IPHY covers exactly 1 in at 100 yd.
        assert!((inches(subtension_m(AngleUnit::Iphy.to_rad(1.0), 100.0 * YARD_M)) - 1.0).abs() < 1e-12);
        // 1 mil = 3.4377 MOA; 1 IPHY = 0.9549 MOA.
        assert!((ratio(AngleUnit::Mil, AngleUnit::Moa) - 3.437_747).abs() < 1e-6);
        assert!((ratio(AngleUnit::Iphy, AngleUnit::Moa) - 0.954_930).abs() < 1e-6);
    }

    #[test]
    fn round_trips_and_parsing() {
        for u in [AngleUnit::Mil, AngleUnit::Moa, AngleUnit::Iphy] {
            assert!((u.from_rad(u.to_rad(4.1)) - 4.1).abs() < 1e-12);
            assert_eq!(AngleUnit::parse(u.as_str()), Some(u));
        }
        assert_eq!(AngleUnit::parse("degrees"), None);
    }

    #[test]
    fn solver_conversions_are_unchanged() {
        let el = 0.012_345_678_9_f64;
        assert_eq!(AngleUnit::Mil.from_rad(el).to_bits(), (el * 1000.0).to_bits());
        assert_eq!(AngleUnit::Moa.from_rad(el).to_bits(), (el.to_degrees() * 60.0).to_bits());
    }
}
