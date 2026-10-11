//! Plan phase 3.5: NWS alerts against responses captured from api.weather.gov on
//! 2026-10-11 (fixtures/nws/alerts_*), and the NWS heat index against the NWS heat index
//! chart (weather.gov/safety/heat-index, image heatindexchart-650.jpg, read 2026-10-11).

use std::collections::HashMap;
use std::path::PathBuf;

use squib_environment::alerts::{self, AlertsLookup, AlertsStep};
use squib_environment::heat::{self, HeatCategory};
use squib_environment::nws::{CachedDoc, DocCache, HttpResponse, ProviderIssue, TransportError};

/// 2026-10-11T05:00Z: the Helena advisory (ends 2026-10-12T12:00Z) is in force.
const NOW: i64 = 1_791_694_800_000;

fn fx(name: &str) -> Vec<u8> {
    std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/nws").join(name)).unwrap()
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
fn captured_alerts_parse_and_only_real_current_ones_show() {
    let a = alerts::parse_alerts(200, &fx("alerts_helena.json"), NOW).unwrap().unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].event, "Winter Weather Advisory");
    assert_eq!(a[0].severity, "Moderate");
    assert_eq!(a[0].sender, "NWS Great Falls MT");
    // "Until" is when the hazard ends (ends), not when the message expires.
    assert_eq!(a[0].until_utc_ms, Some(1_791_806_400_000));
    assert!(a[0].instruction.is_some() && !a[0].description.is_empty());
    assert_eq!(alerts::parse_alerts(200, &fx("alerts_boulder_none.json"), NOW).unwrap(), Some(vec![]));
    assert_eq!(alerts::parse_alerts(400, &fx("alerts_out_of_bounds.json"), NOW).unwrap(), None, "outside the US");
    assert_eq!(alerts::parse_alerts(200, &fx("alerts_test_message.json"), 0).unwrap(), Some(vec![]), "test messages never shown");
    // After the hazard ends it is no longer shown, even from a saved copy.
    assert_eq!(alerts::parse_alerts(200, &fx("alerts_helena.json"), 1_791_806_400_001).unwrap(), Some(vec![]));
    // A cancellation is not an active alert.
    let cancel = String::from_utf8(fx("alerts_helena.json"))
        .unwrap()
        .replace("\"messageType\": \"Update\"", "\"messageType\": \"Cancel\"");
    assert_eq!(alerts::parse_alerts(200, cancel.as_bytes(), NOW).unwrap(), Some(vec![]));
    assert!(matches!(alerts::parse_alerts(200, b"{", NOW), Err(ProviderIssue::Malformed { .. })));
    assert_eq!(alerts::alerts_url((46.59, -112.04)), "https://api.weather.gov/alerts/active?point=46.59,-112.04");
}

fn resp(id: u32, status: u16, body: Vec<u8>, error: Option<TransportError>) -> HttpResponse {
    HttpResponse { id, status, body, etag: None, max_age_s: Some(5), retry_after_s: None, error, unit_random: 0.5 }
}

