//! NWS normalization, fetch planning, resolver policy, and redaction against real
//! api.weather.gov captures (fixtures/nws, captured 2026-10-07; public domain).

use std::collections::HashMap;
use std::path::PathBuf;

use squib_environment::nws::{self, CachedDoc, DocCache, HttpResponse, ProviderIssue, Step, TransportError};
use squib_environment::privacy::{LocationRetention, apply};
use squib_environment::*;

fn fx(name: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/nws").join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

const BOULDER: (f64, f64) = (40.015, -105.2705);
/// Fixture observations are 2026-10-07T21:48Z..22:30Z.
const NOW: i64 = 1_791_413_400_000; // 2026-10-07T22:50:00Z

fn stations() -> Vec<StationRef> {
    nws::parse_stations(200, &fx("stations_boulder.json"), nws::lookup_coords(BOULDER.0, BOULDER.1).unwrap()).unwrap()
}

fn obs(id: &str) -> Vec<MeasurementCandidate> {
    let st = stations().into_iter().find(|s| s.station_id == id).unwrap();
    nws::parse_observation(200, &fx(&format!("obs_{id}.json")), &st, NOW).unwrap().0
}

fn get(c: &[MeasurementCandidate], f: Field) -> &MeasurementCandidate {
    c.iter().find(|x| x.field == f).unwrap_or_else(|| panic!("no {f:?}"))
}

#[derive(Default)]
struct MemCache(HashMap<String, CachedDoc>);
impl DocCache for MemCache {
    fn get(&self, key: &str) -> Option<CachedDoc> {
        self.0.get(key).cloned()
    }
    fn put(&mut self, doc: CachedDoc) {
        self.0.insert(doc.key.clone(), doc);
    }
}

// ---- Normalization (A12) ------------------------------------------------------------

#[test]
fn points_and_stations_parse_with_origin_policy() {
    let info = nws::parse_points(200, &fx("points_boulder.json")).unwrap();
    assert_eq!(info.stations_url, "https://api.weather.gov/gridpoints/BOU/54,75/stations");
    let st = stations();
    assert_eq!(st.len(), nws::MAX_STATIONS);
    assert_eq!(st[0].station_id, "KBDU");
    assert!((st[0].elevation_m.unwrap() - 1611.78).abs() < 0.01);
    assert!(st[0].distance_m.unwrap() < 10_000.0);
    assert_eq!(nws::parse_points(404, &fx("points_outside_us.json")).unwrap_err(), ProviderIssue::UnsupportedLocation);
}

#[test]
fn url_policy_rejects_other_origins() {
    assert!(nws::url_allowed("https://api.weather.gov/stations/KBDU/observations/latest"));
    for bad in [
        "http://api.weather.gov/x",
        "https://api.weather.gov.evil.example/x",
        "https://api.weather.gov@evil.example/x",
        "https://api.weather.gov//evil.example/x",
        "https://evil.example/https://api.weather.gov/x",
    ] {
        assert!(!nws::url_allowed(bad), "{bad}");
    }
    let body = br#"{"properties":{"observationStations":"https://evil.example/stations"}}"#;
    assert!(matches!(nws::parse_points(200, body), Err(ProviderIssue::DisallowedUrl { .. })));
    assert_eq!(nws::latest_observation_url("KBDU/../x"), None);
}

#[test]
fn observation_units_nulls_calm_and_pressure_kinds() {
    let c = obs("KBDU");
    assert!((get(&c, Field::Temperature).value.si().unwrap() - 299.15).abs() < 1e-9);
    assert!((get(&c, Field::RelativeHumidity).value.si().unwrap() - 0.19572267287992).abs() < 1e-9);
    assert_eq!(get(&c, Field::WindSpeed).value, Value::Si(0.0));
    assert_eq!(get(&c, Field::WindDirection).value, Value::Calm, "calm is not north");
    assert_eq!(get(&c, Field::WindGust).value, Value::Missing, "null gust stays missing");
    // A10: NWS barometricPressure is the altimeter setting; never local pressure.
    assert_eq!(get(&c, Field::AltimeterSetting).value, Value::Si(102_170.0));
    assert!(c.iter().all(|x| x.field != Field::LocalPressure && x.field != Field::RemoteStationPressure));
    assert_eq!(get(&c, Field::Temperature).qc.as_deref(), Some("V"));
    assert_eq!(get(&c, Field::Temperature).observed_utc_ms, Some(1_791_411_300_000));
    // A missing RH is not zero.
    assert_eq!(get(&obs("KBJC"), Field::RelativeHumidity).value, Value::Missing);
    let fnl = obs("KFNL");
    assert_eq!(get(&fnl, Field::WindSpeed).value, Value::Missing);
    assert_eq!(get(&fnl, Field::WindDirection).value, Value::Missing, "unknown direction is not calm");
    let lmo = obs("KLMO");
    assert!((get(&lmo, Field::WindSpeed).value.si().unwrap() - 2.6).abs() < 1e-9);
    assert_eq!(get(&lmo, Field::WindDirection).value, Value::Si(140.0));
}

#[test]
fn unknown_units_and_bad_values_reject_only_that_field() {
    let st = stations().remove(0);
    let body = br#"{"properties":{"timestamp":"2026-10-07T22:15:00+00:00",
        "temperature":{"unitCode":"wmoUnit:degRe","value":20,"qualityControl":"V"},
        "relativeHumidity":{"unitCode":"wmoUnit:percent","value":250,"qualityControl":"V"},
        "windSpeed":{"unitCode":"wmoUnit:km_h-1","value":18,"qualityControl":"V"}}}"#;
    let (c, issues) = nws::parse_observation(200, body, &st, NOW).unwrap();
    assert_eq!(c.len(), 1, "only the valid wind speed survives");
    assert!((c[0].value.si().unwrap() - 5.0).abs() < 1e-9);
    assert_eq!(issues.len(), 2);
    // Unknown observation time stays unknown.
    let body = br#"{"properties":{"temperature":{"unitCode":"wmoUnit:degC","value":20}}}"#;
    let (c, _) = nws::parse_observation(200, body, &st, NOW).unwrap();
    assert_eq!(c[0].observed_utc_ms, None);
    assert_eq!(c[0].age_ms(NOW), None);
    // Truncated payloads are malformed, not empty data.
    let truncated = &fx("obs_KBDU.json")[..200];
    assert!(matches!(nws::parse_observation(200, truncated, &st, NOW), Err(ProviderIssue::Malformed { .. })));
}

