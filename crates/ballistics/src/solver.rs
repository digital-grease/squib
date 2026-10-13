//! Point-mass trajectory integration.
//!
//! Frame: origin at the sight (the shooter's eye line), x horizontal along the azimuth of
//! the line of sight, y up, z to the right. The line of sight rises at the look angle
//! `theta`; the bore starts one sight height below it, perpendicular to it, and leaves at
//! `theta + elevation`. Forces: gravity and drag on the velocity relative to the air.

use squib_environment::derived::{ISA_G0, ISA_LAPSE, ISA_R};

use crate::drag::DragCurve;
use crate::{Atmosphere, BallisticsError, Earth, Load, Wind};

pub const G: f64 = ISA_G0;
/// kg/m² in one lb/in², the unit published ballistic coefficients use.
pub const LB_PER_IN2: f64 = 0.453_592_37 / (0.0254 * 0.0254);
/// Speed of sound in dry air per square-root kelvin: sqrt(1.4 * R).
pub fn speed_of_sound(t_k: f64) -> f64 {
    (1.4 * ISA_R * t_k).sqrt()
}

/// One integration step: time, position and velocity (m, m/s), and Mach.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub t: f64,
    pub pos: [f64; 3],
    pub vel: [f64; 3],
    pub mach: f64,
}

impl Sample {
    pub fn speed(&self) -> f64 {
        norm(self.vel)
    }
}

pub(crate) struct Model<'a> {
    pub curve: &'a DragCurve,
    /// pi / (8 * BC): multiplies rho * Cd * v * v_rel to give the drag deceleration.
    pub k: f64,
    pub air: Atmosphere,
    pub wind: [f64; 3],
    /// Earth's rotation in this frame, when Coriolis is on.
    pub omega: Option<[f64; 3]>,
}

impl Model<'_> {
    pub fn new<'a>(curve: &'a DragCurve, load: &Load, air: Atmosphere, wind: Option<Wind>, earth: Option<Earth>) -> Model<'a> {
        Model {
            curve,
            k: std::f64::consts::PI / (8.0 * load.bc_lb_in2 * LB_PER_IN2),
            air,
            wind: wind.map(|w| w.vector()).unwrap_or([0.0; 3]),
            omega: earth.map(|e| e.omega()),
        }
    }

    /// Air density and speed of sound at height `y` above the firing point: the measured
    /// values at the firing point, carried up or down the ISA lapse rate. This is the
    /// only height adjustment; the firing-point values are used as measured.
    fn air_at(&self, y: f64) -> (f64, f64) {
        let t0 = self.air.temperature_k;
        let t = (t0 - ISA_LAPSE * y).max(150.0);
        let rho = self.air.density_kg_m3 * (t / t0).powf(G / (ISA_R * ISA_LAPSE) - 1.0);
        (rho, speed_of_sound(t))
    }

    fn accel(&self, pos: [f64; 3], vel: [f64; 3]) -> ([f64; 3], f64) {
        let (rho, c) = self.air_at(pos[1]);
        let rel = sub(vel, self.wind);
        let speed = norm(rel);
        let mach = speed / c;
        let f = self.k * rho * self.curve.cd(mach) * speed;
        let mut a = [-f * rel[0], -G - f * rel[1], -f * rel[2]];
        if let Some(w) = self.omega {
            // Coriolis, on the velocity over the ground: -2 Omega x v.
            let c = cross(w, vel);
            a = [a[0] - 2.0 * c[0], a[1] - 2.0 * c[1], a[2] - 2.0 * c[2]];
        }
        (a, mach)
    }

    pub fn mach(&self, s: &Sample) -> f64 {
        let (_, c) = self.air_at(s.pos[1]);
        norm(sub(s.vel, self.wind)) / c
    }
}

