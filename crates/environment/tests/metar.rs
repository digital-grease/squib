//! METAR normalization and planning against real aviationweather.gov captures
//! (fixtures/metar, captured 2026-10-10; NOAA data, public domain).

use std::collections::HashMap;
use std::path::PathBuf;

use squib_environment::metar::{self, MetarRefresh, WindDir};
use squib_environment::nws::{CachedDoc, DocCache, HttpResponse, ProviderIssue, Step, TransportError};
use squib_environment::*;

fn fx(name: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/metar").join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// Captures are from 2026-10-10 about 04:00 to 04:30Z.
const NOW: i64 = 1_791_608_400_000; // 2026-10-10T05:00:00Z
const LONDON: (f64, f64) = (51.5, -0.12);

fn of<'a>(c: &'a [MeasurementCandidate], station: &str, f: Field) -> Option<&'a MeasurementCandidate> {
    c.iter().find(|x| x.field == f && x.station.as_ref().is_some_and(|s| s.station_id == station))
}

fn si(c: &[MeasurementCandidate], station: &str, f: Field) -> f64 {
    of(c, station, f).and_then(|x| x.value.si()).unwrap_or_else(|| panic!("{station} {f:?}"))
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
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

#[test]
fn search_is_bounded_rounded_and_on_the_allowed_origin() {
    let lookup = squib_environment::nws::lookup_coords(51.50734, -0.12776).unwrap();
    let url = metar::search_url(lookup);
    assert_eq!(url, "https://aviationweather.gov/api/data/metar?bbox=50.51,-1.74,52.51,1.48&format=json&hours=3");
    assert!(metar::url_allowed(&url));
    assert!(!metar::url_allowed("https://aviationweather.gov.evil.example/api/data/metar"));
    assert!(!metar::url_allowed("https://aviationweather.gov/cgi-bin/other"));
    assert!(!metar::url_allowed("http://aviationweather.gov/api/data/metar"));
    // Polar and antimeridian boxes are clamped, never wrapped or invalid.
    let (s, w, n, e) = metar::search_box((89.5, 179.9));
    assert!(s >= -90.0 && n <= 90.0 && w >= -180.0 && e <= 180.0 && w < e);
}

#[test]
fn london_box_keeps_nearest_stations_with_exact_report_values() {
    let (st, c, issues) = metar::parse_collection(200, &fx("bbox_london.json"), LONDON, NOW).unwrap();
    assert!(issues.is_empty(), "{issues:?}");
    assert!(st.len() <= metar::MAX_STATIONS && !st.is_empty());
    assert!(st.windows(2).all(|w| w[0].distance_m <= w[1].distance_m), "nearest first");
    // EGLL: 27011KT 10/07 Q1013, observed 04:20Z.
    assert!(close(si(&c, "EGLL", Field::Temperature), 283.15, 1e-9));
    assert!(close(si(&c, "EGLL", Field::AltimeterSetting), 101_300.0, 1e-6));
    assert!(close(si(&c, "EGLL", Field::WindSpeed), 11.0 * 1852.0 / 3600.0, 1e-9));
    assert_eq!(of(&c, "EGLL", Field::WindDirection).unwrap().value, Value::Si(270.0));
    let t = of(&c, "EGLL", Field::Temperature).unwrap();
    assert_eq!(t.observed_utc_ms, Some(1_791_606_000_000), "observation time, not fetch time");
    assert_eq!((t.original_value, t.original_unit.as_deref()), (Some(10.0), Some("degC")));
    assert_eq!(t.source, SourceKind::StationObservation);
    // Humidity is derived and says so.
    let rh = of(&c, "EGLL", Field::RelativeHumidity).unwrap();
    assert!(close(rh.value.si().unwrap(), 0.817, 0.005), "{rh:?}");
    assert_eq!(rh.transforms, vec![metar::RH_TRANSFORM.to_string()]);
    assert_eq!(rh.original_unit.as_deref(), Some("degC_dewpoint"));
    // No METAR carries actual pressure at the user's place.
    assert!(c.iter().all(|x| x.field != Field::LocalPressure));
}

#[test]
fn us_reports_use_tenths_inches_and_sea_level_remarks_not_rounded_json() {
    let (_, c, _) = metar::parse_collection(200, &fx("ids_us.json"), (39.86, -104.67), NOW).unwrap();
    // KDEN: 22/M01 A2991 RMK SLP047 T02171011. The JSON says altim 1013 (rounded).
    assert!(close(si(&c, "KDEN", Field::Temperature), 21.7 + 273.15, 1e-9));
    let alt = of(&c, "KDEN", Field::AltimeterSetting).unwrap();
    assert_eq!((alt.original_value, alt.original_unit.as_deref()), (Some(29.91), Some("inHg")));
    assert!(close(alt.value.si().unwrap(), 29.91 * 3386.389, 1e-6));
    assert!(close(si(&c, "KDEN", Field::SeaLevelPressure), 100_470.0, 1e-6));
    assert!(of(&c, "KDEN", Field::Temperature).unwrap().transforms.contains(&"source:metar_t_group".to_string()));
}

#[test]
fn metric_winds_gusts_variable_calm_and_qfe() {
    let (_, c, _) = metar::parse_collection(200, &fx("ids_mps_qfe.json"), (55.97, 37.41), NOW).unwrap();
    // UUEE 21008G14MPS: the JSON converted these to knots; the report's units are kept.
    let w = of(&c, "UUEE", Field::WindSpeed).unwrap();
    assert_eq!((w.original_value, w.original_unit.as_deref(), w.value), (Some(8.0), Some("m_s-1"), Value::Si(8.0)));
    assert_eq!(si(&c, "UUEE", Field::WindGust), 14.0);
    // ZBAA VRB01MPS: variable is a state, not a number.
    assert_eq!(of(&c, "ZBAA", Field::WindDirection).unwrap().value, Value::Variable);
    // UNNT RMK QFE756/1008: actual pressure at the remote station, never local.
    let q = of(&c, "UNNT", Field::RemoteStationPressure).unwrap();
    assert_eq!(q.value, Value::Si(100_800.0));
    assert!(q.transforms.contains(&"source:metar_qfe_remark".to_string()));

    let (_, c, _) = metar::parse_collection(200, &fx("samples_wind_slp.json"), (40.0, -100.0), NOW).unwrap();
    assert_eq!(of(&c, "CYCG", Field::WindDirection).unwrap().value, Value::Variable);
    assert_eq!(si(&c, "KFBR", Field::WindGust), 16.0 * 1852.0 / 3600.0);
    let calm = of(&c, "KMLP", Field::WindDirection).unwrap();
    assert_eq!(calm.value, Value::Calm, "00000KT is calm, not north");
    assert_eq!(si(&c, "KMLP", Field::WindSpeed), 0.0);
    assert!(of(&c, "KSHV", Field::SeaLevelPressure).is_none(), "SLPNO means not available");
}

#[test]
fn raw_parser_ignores_trend_groups_and_handles_missing_values() {
    let m = metar::parse_raw("METAR XXXX 101200Z 27010KT 9999 15/10 Q1012 TEMPO 30025G35KT 12/11");
    assert_eq!(m.wind.as_ref().map(|w| (w.dir, w.speed, w.gust)), Some((WindDir::Degrees(270.0), 10.0, None)));
    assert_eq!((m.temp_c, m.dewpoint_c), (Some(15.0), Some(10.0)));
    let m = metar::parse_raw("METAR XXXX 101200Z /////KT 9999 M05/// Q////");
    assert!(m.wind.is_none() && m.altimeter.is_none());
    assert_eq!((m.temp_c, m.dewpoint_c), (Some(-5.0), None));
    // Visibility fractions and runway groups are not temperatures.
    let m = metar::parse_raw("METAR XXXX 101200Z 18005KT 1/2SM R27L/P2000 M02/M03 A2990");
    assert_eq!((m.temp_c, m.dewpoint_c), (Some(-2.0), Some(-3.0)));
    assert!((metar::rh_from_dewpoint(10.0, 11.0) - 1.0).abs() < 1e-12, "rounded dew point above temperature is saturation");
}

#[test]
fn empty_box_is_a_typed_issue() {
    assert_eq!(metar::parse_collection(204, b"", LONDON, NOW).unwrap_err(), ProviderIssue::NoStationsNearby);
    assert_eq!(metar::parse_collection(200, b"[]", LONDON, NOW).unwrap_err(), ProviderIssue::NoStationsNearby);
    assert!(matches!(metar::parse_collection(200, b"{", LONDON, NOW).unwrap_err(), ProviderIssue::Malformed { .. }));
}

#[test]
fn planner_uses_one_request_and_falls_back_to_cache_offline() {
    let mut cache = MemCache::default();
    let (mut r, step) = MetarRefresh::start(LONDON.0, LONDON.1, NOW, 20_000, &cache).unwrap();
    let Step::Fetch(reqs) = step else { panic!() };
    assert_eq!(reqs.len(), 1);
    let ok = HttpResponse {
        id: reqs[0].id,
        status: 200,
        body: fx("bbox_london.json"),
        etag: Some("W/\"x\"".into()),
        max_age_s: Some(60),
        retry_after_s: None,
        error: None,
    };
    let Step::Done(res) = r.on_response(ok, NOW + 500, &mut cache) else { panic!() };
    assert!(!res.used_cache && !res.candidates.is_empty());

    // Offline later: the cached report is used and keeps its original observation time.
    let (mut r, step) = MetarRefresh::start(LONDON.0, LONDON.1, NOW + 3_600_000, 20_000, &cache).unwrap();
    let Step::Fetch(reqs) = step else { panic!() };
    assert_eq!(reqs[0].if_none_match.as_deref(), Some("W/\"x\""));
    let offline = HttpResponse {
        id: reqs[0].id,
        status: 0,
        body: vec![],
        etag: None,
        max_age_s: None,
        retry_after_s: None,
        error: Some(TransportError::Offline),
    };
    let Step::Done(res) = r.on_response(offline, NOW + 3_600_100, &mut cache) else { panic!() };
    assert!(res.used_cache);
    assert!(matches!(res.issues[0], ProviderIssue::Transport { .. }));
    assert!(res.candidates.iter().all(|c| c.observed_utc_ms.is_some_and(|t| t <= NOW)));
}

#[test]
fn resolver_uses_metar_with_attribution_and_keeps_local_pressure_unavailable() {
    let (_, c, _) = metar::parse_collection(200, &fx("ids_mps_qfe.json"), (55.03, 82.65), NOW).unwrap();
    let place = ReferencePlace { lat: 55.03, lon: 82.65, label: None, source: "manual".into(), elevation_msl_m: None };
    let s = resolve("s", &c, &[], Some(place), vec![], &ResolverPolicy::default(), NOW);
    let f = |x: Field| s.fields.iter().find(|r| r.field == x).unwrap();
    assert!(f(Field::Temperature).chosen.is_some());
    assert!(f(Field::LocalPressure).chosen.is_none(), "QFE at a remote station is not local pressure");
    assert!(s.attribution.iter().any(|a| a == metar::ATTRIBUTION));
}
