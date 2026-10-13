#!/usr/bin/env python3
"""Generate cross-solver references for Squib's optional ballistics terms (plan 6.5):
spin drift and Coriolis, with py-ballisticcalc as the independent reference.

py-ballisticcalc (LGPL-3.0) is a development-time reference only: it is never shipped or
linked. Regenerate with the same pinned venv as gen_pybc_reference.py:

    /tmp/pybc/bin/python -I testing/ballistics/gen_pybc_optional.py

Each case is fired twice with the same bore elevation: once without the optional terms
(the "base" rows) and once with them. The zero is found on a level shot with neither
term, as Squib does. The test compares each term's own contribution (with minus base)
as well as the totals, so a small difference in the base solution does not hide or
excuse an error in the term.

Spin drift: Litz's approximation, 1.25 (SG + 1.2) t^1.83 inches, with SG from Miller's
formula including its velocity and atmosphere corrections. Humidity is 0 in the spin
cases: Miller's atmosphere correction uses pressure and temperature, Squib uses the
equivalent air density ratio, and the two agree exactly only for dry air.

Coriolis: full 3D (latitude and azimuth given), -2 Omega x v in the integration.
"""
import json
import sys
from pathlib import Path

import py_ballisticcalc as pbc
from py_ballisticcalc import Ammo, Atmo, Calculator, DragModel, Shot, TableG1, TableG7, Unit, Weapon, Wind

PINNED = "3.1.0"
OUT = Path(__file__).resolve().parents[2] / "crates/ballistics/tests/fixtures/pybc_optional.json"

# name, model, BC, weight gr, diameter in, length in, twist in (negative: left hand),
# MV fps, sight height in, zero yd, atmosphere (altitude ft, station inHg, F, humidity %),
# wind (mph, py-ballisticcalc direction_from: 0 from behind, 90 from the left),
# look angle deg, latitude deg or None, azimuth deg or None, range yd, step yd.
CASES = [
    ("spin_308_175_rh10", "G7", 0.243, 175, 0.308, 1.240, 10, 2600, 1.5, 100,
     (0, 29.92, 59, 0), (0, 0), 0, None, None, 1000, 50),
    ("spin_65_140_lh8_hot_high", "G7", 0.315, 140, 0.264, 1.370, -8, 2700, 1.75, 100,
     (5000, 24.9, 95, 0), (0, 0), 0, None, None, 1000, 50),
    ("spin_223_77_rh7_wind", "G1", 0.372, 77, 0.224, 0.995, 7, 2750, 2.6, 100,
     (0, 30.1, 40, 0), (10, 270), 0, None, None, 600, 25),
    ("coriolis_308_lat45_east", "G7", 0.243, 175, 0.308, 1.240, 0, 2600, 1.5, 100,
     (0, 29.92, 59, 0), (0, 0), 0, 45.0, 90.0, 1000, 50),
    ("coriolis_308_lat-35_north", "G7", 0.243, 175, 0.308, 1.240, 0, 2600, 1.5, 100,
     (0, 29.92, 59, 0), (0, 0), 0, -35.0, 0.0, 1000, 50),
    ("coriolis_65_lat60_west_uphill_wind", "G7", 0.315, 140, 0.264, 1.370, 0, 2700, 1.75, 100,
     (2000, 27.8, 50, 40), (8, 90), 20, 60.0, 270.0, 1000, 50),
    ("both_65_lat40_south_rh8_crosswind", "G7", 0.315, 140, 0.264, 1.370, 8, 2700, 1.75, 100,
     (0, 29.92, 59, 0), (10, 270), 0, 40.0, 180.0, 1500, 50),
]


def to_mine(direction_from_deg):
    """py-ballisticcalc's direction_from (0 from behind, 90 from the left) to Squib's
    (0 from the target, 90 from the right)."""
    return (180.0 + direction_from_deg) % 360.0


def rows_of(result):
    return [
        {
            "t": r.time,
            "x": r.distance >> Unit.Meter,
            "slant_m": r.slant_distance >> Unit.Meter,
            "slant_height_m": r.slant_height >> Unit.Meter,
            "z": r.windage >> Unit.Meter,
        }
        for r in result.samples
    ]


def main():
    if pbc.__version__ != PINNED:
        sys.exit(f"py-ballisticcalc {PINNED} required, found {pbc.__version__}")
    calc = Calculator(engine="python+rk4")
    cases = []
    for (name, model, bc, gr, dia, length, twist, mv, sh, zero, (alt, inhg, tf, rh), (wmph, wdir), look, lat, az,
         rng, step) in CASES:
        table = TableG1 if model == "G1" else TableG7
        dm = DragModel(bc, table, Unit.Grain(gr), Unit.Inch(dia), Unit.Inch(length))
        ammo = Ammo(dm, Unit.FPS(mv))
        atmo = Atmo(altitude=Unit.Foot(alt), pressure=Unit.InHg(inhg), temperature=Unit.Fahrenheit(tf), humidity=rh)
        plain = Weapon(sight_height=Unit.Inch(sh), twist=0)
        zero_elev = calc.set_weapon_zero(Shot(weapon=plain, ammo=ammo, atmo=atmo), Unit.Yard(zero))
        winds = [Wind(Unit.MPH(wmph), Unit.Degree(wdir))] if wmph else []

        def fire(twist_in, latitude, azimuth):
            weapon = Weapon(sight_height=Unit.Inch(sh), twist=Unit.Inch(twist_in), zero_elevation=zero_elev)
            shot = Shot(weapon=weapon, ammo=ammo, atmo=atmo, winds=winds, look_angle=Unit.Degree(look),
                        latitude=latitude, azimuth=azimuth)
            return calc.fire(shot, trajectory_range=Unit.Yard(rng), trajectory_step=Unit.Yard(step))

        base = fire(0, None, None)
        full = fire(twist, lat, az)
        props = full.props if hasattr(full, "props") else None
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
                "twist_m": twist * 0.0254 if twist else None,
                "diameter_m": dia * 0.0254,
                "length_m": length * 0.0254,
                "latitude_deg": lat,
                "azimuth_deg": az,
            },
            "reference_zero_elevation_rad": zero_elev >> Unit.Radian,
            "reference_stability": props.stability_coefficient if props and twist else None,
            "base": rows_of(base),
            "full": rows_of(full),
        })
    doc = {
        "generator": "testing/ballistics/gen_pybc_optional.py",
        "reference": f"py-ballisticcalc {PINNED} (LGPL-3.0, development-time reference only), python+rk4 engine",
        "conventions": "SI; x downrange horizontal, slant along the line of sight, z right; base rows without the "
                       "optional terms, full rows with them, both at the same bore elevation",
        "cases": cases,
    }
    OUT.write_text(json.dumps(doc, indent=1, sort_keys=True) + "\n")
    print(f"{len(cases)} cases -> {OUT}")


if __name__ == "__main__":
    main()
