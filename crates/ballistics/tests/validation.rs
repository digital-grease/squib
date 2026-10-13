//! Validation gate for the ballistics solver (plan 6.2, ADR-061). See each test for its
//! source and the reason for its tolerance.

use serde_json::Value;
use squib_ballistics::solver::{self, G};
use squib_ballistics::*;

fn fixture(name: &str) -> Value {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn f(v: &Value, k: &str) -> f64 {
    v[k].as_f64().unwrap_or_else(|| panic!("{k}"))
}

fn shot_from(i: &Value) -> Shot {
    let model = if i["model"] == "G1" { DragModel::G1 } else { DragModel::G7 };
    Shot {
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
    }
}

/// (c) Cross-solver: py-ballisticcalc 3.1.0 (independent point-mass implementation,
/// development-time reference only; `testing/ballistics/gen_pybc_reference.py`). Each
/// reference row is at a scheduled horizontal distance; Squib's state is taken at the
/// same distance and height, windage, velocity and flight time compared. Tolerances,
/// with the observed differences well inside them:
/// - zero elevation 0.002 mrad (observed 0.0004);
/// - height and windage 3 mm + 0.02 mrad of distance on level shots (observed 1.1 cm at
///   1372 m in the transonic crosswind case, 0.008 mrad): the reference evaluates drag once per step instead of at each Runge-Kutta
///   stage, and holds density constant within 9 m of the firing point;
/// - on inclined shots another 3 cm (observed 2 cm): the reference puts the sight
///   vertically above the bore, Squib perpendicular to the sight line, a fixed offset of
///   2.2 cm along and 0.6 cm across at 30 degrees (observed 7 mm);
/// - velocity 0.25 m/s (observed 0.20); flight time 0.2 ms plus 0.02 percent (observed
///   0.6 ms at 2.78 s, where the reference interpolates time to its scheduled distance).
#[test]
fn matches_reference_solver() {
    let doc = fixture("pybc_reference.json");
    let cases = doc["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 8);
    for c in cases {
        let name = c["name"].as_str().unwrap();
        let shot = shot_from(&c["input"]);
        let zero = zero_elevation(&shot.load, shot.air, STEP_S).unwrap();
        let zref = f(c, "reference_zero_elevation_rad");
        assert!((zero - zref).abs() < 2e-6, "{name}: zero {zero} vs {zref}");
        let rows = c["rows"].as_array().unwrap();
        let far = rows.iter().map(|r| f(r, "slant_m")).fold(0.0, f64::max);
        let p = path(&shot, zero, far + 50.0, STEP_S).unwrap();
        let incline = if shot.look_angle_deg != 0.0 { 0.03 } else { 0.0 };
        let mut worst = [0.0f64; 3];
        for r in rows.iter().filter(|r| f(r, "x") > 0.0) {
            let x = f(r, "x");
            // The horizontal distance is the sight-line distance of a level sight line.
            let s = solver::at_slant(&p, 0.0, x).unwrap_or_else(|| panic!("{name}: no state at x={x}"));
            let dist = x.hypot(f(r, "y"));
            let tol = 0.003 + 2e-5 * dist + incline;
            for (k, mine) in [("y", s.pos[1]), ("z", s.pos[2])] {
                let d = (mine - f(r, k)).abs();
                assert!(d <= tol, "{name} x={x:.1}: {k} differs by {d:.4} m (tolerance {tol:.4})");
                worst[0] = worst[0].max(d);
            }
            let dv = (s.speed() - f(r, "v")).abs();
            assert!(dv <= 0.25, "{name} x={x:.1}: velocity differs by {dv:.3} m/s");
            let dt = (s.t - f(r, "t")).abs();
            assert!(dt <= 2e-4 + 2e-4 * f(r, "t"), "{name} x={x:.1}: time differs by {dt:.6} s");
            worst[1] = worst[1].max(dv);
            worst[2] = worst[2].max(dt);
        }
        eprintln!("{name:34} worst height/windage {:.4} m, velocity {:.3} m/s, time {:.6} s", worst[0], worst[1], worst[2]);
    }
}

/// The range card the app shows (drop and windage at distances along the sight line)
/// agrees with the reference's sight-line coordinates on level shots, within the same
/// tolerances; corrections are the subtended angles.
#[test]
fn range_card_matches_reference_on_level_shots() {
    let doc = fixture("pybc_reference.json");
    for c in doc["cases"].as_array().unwrap() {
        let shot = shot_from(&c["input"]);
        if shot.look_angle_deg != 0.0 {
            continue;
        }
        let rows: Vec<&Value> = c["rows"].as_array().unwrap().iter().filter(|r| f(r, "slant_m") > 0.0).collect();
        let ranges: Vec<f64> = rows.iter().map(|r| f(r, "slant_m")).collect();
        let sol = solve(&shot, &ranges).unwrap();
        assert!(sol.unreached_m.is_empty());
        assert!(sol.convergence_m < CONVERGENCE_M);
        for (p, r) in sol.points.iter().zip(&rows) {
            let tol = 0.003 + 2e-5 * p.range_m;
            assert!(
                (p.drop_m - f(r, "slant_height_m")).abs() <= tol,
                "{} at {:.0} m: drop {} vs {}",
                c["name"],
                p.range_m,
                p.drop_m,
                f(r, "slant_height_m")
            );
            assert!((p.windage_m - f(r, "z")).abs() <= tol);
            assert!((p.time_s - f(r, "t")).abs() <= 0.002, "time");
            let el = -(p.drop_m / p.range_m).atan();
            assert!((p.elevation_mil - el * 1000.0).abs() < 1e-9);
            assert!((p.elevation_moa / p.elevation_mil - 3.437_746_77).abs() < 1e-6, "1 mil = 3.4377 MOA");
        }
    }
}

/// (b) The drag tables against a second, independent transcription of the BRL tables
/// (archived alternatewars.com page): every CD equal, every Mach equal at the source's
/// two printed decimals, and the source's KD column (CD * 0.3927) consistent.
#[test]
// The source's KD column uses 0.3927 (pi/8 to four places); the check uses its constant.
#[allow(clippy::approx_constant)]
fn drag_tables_match_second_transcription() {
    let doc = fixture("brl_drag_tables.json");
    for model in [DragModel::G1, DragModel::G7] {
        let src = doc[model.name()].as_array().unwrap();
        let table = model.table();
        assert_eq!(src.len(), table.len(), "{model:?}: row count");
        for (row, &(m, cd)) in src.iter().zip(table) {
            let r: Vec<f64> = row.as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
            assert!((r[0] - m).abs() <= 0.005 + 1e-9, "{model:?}: Mach {m} vs {}", r[0]);
            assert_eq!(r[1], cd, "{model:?}: CD at Mach {m}");
            assert!((r[1] * 0.3927 - r[2]).abs() < 6e-5, "{model:?}: KD column at Mach {m}");
        }
    }
}

fn level_shot() -> Shot {
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
    }
}

/// (a) Exact: with negligible air the path is the vacuum parabola from the bore.
#[test]
fn vacuum_flight_is_a_parabola() {
    let mut shot = level_shot();
    shot.air.density_kg_m3 = 1e-15;
    for (look, el) in [(0.0f64, 0.002f64), (25.0, 0.001), (-15.0, 0.003)] {
        shot.look_angle_deg = look;
        let p = path(&shot, el, 1500.0, STEP_S).unwrap();
        let th = look.to_radians();
        let a = th + el;
        let v0 = shot.load.muzzle_velocity_mps;
        let (x0, y0) = (shot.load.sight_height_m * th.sin(), -shot.load.sight_height_m * th.cos());
        for s in p.iter().step_by(97) {
            let t = s.t;
            let x = x0 + v0 * a.cos() * t;
            let y = y0 + v0 * a.sin() * t - 0.5 * G * t * t;
            assert!((s.pos[0] - x).abs() < 1e-6 && (s.pos[1] - y).abs() < 1e-6, "look {look}, t {t}");
            assert!(s.pos[2].abs() < 1e-12);
        }
    }
}

/// (a) Exact: integration error is negligible at the step used, and the solution
/// converges as the step shrinks (RK4: halving the step cuts the error about 16 times).
#[test]
fn step_size_convergence() {
    let shot = Shot { wind: Some(Wind { speed_mps: 4.0, from_deg: 90.0 }), ..level_shot() };
    let e = zero_elevation(&shot.load, shot.air, STEP_S / 8.0).unwrap();
    let at = |dt: f64| {
        let p = path(&shot, e, 1201.0, dt).unwrap();
        let s = solver::at_time(&p, 1.5).unwrap();
        (s.pos[1], s.pos[2])
    };
    let fine = at(STEP_S / 8.0);
    let e1 = at(STEP_S);
    let e2 = at(STEP_S / 2.0);
    let err1 = (e1.0 - fine.0).abs().max((e1.1 - fine.1).abs());
    let err2 = (e2.0 - fine.0).abs().max((e2.1 - fine.1).abs());
    assert!(err1 < 1e-5, "step error {err1} m");
    assert!(err2 <= err1, "halving the step does not make it worse ({err1} then {err2})");
}

/// Physical sense checks the reference cannot catch by itself.
#[test]
fn physical_behaviour() {
    let ranges = [100.0, 300.0, 600.0];
    let level = solve(&level_shot(), &ranges).unwrap();
    // The zero is a crossing of the line of sight; past it the bullet is below.
    let z = solve(&level_shot(), &[91.44]).unwrap();
    assert!(z.points[0].drop_m.abs() < 1e-4);
    assert!(level.points.iter().all(|p| p.drop_m < 0.0 && p.elevation_mil > 0.0));
    assert!(level.points.windows(2).all(|w| w[1].velocity_mps < w[0].velocity_mps && w[1].time_s > w[0].time_s));
    // Energy is kinetic energy of the given mass.
    let p = &level.points[1];
    assert!((p.energy_j - 0.5 * 0.011_34 * p.velocity_mps * p.velocity_mps).abs() < 1e-9);
    // A wind from the right pushes the bullet left; the correction is to the right.
    let right = solve(&Shot { wind: Some(Wind { speed_mps: 5.0, from_deg: 90.0 }), ..level_shot() }, &ranges).unwrap();
    assert!(right.points.iter().all(|p| p.windage_m < 0.0 && p.windage_mil > 0.0));
    let left = solve(&Shot { wind: Some(Wind { speed_mps: 5.0, from_deg: 270.0 }), ..level_shot() }, &ranges).unwrap();
    for (a, b) in right.points.iter().zip(&left.points) {
        assert!((a.windage_m + b.windage_m).abs() < 1e-9, "mirror image");
    }
    // A headwind slows the bullet more than a tailwind, so it drops more.
    let head = solve(&Shot { wind: Some(Wind { speed_mps: 5.0, from_deg: 0.0 }), ..level_shot() }, &ranges).unwrap();
    let tail = solve(&Shot { wind: Some(Wind { speed_mps: 5.0, from_deg: 180.0 }), ..level_shot() }, &ranges).unwrap();
    assert!(head.points[2].drop_m < level.points[2].drop_m && level.points[2].drop_m < tail.points[2].drop_m);
    // Uphill and downhill shots drop less (relative to the sight line) than level ones.
    for look in [25.0, -25.0] {
        let s = solve(&Shot { look_angle_deg: look, ..level_shot() }, &ranges).unwrap();
        assert!(s.points[2].drop_m > level.points[2].drop_m, "look {look}");
    }
    // Thinner air: less drop.
    let thin = solve(&Shot { air: Atmosphere { density_kg_m3: 0.95, temperature_k: 288.15 }, ..level_shot() }, &ranges).unwrap();
    assert!(thin.points[2].drop_m > level.points[2].drop_m);
    // Zeroed in other air, then shot here: the zero no longer holds exactly.
    let moved = Shot { zero_air: Some(Atmosphere { density_kg_m3: 0.95, temperature_k: 288.15 }), ..level_shot() };
    assert!(solve(&moved, &[91.44]).unwrap().points[0].drop_m.abs() > 1e-4);
    // Points below Mach 1.2 are flagged.
    let far = solve(&level_shot(), &[1500.0]).unwrap();
    assert!(far.points[0].transonic && !level.points[0].transonic);
    assert!(level.method.starts_with("squib-pointmass-v1"));
}

#[test]
fn unreachable_and_invalid_inputs() {
    let mut slow = level_shot();
    slow.load.muzzle_velocity_mps = 300.0;
    slow.load.bc_lb_in2 = 0.05;
    slow.load.zero_range_m = 20.0;
    let s = solve(&slow, &[50.0, 3000.0]).unwrap();
    assert_eq!(s.unreached_m, vec![3000.0]);
    assert_eq!(s.points.len(), 1);
    for bad in [
        Shot { load: Load { bc_lb_in2: 0.0, ..level_shot().load }, ..level_shot() },
        Shot { load: Load { muzzle_velocity_mps: f64::NAN, ..level_shot().load }, ..level_shot() },
        Shot { look_angle_deg: 75.0, ..level_shot() },
        Shot { air: Atmosphere { density_kg_m3: 3.0, temperature_k: 288.15 }, ..level_shot() },
        Shot { wind: Some(Wind { speed_mps: 100.0, from_deg: 0.0 }), ..level_shot() },
    ] {
        assert!(matches!(solve(&bad, &[100.0]), Err(BallisticsError::Input(_))), "{bad:?}");
    }
    assert!(solve(&level_shot(), &[]).is_err());
    assert!(solve(&level_shot(), &[-1.0]).is_err());
    assert!(solve(&level_shot(), &vec![100.0; 201]).is_err());
}

#[test]
fn station_air_uses_the_conditions_formula() {
    let a = Atmosphere::from_station(101_325.0, 15.0, 0.0);
    assert!((a.density_kg_m3 - 1.2250).abs() < 0.001, "{}", a.density_kg_m3);
    assert!((solver::speed_of_sound(288.15) - 340.29).abs() < 0.01);
    let humid = Atmosphere::from_station(101_325.0, 30.0, 1.0);
    let dry = Atmosphere::from_station(101_325.0, 30.0, 0.0);
    assert!(humid.density_kg_m3 < dry.density_kg_m3, "moist air is lighter");
}

#[path = "../../../testing/mutate.rs"]
mod mutate;

/// Random inputs, inside and outside the accepted ranges, never panic: a solve either
/// returns finite values or a clear error.
#[test]
fn robust_random_inputs() {
    let mut rng = mutate::Rng::new(mutate::base_seed());
    let mut pick = |lo: f64, hi: f64| lo + (hi - lo) * (rng.next() % 1_000_000) as f64 / 1_000_000.0;
    let n = mutate::iterations(300);
    let mut ok = 0;
    for i in 0..n {
        let shot = Shot {
            load: Load {
                model: if i % 2 == 0 { DragModel::G1 } else { DragModel::G7 },
                bc_lb_in2: pick(-0.1, 2.2),
                mass_kg: pick(0.0, 0.25),
                muzzle_velocity_mps: pick(0.0, 1800.0),
                sight_height_m: pick(-0.05, 0.35),
                zero_range_m: pick(0.0, 1100.0),
            },
            air: Atmosphere { density_kg_m3: pick(0.3, 1.7), temperature_k: pick(200.0, 340.0) },
            zero_air: None,
            wind: Some(Wind { speed_mps: pick(0.0, 45.0), from_deg: pick(-720.0, 720.0) }),
            look_angle_deg: pick(-70.0, 70.0),
        };
        let ranges = [pick(0.0, 3200.0), pick(0.0, 1000.0)];
        let r = std::panic::catch_unwind(|| solve(&shot, &ranges));
        match r {
            Err(_) => panic!("panicked on {shot:?} {ranges:?}"),
            Ok(Ok(s)) => {
                ok += 1;
                for p in &s.points {
                    let v = [p.drop_m, p.windage_m, p.elevation_mil, p.windage_moa, p.velocity_mps, p.time_s, p.energy_j];
                    assert!(v.iter().all(|x| x.is_finite()), "{shot:?} {p:?}");
                }
            }
            Ok(Err(_)) => {}
        }
    }
    eprintln!("random inputs: {ok} of {n} solved, the rest refused with an error");
}

/// (d) Published manufacturer tables: eight Hornady loads, G1 BC as published, retrieved
/// from the product pages (`hornady_published.json`). Hornady does not state its
/// atmosphere; with the Army Standard Metro atmosphere (59 °F, 29.53 inHg, 78 percent
/// humidity), which US ammunition makers have long used, Squib reproduces the tables at
/// their own rounding. Tolerances: velocity 2.5 fps (published to 1 fps; observed 1.5),
/// trajectory 0.15 in (published to 0.1 in; observed 0.06), out to 500 yd. Two published entries are left out,
/// each with its reason in the fixture: the 300 PRC drop at 500 yd (inconsistent with
/// its own column) and the 6.5 Creedmoor table (it does not follow from the BC printed
/// beside it).
#[test]
fn matches_published_manufacturer_tables() {
    let doc = fixture("hornady_published.json");
    let metro = Atmosphere::from_station(29.53 * 3_386.389, 15.0, 0.78);
    let products = doc["products"].as_array().unwrap();
    assert_eq!(products.len(), 8);
    let (yd, inch, fps) = (0.9144, 0.0254, 0.3048);
    let (mut worst_v, mut worst_d, mut skipped) = (0.0f64, 0.0f64, 0);
    for p in products {
        let name = p["product"].as_str().unwrap();
        assert_eq!(p["bc_label"], "G1", "{name}");
        let ranges: Vec<f64> = p["ranges_yd"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
        let traj: Vec<f64> = p["trajectory_in"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
        let vel: Vec<f64> = p["velocity_fps"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
        let zero_yd = ranges[traj.iter().skip(1).position(|t| *t == 0.0).unwrap() + 1];
        let shot = Shot {
            load: Load {
                model: DragModel::G1,
                bc_lb_in2: f(p, "g1_bc"),
                mass_kg: f(p, "weight_gr") * 0.000_064_798_91,
                muzzle_velocity_mps: f(p, "muzzle_velocity_fps") * fps,
                sight_height_m: -traj[0] * inch,
                zero_range_m: zero_yd * yd,
            },
            air: metro,
            zero_air: None,
            wind: None,
            look_angle_deg: 0.0,
        };
        // Published values the fixture marks as excluded, each with its reason.
        let excl = p["excluded"].as_array().cloned().unwrap_or_default();
        if excl.iter().any(|e| e["range_yd"] == "all") {
            skipped += 1;
            continue;
        }
        let excluded: Vec<f64> = excl.iter().map(|e| f(e, "range_yd")).collect();
        let r: Vec<f64> = ranges[1..].iter().map(|y| y * yd).collect();
        let sol = solve(&shot, &r).unwrap();
        for (k, pt) in sol.points.iter().enumerate() {
            let dv = (pt.velocity_mps / fps - vel[k + 1]).abs();
            let dd = (pt.drop_m / inch - traj[k + 1]).abs();
            eprintln!(
                "{name:58} {:3} yd  v {:7.1} vs {:5}  drop {:+6.2} vs {:+5.1}",
                ranges[k + 1],
                pt.velocity_mps / fps,
                vel[k + 1],
                pt.drop_m / inch,
                traj[k + 1]
            );
            assert!(dv <= 2.5, "{name} at {} yd: velocity off by {dv:.1} fps", ranges[k + 1]);
            worst_v = worst_v.max(dv);
            if excluded.contains(&ranges[k + 1]) {
                continue;
            }
            assert!(dd <= 0.15, "{name} at {} yd: trajectory off by {dd:.2} in", ranges[k + 1]);
            worst_d = worst_d.max(dd);
        }
    }
    assert_eq!(skipped, 1, "only the documented inconsistent table is left out");
    eprintln!("published tables: worst velocity {worst_v:.1} fps, worst trajectory {worst_d:.2} in");
}
