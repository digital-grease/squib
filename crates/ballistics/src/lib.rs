//! Squib's point-mass (three degrees of freedom) ballistics solver (plan phase 6, owner
//! ruling D14). Experimental: it is shown in the app only inside the envelope the
//! validation suite covers (`tests/validation.rs`, ADR-061).
//!
//! Model: a G1 or G7 standard drag function scaled by the published ballistic
//! coefficient; gravity; drag on the velocity relative to a constant horizontal wind; air
//! density and speed of sound from the firing point as measured, carried along the path
//! by the ISA lapse rate; fixed-step fourth-order Runge-Kutta integration, checked by
//! repeating at half the step; the weapon zeroed on a level shot. Optional, off unless
//! asked for (plan 6.5, ADR-064): spin drift by Litz's approximation with Miller's
//! stability, and Coriolis from the Earth's rotation. Not modelled: aerodynamic jump,
//! wind that changes along the path.

pub mod angle;
pub mod drag;
pub mod solver;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use angle::AngleUnit;
pub use drag::{DragCurve, DragModel};
use solver::{Model, Sample, at_slant, integrate, sight_coords};

macro_rules! method_base {
    () => {
        "squib-pointmass-v1: BRL G1/G7 tables (PCHIP) scaled by BC; RK4 at 0.5 ms with a half-step check; \
         firing-point air as measured, ISA lapse along the path; level zero; constant horizontal wind"
    };
}

/// Recorded with every result: the method and its version. This is the method with
/// neither optional term; `method` names the terms a result used.
pub const METHOD: &str = concat!(method_base!(), "; no spin drift, Coriolis or aerodynamic jump");
const SPIN_METHOD: &str = "spin drift: Litz 1.25 (SG + 1.2) t^1.83 in, Miller SG with velocity and density corrections";
const CORIOLIS_METHOD: &str = "Coriolis: -2 Omega x v integrated, not in the zero";
/// Longest method string (storage keeps up to 400 characters).
pub const MAX_METHOD_CHARS: usize = 400;

/// The method string for a shot, naming the optional terms it used.
pub fn method(spin: bool, coriolis: bool) -> String {
    match (spin, coriolis) {
        (false, false) => METHOD.into(),
        (true, false) => format!("{}; {SPIN_METHOD}; no Coriolis or aerodynamic jump", method_base!()),
        (false, true) => format!("{}; {CORIOLIS_METHOD}; no spin drift or aerodynamic jump", method_base!()),
        (true, true) => format!("{}; {SPIN_METHOD}; {CORIOLIS_METHOD}; no aerodynamic jump", method_base!()),
    }
}

/// Earth's rotation rate, rad/s (sidereal).
pub const EARTH_RATE: f64 = 7.292_115_9e-5;
/// Below this Miller stability the bullet is not gyroscopically stable, and Litz's
/// approximation does not apply.
pub const MIN_STABILITY: f64 = 1.0;

pub const STEP_S: f64 = 0.0005;
/// Half-step results must agree within this, metres of drop or windage at every range.
pub const CONVERGENCE_M: f64 = 0.001;
/// Below this Mach, G1/G7 drag scaled by one BC is less reliable: points are flagged.
pub const TRANSONIC_MACH: f64 = 1.2;
pub const MAX_POINTS: usize = 200;
pub const MAX_RANGE_M: f64 = 3000.0;
const MAX_TIME_S: f64 = 30.0;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum BallisticsError {
    #[error("{0}")]
    Input(String),
    #[error("outside the model: {0}")]
    OutOfModel(String),
    #[error("could not find a zero: {0}")]
    NoZero(String),
    #[error("the integration did not converge (step check differed by {0:.4} m)")]
    NotConverged(f64),
}

pub type Result<T> = std::result::Result<T, BallisticsError>;

/// A cartridge and rifle: what the shooter enters, in SI units.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Load {
    pub model: DragModel,
    /// Ballistic coefficient for `model`, lb/in² (as published).
    pub bc_lb_in2: f64,
    pub mass_kg: f64,
    pub muzzle_velocity_mps: f64,
    /// Centre of the sight above the centre of the bore.
    pub sight_height_m: f64,
    /// Distance along the line of sight at which the bullet crosses it, zeroed level.
    pub zero_range_m: f64,
}

/// Air at the firing point.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Atmosphere {
    pub density_kg_m3: f64,
    pub temperature_k: f64,
}

impl Atmosphere {
    /// From station pressure (Pa, as measured, not corrected to sea level), temperature
    /// (C) and relative humidity (0 to 1), with the same moist-air formula the
    /// conditions screen uses.
    pub fn from_station(pressure_pa: f64, temperature_c: f64, relative_humidity: f64) -> Self {
        let t_k = temperature_c + 273.15;
        Atmosphere {
            density_kg_m3: squib_environment::derived::moist_air_density(pressure_pa, t_k, relative_humidity),
            temperature_k: t_k,
        }
    }
}