/// Integrate from the muzzle with classical fourth-order Runge-Kutta at a fixed step,
/// until the bullet passes `max_slant_m` along the line of sight, slows below
/// `min_speed`, or `max_time` elapses.
#[allow(clippy::too_many_arguments)]
pub(crate) fn integrate(
    m: &Model<'_>,
    muzzle_velocity: f64,
    sight_height: f64,
    look: f64,
    elevation: f64,
    max_slant_m: f64,
    dt: f64,
    max_time: f64,
) -> Result<Vec<Sample>, BallisticsError> {
    let launch = look + elevation;
    let pos = [sight_height * look.sin(), -sight_height * look.cos(), 0.0];
    let vel = [muzzle_velocity * launch.cos(), muzzle_velocity * launch.sin(), 0.0];
    let mut s = Sample { t: 0.0, pos, vel, mach: 0.0 };
    s.mach = m.mach(&s);
    if s.mach > crate::drag::MAX_MACH {
        return Err(BallisticsError::OutOfModel(format!("Mach {:.2} at the muzzle is above the drag tables", s.mach)));
    }
    let (cl, sl) = (look.cos(), look.sin());
    let mut out = vec![s];
    let min_speed = 0.2 * speed_of_sound(m.air.temperature_k);
    while s.t < max_time {
        let f = |p: [f64; 3], v: [f64; 3]| m.accel(p, v).0;
        let (p, v) = (s.pos, s.vel);
        let a1 = f(p, v);
        let (p2, v2) = (add(p, scale(v, dt / 2.0)), add(v, scale(a1, dt / 2.0)));
        let a2 = f(p2, v2);
        let (p3, v3) = (add(p, scale(v2, dt / 2.0)), add(v, scale(a2, dt / 2.0)));
        let a3 = f(p3, v3);
        let (p4, v4) = (add(p, scale(v3, dt)), add(v, scale(a3, dt)));
        let a4 = f(p4, v4);
        let pos = add(p, scale(add(add(v, scale(add(v2, v3), 2.0)), v4), dt / 6.0));
        let vel = add(v, scale(add(add(a1, scale(add(a2, a3), 2.0)), a4), dt / 6.0));
        s = Sample { t: s.t + dt, pos, vel, mach: 0.0 };
        s.mach = m.mach(&s);
        out.push(s);
        let slant = s.pos[0] * cl + s.pos[1] * sl;
        if slant > max_slant_m || s.speed() < min_speed {
            break;
        }
    }
    Ok(out)
}

/// Line-of-sight coordinates of a position: distance along the sight line and height
/// above it (perpendicular, positive up).
pub fn sight_coords(pos: [f64; 3], look: f64) -> (f64, f64) {
    (pos[0] * look.cos() + pos[1] * look.sin(), -pos[0] * look.sin() + pos[1] * look.cos())
}

/// The sample state where the path crosses `slant` along the line of sight, linearly
/// interpolated between the two integration steps either side (steps are a fraction of
/// a metre apart, so the interpolation error is far below a millimetre).
pub fn at_slant(path: &[Sample], look: f64, slant: f64) -> Option<Sample> {
    let sc = |s: &Sample| sight_coords(s.pos, look).0;
    if slant <= sc(&path[0]) {
        return Some(path[0]);
    }
    let i = path.windows(2).position(|w| sc(&w[0]) <= slant && sc(&w[1]) >= slant)?;
    let (a, b) = (&path[i], &path[i + 1]);
    let f = (slant - sc(a)) / (sc(b) - sc(a));
    let lerp3 = |x: [f64; 3], y: [f64; 3]| [x[0] + f * (y[0] - x[0]), x[1] + f * (y[1] - x[1]), x[2] + f * (y[2] - x[2])];
    Some(Sample {
        t: a.t + f * (b.t - a.t),
        pos: lerp3(a.pos, b.pos),
        vel: lerp3(a.vel, b.vel),
        mach: a.mach + f * (b.mach - a.mach),
    })
}

/// The state at time `t`, by cubic Hermite interpolation between steps (positions with
/// their velocities as derivatives). Used by the validation tests, which compare with a
/// reference solver at equal flight times.
pub fn at_time(path: &[Sample], t: f64) -> Option<Sample> {
    let i = path.windows(2).position(|w| w[0].t <= t && w[1].t >= t)?;
    let (a, b) = (&path[i], &path[i + 1]);
    let h = b.t - a.t;
    let u = (t - a.t) / h;
    let (u2, u3) = (u * u, u * u * u);
    let (h00, h10, h01, h11) = (2.0 * u3 - 3.0 * u2 + 1.0, u3 - 2.0 * u2 + u, -2.0 * u3 + 3.0 * u2, u3 - u2);
    let mut pos = [0.0; 3];
    let mut vel = [0.0; 3];
    for k in 0..3 {
        pos[k] = h00 * a.pos[k] + h10 * h * a.vel[k] + h01 * b.pos[k] + h11 * h * b.vel[k];
        vel[k] = a.vel[k] + u * (b.vel[k] - a.vel[k]);
    }
    Some(Sample { t, pos, vel, mach: a.mach + u * (b.mach - a.mach) })
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: [f64; 3], k: f64) -> [f64; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(a: [f64; 3]) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}