#[test]
fn lookup_uses_the_rounded_place_retries_and_shows_saved_alerts_with_their_age_offline() {
    let mut cache = MemCache::default();
    let (mut l, step) = AlertsLookup::start(46.5912, -112.0391, NOW, 20_000, 2_000_000).unwrap();
    let AlertsStep::Fetch(reqs) = step else { panic!() };
    assert_eq!(reqs[0].url, "https://api.weather.gov/alerts/active?point=46.59,-112.04", "about 1 km, like weather");
    let AlertsStep::Fetch(retry) = l.on_response(resp(2_000_000, 503, vec![], None), NOW + 500, &mut cache) else { panic!() };
    let AlertsStep::Done(r) = l.on_response(resp(retry[0].id, 200, fx("alerts_helena.json"), None), NOW + 1_000, &mut cache)
    else {
        panic!()
    };
    assert_eq!((r.alerts.as_ref().map(|a| a.len()), r.fetched_utc_ms, r.retries), (Some(1), Some(NOW + 1_000), 1));
    // Later, offline: the saved alerts are shown with their fetch time and the issue.
    let (mut l, step) = AlertsLookup::start(46.5912, -112.0391, NOW + 600_000, 20_000, 2_000_000).unwrap();
    let AlertsStep::Fetch(reqs) = step else { panic!() };
    let AlertsStep::Done(r) =
        l.on_response(resp(reqs[0].id, 0, vec![], Some(TransportError::Offline)), NOW + 600_100, &mut cache)
    else {
        panic!()
    };
    assert_eq!(r.fetched_utc_ms, Some(NOW + 1_000), "age is from the fetch, not now");
    assert_eq!(r.alerts.map(|a| a.len()), Some(1));
    assert!(matches!(r.issues[0], ProviderIssue::Transport { .. }));
}

#[test]
fn heat_index_matches_the_nws_chart() {
    // (temperature °F, RH %, chart value °F). The chart shows rounded values.
    let chart = [
        (80.0, 40.0, 80.0),
        (90.0, 40.0, 91.0),
        (90.0, 70.0, 105.0),
        (100.0, 40.0, 109.0),
        (100.0, 55.0, 124.0),
        (86.0, 90.0, 105.0),
        (110.0, 40.0, 136.0),
        (96.0, 65.0, 121.0),
        (88.0, 85.0, 110.0),
        (84.0, 60.0, 88.0),
        (104.0, 45.0, 124.0),
    ];
    for (t, rh, want) in chart {
        let got = heat::heat_index_f(t, rh);
        assert!((got.round() - want).abs() <= 1.0, "{t} °F {rh} %: {got:.1} vs chart {want}");
    }
    // In the humid corner (RH > 85 %, 80 to 87 °F) the chart shows the raw regression without
    // the high-humidity adjustment that the WPC algorithm adds; Squib follows the
    // algorithm, so it equals the chart plus that adjustment.
    for (t, rh, chart_value) in [(80.0, 100.0, 87.0), (82.0, 95.0, 93.0), (82.0, 100.0, 95.0), (80.0, 95.0, 86.0)] {
        let adjustment = ((rh - 85.0) / 10.0) * ((87.0 - t) / 5.0);
        let got = heat::heat_index_f(t, rh);
        assert!(((got - adjustment).round() - chart_value).abs() <= 1.0, "{t} °F {rh} %: {got:.1}");
        assert!(got > chart_value, "the adjustment raises the value");
    }
    // NWS safety page example: 96 °F, 65 % "feels like" 121 °F.
    assert_eq!(heat::heat_index_f(96.0, 65.0).round(), 121.0);
    // Below about 80 °F the simple formula applies and stays near the temperature.
    assert!((heat::heat_index_f(70.0, 50.0) - 69.0).abs() < 2.0);
    // Low humidity adjustment lowers, high humidity adjustment raises.
    assert!(heat::heat_index_f(100.0, 5.0) < heat::heat_index_f(100.0, 13.0));
    assert!(heat::heat_index_f(84.0, 95.0) > heat::heat_index_f(84.0, 85.0));
}

#[test]
fn categories_follow_the_chart_bands() {
    assert_eq!(HeatCategory::of(79.4), None);
    assert_eq!(HeatCategory::of(80.0), Some(HeatCategory::Caution));
    assert_eq!(HeatCategory::of(90.0), Some(HeatCategory::Caution));
    assert_eq!(HeatCategory::of(91.0), Some(HeatCategory::ExtremeCaution));
    assert_eq!(HeatCategory::of(103.0), Some(HeatCategory::Danger));
    assert_eq!(HeatCategory::of(124.0), Some(HeatCategory::Danger));
    assert_eq!(HeatCategory::of(125.0), Some(HeatCategory::ExtremeDanger));
}