/// Constant horizontal wind. `from_deg` is where it blows from, clockwise from the
/// target: 0 a headwind, 90 from the right, 180 a tailwind, 270 from the left.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Wind {
    pub speed_mps: f64,
    pub from_deg: f64,
}

impl Wind {
    /// Air velocity in the solver frame (x downrange, z right).
    pub fn vector(&self) -> [f64; 3] {
        let a = self.from_deg.to_radians();
        [-self.speed_mps * a.cos(), 0.0, -self.speed_mps * a.sin()]
    }
}

/// The bullet and barrel, for spin drift.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Spin {
    /// Length of one turn of the rifling; negative for a left-hand twist.
    pub twist_m: f64,
    pub diameter_m: f64,
    pub length_m: f64,
}

/// Where and which way the shot is fired, for Coriolis.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Earth {
    /// Positive north of the equator.
    pub latitude_deg: f64,
    /// Direction of the shot, clockwise from true north.
    pub azimuth_deg: f64,
}

impl Earth {
    /// The Earth's rotation in the solver frame (x downrange, y up, z right), rad/s.
    pub fn omega(&self) -> [f64; 3] {
        let (l, a) = (self.latitude_deg.to_radians(), self.azimuth_deg.to_radians());
        [EARTH_RATE * l.cos() * a.cos(), EARTH_RATE * l.sin(), -EARTH_RATE * l.cos() * a.sin()]
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shot {
    pub load: Load,
    pub air: Atmosphere,
    /// Air the zero was made in; `None` means the same air as the shot.
    pub zero_air: Option<Atmosphere>,
    pub wind: Option<Wind>,
    /// Line of sight above (positive) or below the horizontal, degrees.
    pub look_angle_deg: f64,
    /// Spin drift, when set. Absent in cards saved before plan 6.5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spin: Option<Spin>,
    /// Coriolis, when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coriolis: Option<Earth>,
}

/// The solution at one distance along the line of sight.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RangePoint {
    pub range_m: f64,
    /// Bullet above (positive) or below the line of sight, perpendicular to it.
    pub drop_m: f64,
    /// Bullet right (positive) or left of the line of sight, spin drift included.
    pub windage_m: f64,
    /// The part of `windage_m` that is spin drift.
    #[serde(default)]
    pub spin_drift_m: f64,
    /// Elevation correction to hit: positive means dial or hold up.
    pub elevation_mil: f64,
    pub elevation_moa: f64,
    /// Windage correction to hit: positive means dial or hold right.
    pub windage_mil: f64,
    pub windage_moa: f64,
    pub velocity_mps: f64,
    pub mach: f64,
    pub time_s: f64,
    pub energy_j: f64,
    /// Below `TRANSONIC_MACH`: outside the best-supported part of the model.
    pub transonic: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Solution {
    pub method: String,
    /// Bore above the line of sight at the muzzle, radians, from the level zero.
    pub zero_elevation_rad: f64,
    pub points: Vec<RangePoint>,
    /// Requested distances the bullet did not reach (it slowed too much or fell).
    pub unreached_m: Vec<f64>,
    /// Largest difference between the full-step and half-step solutions, metres.
    pub convergence_m: f64,
    /// Miller stability factor, when spin drift was asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stability: Option<f64>,
}

fn check(ok: bool, what: &str) -> Result<()> {
    if ok { Ok(()) } else { Err(BallisticsError::Input(what.into())) }
}

/// Whether a load is inside the solver's accepted ranges.
pub fn validate_load(l: &Load) -> Result<()> {
    let finite = |x: f64| x.is_finite();
    check(finite(l.bc_lb_in2) && (0.02..=2.0).contains(&l.bc_lb_in2), "ballistic coefficient must be 0.02 to 2.0 lb/in²")?;
    check(finite(l.mass_kg) && (0.000_1..=0.2).contains(&l.mass_kg), "bullet weight must be 0.1 to 200 g")?;
    check(
        finite(l.muzzle_velocity_mps) && (60.0..=1600.0).contains(&l.muzzle_velocity_mps),
        "muzzle velocity must be 60 to 1600 m/s",
    )?;
    check(finite(l.sight_height_m) && (0.0..=0.3).contains(&l.sight_height_m), "sight height must be 0 to 30 cm")?;
    check(finite(l.zero_range_m) && (5.0..=1000.0).contains(&l.zero_range_m), "zero range must be 5 to 1000 m")?;
    Ok(())
}

pub fn validate(shot: &Shot, ranges_m: &[f64]) -> Result<()> {
    let finite = |x: f64| x.is_finite();
    validate_load(&shot.load)?;
    for a in std::iter::once(&shot.air).chain(shot.zero_air.as_ref()) {
        check(finite(a.density_kg_m3) && (0.4..=1.6).contains(&a.density_kg_m3), "air density must be 0.4 to 1.6 kg/m³")?;
        check(finite(a.temperature_k) && (220.0..=330.0).contains(&a.temperature_k), "temperature must be -53 to 57 °C")?;
    }
    if let Some(w) = shot.wind {
        check(finite(w.speed_mps) && (0.0..=40.0).contains(&w.speed_mps), "wind must be 0 to 40 m/s")?;
        check(finite(w.from_deg), "wind direction must be a number")?;
    }
    check(finite(shot.look_angle_deg) && shot.look_angle_deg.abs() <= 60.0, "shot angle must be within 60° of level")?;
    if let Some(s) = shot.spin {
        check(finite(s.twist_m) && (0.1..=1.0).contains(&s.twist_m.abs()), "twist must be one turn in 10 to 100 cm")?;
        check(finite(s.diameter_m) && (0.002..=0.03).contains(&s.diameter_m), "bullet diameter must be 2 to 30 mm")?;
        check(finite(s.length_m) && (0.005..=0.08).contains(&s.length_m), "bullet length must be 5 to 80 mm")?;
        check(s.length_m > s.diameter_m, "bullet length must be more than its diameter")?;
    }
    if let Some(e) = shot.coriolis {
        check(finite(e.latitude_deg) && e.latitude_deg.abs() <= 90.0, "latitude must be -90 to 90°")?;
        check(finite(e.azimuth_deg) && (0.0..360.0).contains(&e.azimuth_deg), "shot direction must be 0 to 359°")?;
    }
    check(!ranges_m.is_empty() && ranges_m.len() <= MAX_POINTS, "choose 1 to 200 distances")?;
    check(ranges_m.iter().all(|r| r.is_finite() && (0.0..=MAX_RANGE_M).contains(r)), "distances must be 0 to 3000 m")?;
    Ok(())
}

/// Bore elevation (radians, relative to the line of sight) that puts a level shot on
/// the line of sight at the zero range, in still air.
pub fn zero_elevation(load: &Load, air: Atmosphere, dt: f64) -> Result<f64> {
    let curve = DragCurve::new(load.model);
    let m = Model::new(&curve, load, air, None, None);
    let height_at_zero = |e: f64| -> Result<f64> {
        let path = integrate(&m, load.muzzle_velocity_mps, load.sight_height_m, 0.0, e, load.zero_range_m + 1.0, dt, MAX_TIME_S)?;
        let s = at_slant(&path, 0.0, load.zero_range_m)
            .ok_or_else(|| BallisticsError::NoZero(format!("the bullet does not reach {:.0} m", load.zero_range_m)))?;
        Ok(sight_coords(s.pos, 0.0).1)
    };
    // Secant iteration from a vacuum-flight first guess; height at the zero range rises
    // monotonically with elevation over this range of angles.
    let tof = load.zero_range_m / load.muzzle_velocity_mps;
    let mut e0 = (load.sight_height_m + 0.5 * solver::G * tof * tof) / load.zero_range_m;
    let mut e1 = e0 * 1.1 + 1e-4;
    let (mut h0, mut h1) = (height_at_zero(e0)?, height_at_zero(e1)?);
    for _ in 0..40 {
        if h1.abs() < 1e-7 {
            return Ok(e1);
        }
        if h1 == h0 {
            break;
        }
        let e2 = (e1 - h1 * (e1 - e0) / (h1 - h0)).clamp(-0.2, 0.3);
        (e0, h0) = (e1, h1);
        e1 = e2;
        h1 = height_at_zero(e1)?;
    }
    if h1.abs() < 1e-5 {
        Ok(e1)
    } else {
        Err(BallisticsError::NoZero(format!("closest miss {:.3} m at {:.0} m", h1, load.zero_range_m)))
    }
}

/// Dense path for a shot at a given bore elevation (relative to the line of sight).
pub fn path(shot: &Shot, elevation_rad: f64, max_slant_m: f64, dt: f64) -> Result<Vec<Sample>> {
    let curve = DragCurve::new(shot.load.model);
    let m = Model::new(&curve, &shot.load, shot.air, shot.wind, shot.coriolis);
    integrate(
        &m,
        shot.load.muzzle_velocity_mps,
        shot.load.sight_height_m,
        shot.look_angle_deg.to_radians(),
        elevation_rad,
        max_slant_m,
        dt,
        MAX_TIME_S,
    )
}

/// Miller's stability factor for a spin-stabilised bullet, with his velocity correction
/// and his atmosphere correction (temperature and pressure relative to 59 F and 29.92
/// inHg), written here as the equivalent ratio of dry-air densities.
pub fn miller_stability(load: &Load, spin: &Spin, air: Atmosphere) -> f64 {
    const IN: f64 = 0.0254;
    let grains = load.mass_kg / 0.000_064_798_91;
    let d = spin.diameter_m / IN;
    let twist_cal = spin.twist_m.abs() / spin.diameter_m;
    let len_cal = spin.length_m / spin.diameter_m;
    let sg = 30.0 * grains / (twist_cal.powi(2) * d.powi(3) * len_cal * (1.0 + len_cal.powi(2)));
    let fv = (load.muzzle_velocity_mps / 0.3048 / 2800.0).cbrt();
    // Dry air at 59 F (15 C) and 29.92 inHg.
    let rho_std = 29.92 * 3386.389 / (squib_environment::derived::ISA_R * 288.15);
    sg * fv * rho_std / air.density_kg_m3
}

/// Litz's spin drift, metres, after `t` seconds: to the right for a right-hand twist.
pub fn spin_drift(stability: f64, twist_m: f64, t: f64) -> f64 {
    twist_m.signum() * 1.25 * (stability + 1.2) * t.powf(1.83) * 0.0254
}

fn points_for(shot: &Shot, zero: f64, ranges: &[f64], dt: f64) -> Result<(Vec<RangePoint>, Vec<f64>)> {
    let max = ranges.iter().copied().fold(0.0, f64::max);
    let look = shot.look_angle_deg.to_radians();
    let p = path(shot, zero, max + 1.0, dt)?;
    let spin = shot.spin.map(|s| (miller_stability(&shot.load, &s, shot.air), s.twist_m));
    let mut points = Vec::new();
    let mut unreached = Vec::new();
    for &r in ranges {
        let Some(s) = at_slant(&p, look, r) else {
            unreached.push(r);
            continue;
        };
        let (_, drop) = sight_coords(s.pos, look);
        let spin_drift_m = spin.map(|(sg, twist)| spin_drift(sg, twist, s.t)).unwrap_or(0.0);
        let windage = s.pos[2] + spin_drift_m;
        let v = s.speed();
        // Angular corrections subtend the miss at the distance; none at the muzzle.
        let ang = |miss: f64| if r > 0.0 { -(miss / r).atan() } else { 0.0 };
        let (el, wd) = (ang(drop), ang(windage));
        points.push(RangePoint {
            range_m: r,
            drop_m: drop,
            windage_m: windage,
            spin_drift_m,
            elevation_mil: AngleUnit::Mil.from_rad(el),
            elevation_moa: AngleUnit::Moa.from_rad(el),
            windage_mil: AngleUnit::Mil.from_rad(wd),
            windage_moa: AngleUnit::Moa.from_rad(wd),
            velocity_mps: v,
            mach: s.mach,
            time_s: s.t,
            energy_j: 0.5 * shot.load.mass_kg * v * v,
            transonic: s.mach < TRANSONIC_MACH,
        });
    }
    Ok((points, unreached))
}

/// Solve a range card: the level zero, then the shot at each distance (along the line
/// of sight). The whole solution is repeated at half the step; if any drop or windage
/// differs by more than `CONVERGENCE_M`, it is refused. The zero never includes the
/// optional terms: it is the rifle's level zero as made.
pub fn solve(shot: &Shot, ranges_m: &[f64]) -> Result<Solution> {
    validate(shot, ranges_m)?;
    let stability = shot.spin.map(|s| miller_stability(&shot.load, &s, shot.air));
    if let Some(sg) = stability
        && sg < MIN_STABILITY
    {
        return Err(BallisticsError::OutOfModel(format!(
            "the bullet is not stable at this twist (Miller stability {sg:.2}), so spin drift does not apply"
        )));
    }
    let zero_air = shot.zero_air.unwrap_or(shot.air);
    let zero = zero_elevation(&shot.load, zero_air, STEP_S)?;
    let (points, unreached) = points_for(shot, zero, ranges_m, STEP_S)?;
    let zero_half = zero_elevation(&shot.load, zero_air, STEP_S / 2.0)?;
    let (half, _) = points_for(shot, zero_half, ranges_m, STEP_S / 2.0)?;
    let convergence = points
        .iter()
        .zip(&half)
        .map(|(a, b)| (a.drop_m - b.drop_m).abs().max((a.windage_m - b.windage_m).abs()))
        .fold(0.0, f64::max);
    if convergence > CONVERGENCE_M {
        return Err(BallisticsError::NotConverged(convergence));
    }
    Ok(Solution {
        method: method(shot.spin.is_some(), shot.coriolis.is_some()),
        zero_elevation_rad: zero,
        points,
        unreached_m: unreached,
        convergence_m: convergence,
        stability,
    })
}
