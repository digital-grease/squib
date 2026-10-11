//! Engine-level range conditions flows with a simulated HTTP transport serving real
//! NWS captures. Desktop-tested; the Android HTTP and sensor adapters are separate.

use std::path::PathBuf;
use std::sync::Arc;

use squib_mobile::*;

const NOW: i64 = 1_791_413_400_000; // 2026-10-07T22:50Z; fixtures observed 21:48-22:30Z

fn fx(name: &str) -> Vec<u8> {
    std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/nws").join(name)).unwrap()
}

fn engine(name: &str) -> Arc<SquibEngine> {
    let d = std::env::temp_dir().join(format!("squib-cond-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    SquibEngine::new(d.join("journal.db").to_string_lossy().into(), NOW).unwrap()
}

/// METAR captures (fixtures/metar) were observed 2026-10-10 about 04:20Z.
const NOW_METAR: i64 = 1_791_608_400_000;

fn fx_metar(name: &str) -> Vec<u8> {
    std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/metar").join(name)).unwrap()
}

fn serve(url: &str) -> (u16, Vec<u8>) {
    if url.starts_with("https://aviationweather.gov/api/data/metar?bbox=50.") {
        return (200, fx_metar("bbox_london.json"));
    }
    if url.starts_with("https://aviationweather.gov/api/data/metar?") {
        return (204, vec![]);
    }
    if url.contains("/points/51.5") {
        return (404, fx("points_outside_us.json"));
    }
    if url.contains("/points/") {
        return (200, fx("points_boulder.json"));
    }
    if url.contains("/stations?") || url.contains("/stations") && url.contains("gridpoints") {
        return (200, fx("stations_boulder.json"));
    }
    for id in ["KBDU", "KLMO", "KEIK", "KBJC", "KFNL"] {
        if url.ends_with(&format!("/stations/{id}/observations/latest")) {
            return (200, fx(&format!("obs_{id}.json")));
        }
    }
    (404, b"{}".to_vec())
}

/// Drive a refresh to completion. `offline` makes every request fail at transport.
fn refresh(e: &SquibEngine, now: i64, offline: bool) -> usize {
    let mut step = e.conditions_refresh(now).unwrap();
    let mut queue = step.requests.clone();
    let mut n = 0;
    while !step.done {
        let req = queue.remove(0);
        assert_eq!(req.user_agent, "Squib/0.1 (https://github.com/digital-grease/squib)");
        assert!(req.url.starts_with("https://api.weather.gov/") || req.url.starts_with("https://aviationweather.gov/api/data/"));
        n += 1;
        let resp = if offline {
            HttpResponseFfi {
                id: req.id,
                status: 0,
                body: vec![],
                etag: None,
                max_age_s: None,
                retry_after_s: None,
                error: Some("offline".into()),
                unit_random: 0.5,
            }
        } else {
            let (status, body) = serve(&req.url);
            HttpResponseFfi {
                id: req.id,
                status,
                body,
                etag: None,
                max_age_s: Some(300),
                retry_after_s: None,
                error: None,
                unit_random: 0.5,
            }
        };
        step = e.conditions_response(resp, now);
        queue.extend(step.requests.clone());
    }
    n
}

fn field<'a>(v: &'a ConditionsView, f: &str) -> &'a FieldView {
    v.fields.iter().find(|x| x.field == f).unwrap()
}

fn boulder(e: &SquibEngine) {
    e.set_place(PlaceInput { lat: 40.015, lon: -105.2705, label: Some("Practice bay".into()), source: "manual".into() }).unwrap();
}

#[test]
fn weather_lookup_is_off_until_enabled() {
    let e = engine("off");
    boulder(&e);
    assert!(!e.conditions_view(NOW).weather_enabled);
    assert!(e.conditions_refresh(NOW).is_err(), "no network request before the user enables lookup");
    // Manual values work without any lookup (offline first use).
    e.set_override("temperature".into(), 288.15, 15.0, "degC".into(), NOW).unwrap();
    let t = field(&e.conditions_view(NOW), "temperature").clone();
    assert_eq!(t.origin, "manual");
    assert_eq!(t.value_si, Some(288.15));
}

