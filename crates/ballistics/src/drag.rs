//! Standard drag functions: drag coefficient against Mach number for the G1 and G7
//! reference projectiles, and their interpolation.
//!
//! Values are the US Army Ballistic Research Laboratory tables (G1: BRL N-I-50, 1942,
//! flat base, 2-calibre ogive; G7: BRL N-I-119 Rev. 2, 1952, 7.5 degree boat tail,
//! 10-calibre tangent ogive), US Government work in the public domain. Two independent
//! transcriptions agree on every value (see `tests/fixtures/brl_drag_tables.json` and
//! ADR-061); a test checks this table against one of them.
//!
//! Between table points the curve is a monotone piecewise cubic (PCHIP, Fritsch and
//! Carlson 1980): C1-continuous, with no overshoot through the transonic rise.

use serde::{Deserialize, Serialize};

/// The reference projectile a ballistic coefficient was measured against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DragModel {
    G1,
    G7,
}

impl DragModel {
    pub fn table(self) -> &'static [(f64, f64)] {
        match self {
            DragModel::G1 => G1,
            DragModel::G7 => G7,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            DragModel::G1 => "G1",
            DragModel::G7 => "G7",
        }
    }
}

/// Highest Mach number in the tables; faster flight is outside the model.
pub const MAX_MACH: f64 = 5.0;

/// A drag curve prepared for evaluation: knots and the slope at each knot.
#[derive(Debug, Clone)]
pub struct DragCurve {
    mach: Vec<f64>,
    cd: Vec<f64>,
    slope: Vec<f64>,
}

impl DragCurve {
    pub fn new(model: DragModel) -> Self {
        let (mach, cd): (Vec<f64>, Vec<f64>) = model.table().iter().copied().unzip();
        let slope = pchip_slopes(&mach, &cd);
        DragCurve { mach, cd, slope }
    }

    /// Drag coefficient at `m` (clamped to the table's range; callers reject Mach
    /// numbers above `MAX_MACH` before integrating).
    pub fn cd(&self, m: f64) -> f64 {
        let n = self.mach.len();
        let m = m.clamp(self.mach[0], self.mach[n - 1]);
        // Last knot not above m, so m lies in [mach[i], mach[i + 1]].
        let i = self.mach.partition_point(|&x| x <= m).saturating_sub(1).min(n - 2);
        let (x0, x1) = (self.mach[i], self.mach[i + 1]);
        let h = x1 - x0;
        let t = (m - x0) / h;
        let (y0, y1, d0, d1) = (self.cd[i], self.cd[i + 1], self.slope[i], self.slope[i + 1]);
        // Cubic Hermite basis.
        let t2 = t * t;
        let t3 = t2 * t;
        (2.0 * t3 - 3.0 * t2 + 1.0) * y0 + (t3 - 2.0 * t2 + t) * h * d0 + (-2.0 * t3 + 3.0 * t2) * y1 + (t3 - t2) * h * d1
    }
}

/// Knot slopes for a shape-preserving piecewise cubic Hermite interpolant: weighted
/// harmonic means of neighbouring secants at interior knots (zero where the data turn),
/// and a one-sided three-point estimate, limited to keep its shape, at each end.
fn pchip_slopes(x: &[f64], y: &[f64]) -> Vec<f64> {
    let n = x.len();
    let h: Vec<f64> = x.windows(2).map(|w| w[1] - w[0]).collect();
    let delta: Vec<f64> = (0..n - 1).map(|k| (y[k + 1] - y[k]) / h[k]).collect();
    let mut d = vec![0.0; n];
    for k in 1..n - 1 {
        let (a, b) = (delta[k - 1], delta[k]);
        if a == 0.0 || b == 0.0 || a.signum() != b.signum() {
            continue;
        }
        let w1 = 2.0 * h[k] + h[k - 1];
        let w2 = h[k] + 2.0 * h[k - 1];
        d[k] = (w1 + w2) / (w1 / a + w2 / b);
    }
    let end = |h0: f64, h1: f64, d0: f64, d1: f64| {
        let s = ((2.0 * h0 + h1) * d0 - h0 * d1) / (h0 + h1);
        if s.signum() != d0.signum() {
            0.0
        } else if d0.signum() != d1.signum() && s.abs() > 3.0 * d0.abs() {
            3.0 * d0
        } else {
            s
        }
    };
    d[0] = end(h[0], h[1], delta[0], delta[1]);
    d[n - 1] = end(h[n - 2], h[n - 3], delta[n - 2], delta[n - 3]);
    d
}

