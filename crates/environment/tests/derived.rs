//! Plan phase 3.4: derived atmospheric metrics checked against independent references.
//! - ICAO Standard Atmosphere values from two sources: Wikipedia "International Standard
//!   Atmosphere" (geopotential 11 000 m: 22 632 Pa, 0.3639 kg/m3) and Engineering ToolBox
//!   "U.S. Standard Atmosphere" (1976, identical to ISA in the troposphere) for 1000 to
//!   3000 m. The ToolBox values fit geometric heights (at 3000 m its 0.9093 kg/m3 matches
//!   geometric 3000 m, 0.90926, not geopotential, 0.90912), so the heights are converted.
//! - CIPM-2007 moist-air density (Picard et al., Metrologia 45 (2008) 149-155),
//!   implemented below from the paper's equations (1, 5b, A1.1-A1.4) as an independent
//!   oracle, and checked against a published example (ISOBudgets, "Moist Air Density":
//!   20 °C, 1013.25 hPa, 50 % RH -> 1.19929 kg/m3, from the OIML R111 approximation of
//!   CIPM-2007, whose stated relative uncertainty is about 2e-4).

use squib_environment::derived::{self, NotDerived};
use squib_environment::*;

const NOW: i64 = 1_791_633_600_000;

/// CIPM-2007, xCO2 = 400 µmol/mol. `t_c` °C, `p` Pa, `h` relative humidity 0..1.
fn cipm2007(p: f64, t_c: f64, h: f64) -> f64 {
    let t = t_c + 273.15;
    let psv = (1.237_884_7e-5 * t * t - 1.912_131_6e-2 * t + 33.937_110_47 - 6.343_164_5e3 / t).exp();
    let f = 1.000_62 + 3.14e-8 * p + 5.6e-7 * t_c * t_c;
    let xv = h * f * psv / p;
    let (a0, a1, a2) = (1.581_23e-6, -2.9331e-8, 1.1043e-10);
    let (b0, b1, c0, c1) = (5.707e-6, -2.051e-8, 1.9898e-4, -2.376e-6);
    let (d, e) = (1.83e-11, -0.765e-8);
    let z = 1.0 - p / t * (a0 + a1 * t_c + a2 * t_c * t_c + (b0 + b1 * t_c) * xv + (c0 + c1 * t_c) * xv * xv)
        + p * p / (t * t) * (d + e * xv * xv);
    3.483_740e-3 * p / (z * t) * (1.0 - 0.3780 * xv)
}

#[test]
fn the_cipm_oracle_reproduces_the_published_example() {
    let rho = cipm2007(101_325.0, 20.0, 0.5);
    assert!((rho - 1.19929).abs() / 1.19929 < 1e-4, "CIPM-2007 oracle {rho}");
}

#[test]
fn moist_density_agrees_with_cipm_2007_across_its_range() {
    let mut worst: f64 = 0.0;
    for p in [60_000.0, 80_000.0, 101_325.0, 110_000.0] {
        for t in [15.0, 20.0, 27.0] {
            for h in [0.0, 0.5, 1.0] {
                let ours = derived::moist_air_density(p, t + 273.15, h);
                let rel = (ours - cipm2007(p, t, h)).abs() / cipm2007(p, t, h);
                worst = worst.max(rel);
                assert!(rel < 1e-3, "p {p} t {t} h {h}: {ours} vs {}", cipm2007(p, t, h));
            }
        }
    }
    eprintln!("worst relative difference from CIPM-2007: {:.4} %", worst * 100.0);
    // Humid air is lighter than dry air at the same pressure and temperature.
    assert!(derived::moist_air_density(101_325.0, 303.15, 1.0) < derived::moist_air_density(101_325.0, 303.15, 0.0));
}

#[test]
fn saturation_vapour_pressure_agrees_with_the_cipm_formula() {
    for t_c in [0.0_f64, 10.0, 20.0, 30.0, 40.0] {
        let t = t_c + 273.15;
        let cipm = (1.237_884_7e-5 * t * t - 1.912_131_6e-2 * t + 33.937_110_47 - 6.343_164_5e3 / t).exp();
        let ae = derived::saturation_vapour_pressure_pa(t_c);
        assert!((ae - cipm).abs() / cipm < 4e-3, "{t_c} °C: {ae} vs {cipm}");
    }
}