// ---- Fetch planner ------------------------------------------------------------------

fn serve(url: &str) -> (u16, Vec<u8>) {
    if url.contains("/points/") {
        return (200, fx("points_boulder.json"));
    }
    if url.contains("/gridpoints/BOU/54,75/stations") {
        return (200, fx("stations_boulder.json"));
    }
    for id in ["KBDU", "KLMO", "KEIK", "KBJC", "KFNL"] {
        if url.ends_with(&format!("/stations/{id}/observations/latest")) {
            return (200, fx(&format!("obs_{id}.json")));
        }
    }
    (404, b"{}".to_vec())
}

fn run_refresh(cache: &mut MemCache, fail: impl Fn(&str) -> Option<TransportError>) -> (nws::RefreshResult, Vec<String>, usize) {
    let (mut r, mut step) = nws::NwsRefresh::start(BOULDER.0, BOULDER.1, NOW, 20_000, cache).unwrap();
    let mut urls = Vec::new();
    let mut max_outstanding = 0;
    let mut pending: Vec<nws::HttpRequest> = Vec::new();
    loop {
        match step {
            Step::Done(res) => return (res, urls, max_outstanding),
            Step::Fetch(reqs) => {
                pending.extend(reqs);
                max_outstanding = max_outstanding.max(pending.len());
                let req = pending.remove(0);
                assert!(nws::url_allowed(&req.url), "{}", req.url);
                urls.push(req.url.clone());
                let resp = match fail(&req.url) {
                    Some(e) => HttpResponse {
                        id: req.id,
                        status: 0,
                        body: vec![],
                        etag: None,
                        max_age_s: None,
                        retry_after_s: None,
                        error: Some(e),
                    },
                    None => {
                        let (status, body) = serve(&req.url);
                        HttpResponse {
                            id: req.id,
                            status,
                            body,
                            etag: Some("\"e1\"".into()),
                            max_age_s: Some(300),
                            retry_after_s: None,
                            error: None,
                        }
                    }
                };
                step = r.on_response(resp, NOW + 1000, cache);
            }
        }
    }
}

