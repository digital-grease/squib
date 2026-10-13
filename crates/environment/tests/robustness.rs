//! Plan 5.2: provider payload robustness. Every parser is fed mutated real captures
//! (fixtures/metar, fixtures/nws, fixtures/epqs), deeply nested JSON, and oversized input.
//! Properties: no panic, bounded time, and anything accepted is finite and resolvable.
//! See testing/mutate.rs for iteration and seed control.

#[path = "../../../testing/mutate.rs"]
mod mutate;

use std::path::PathBuf;
use std::time::Duration;

use mutate::{JSON_TOKENS, deeply_nested, fuzz, iterations};
use squib_environment::*;

const NOW: i64 = 1_791_608_400_000;
const BOULDER: (f64, f64) = (40.015, -105.2705);
const LONDON: (f64, f64) = (51.5, -0.12);
const LIMIT: Duration = Duration::from_millis(500);

fn fx(dir: &str, name: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures").join(dir).join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn finite(c: &MeasurementCandidate) -> Result<(), String> {
    let ok = |x: Option<f64>| x.is_none_or(f64::is_finite);
    if let Value::Si(v) = c.value
        && !v.is_finite()
    {
        return Err(format!("{}: non-finite value {v}", c.id));
    }
    if !ok(c.original_value) || !ok(c.accuracy) {
        return Err(format!("{}: non-finite original value or accuracy", c.id));
    }
    if let Some(s) = &c.station
        && !(s.lat.is_finite() && s.lon.is_finite() && ok(s.elevation_m) && ok(s.distance_m))
    {
        return Err(format!("{}: non-finite station", c.id));
    }
    Ok(())
}

/// Accepted candidates must survive the rest of the pipeline: resolve, then derive.
fn downstream(cands: &[MeasurementCandidate]) -> Result<(), String> {
    for c in cands {
        finite(c)?;
    }
    let snap = resolve("fuzz", cands, &[], None, vec![], &ResolverPolicy::default(), NOW);
    let _ = derived::derive(&snap);
    Ok(())
}

fn station(json: &[u8]) -> StationRef {
    nws::parse_stations(200, json, nws::lookup_coords(BOULDER.0, BOULDER.1).unwrap()).unwrap().remove(0)
}

#[test]
fn robust_metar_collections() {
    let seeds: Vec<Vec<u8>> = ["bbox_london.json", "ids_mps_qfe.json", "ids_us.json", "ids_world.json", "samples_wind_slp.json"]
        .iter()
        .map(|n| fx("metar", n))
        .collect();
    fuzz("metar collection", &seeds, JSON_TOKENS, iterations(400), LIMIT, |b| {
        match metar::parse_collection(200, b, LONDON, NOW) {
            Ok((stations, cands, _)) => {
                if stations.iter().any(|s| !s.lat.is_finite() || !s.lon.is_finite()) {
                    return Err("non-finite station position".into());
                }
                downstream(&cands)
            }
            Err(_) => Ok(()),
        }
    });
}

#[test]
fn robust_metar_raw_reports() {
    // Every raw report in the captures is a seed; mutations hit groups and remarks.
    let mut seeds = vec![];
    for n in ["bbox_london.json", "ids_mps_qfe.json", "ids_us.json", "ids_world.json", "samples_wind_slp.json"] {
        let v: serde_json::Value = serde_json::from_slice(&fx("metar", n)).unwrap();
        for o in v.as_array().unwrap() {
            if let Some(r) = o.get("rawOb").and_then(|r| r.as_str()) {
                seeds.push(r.as_bytes().to_vec());
            }
        }
    }
    assert!(seeds.len() > 10, "raw reports found");
    let tokens: &[&[u8]] = &[
        b" RMK ",
        b" T",
        b" SLP",
        b" Q",
        b" A",
        b" QFE",
        b"M",
        b"/",
        b"//",
        b"KT",
        b"MPS",
        b"KMH",
        b"VRB",
        b"00000KT",
        b"99999",
        b"G",
        b" 9999 ",
        b"M99/M99",
        b"T99999999",
        b"SLP999",
        b"A9999",
        b"Q0000",
        b"QFE999/9999",
        b"\xc3\xa9",
    ];
    fuzz("metar raw", &seeds, tokens, iterations(2000), LIMIT, |b| {
        let m = metar::parse_raw(&String::from_utf8_lossy(b));
        let all = [m.temp_c, m.dewpoint_c, m.altimeter.map(|a| a.0), m.sea_level_hpa, m.qfe.map(|q| q.0)];
        if all.iter().flatten().any(|v| !v.is_finite()) {
            return Err(format!("non-finite field in {m:?}"));
        }
        Ok(())
    });
}

#[test]
fn robust_nws_documents() {
    let lookup = nws::lookup_coords(BOULDER.0, BOULDER.1).unwrap();
    let points = vec![fx("nws", "points_boulder.json"), fx("nws", "points_outside_us.json")];
    fuzz("nws points", &points, JSON_TOKENS, iterations(300), LIMIT, |b| {
        let _ = nws::parse_points(200, b);
        let _ = nws::parse_points(404, b);
        Ok(())
    });
    fuzz("nws stations", &[fx("nws", "stations_boulder.json")], JSON_TOKENS, iterations(300), LIMIT, |b| {
        if let Ok(st) = nws::parse_stations(200, b, lookup)
            && st.iter().any(|s| !s.lat.is_finite() || !s.lon.is_finite())
        {
            return Err("non-finite station".into());
        }
        Ok(())
    });
    let st = station(&fx("nws", "stations_boulder.json"));
    let obs: Vec<Vec<u8>> = ["obs_KBDU.json", "obs_KBJC.json", "obs_KEIK.json", "obs_KFNL.json", "obs_KLMO.json"]
        .iter()
        .map(|n| fx("nws", n))
        .collect();
    fuzz("nws observation", &obs, JSON_TOKENS, iterations(600), LIMIT, |b| match nws::parse_observation(200, b, &st, NOW) {
        Ok((cands, _)) => downstream(&cands),
        Err(_) => Ok(()),
    });
}