#[test]
fn standard_atmosphere_values_from_two_sources() {
    // Sea level is zero for both.
    assert!(derived::density_altitude_m(1.225).abs() < 0.01);
    assert!(derived::pressure_altitude_m(101_325.0).abs() < 0.01);
    // Wikipedia ISA: 11 000 m, 22 632 Pa, 0.3639 kg/m3.
    assert!((derived::pressure_altitude_m(22_632.0) - 11_000.0).abs() < 2.0);
    assert!((derived::density_altitude_m(0.3639) - 11_000.0).abs() < 5.0);
    // Engineering ToolBox (US Standard Atmosphere 1976) at geometric heights, rounded to 4
    // figures: tolerance is half a unit in the last figure, as height, plus 0.1 m.
    let geopotential = |z: f64| 6_356_766.0 * z / (6_356_766.0 + z);
    for (z, p, p_unit, rho, rho_unit) in [
        (1_000.0, 89_880.0, 10.0, 1.112, 0.001),
        (2_000.0, 79_500.0, 100.0, 1.007, 0.001),
        (3_000.0, 70_120.0, 100.0, 0.9093, 0.0001),
    ] {
        let h = geopotential(z);
        let (r, pp) = derived::isa_at(h);
        let (r2, pp2) = derived::isa_at(h + 1.0);
        let tol_p = 0.5 * p_unit / (pp - pp2) + 0.1;
        let tol_rho = 0.5 * rho_unit / (r - r2) + 0.1;
        assert!(
            (derived::pressure_altitude_m(p) - h).abs() < tol_p,
            "pressure altitude at {z} m: {}",
            derived::pressure_altitude_m(p)
        );
        assert!(
            (derived::density_altitude_m(rho) - h).abs() < tol_rho,
            "density altitude at {z} m: {}",
            derived::density_altitude_m(rho)
        );
    }
    // The inverse and forward forms agree.
    let (rho, p) = derived::isa_at(1_500.0);
    assert!((derived::density_altitude_m(rho) - 1_500.0).abs() < 1e-6);
    assert!((derived::pressure_altitude_m(p) - 1_500.0).abs() < 1e-6);
}

fn snap(pressure: Option<(Field, f64)>, t_c: Option<f64>, rh: Option<f64>) -> EnvironmentSnapshot {
    let mut c = vec![];
    if let Some((f, v)) = pressure {
        c.push(local::manual(f, v, v / 100.0, "hPa", NOW).unwrap());
    }
    if let Some(t) = t_c {
        c.push(local::manual(Field::Temperature, t + 273.15, t, "degC", NOW).unwrap());
    }
    if let Some(h) = rh {
        c.push(local::manual(Field::RelativeHumidity, h, h * 100.0, "%", NOW).unwrap());
    }
    // As manual overrides, the way the app stores entries.
    let overrides: Vec<Override> =
        c.into_iter().map(|candidate| Override { candidate, set_utc_ms: NOW, expires_utc_ms: None }).collect();
    resolve("s", &[], &overrides, None, vec![], &ResolverPolicy::default(), NOW)
}

#[test]
fn derives_only_from_actual_local_pressure_and_says_what_is_missing() {
    // A range at about 1600 m on a warm day: thinner air than standard at that height.
    let s = snap(Some((Field::LocalPressure, 83_500.0)), Some(30.0), Some(0.2));
    let d = derived::derive(&s).unwrap();
    assert_eq!(d.method, "moist-ideal-ae96-isa-v1");
    assert!((d.density_kg_m3 - derived::moist_air_density(83_500.0, 303.15, 0.2)).abs() < 1e-12);
    assert!(d.density_altitude_m > d.pressure_altitude_m, "hot air: density altitude above pressure altitude");
    assert_eq!(d.inputs.len(), 3);
    assert!(d.inputs.iter().all(|i| i.origin == Origin::Manual));
    // An altimeter setting is not actual pressure and is never used in its place.
    let s = snap(Some((Field::AltimeterSetting, 101_600.0)), Some(30.0), Some(0.2));
    assert_eq!(derived::derive(&s), Err(NotDerived::Missing(vec![Field::LocalPressure])));
    let s = snap(None, None, Some(0.2));
    assert_eq!(derived::derive(&s), Err(NotDerived::Missing(vec![Field::LocalPressure, Field::Temperature])));
    let s = snap(Some((Field::LocalPressure, 83_500.0)), Some(55.0), Some(0.2));
    assert!(matches!(derived::derive(&s), Err(NotDerived::OutOfRange(_))));
}