#[test]
fn bounded_refresh_points_stations_five_observations_two_at_a_time() {
    let mut cache = MemCache::default();
    let (res, urls, max_out) = run_refresh(&mut cache, |_| None);
    assert_eq!(urls[0], "https://api.weather.gov/points/40.02,-105.27", "lookup rounded to ~1 km");
    assert_eq!(urls.iter().filter(|u| u.ends_with("/observations/latest")).count(), 5);
    assert!(max_out <= nws::MAX_CONCURRENT, "{max_out} outstanding");
    assert_eq!(res.stations.len(), 5);
    assert!(!res.used_cache);
    assert!(res.candidates.iter().any(|c| c.field == Field::Temperature));
    // Metadata cached for 24 h: a second refresh skips points and stations.
    let (_, urls2, _) = run_refresh(&mut cache, |_| None);
    assert!(urls2.iter().all(|u| u.ends_with("/observations/latest")), "{urls2:?}");
}

#[test]
fn offline_uses_cache_with_original_observation_times() {
    let mut cache = MemCache::default();
    run_refresh(&mut cache, |_| None);
    // Everything expired and the network is down.
    for d in cache.0.values_mut() {
        d.expires_utc_ms = 0;
    }
    let (res, _, _) = run_refresh(&mut cache, |_| Some(TransportError::Offline));
    assert!(res.used_cache);
    assert!(res.issues.iter().any(|i| matches!(i, ProviderIssue::Transport { .. })));
    let t = res.candidates.iter().find(|c| c.field == Field::Temperature).unwrap();
    assert_eq!(t.observed_utc_ms, Some(1_791_411_300_000), "age comes from observation, not cache read");
    // First use with no cache: nothing, explicitly.
    let mut empty = MemCache::default();
    let (res, _, _) = run_refresh(&mut empty, |_| Some(TransportError::Offline));
    assert!(res.candidates.is_empty() && !res.issues.is_empty());
}

#[test]
fn one_failing_station_does_not_lose_the_others() {
    let mut cache = MemCache::default();
    let (res, _, _) = run_refresh(&mut cache, |u| u.contains("KLMO").then_some(TransportError::Timeout));
    let ids: std::collections::BTreeSet<_> =
        res.candidates.iter().filter_map(|c| c.station.as_ref().map(|s| s.station_id.clone())).collect();
    assert_eq!(ids.len(), 4);
    assert!(!ids.contains("KLMO"));
}

#[test]
fn unsupported_location_is_typed() {
    let mut cache = MemCache::default();
    let (mut r, step) = nws::NwsRefresh::start(51.5007, -0.1246, NOW, 20_000, &cache).unwrap();
    let Step::Fetch(reqs) = step else { panic!() };
    let resp = HttpResponse {
        id: reqs[0].id,
        status: 404,
        body: fx("points_outside_us.json"),
        etag: None,
        max_age_s: None,
        retry_after_s: None,
        error: None,
    };
    let Step::Done(res) = r.on_response(resp, NOW, &mut cache) else { panic!() };
    assert_eq!(res.issues, vec![ProviderIssue::UnsupportedLocation]);
    assert!(nws::NwsRefresh::start(f64::NAN, 0.0, NOW, 1, &cache).is_none());
    assert!(nws::NwsRefresh::start(91.0, 0.0, NOW, 1, &cache).is_none());
}

// ---- Resolver (A09, A10, A11, A13) -----------------------------------------------------

fn all_obs() -> Vec<MeasurementCandidate> {
    ["KBDU", "KLMO", "KEIK", "KBJC", "KFNL"].iter().flat_map(|id| obs(id)).collect()
}

fn place() -> ReferencePlace {
    ReferencePlace {
        lat: 40.02,
        lon: -105.27,
        label: Some("Test range".into()),
        source: "manual".into(),
        elevation_msl_m: Some(1620.0),
    }
}

#[test]
fn resolves_nearest_fresh_station_and_keeps_alternatives() {
    let s = resolve("s1", &all_obs(), &[], Some(place()), vec![], &ResolverPolicy::default(), NOW);
    let t = s.field(Field::Temperature).unwrap();
    assert_eq!(t.origin, Origin::NearbyObservation);
    assert_eq!(t.chosen.as_ref().unwrap().station.as_ref().unwrap().station_id, "KBDU");
    assert_eq!(t.alternatives.len(), 3);
    assert_eq!(t.freshness, Some(Freshness::Fresh));
    assert!(!t.disagreement);
    // KBJC's missing RH is never chosen; another station supplies humidity.
    let rh = s.field(Field::RelativeHumidity).unwrap();
    assert_ne!(rh.chosen.as_ref().unwrap().station.as_ref().unwrap().station_id, "KBJC");
    assert_eq!(s.attribution, vec![nws::ATTRIBUTION.to_string()]);
}