#[test]
fn robust_nws_alerts() {
    let seeds: Vec<Vec<u8>> =
        ["alerts_boulder_none.json", "alerts_helena.json", "alerts_out_of_bounds.json", "alerts_test_message.json"]
            .iter()
            .map(|n| fx("nws", n))
            .collect();
    fuzz("nws alerts", &seeds, JSON_TOKENS, iterations(400), LIMIT, |b| {
        if let Ok(Some(alerts)) = alerts::parse_alerts(200, b, NOW)
            && alerts.iter().any(|a| matches!((a.onset_utc_ms, a.until_utc_ms), (Some(o), Some(u)) if u < o))
        {
            return Err("an alert ends before it starts".into());
        }
        Ok(())
    });
}

#[test]
fn robust_terrain_points() {
    let seeds: Vec<Vec<u8>> =
        ["boulder_lidar_1m.json", "hilo_hawaii.json", "nebraska.json", "pacific_ocean.json", "bad_request.txt"]
            .iter()
            .map(|n| fx("epqs", n))
            .collect();
    let lookup = terrain::lookup_coords(BOULDER.0, BOULDER.1).unwrap();
    fuzz("terrain point", &seeds, JSON_TOKENS, iterations(400), LIMIT, |b| match terrain::parse_point(200, b, lookup, NOW) {
        Ok(c) => downstream(&[c]),
        Err(_) => Ok(()),
    });
}

#[test]
fn robust_timestamps() {
    let seeds =
        vec![b"2026-10-10T04:51:00Z".to_vec(), b"2026-10-07T22:50:00+00:00".to_vec(), b"2026-10-10T04:51:00.123-06:00".to_vec()];
    let tokens: &[&[u8]] = &[b"9999", b"-", b"+", b":", b"T", b"Z", b".", b"99", b"00", b"+99:99", b"\xc3\xa9"];
    fuzz("rfc3339", &seeds, tokens, iterations(3000), LIMIT, |b| {
        let _ = time::parse_rfc3339_ms(&String::from_utf8_lossy(b));
        Ok(())
    });
}

/// Nesting far past serde_json's recursion limit, and bodies past every size cap,
/// are refused rather than overflowing the stack or being parsed.
#[test]
fn deeply_nested_and_oversized_payloads_are_refused() {
    let deep = deeply_nested(200_000);
    let lookup = nws::lookup_coords(BOULDER.0, BOULDER.1).unwrap();
    let st = station(&fx("nws", "stations_boulder.json"));
    assert!(metar::parse_collection(200, &deep, LONDON, NOW).is_err());
    assert!(nws::parse_points(200, &deep).is_err());
    assert!(nws::parse_stations(200, &deep, lookup).is_err());
    assert!(nws::parse_observation(200, &deep, &st, NOW).is_err());
    assert!(alerts::parse_alerts(200, &deep, NOW).is_err());
    assert!(terrain::parse_point(200, &deep, terrain::lookup_coords(BOULDER.0, BOULDER.1).unwrap(), NOW).is_err());
    // Size caps sit in the refresh planners (tested with their transport fakes); the
    // parsers themselves must still finish quickly on a large valid-looking body.
    let mut big = b"[".to_vec();
    for i in 0..60_000 {
        big.extend_from_slice(
            format!("{{\"icaoId\":\"X{i}\",\"lat\":51.5,\"lon\":-0.1,\"rawOb\":\"X 101250Z 00000KT\"}},").as_bytes(),
        );
    }
    big.pop();
    big.push(b']');
    assert!(big.len() > metar::MAX_BODY_BYTES, "bigger than the refresh cap");
    let t = std::time::Instant::now();
    let _ = metar::parse_collection(200, &big, LONDON, NOW);
    assert!(t.elapsed() < Duration::from_secs(5), "large body parsed in {:?}", t.elapsed());
}

/// Inputs the fuzzer found, pinned so they stay fixed.
#[test]
fn regressions_found_by_fuzzing() {
    // A non-ASCII character inside a wind group split a string off a character boundary.
    let m = metar::parse_raw("METAR YSSY 100400Z 27\u{fffd}13KT CAVOK 29/11 Q1000");
    assert!(m.wind.is_none());
    assert_eq!(m.temp_c, Some(29.0));
    // An epoch-seconds observation time too large to convert to milliseconds is unknown, not a panic.
    let body = br#"[{"icaoId":"EGLL","lat":51.47,"lon":-0.45,"obsTime":9223372036854775807,"rawOb":"EGLL 100420Z 24008KT 12/08 Q1012"}]"#;
    let (_, cands, _) = metar::parse_collection(200, body, LONDON, NOW).unwrap();
    assert!(!cands.is_empty() && cands.iter().all(|c| c.observed_utc_ms.is_none()));
    // An alert whose onset is after its end is still shown, without the impossible onset.
    let body = br#"{"features":[{"properties":{"status":"Actual","messageType":"Alert","event":"Heat Advisory",
        "onset":"2026-10-12T00:00:00Z","ends":"2026-10-11T00:00:00Z"}}]}"#;
    let a = alerts::parse_alerts(200, body, NOW).unwrap().unwrap();
    assert_eq!((a.len(), a[0].onset_utc_ms), (1, None));
}