#[test]
fn refresh_resolves_fields_with_provenance() {
    let e = engine("refresh");
    e.set_weather_enabled(true).unwrap();
    boulder(&e);
    let requests = refresh(&e, NOW, false);
    assert_eq!(requests, 7, "points + stations + five observations");
    let v = e.conditions_view(NOW);
    let t = field(&v, "temperature");
    assert_eq!(t.origin, "nearby_observation");
    assert_eq!(t.station_id.as_deref(), Some("KBDU"));
    assert_eq!(t.age_ms, Some(35 * 60_000), "age from the 22:15Z observation");
    assert_eq!(t.freshness.as_deref(), Some("fresh"));
    assert_eq!(field(&v, "wind_direction").value_state, "calm");
    let p = field(&v, "local_pressure");
    assert_eq!(p.origin, "unavailable");
    assert!(p.reasons[0].contains("station values are not local pressure"));
    assert_eq!(field(&v, "altimeter_setting").origin, "nearby_observation");
    assert_eq!(v.attribution.len(), 1);
    // Device barometer fills local pressure; ellipsoid vs MSL is not compared.
    e.set_device_readings(DeviceReadings {
        barometer_hpa: Some(836.4),
        barometer_utc_ms: Some(NOW - 5_000),
        ellipsoid_m: Some(1605.0),
        ellipsoid_accuracy_m: Some(6.0),
        msl_m: Some(1621.0),
        msl_accuracy_m: None,
        location_utc_ms: Some(NOW - 5_000),
    });
    let v = e.conditions_view(NOW);
    assert_eq!(field(&v, "local_pressure").origin, "here");
    assert!(v.elevation_notes.iter().any(|n| n.contains("comparison unavailable")));
}

#[test]
fn a13_runs_pin_redacted_snapshots_that_later_refreshes_do_not_change() {
    let e = engine("pin");
    e.set_weather_enabled(true).unwrap();
    boulder(&e);
    refresh(&e, NOW, false);
    e.set_override("wind_speed".into(), 4.47, 10.0, "mph".into(), NOW).unwrap();
    let req = ArmRequest {
        mode: Mode::ParOnly,
        delay: StartDelay::Instant,
        unit_random: 0.0,
        pars_ms: vec![],
        auto_stop_grace_ms: None,
        auto_stop_count: None,
        expected_count: None,
        threshold_db: None,
        calibration_id: None,
        route: None,
        now_utc_ms: NOW,
        tz_offset_min: 0,
        app_build: "test".into(),
        expected_rate_hz: 48_000,
        drill_id: None,
        drill_version: None,
        plan_item_id: None,
        video: false,
        cue_channel: CueChannel::Sound,
        diagnostic_root: None,
    };
    let v = e.arm("a1".into(), req, 1_000_000_000).unwrap();
    let run_id = v.run_id.unwrap();
    let pinned = e.run_conditions(run_id.clone()).unwrap().expect("conditions pinned at arm");
    assert!(pinned.redacted, "default retention redacts");
    assert!(pinned.place.is_none());
    let t = field(&pinned, "temperature");
    assert_eq!(t.value_si, Some(299.15));
    assert_eq!(t.origin, "nearby_observation");
    assert_eq!(t.station_id, None, "station identity removed");
    assert_eq!(field(&pinned, "wind_speed").origin, "manual");
    // Changing the live conditions afterwards leaves the run's snapshot unchanged.
    e.clear_override("wind_speed".into()).unwrap();
    e.set_override("temperature".into(), 300.0, 300.0, "K".into(), NOW).unwrap();
    let again = e.run_conditions(run_id).unwrap().unwrap();
    assert_eq!(again, pinned);
}

#[test]
fn precise_retention_is_opt_in() {
    let e = engine("precise");
    e.set_weather_enabled(true).unwrap();
    e.set_precise_retention(true).unwrap();
    boulder(&e);
    refresh(&e, NOW, false);
    let req = ArmRequest {
        mode: Mode::ParOnly,
        delay: StartDelay::Instant,
        unit_random: 0.0,
        pars_ms: vec![],
        auto_stop_grace_ms: None,
        auto_stop_count: None,
        expected_count: None,
        threshold_db: None,
        calibration_id: None,
        route: None,
        now_utc_ms: NOW,
        tz_offset_min: 0,
        app_build: "test".into(),
        expected_rate_hz: 48_000,
        drill_id: None,
        drill_version: None,
        plan_item_id: None,
        video: false,
        cue_channel: CueChannel::Sound,
        diagnostic_root: None,
    };
    let run_id = e.arm("a1".into(), req, 1_000_000_000).unwrap().run_id.unwrap();
    let pinned = e.run_conditions(run_id).unwrap().unwrap();
    assert!(!pinned.redacted);
    assert_eq!(field(&pinned, "temperature").station_id.as_deref(), Some("KBDU"));
}