#[test]
fn a09_old_observation_shows_its_own_age_even_when_newly_fetched() {
    let mut c = obs("KBDU");
    for x in &mut c {
        x.fetched_utc_ms = NOW; // fetched just now
    }
    let p = ResolverPolicy::default();
    // 2 h after observation: stale, still selectable, age from observation time.
    let s = resolve("s", &c, &[], None, vec![], &p, 1_791_411_300_000 + 2 * 3_600_000);
    let t = s.field(Field::Temperature).unwrap();
    assert_eq!(t.freshness, Some(Freshness::Stale));
    assert_eq!(t.chosen.as_ref().unwrap().age_ms(1_791_411_300_000 + 2 * 3_600_000), Some(2 * 3_600_000));
    // 4 h: expired for automatic selection.
    let later = 1_791_411_300_000 + 4 * 3_600_000;
    let s = resolve("s", &c, &[], None, vec![], &p, later);
    let t = s.field(Field::Temperature).unwrap();
    assert_eq!(t.origin, Origin::Unavailable);
    assert_eq!(t.reasons, vec![Reason::AllExpired]);
    // Explicit "use older data": selected and labelled.
    let s = resolve("s", &c, &[], None, vec![], &ResolverPolicy { allow_older: true, ..p }, later);
    let t = s.field(Field::Temperature).unwrap();
    assert_eq!(t.freshness, Some(Freshness::Expired));
    assert!(t.reasons.contains(&Reason::OlderDataAllowed));
}

#[test]
fn a10_reduced_or_remote_pressure_never_becomes_local() {
    let s = resolve("s", &all_obs(), &[], Some(place()), vec![], &ResolverPolicy::default(), NOW);
    let local = s.field(Field::LocalPressure).unwrap();
    assert_eq!(local.origin, Origin::Unavailable);
    assert_eq!(local.reasons, vec![Reason::LocalPressureRequiresLocalSource]);
    assert!(local.alternatives.is_empty());
    assert_eq!(s.field(Field::AltimeterSetting).unwrap().origin, Origin::NearbyObservation);
    // With a barometer, local pressure is "here"; the altimeter slot is untouched.
    let mut c = all_obs();
    c.push(local::barometer(835.2, NOW - 60_000).unwrap());
    let s = resolve("s", &c, &[], Some(place()), vec![], &ResolverPolicy::default(), NOW);
    let local = s.field(Field::LocalPressure).unwrap();
    assert_eq!(local.origin, Origin::Here);
    assert_eq!(local.chosen.as_ref().unwrap().value, Value::Si(83_520.0));
    assert!(s.field(Field::AltimeterSetting).unwrap().chosen.as_ref().unwrap().value.si().unwrap() > 100_000.0);
    // A stale barometer reading is not "here, now".
    let mut c = all_obs();
    c.push(local::barometer(835.2, NOW - 3_600_000).unwrap());
    let s = resolve("s", &c, &[], Some(place()), vec![], &ResolverPolicy::default(), NOW);
    assert_eq!(s.field(Field::LocalPressure).unwrap().origin, Origin::Unavailable);
}

#[test]
fn a11_ellipsoid_and_msl_are_not_compared() {
    let mut c = local::location_heights(Some(1605.0), Some(4.0), Some(1621.0), Some(5.0), NOW);
    c.push(local::manual(Field::ElevationMsl, 1630.0, 1630.0, "m", NOW).unwrap());
    let s = resolve("s", &c, &[], None, vec![], &ResolverPolicy::default(), NOW);
    let kinds: Vec<&str> = s
        .elevation_comparisons
        .iter()
        .map(|e| match e {
            ElevationComparison::Compatible { .. } => "compatible",
            ElevationComparison::DatumMismatch { .. } => "mismatch",
            ElevationComparison::UnknownDatum { .. } => "unknown",
        })
        .collect();
    assert_eq!(kinds.iter().filter(|k| **k == "mismatch").count(), 2, "{kinds:?}");
    assert_eq!(kinds.iter().filter(|k| **k == "compatible").count(), 1);
    let compat = s.elevation_comparisons.iter().find_map(|e| match e {
        ElevationComparison::Compatible { difference_m, .. } => Some(*difference_m),
        _ => None,
    });
    assert_eq!(compat.map(f64::abs), Some(9.0));
    // Station elevation is never "here".
    let s = resolve("s", &all_obs(), &[], None, vec![], &ResolverPolicy::default(), NOW);
    assert_eq!(s.field(Field::ElevationMsl).unwrap().origin, Origin::Unavailable);
}

