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

fn serve(url: &str) -> (u16, Vec<u8>) {
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
        assert!(req.url.starts_with("https://api.weather.gov/"));
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
            }
        } else {
            let (status, body) = serve(&req.url);
            HttpResponseFfi { id: req.id, status, body, etag: None, max_age_s: Some(300), retry_after_s: None, error: None }
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
        expected_count: None,
        threshold_db: None,
        calibration_id: None,
        route: None,
        now_utc_ms: NOW,
        tz_offset_min: 0,
        app_build: "test".into(),
        expected_rate_hz: 48_000,
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
        expected_count: None,
        threshold_db: None,
        calibration_id: None,
        route: None,
        now_utc_ms: NOW,
        tz_offset_min: 0,
        app_build: "test".into(),
        expected_rate_hz: 48_000,
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

    e.set_place(PlaceInput { lat: 51.5007, lon: -0.1246, label: None, source: "manual".into() }).unwrap();
    refresh(&e, NOW, false);
    let v = e.conditions_view(NOW);
    assert!(v.issues[0].contains("no data for this place"));
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