#[test]
fn offline_shows_cached_values_with_original_age_and_unsupported_is_explained() {
    let e = engine("offline");
    e.set_weather_enabled(true).unwrap();
    boulder(&e);
    refresh(&e, NOW, false);
    // Two hours later, offline: cached values return with their true (stale) age.
    let later = NOW + 2 * 3_600_000;
    refresh(&e, later, true);
    let v = e.conditions_view(later);
    let t = field(&v, "temperature");
    assert!(v.used_cache);
    assert_eq!(t.age_ms, Some(35 * 60_000 + 2 * 3_600_000));
    assert_eq!(t.freshness.as_deref(), Some("stale"));
    assert!(v.issues.iter().any(|i| i.contains("Could not reach")));
    // Clearing the cache leaves nothing to show offline.
    e.clear_weather_cache().unwrap();
    refresh(&e, later, true);
    assert_eq!(field(&e.conditions_view(later), "temperature").origin, "unavailable");

    // Outside the US, Auto continues with METAR airport reports after NWS declines.
    e.set_place(PlaceInput { lat: 51.5007, lon: -0.1246, label: None, source: "manual".into() }).unwrap();
    assert_eq!(refresh(&e, NOW_METAR, false), 2, "one NWS points request, then one METAR search");
    let v = e.conditions_view(NOW_METAR);
    assert_eq!(v.weather_provider, "auto");
    assert!(!v.issues.iter().any(|i| i.contains("no data for this place")), "{:?}", v.issues);
    let t = field(&v, "temperature");
    assert_eq!(t.origin, "nearby_observation");
    assert!(v.attribution.iter().any(|a| a.contains("aviationweather.gov")));
    assert_eq!(field(&v, "local_pressure").origin, "unavailable", "airport QNH is never local pressure");
    assert_eq!(field(&v, "altimeter_setting").origin, "nearby_observation");
}

#[test]
fn metar_everywhere_skips_nws_and_reports_empty_areas() {
    let e = engine("metar-only");
    e.set_weather_enabled(true).unwrap();
    assert!(e.set_weather_provider("other".into()).is_err());
    e.set_weather_provider("metar".into()).unwrap();
    boulder(&e);
    let step = e.conditions_refresh(NOW_METAR).unwrap();
    assert_eq!(step.requests.len(), 1);
    assert!(step.requests[0].url.starts_with("https://aviationweather.gov/api/data/metar?bbox=39.02,"));
    assert_eq!(step.requests[0].accept, "application/json");
    refresh(&e, NOW_METAR, false);
    let v = e.conditions_view(NOW_METAR);
    assert_eq!(v.weather_provider, "metar");
    assert!(v.issues.iter().any(|i| i.contains("No airport weather station")), "{:?}", v.issues);
    assert_eq!(field(&v, "temperature").origin, "unavailable");
}

