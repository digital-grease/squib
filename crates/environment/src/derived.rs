//! Derived atmospheric metrics (docs/squib/05 "Derived atmospheric metrics"; plan phase
//! 3.4, ADR-050): moist-air density, density altitude and pressure altitude.
//!
//! Method `moist-ideal-ae96-isa-v1`, named and versioned so stored results stay explicable:
//! - Saturation vapour pressure over water: Alduchov and Eskridge (1996),
//!   `es = 610.94 Pa * exp(17.625 t / (t + 243.04))`, t in °C, stated for -40 to 50 °C.
//!   Vapour pressure `e = RH * es`; the enhancement factor (about 0.4 % on `e`) is omitted.
//! - Moist air as an ideal-gas mixture: `rho = (p - e) / (Rd T) + e / (Rv T)`, with
//!   `Rd = 287.058` and `Rv = 461.495` J/(kg K). This ignores compressibility (`Z`, about
//!   0.9996 near sea level); against CIPM-2007 within its stated range (600 to 1100 hPa,
//!   15 to 27 °C) the difference stays below 0.1 % (tested).
//! - Density altitude and pressure altitude: the geopotential height in the ICAO Standard
//!   Atmosphere troposphere (T0 288.15 K, lapse 0.0065 K/m, p0 101325 Pa, rho0 1.225
//!   kg/m3, g0 9.80665 m/s2, R 287.05287 J/(kg K)) whose density, respectively pressure,
//!   equals the value here. Results outside -2000 to 11000 m are not given.
//!
//! Inputs are the resolved local actual pressure, temperature and humidity. Altimeter
//! setting and sea-level pressure are never used (they are reduced quantities), and no
//! altitude reduction is applied to actual pressure. Missing inputs give no value.
//! Density altitude is a property of the air, not a height above anything.

use crate::candidate::{Field, Origin};
use crate::resolver::{EnvironmentSnapshot, Freshness, ResolvedField};

pub const METHOD: &str = "moist-ideal-ae96-isa-v1";
pub const R_DRY: f64 = 287.058;
pub const R_VAPOUR: f64 = 461.495;
pub const ISA_T0: f64 = 288.15;
pub const ISA_LAPSE: f64 = 0.0065;
pub const ISA_P0: f64 = 101_325.0;
pub const ISA_RHO0: f64 = 1.225;
pub const ISA_G0: f64 = 9.80665;
pub const ISA_R: f64 = 287.05287;
/// Geopotential range over which the troposphere formulas are used.
pub const ALTITUDE_RANGE_M: (f64, f64) = (-2_000.0, 11_000.0);

/// Saturation vapour pressure over liquid water, Pa (Alduchov and Eskridge 1996).
pub fn saturation_vapour_pressure_pa(t_c: f64) -> f64 {
    610.94 * (17.625 * t_c / (t_c + 243.04)).exp()
}

/// Moist-air density, kg/m3, from actual pressure (Pa), temperature (K) and relative
/// humidity (fraction 0 to 1).
pub fn moist_air_density(p_pa: f64, t_k: f64, rh: f64) -> f64 {
    let e = rh * saturation_vapour_pressure_pa(t_k - 273.15);
    (p_pa - e) / (R_DRY * t_k) + e / (R_VAPOUR * t_k)
}

fn isa_exponent() -> f64 {
    ISA_G0 / (ISA_R * ISA_LAPSE)
}

/// ISA troposphere: geopotential height whose standard density is `rho`.
pub fn density_altitude_m(rho: f64) -> f64 {
    (ISA_T0 / ISA_LAPSE) * (1.0 - (rho / ISA_RHO0).powf(1.0 / (isa_exponent() - 1.0)))
}

/// ISA troposphere: geopotential height whose standard pressure is `p_pa`.
pub fn pressure_altitude_m(p_pa: f64) -> f64 {
    (ISA_T0 / ISA_LAPSE) * (1.0 - (p_pa / ISA_P0).powf(1.0 / isa_exponent()))
}