#[test]
fn a13_overrides_are_per_field_and_keep_automatic_candidates() {
    let o = Override {
        candidate: local::manual(Field::WindSpeed, 6.0, 6.0, "m/s", NOW).unwrap(),
        set_utc_ms: NOW,
        expires_utc_ms: None,
    };
    let s = resolve("s", &all_obs(), std::slice::from_ref(&o), Some(place()), vec![], &ResolverPolicy::default(), NOW);
    let w = s.field(Field::WindSpeed).unwrap();
    assert_eq!(w.origin, Origin::Manual);
    assert!(!w.alternatives.is_empty(), "automatic values remain visible");
    assert_eq!(s.field(Field::Temperature).unwrap().origin, Origin::NearbyObservation, "other fields unaffected");
    let expired = Override { expires_utc_ms: Some(NOW - 1), ..o };
    let s = resolve("s", &all_obs(), &[expired], Some(place()), vec![], &ResolverPolicy::default(), NOW);
    assert_eq!(s.field(Field::WindSpeed).unwrap().origin, Origin::NearbyObservation);
}

#[test]
fn qc_exclusion_and_circular_direction_disagreement() {
    let mut c = obs("KBDU");
    for x in &mut c {
        x.qc = Some("X".into());
    }
    let s = resolve("s", &c, &[], None, vec![], &ResolverPolicy::default(), NOW);
    assert_eq!(s.field(Field::Temperature).unwrap().reasons, vec![Reason::AllExcludedByQc]);

    let mk = |id: &str, dir: f64, dist: f64| {
        let mut c = obs("KLMO").into_iter().find(|c| c.field == Field::WindDirection).unwrap();
        c.id = id.into();
        c.value = Value::Si(dir);
        c.station.as_mut().unwrap().distance_m = Some(dist);
        c
    };
    let near_north = [mk("a", 359.0, 1000.0), mk("b", 1.0, 2000.0)];
    let s = resolve("s", &near_north, &[], None, vec![], &ResolverPolicy::default(), NOW);
    assert!(!s.field(Field::WindDirection).unwrap().disagreement, "359 vs 1 is 2 degrees apart");
    let opposite = [mk("a", 10.0, 1000.0), mk("b", 200.0, 2000.0)];
    let s = resolve("s", &opposite, &[], None, vec![], &ResolverPolicy::default(), NOW);
    let d = s.field(Field::WindDirection).unwrap();
    assert!(d.disagreement);
    assert_eq!(d.chosen.as_ref().unwrap().value, Value::Si(10.0), "one representative, never averaged");
}

// ---- Privacy (A19 environmental portion) ----------------------------------------------

#[test]
fn redacted_snapshot_drops_location_revealing_provenance() {
    let mut c = all_obs();
    c.extend(local::location_heights(Some(1605.0), None, Some(1621.0), None, NOW));
    let s = resolve("s", &c, &[], Some(place()), vec![], &ResolverPolicy::default(), NOW);
    let full = serde_json::to_string(&s).unwrap();
    assert!(full.contains("KBDU") && full.contains("Test range"));
    let r = apply(&s, LocationRetention::Redacted);
    let json = serde_json::to_string(&r).unwrap();
    for leak in ["KBDU", "KLMO", "Boulder", "Test range", "40.02", "-105.27", "1621", "1611.7"] {
        assert!(!json.contains(leak), "redacted snapshot leaks {leak}");
    }
    assert!(r.redacted && r.reference.is_none());
    // Values, origin, QC, and observation times survive.
    let t = r.field(Field::Temperature).unwrap();
    assert_eq!(t.origin, Origin::NearbyObservation);
    assert_eq!(t.chosen.as_ref().unwrap().value, Value::Si(299.15));
    assert_eq!(t.chosen.as_ref().unwrap().qc.as_deref(), Some("V"));
    assert!(t.chosen.as_ref().unwrap().observed_utc_ms.is_some());
    assert!(r.field(Field::ElevationMsl).unwrap().reasons.contains(&Reason::PreciseProvenanceNotRetained));
    assert_eq!(apply(&s, LocationRetention::Precise), s);
}