#[test]
fn place_persistence_respects_retention() {
    let d = std::env::temp_dir().join(format!("squib-cond-persist-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let db = d.join("journal.db").to_string_lossy().to_string();
    {
        let e = SquibEngine::new(db.clone(), NOW).unwrap();
        e.set_place(PlaceInput { lat: 40.015, lon: -105.2705, label: Some("Bay".into()), source: "manual".into() }).unwrap();
    }
    let e = SquibEngine::new(db.clone(), NOW).unwrap();
    assert_eq!(e.conditions_view(NOW).place.unwrap().label.as_deref(), Some("Bay"), "entered place remembered");
    // A GPS place is not stored by default; the previous stored place is cleared too.
    e.set_place(PlaceInput { lat: 30.2672, lon: -97.7431, label: None, source: "gps".into() }).unwrap();
    drop(e);
    let e = SquibEngine::new(db.clone(), NOW).unwrap();
    assert!(e.conditions_view(NOW).place.is_none(), "GPS place not persisted without precise retention");
    e.set_precise_retention(true).unwrap();
    e.set_place(PlaceInput { lat: 30.2672, lon: -97.7431, label: None, source: "gps".into() }).unwrap();
    drop(e);
    let e = SquibEngine::new(db, NOW).unwrap();
    assert_eq!(e.conditions_view(NOW).place.unwrap().source, "gps");
}

#[test]
fn invalid_inputs_are_rejected() {
    let e = engine("invalid");
    assert!(e.set_place(PlaceInput { lat: 95.0, lon: 0.0, label: None, source: "manual".into() }).is_err());
    assert!(e.set_place(PlaceInput { lat: 10.0, lon: 0.0, label: None, source: "satellite".into() }).is_err());
    assert!(e.set_override("temperature".into(), 500.0, 500.0, "K".into(), NOW).is_err());
    assert!(e.set_override("not_a_field".into(), 1.0, 1.0, "x".into(), NOW).is_err());
    assert!(e.add_saved_place("Range".into(), 0.0, 200.0, NOW).is_err());
    let p = e.add_saved_place("  Club range ".into(), 40.0, -105.0, NOW).unwrap();
    assert_eq!(p.label, "Club range");
    assert_eq!(e.list_saved_places().len(), 1);
}

#[test]
fn a_transient_server_error_is_retried_after_a_delay_and_reported_as_a_retry() {
    let e = engine("retry");
    e.set_weather_enabled(true).unwrap();
    boulder(&e);
    let mut step = e.conditions_refresh(NOW).unwrap();
    let mut queue = step.requests.clone();
    let mut sent = vec![];
    while !step.done {
        let req = queue.remove(0);
        sent.push((req.url.clone(), req.delay_ms));
        let first_points = sent.iter().filter(|(u, _)| u.contains("/points/")).count() == 1 && req.url.contains("/points/");
        let resp = if first_points {
            HttpResponseFfi {
                id: req.id,
                status: 503,
                body: vec![],
                etag: None,
                max_age_s: None,
                retry_after_s: None,
                error: None,
                unit_random: 0.0,
            }
        } else {
            let (status, body) = serve(&req.url);
            HttpResponseFfi {
                id: req.id,
                status,
                body,
                etag: None,
                max_age_s: Some(300),
                retry_after_s: None,
                error: None,
                unit_random: 0.5,
            }
        };
        step = e.conditions_response(resp, NOW + 1_000);
        queue.extend(step.requests.clone());
    }
    assert_eq!(sent[0].1, 0);
    assert!(sent[1].0.contains("/points/") && sent[1].1 == 250, "{sent:?}");
    let v = e.conditions_view(NOW + 1_000);
    assert_eq!(v.retries, 1);
    assert!(v.issues.is_empty(), "{:?}", v.issues);
    assert_eq!(field(&v, "temperature").origin, "nearby_observation");
}

fn fx_epqs(name: &str) -> Vec<u8> {
    std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/epqs").join(name)).unwrap()
}

#[test]
fn terrain_is_opt_in_precise_only_while_on_and_never_holds_back_weather() {
    let dir = std::env::temp_dir().join(format!("squib-cond-terrain-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("journal.db").to_string_lossy().to_string();
    let e = SquibEngine::new(db.clone(), NOW).unwrap();
    e.set_weather_enabled(true).unwrap();
    // A place chosen before terrain was turned on has no 10 m point.
    e.set_place(PlaceInput { lat: 40.015049, lon: -105.270549, label: None, source: "manual".into() }).unwrap();
    e.set_terrain_enabled(true).unwrap();
    assert!(e.conditions_view(NOW).terrain_needs_place);
    e.set_place(PlaceInput { lat: 40.015049, lon: -105.270549, label: None, source: "manual".into() }).unwrap();
    let v = e.conditions_view(NOW);
    assert!(v.terrain_enabled && !v.terrain_needs_place);
    assert_eq!(v.place.as_ref().map(|p| (p.lat, p.lon)), Some((40.02, -105.27)), "weather still gets the 1 km place");

    let mut step = e.conditions_refresh(NOW).unwrap();
    let terrain_req =
        step.requests.iter().find(|r| r.url.starts_with("https://epqs.nationalmap.gov/")).cloned().expect("terrain requested");
    assert_eq!(
        terrain_req.url,
        "https://epqs.nationalmap.gov/v1/json?x=-105.2705&y=40.0150&wkid=4326&units=Meters&includeDate=true"
    );
    assert_eq!(terrain_req.timeout_ms, 30_000);
    // Weather completes first while the slow terrain request is still out.
    let mut queue: Vec<HttpRequestFfi> = step.requests.into_iter().filter(|r| r.id != terrain_req.id).collect();
    while let Some(req) = (!queue.is_empty()).then(|| queue.remove(0)) {
        let (status, body) = serve(&req.url);
        step = e.conditions_response(
            HttpResponseFfi {
                id: req.id,
                status,
                body,
                etag: None,
                max_age_s: Some(300),
                retry_after_s: None,
                error: None,
                unit_random: 0.5,
            },
            NOW + 1_000,
        );
        queue.extend(step.requests.clone());
    }
    assert!(!step.done, "not done while terrain is outstanding");
    let v = e.conditions_view(NOW + 1_000);
    assert_eq!(field(&v, "temperature").origin, "nearby_observation", "weather shown without waiting for terrain");
    assert!(v.refreshing);
    step = e.conditions_response(
        HttpResponseFfi {
            id: terrain_req.id,
            status: 200,
            body: fx_epqs("boulder_lidar_1m.json"),
            etag: None,
            max_age_s: None,
            retry_after_s: None,
            error: None,
            unit_random: 0.5,
        },
        NOW + 20_000,
    );
    assert!(step.done && step.requests.is_empty());
    let v = e.conditions_view(NOW + 20_000);
    let t = field(&v, "elevation_terrain");
    assert_eq!((t.origin.as_str(), t.datum.as_deref()), ("terrain", Some("navd88")));
    assert!((t.value_si.unwrap() - 1621.26).abs() < 0.01);
    assert!(v.attribution.iter().any(|a| a.contains("U.S. Geological Survey")));

    // After a restart the point is remembered (a typed place) and the cache answers.
    let e2 = SquibEngine::new(db.clone(), NOW + 30_000).unwrap();
    let step = e2.conditions_refresh(NOW + 30_000).unwrap();
    assert!(step.requests.iter().all(|r| !r.url.contains("epqs")), "terrain served from cache");
    assert!(field(&e2.conditions_view(NOW + 30_000), "elevation_terrain").value_si.is_some());

    // Turning terrain off forgets the 10 m point and the value; nothing is requested.
    e2.set_terrain_enabled(false).unwrap();
    let v = e2.conditions_view(NOW + 30_000);
    assert_eq!(field(&v, "elevation_terrain").origin, "unavailable");
    let e3 = SquibEngine::new(db, NOW + 40_000).unwrap();
    e3.set_terrain_enabled(true).unwrap();
    assert!(e3.conditions_view(NOW + 40_000).terrain_needs_place, "point was not kept after turning terrain off");
}

#[test]
fn terrain_alone_works_without_weather_lookup() {
    let e = engine("terrain-only");
    e.set_terrain_enabled(true).unwrap();
    assert!(e.conditions_refresh(NOW).is_err(), "no place yet");
    e.set_place(PlaceInput { lat: 51.5007, lon: -0.1246, label: None, source: "manual".into() }).unwrap();
    let step = e.conditions_refresh(NOW).unwrap();
    assert_eq!(step.requests.len(), 1, "only terrain, weather is off");
    let id = step.requests[0].id;
    let step = e.conditions_response(
        HttpResponseFfi {
            id,
            status: 200,
            body: fx_epqs("london_outside.txt"),
            etag: None,
            max_age_s: None,
            retry_after_s: None,
            error: None,
            unit_random: 0.5,
        },
        NOW + 27_000,
    );
    assert!(step.done);
    let v = e.conditions_view(NOW + 27_000);
    assert!(v.issues.iter().any(|i| i.contains("does not cover this place")), "{:?}", v.issues);
    e.set_terrain_enabled(false).unwrap();
    assert!(e.conditions_refresh(NOW).is_err(), "both lookups off");
}

#[test]
fn derived_metrics_need_local_pressure_and_name_their_inputs() {
    let e = engine("derived");
    e.set_weather_enabled(true).unwrap();
    boulder(&e);
    refresh(&e, NOW, false);
    // Stations give temperature and humidity but never pressure here.
    let v = e.conditions_view(NOW);
    assert!(v.derived.is_none());
    assert_eq!(v.derived_unavailable.as_deref(), Some("Needs pressure here (actual)."));
    // With actual pressure entered (an altimeter setting would not count), it derives.
    e.set_override("local_pressure".into(), 83_500.0, 835.0, "hPa".into(), NOW).unwrap();
    let v = e.conditions_view(NOW);
    let d = v.derived.expect("derived");
    assert_eq!(d.method, "moist-ideal-ae96-isa-v1");
    assert!(d.density_kg_m3 > 0.8 && d.density_kg_m3 < 1.1, "{}", d.density_kg_m3);
    assert!(d.density_altitude_m > 1_000.0 && d.density_altitude_m < 3_500.0, "{}", d.density_altitude_m);
    assert!(d.inputs.iter().any(|i| i.starts_with("Pressure here (actual): Your entry")), "{:?}", d.inputs);
    assert!(d.inputs.iter().any(|i| i.starts_with("Temperature: K")), "station named: {:?}", d.inputs);
    assert!(v.derived_unavailable.is_none());
}