/// Standard density and pressure at a geopotential height (used by tests and display).
pub fn isa_at(h_m: f64) -> (f64, f64) {
    let ratio = 1.0 - ISA_LAPSE * h_m / ISA_T0;
    (ISA_RHO0 * ratio.powf(isa_exponent() - 1.0), ISA_P0 * ratio.powf(isa_exponent()))
}

/// One input as used, for display next to the result.
#[derive(Debug, Clone, PartialEq)]
pub struct DerivedInput {
    pub field: Field,
    pub value_si: f64,
    pub origin: Origin,
    pub freshness: Option<Freshness>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DerivedMetrics {
    pub method: &'static str,
    pub density_kg_m3: f64,
    pub density_altitude_m: f64,
    pub pressure_altitude_m: f64,
    pub vapour_pressure_pa: f64,
    pub inputs: Vec<DerivedInput>,
    /// An input is stale, past its freshness limit, or of unknown age.
    pub uses_older_data: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NotDerived {
    /// These inputs have no usable value.
    Missing(Vec<Field>),
    /// An input or result is outside the method's range (what and why).
    OutOfRange(String),
}

fn value(f: Option<&ResolvedField>) -> Option<(f64, &ResolvedField)> {
    let f = f?;
    Some((f.chosen.as_ref()?.value.si().filter(|v| v.is_finite())?, f))
}

/// Derive the metrics from a resolved snapshot, or say why not.
pub fn derive(s: &EnvironmentSnapshot) -> Result<DerivedMetrics, NotDerived> {
    let wanted = [Field::LocalPressure, Field::Temperature, Field::RelativeHumidity];
    let got: Vec<Option<(f64, &ResolvedField)>> = wanted.iter().map(|f| value(s.field(*f))).collect();
    let missing: Vec<Field> = wanted.iter().zip(&got).filter(|(_, g)| g.is_none()).map(|(f, _)| *f).collect();
    if !missing.is_empty() {
        return Err(NotDerived::Missing(missing));
    }
    let (p, t, rh) = (got[0].unwrap().0, got[1].unwrap().0, got[2].unwrap().0);
    let t_c = t - 273.15;
    if !(-40.0..=50.0).contains(&t_c) {
        return Err(NotDerived::OutOfRange(format!("temperature {t_c:.1} °C is outside -40 to 50 °C")));
    }
    if !(30_000.0..=110_000.0).contains(&p) {
        return Err(NotDerived::OutOfRange(format!("pressure {:.0} hPa is outside 300 to 1100 hPa", p / 100.0)));
    }
    if !(0.0..=1.0).contains(&rh) {
        return Err(NotDerived::OutOfRange("humidity is outside 0 to 100 %".into()));
    }
    let rho = moist_air_density(p, t, rh);
    let (da, pa) = (density_altitude_m(rho), pressure_altitude_m(p));
    let (lo, hi) = ALTITUDE_RANGE_M;
    if !(lo..=hi).contains(&da) || !(lo..=hi).contains(&pa) {
        return Err(NotDerived::OutOfRange("the result is outside the standard troposphere".into()));
    }
    let inputs: Vec<DerivedInput> = wanted
        .iter()
        .zip(&got)
        .map(|(f, g)| {
            let (v, r) = g.unwrap();
            DerivedInput { field: *f, value_si: v, origin: r.origin, freshness: r.freshness }
        })
        .collect();
    let uses_older_data =
        inputs.iter().any(|i| matches!(i.freshness, Some(Freshness::Stale | Freshness::Expired | Freshness::UnknownAge)));
    Ok(DerivedMetrics {
        method: METHOD,
        density_kg_m3: rho,
        density_altitude_m: da,
        pressure_altitude_m: pa,
        vapour_pressure_pa: rh * saturation_vapour_pressure_pa(t_c),
        inputs,
        uses_older_data,
    })
}
