#!/usr/bin/env python3
"""Generate cross-solver reference trajectories for the Squib ballistics validation gate
(plan 6.2) with py-ballisticcalc, an independent point-mass solver.

py-ballisticcalc (LGPL-3.0) is a development-time reference only: it is never shipped or
linked. This script and its pinned version are kept with the generated data so anyone
can regenerate and audit it:

    uv venv /tmp/pybc && uv pip install --python /tmp/pybc/bin/python py-ballisticcalc==3.1.0
    /tmp/pybc/bin/python -I testing/ballistics/gen_pybc_reference.py

Settings that make the two solvers model the same physics: RK4 engine with a 0.25 ms
step (py-ballisticcalc's default), no spin drift (twist 0), no Coriolis (no latitude),
constant wind over the whole path, weapon zeroed on a level shot.
"""
import json
import math
import sys
from pathlib import Path

import py_ballisticcalc as pbc
from py_ballisticcalc import Ammo, Atmo, Calculator, DragModel, Shot, TableG1, TableG7, Unit, Weapon, Wind

PINNED = "3.1.0"
OUT = Path(__file__).resolve().parents[2] / "crates/ballistics/tests/fixtures/pybc_reference.json"

# name, model, BC (lb/in²), weight gr, diameter in, MV fps, sight height in, zero yd,
# atmosphere (altitude ft, pressure inHg at the station, temperature F, humidity %),
# wind (mph, py-ballisticcalc direction_from degrees: 0 from behind, 90 from the left),
# look angle degrees, range yd, step yd.
CASES = [
    ("g1_308_168_standard", "G1", 0.462, 168, 0.308, 2650, 1.5, 100, (0, 29.92, 59, 0), (0, 0), 0, 1000, 50),
    ("g7_308_175_standard", "G7", 0.243, 175, 0.308, 2600, 1.5, 100, (0, 29.92, 59, 0), (0, 0), 0, 1000, 50),
    ("g7_65_140_crosswind_transonic", "G7", 0.315, 140, 0.264, 2700, 1.75, 100, (0, 29.92, 59, 0), (10, 270), 0, 1500, 50),
    ("g1_223_55_hot_high_quartering", "G1", 0.255, 55, 0.224, 3240, 2.6, 50, (5000, 24.9, 95, 30), (15, 225), 0, 600, 25),
    ("g7_308_175_cold_head_tail", "G7", 0.243, 175, 0.308, 2600, 1.5, 100, (0, 30.4, 5, 80), (12, 0), 0, 1000, 50),
    ("g7_65_140_uphill_30", "G7", 0.315, 140, 0.264, 2700, 1.75, 100, (2000, 27.8, 50, 40), (8, 90), 30, 800, 50),
    ("g7_65_140_downhill_20", "G7", 0.315, 140, 0.264, 2700, 1.75, 100, (2000, 27.8, 50, 40), (0, 0), -20, 800, 50),
    ("g1_22lr_40_subsonic", "G1", 0.125, 40, 0.223, 1080, 1.0, 50, (0, 29.92, 59, 0), (5, 270), 0, 300, 10),
]


def to_mine(direction_from_deg):
    """py-ballisticcalc's direction_from (0 from behind, 90 from the left) to Squib's
    (0 from the target, 90 from the right)."""
    return (180.0 + direction_from_deg) % 360.0


def main():
    if pbc.__version__ != PINNED:
        sys.exit(f"py-ballisticcalc {PINNED} required, found {pbc.__version__}")
    calc = Calculator(engine="python+rk4")
    cases = []
    for name, model, bc, gr, dia, mv, sh, zero, (alt, inhg, tf, rh), (wmph, wdir), look, rng, step in CASES:
        table = TableG1 if model == "G1" else TableG7
        dm = DragModel(bc, table, Unit.Grain(gr), Unit.Inch(dia), Unit.Inch(1.2))
        ammo = Ammo(dm, Unit.FPS(mv))
        atmo = Atmo(altitude=Unit.Foot(alt), pressure=Unit.InHg(inhg), temperature=Unit.Fahrenheit(tf), humidity=rh)
        weapon = Weapon(sight_height=Unit.Inch(sh), twist=0)
        level = Shot(weapon=weapon, ammo=ammo, atmo=atmo)
        zero_elev = calc.set_weapon_zero(level, Unit.Yard(zero))
        winds = [Wind(Unit.MPH(wmph), Unit.Degree(wdir))] if wmph else []
        shot = Shot(weapon=weapon, ammo=ammo, atmo=atmo, winds=winds, look_angle=Unit.Degree(look))
        result = calc.fire(shot, trajectory_range=Unit.Yard(rng), trajectory_step=Unit.Yard(step))
        rows = []
        for r in result.samples:
            rows.append({
                "t": r.time,
                "x": r.distance >> Unit.Meter,
                "y": r.height >> Unit.Meter,
                "z": r.windage >> Unit.Meter,
                "v": r.velocity >> Unit.MPS,
                "mach": r.mach,
                "slant_m": r.slant_distance >> Unit.Meter,
                "slant_height_m": r.slant_height >> Unit.Meter,
            })
        cases.append({
            "name": name,
            "input": {
                "model": model,
                "bc_lb_in2": bc,
                "mass_kg": gr * 0.00006479891,
                "muzzle_velocity_mps": mv * 0.3048,
                "sight_height_m": sh * 0.0254,
                "zero_range_m": zero * 0.9144,
                "air_density_kg_m3": atmo.density_metric,
                "temperature_k": (tf - 32) * 5 / 9 + 273.15,
                "station": {"altitude_ft": alt, "pressure_inhg": inhg, "temperature_f": tf, "humidity_pct": rh},
                "wind_mps": wmph * 0.44704,
                "wind_from_deg": to_mine(wdir) if wmph else 0.0,
                "look_angle_deg": look,
            },
            "reference_zero_elevation_rad": zero_elev >> Unit.Radian,
            "rows": rows,
        })
    doc = {
        "generator": "testing/ballistics/gen_pybc_reference.py",
        "reference": f"py-ballisticcalc {PINNED} (LGPL-3.0, development-time reference only), python+rk4 engine",
        "conventions": "SI; x downrange horizontal from the muzzle, y up from the line of sight's origin, z right; "
                       "times from the muzzle; wind_from_deg 0 from the target, 90 from the right",
        "cases": cases,
    }
    OUT.write_text(json.dumps(doc, indent=1, sort_keys=True) + "\n")
    print(f"{len(cases)} cases, {sum(len(c['rows']) for c in cases)} rows -> {OUT}")


if __name__ == "__main__":
    main()