// Measured data: values that happen to sit near mathematical constants are not them.
#[allow(clippy::approx_constant)]
const G1: &[(f64, f64)] = &[
    (0.000, 0.2629),
    (0.050, 0.2558),
    (0.100, 0.2487),
    (0.150, 0.2413),
    (0.200, 0.2344),
    (0.250, 0.2278),
    (0.300, 0.2214),
    (0.350, 0.2155),
    (0.400, 0.2104),
    (0.450, 0.2061),
    (0.500, 0.2032),
    (0.550, 0.2020),
    (0.600, 0.2034),
    (0.700, 0.2165),
    (0.725, 0.2230),
    (0.750, 0.2313),
    (0.775, 0.2417),
    (0.800, 0.2546),
    (0.825, 0.2706),
    (0.850, 0.2901),
    (0.875, 0.3136),
    (0.900, 0.3415),
    (0.925, 0.3734),
    (0.950, 0.4084),
    (0.975, 0.4448),
    (1.000, 0.4805),
    (1.025, 0.5136),
    (1.050, 0.5427),
    (1.075, 0.5677),
    (1.100, 0.5883),
    (1.125, 0.6053),
    (1.150, 0.6191),
    (1.200, 0.6393),
    (1.250, 0.6518),
    (1.300, 0.6589),
    (1.350, 0.6621),
    (1.400, 0.6625),
    (1.450, 0.6607),
    (1.500, 0.6573),
    (1.550, 0.6528),
    (1.600, 0.6474),
    (1.650, 0.6413),
    (1.700, 0.6347),
    (1.750, 0.6280),
    (1.800, 0.6210),
    (1.850, 0.6141),
    (1.900, 0.6072),
    (1.950, 0.6003),
    (2.000, 0.5934),
    (2.050, 0.5867),
    (2.100, 0.5804),
    (2.150, 0.5743),
    (2.200, 0.5685),
    (2.250, 0.5630),
    (2.300, 0.5577),
    (2.350, 0.5527),
    (2.400, 0.5481),
    (2.450, 0.5438),
    (2.500, 0.5397),
    (2.600, 0.5325),
    (2.700, 0.5264),
    (2.800, 0.5211),
    (2.900, 0.5168),
    (3.000, 0.5133),
    (3.100, 0.5105),
    (3.200, 0.5084),
    (3.300, 0.5067),
    (3.400, 0.5054),
    (3.500, 0.5040),
    (3.600, 0.5030),
    (3.700, 0.5022),
    (3.800, 0.5016),
    (3.900, 0.5010),
    (4.000, 0.5006),
    (4.200, 0.4998),
    (4.400, 0.4995),
    (4.600, 0.4992),
    (4.800, 0.4990),
    (5.000, 0.4988),
];

#[allow(clippy::approx_constant)]
const G7: &[(f64, f64)] = &[
    (0.000, 0.1198),
    (0.050, 0.1197),
    (0.100, 0.1196),
    (0.150, 0.1194),
    (0.200, 0.1193),
    (0.250, 0.1194),
    (0.300, 0.1194),
    (0.350, 0.1194),
    (0.400, 0.1193),
    (0.450, 0.1193),
    (0.500, 0.1194),
    (0.550, 0.1193),
    (0.600, 0.1194),
    (0.650, 0.1197),
    (0.700, 0.1202),
    (0.725, 0.1207),
    (0.750, 0.1215),
    (0.775, 0.1226),
    (0.800, 0.1242),
    (0.825, 0.1266),
    (0.850, 0.1306),
    (0.875, 0.1368),
    (0.900, 0.1464),
    (0.925, 0.1660),
    (0.950, 0.2054),
    (0.975, 0.2993),
    (1.000, 0.3803),
    (1.025, 0.4015),
    (1.050, 0.4043),
    (1.075, 0.4034),
    (1.100, 0.4014),
    (1.125, 0.3987),
    (1.150, 0.3955),
    (1.200, 0.3884),
    (1.250, 0.3810),
    (1.300, 0.3732),
    (1.350, 0.3657),
    (1.400, 0.3580),
    (1.500, 0.3440),
    (1.550, 0.3376),
    (1.600, 0.3315),
    (1.650, 0.3260),
    (1.700, 0.3209),
    (1.750, 0.3160),
    (1.800, 0.3117),
    (1.850, 0.3078),
    (1.900, 0.3042),
    (1.950, 0.3010),
    (2.000, 0.2980),
    (2.050, 0.2951),
    (2.100, 0.2922),
    (2.150, 0.2892),
    (2.200, 0.2864),
    (2.250, 0.2835),
    (2.300, 0.2807),
    (2.350, 0.2779),
    (2.400, 0.2752),
    (2.450, 0.2725),
    (2.500, 0.2697),
    (2.550, 0.2670),
    (2.600, 0.2643),
    (2.650, 0.2615),
    (2.700, 0.2588),
    (2.750, 0.2561),
    (2.800, 0.2533),
    (2.850, 0.2506),
    (2.900, 0.2479),
    (2.950, 0.2451),
    (3.000, 0.2424),
    (3.100, 0.2368),
    (3.200, 0.2313),
    (3.300, 0.2258),
    (3.400, 0.2205),
    (3.500, 0.2154),
    (3.600, 0.2106),
    (3.700, 0.2060),
    (3.800, 0.2017),
    (3.900, 0.1975),
    (4.000, 0.1935),
    (4.200, 0.1861),
    (4.400, 0.1793),
    (4.600, 0.1730),
    (4.800, 0.1672),
    (5.000, 0.1618),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_through_every_table_point_and_is_monotone_between_them() {
        for model in [DragModel::G1, DragModel::G7] {
            let c = DragCurve::new(model);
            for &(m, cd) in model.table() {
                assert!((c.cd(m) - cd).abs() < 1e-12, "{model:?} at Mach {m}");
            }
            // No overshoot: between two knots the curve stays within their values.
            for w in model.table().windows(2) {
                let (lo, hi) = (w[0].1.min(w[1].1), w[0].1.max(w[1].1));
                for k in 1..20 {
                    let m = w[0].0 + (w[1].0 - w[0].0) * k as f64 / 20.0;
                    let v = c.cd(m);
                    assert!(v >= lo - 1e-12 && v <= hi + 1e-12, "{model:?} overshoot at Mach {m}: {v}");
                }
            }
        }
    }

    #[test]
    fn tables_are_well_formed() {
        for model in [DragModel::G1, DragModel::G7] {
            let t = model.table();
            assert_eq!(t[0].0, 0.0);
            assert_eq!(t[t.len() - 1].0, MAX_MACH);
            assert!(t.windows(2).all(|w| w[1].0 > w[0].0), "{model:?} Mach strictly increasing");
            assert!(t.iter().all(|&(_, cd)| cd > 0.05 && cd < 1.0));
        }
        assert_eq!((DragModel::G1.table().len(), DragModel::G7.table().len()), (79, 84));
    }
}
