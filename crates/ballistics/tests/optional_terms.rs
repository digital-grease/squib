//! Validation of the optional terms, spin drift and Coriolis (plan 6.5, ADR-064). Both
//! are off unless a shot asks for them; this suite must pass before the app offers them.

use serde_json::Value;
use squib_ballistics::solver;
use squib_ballistics::*;

fn fixture(name: &str) -> Value {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn f(v: &Value, k: &str) -> f64 {
    v[k].as_f64().unwrap_or_else(|| panic!("{k}"))
}

/// The case's shot with and without its optional terms.
fn shots(i: &Value) -> (Shot, Shot) {
    let model = if i["model"] == "G1" { DragModel::G1 } else { DragModel::G7 };
    let base = Shot {
        load: Load {
            model,
            bc_lb_in2: f(i, "bc_lb_in2"),
            mass_kg: f(i, "mass_kg"),
            muzzle_velocity_mps: f(i, "muzzle_velocity_mps"),
            sight_height_m: f(i, "sight_height_m"),
            zero_range_m: f(i, "zero_range_m"),
        },
        air: Atmosphere { density_kg_m3: f(i, "air_density_kg_m3"), temperature_k: f(i, "temperature_k") },
        zero_air: None,
        wind: (f(i, "wind_mps") > 0.0).then(|| Wind { speed_mps: f(i, "wind_mps"), from_deg: f(i, "wind_from_deg") }),
        look_angle_deg: f(i, "look_angle_deg"),
        spin: None,
        coriolis: None,
    };
    let full = Shot {
        spin: i["twist_m"].as_f64().map(|t| Spin { twist_m: t, diameter_m: f(i, "diameter_m"), length_m: f(i, "length_m") }),
        coriolis: i["latitude_deg"].as_f64().map(|l| Earth { latitude_deg: l, azimuth_deg: f(i, "azimuth_deg") }),
        ..base.clone()
    };
    (base, full)
}

/// Cross-solver against py-ballisticcalc 3.1.0 (`testing/ballistics/gen_pybc_optional.py`,
/// development-time reference only). Each case is solved with and without its terms at
/// the reference's distances along the line of sight, and each term's own contribution
/// (with minus without, in both solvers) is compared, so the comparison is not blurred by
/// the base solutions' own small difference (ADR-061). Tolerances, with what was observed:
/// - Miller stability 0.2 percent (observed 0.06: the reference uses 460 for 459.67 and
///   its pressure and temperature where Squib uses the equal dry-air density ratio);
/// - each term's contribution 1 mm + 1 percent of it (observed at most 0.12 mm, at
///   1372 m, where the two solvers' flight times differ by 0.5 ms);
/// - the totals as in the base comparison, 3 mm + 0.02 mrad of distance, plus 3 cm on
///   the inclined case (the sight-offset convention, ADR-061).
#[test]
fn matches_reference_solver() {
    let doc = fixture("pybc_optional.json");
    let cases = doc["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 7);
    let mut worst_term: f64 = 0.0;
    for c in cases {
        let name = c["name"].as_str().unwrap();
        let i = &c["input"];
        let (base, full) = shots(i);
        let rows = |k: &str| -> Vec<&Value> { c[k].as_array().unwrap().iter().filter(|r| f(r, "slant_m") > 1.0).collect() };
        let (ref_base, ref_full) = (rows("base"), rows("full"));
        assert_eq!(ref_base.len(), ref_full.len());
        let ranges: Vec<f64> = ref_full.iter().map(|r| f(r, "slant_m")).collect();
        let base_ranges: Vec<f64> = ref_base.iter().map(|r| f(r, "slant_m")).collect();
        let ours_base = solve(&base, &base_ranges).unwrap();
        let ours_full = solve(&full, &ranges).unwrap();
        assert_eq!(ours_full.points.len(), ranges.len(), "{name}: every distance reached");

        match (ours_full.stability, c["reference_stability"].as_f64()) {
            (Some(ours), Some(theirs)) => {
                assert!((ours / theirs - 1.0).abs() < 0.002, "{name}: stability {ours} vs {theirs}")
            }
            (None, None) => {}
            other => panic!("{name}: stability {other:?}"),
        }

        let inclined = f(i, "look_angle_deg") != 0.0;
        for ((rb, rf), (ob, of)) in ref_base.iter().zip(&ref_full).zip(ours_base.points.iter().zip(&ours_full.points)) {
            let r = of.range_m;
            let ref_dz = f(rf, "z") - f(rb, "z");
            let ref_dy = f(rf, "slant_height_m") - f(rb, "slant_height_m");
            let (dz, dy) = (of.windage_m - ob.windage_m, of.drop_m - ob.drop_m);
            for (what, ours, theirs) in [("windage", dz, ref_dz), ("height", dy, ref_dy)] {
                let tol = 0.001 + 0.01 * theirs.abs();
                let diff = (ours - theirs).abs();
                worst_term = worst_term.max(diff);
                assert!(diff < tol, "{name} at {r:.0} m: {what} from the terms {ours:.4} vs {theirs:.4}");
            }
            let total_tol = 0.003 + 0.000_02 * r + if inclined { 0.03 } else { 0.0 };
            assert!((of.windage_m - f(rf, "z")).abs() < total_tol, "{name} at {r:.0} m: windage");
            assert!((of.drop_m - f(rf, "slant_height_m")).abs() < total_tol, "{name} at {r:.0} m: height");
            if full.spin.is_some() && full.coriolis.is_none() {
                assert!((of.spin_drift_m - dz).abs() < 1e-9, "{name}: spin drift reported is the windage it adds");
            }
        }
    }
    eprintln!("optional terms: worst difference in a term's contribution {:.2} mm", worst_term * 1000.0);
}

fn level() -> Shot {
    Shot {
        load: Load {
            model: DragModel::G7,
            bc_lb_in2: 0.243,
            mass_kg: 0.011_34,
            muzzle_velocity_mps: 792.5,
            sight_height_m: 0.038,
            zero_range_m: 91.44,
        },
        air: Atmosphere { density_kg_m3: 1.225, temperature_k: 288.15 },
        zero_air: None,
        wind: None,
        look_angle_deg: 0.0,
        spin: None,
        coriolis: None,
    }
}

/// Gravity, rounded up, for bounds.
const G_LIKE: f64 = 10.0;

const SMK: Spin = Spin { twist_m: 0.254, diameter_m: 0.007_823, length_m: 0.031_5 };

/// Exact and physical checks on spin drift: Litz's formula as written, mirrored for a
/// left-hand twist, absent unless asked for, and a bullet that is not stable refused.
#[test]
fn spin_drift_behaviour() {
    let ranges = [100.0, 500.0, 1000.0];
    let plain = solve(&level(), &ranges).unwrap();
    assert!(plain.stability.is_none() && plain.points.iter().all(|p| p.spin_drift_m == 0.0));
    assert_eq!(plain.method, METHOD);

    let rh = solve(&Shot { spin: Some(SMK), ..level() }, &ranges).unwrap();
    let lh = solve(&Shot { spin: Some(Spin { twist_m: -SMK.twist_m, ..SMK }), ..level() }, &ranges).unwrap();
    let sg = rh.stability.unwrap();
    // Miller by hand for the 175 gr .308 at 2600 fps, 1 in 10: 2.40 (as py-ballisticcalc).
    assert!((sg - 2.40).abs() < 0.01, "{sg}");
    for ((p, r), l) in plain.points.iter().zip(&rh.points).zip(&lh.points) {
        let litz = 1.25 * (sg + 1.2) * r.time_s.powf(1.83) * 0.0254;
        assert!((r.spin_drift_m - litz).abs() < 1e-12);
        assert!((r.windage_m - p.windage_m - litz).abs() < 1e-9, "added to the windage");
        assert_eq!(r.drop_m, p.drop_m, "spin drift is sideways only");
        assert!((l.spin_drift_m + r.spin_drift_m).abs() < 1e-12, "left-hand twist mirrors it");
        assert!(r.windage_mil < 0.0 && l.windage_mil > 0.0, "a right-hand twist drifts right, so hold left");
    }
    assert!(rh.points[0].spin_drift_m < rh.points[1].spin_drift_m && rh.points[1].spin_drift_m < rh.points[2].spin_drift_m);
    assert!(rh.method.contains("Litz") && !rh.method.contains("no spin drift"));
    for (a, b) in [(false, false), (true, false), (false, true), (true, true)] {
        assert!(method(a, b).chars().count() <= MAX_METHOD_CHARS, "{}", method(a, b));
    }

    // Stability rises with twist rate and falls in thick air.
    let slow = solve(&Shot { spin: Some(Spin { twist_m: 0.305, ..SMK }), ..level() }, &ranges).unwrap();
    assert!(slow.stability.unwrap() < sg);
    let thick = Atmosphere { density_kg_m3: 1.35, temperature_k: 263.15 };
    assert!(miller_stability(&level().load, &SMK, thick) < sg);

    // 1 in 16 is far too slow for this bullet.
    let unstable = solve(&Shot { spin: Some(Spin { twist_m: 0.406, ..SMK }), ..level() }, &ranges);
    assert!(matches!(unstable, Err(BallisticsError::OutOfModel(ref m)) if m.contains("not stable")), "{unstable:?}");
    for bad in [
        Spin { twist_m: 0.05, ..SMK },
        Spin { twist_m: f64::NAN, ..SMK },
        Spin { diameter_m: 0.04, ..SMK },
        Spin { length_m: 0.007, ..SMK },
    ] {
        assert!(matches!(solve(&Shot { spin: Some(bad), ..level() }, &ranges), Err(BallisticsError::Input(_))), "{bad:?}");
    }
}

/// Exact: in negligible air, to first order in the Earth's rate (the next term, about
/// Omega^2 v t^3, is under 0.1 mm here), Coriolis moves the bullet by -2 Omega x P, where P is the time
/// integral of the drag-free displacement from the muzzle. This checks the frame and
/// every sign independently of the reference solver.
#[test]
fn coriolis_in_vacuum_is_exact() {
    let mut shot = level();
    shot.air.density_kg_m3 = 1e-15;
    let el = 0.002;
    for (lat, az, look) in [(45.0, 0.0, 0.0), (-30.0, 90.0, 10.0), (60.0, 225.0, -20.0), (0.0, 270.0, 0.0), (89.0, 10.0, 5.0)] {
        shot.look_angle_deg = look;
        let plain = path(&Shot { coriolis: None, ..shot.clone() }, el, 1500.0, STEP_S).unwrap();
        let earth = Earth { latitude_deg: lat, azimuth_deg: az };
        let rotating = path(&Shot { coriolis: Some(earth), ..shot.clone() }, el, 1500.0, STEP_S).unwrap();
        let w = earth.omega();
        let a = look.to_radians() + el;
        let (u, v) = (shot.load.muzzle_velocity_mps * a.cos(), shot.load.muzzle_velocity_mps * a.sin());
        for (p, r) in plain.iter().zip(&rotating).step_by(211) {
            let t = p.t;
            let big_p = [u * t * t / 2.0, v * t * t / 2.0 - solver::G * t * t * t / 6.0, 0.0];
            let c = [w[1] * big_p[2] - w[2] * big_p[1], w[2] * big_p[0] - w[0] * big_p[2], w[0] * big_p[1] - w[1] * big_p[0]];
            let second_order = 1e-7 + 10.0 * EARTH_RATE * EARTH_RATE * shot.load.muzzle_velocity_mps * t.powi(3);
            for (k, ck) in c.iter().enumerate() {
                let moved = r.pos[k] - p.pos[k];
                assert!((moved + 2.0 * ck).abs() < second_order, "lat {lat} az {az} t {t} axis {k}: {moved} vs {}", -2.0 * ck);
            }
        }
    }
}

/// Physical checks on Coriolis with drag: sideways to the right in the north and the
/// left in the south (not an exact mirror: the part from the bullet's fall, which is the
/// northward component of the rotation times the vertical velocity, does not change
/// sign); at the pole the same whatever the direction; up firing east and
/// down firing west (Eotvos); firing north on the equator, only the fall is deflected
/// (east, about a millimetre). Litz's flat-fire estimate Omega sin(latitude) x t
/// overstates the sideways part, because it leaves out the drag that damps the sideways
/// velocity Coriolis gives the bullet: Squib is 4, 9 and 15 percent below it at 300, 600
/// and 1000 m here (py-ballisticcalc agrees, `matches_reference_solver`).
#[test]
fn coriolis_behaviour() {
    let ranges = [300.0, 600.0, 1000.0];
    let plain = solve(&level(), &ranges).unwrap();
    let with = |lat: f64, az: f64| {
        solve(&Shot { coriolis: Some(Earth { latitude_deg: lat, azimuth_deg: az }), ..level() }, &ranges).unwrap()
    };
    let north_45 = with(45.0, 0.0);
    let south_45 = with(-45.0, 0.0);
    let (pole_a, pole_b) = (with(90.0, 123.0), with(90.0, 301.0));
    for (i, p) in plain.points.iter().enumerate() {
        let (x, t) = (p.range_m, p.time_s);
        let flat = EARTH_RATE * 45f64.to_radians().sin() * x * t;
        let dz = north_45.points[i].windage_m - p.windage_m;
        assert!(dz > 0.0 && (0.8..0.97).contains(&(dz / flat)), "north: {dz} vs flat-fire {flat}");
        let south = south_45.points[i].windage_m - p.windage_m;
        assert!(south < 0.0 && (south + dz).abs() < 0.1 * dz, "about mirrored in the south: {south} vs {dz}");
        assert!((pole_a.points[i].windage_m - pole_b.points[i].windage_m).abs() < 1e-6);
        assert!((pole_a.points[i].drop_m - pole_b.points[i].drop_m).abs() < 1e-6);
    }
    let east = with(0.0, 90.0);
    let west = with(0.0, 270.0);
    let north_eq = with(0.0, 0.0);
    for (i, p) in plain.points.iter().enumerate() {
        let up = east.points[i].drop_m - p.drop_m;
        let down = west.points[i].drop_m - p.drop_m;
        assert!(up > 0.0 && down < 0.0 && (up + down).abs() < 0.05 * up, "Eotvos: east {up}, west {down}");
        // Only the bullet's fall is deflected, east, which is to the right: about Omega g t^3 / 3.
        let east_of_fall = north_eq.points[i].windage_m - p.windage_m;
        let t = p.time_s;
        assert!(east_of_fall > 0.0 && east_of_fall < EARTH_RATE * G_LIKE * t.powi(3), "{east_of_fall}");
        assert!((north_eq.points[i].drop_m - p.drop_m).abs() < 1e-5);
    }
    // At 1000 m east on the equator the bullet rises a few centimetres.
    let up = east.points[2].drop_m - plain.points[2].drop_m;
    assert!((0.03..0.15).contains(&up), "{up}");
    assert!(east.method.contains("Coriolis") && east.method.contains("no spin drift"));
    assert_eq!(east.zero_elevation_rad, plain.zero_elevation_rad, "the zero is the rifle's, without Coriolis");

    for bad in [Earth { latitude_deg: 91.0, azimuth_deg: 0.0 }, Earth { latitude_deg: 0.0, azimuth_deg: 360.0 }] {
        assert!(matches!(solve(&Shot { coriolis: Some(bad), ..level() }, &ranges), Err(BallisticsError::Input(_))));
    }
    let both =
        solve(&Shot { spin: Some(SMK), coriolis: Some(Earth { latitude_deg: 40.0, azimuth_deg: 10.0 }), ..level() }, &ranges)
            .unwrap();
    assert!(both.method.ends_with("; no aerodynamic jump") && both.convergence_m <= CONVERGENCE_M);
}

/// Cards saved before 6.5 have no optional fields: they read back with the terms off and
/// write back byte for byte, so a stored card is never changed by reading it.
#[test]
fn older_records_read_unchanged() {
    let shot = level();
    let old = serde_json::to_string(&shot).unwrap();
    assert!(!old.contains("spin") && !old.contains("coriolis"));
    let back: Shot = serde_json::from_str(&old).unwrap();
    assert_eq!(back, shot);
    let sol = solve(&shot, &[100.0]).unwrap();
    let mut v: Value = serde_json::to_value(&sol).unwrap();
    v["points"][0].as_object_mut().unwrap().remove("spin_drift_m");
    let old_sol: Solution = serde_json::from_value(v).unwrap();
    assert_eq!(old_sol, sol);
    assert!(!serde_json::to_string(&sol).unwrap().contains("stability"));
}
