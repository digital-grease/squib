//! Plan phase 3.3: USGS EPQS terrain elevation, against responses captured from the live
//! service on 2026-10-10 (fixtures/epqs).

use std::collections::HashMap;
use std::path::PathBuf;

use squib_environment::nws::{CachedDoc, DocCache, HttpResponse, ProviderIssue, Step, TransportError};
use squib_environment::terrain::{self, TerrainLookup};
use squib_environment::*;

const NOW: i64 = 1_791_633_600_000;
const BOULDER: (f64, f64) = (40.015, -105.2705);

fn fx(name: &str) -> Vec<u8> {
    std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/epqs").join(name)).unwrap()
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

fn resp(id: u32, status: u16, body: Vec<u8>, error: Option<TransportError>) -> HttpResponse {
    HttpResponse { id, status, body, etag: None, max_age_s: None, retry_after_s: None, error, unit_random: 0.5 }
}

#[test]
fn captured_responses_parse_with_their_datum_and_dataset() {
    let c = terrain::parse_point(200, &fx("boulder_lidar_1m.json"), BOULDER, NOW).unwrap();
    assert_eq!(c.field, Field::ElevationTerrain);
    assert!((c.value.si().unwrap() - 1621.260986328).abs() < 1e-6);
    assert_eq!(c.datum, Some(VerticalDatum::Navd88));
    assert_eq!(c.observed_utc_ms, None, "a dataset value has no observation time");
    assert_eq!(c.dataset.as_deref(), Some("3DEP raster 71543, resolution 1 m, acquired 2/6/2021"));
    assert_eq!(c.accuracy, Some(0.53));
    let ne = terrain::parse_point(200, &fx("nebraska.json"), (40.0, -100.0), NOW).unwrap();
    assert!((ne.value.si().unwrap() - 688.633361816).abs() < 1e-6);
    // Hawaii is 3DEP but not NAVD88: the datum is unknown, not assumed.
    let hi = terrain::parse_point(200, &fx("hilo_hawaii.json"), (19.72, -155.09), NOW).unwrap();
    assert_eq!(hi.datum, Some(VerticalDatum::Unknown));
    // Outside coverage the service answers 200 with plain text; open water reads 0.
    assert_eq!(
        terrain::parse_point(200, &fx("london_outside.txt"), (51.5007, -0.1246), NOW),
        Err(ProviderIssue::TerrainNotCovered)
    );
    assert_eq!(terrain::parse_point(200, &fx("pacific_ocean.json"), (40.0, -124.9), NOW), Err(ProviderIssue::TerrainNoData));
    // The service gives the same text for a malformed request; Squib only builds valid
    // URLs, so this reads as not covered.
    assert_eq!(terrain::parse_point(200, &fx("bad_request.txt"), BOULDER, NOW), Err(ProviderIssue::TerrainNotCovered));
    assert!(matches!(terrain::parse_point(200, b"{\"value\":\"abc\"}", BOULDER, NOW), Err(ProviderIssue::Malformed { .. })));
    assert!(matches!(terrain::parse_point(200, b"{\"value\":\"-1000000\"}", BOULDER, NOW), Err(ProviderIssue::TerrainNoData)));
    assert!(matches!(terrain::parse_point(502, b"", BOULDER, NOW), Err(ProviderIssue::Http { status: 502, .. })));
}

#[test]
fn the_seamless_dem_reports_resolution_in_degrees() {
    let body =
        br#"{"value":"5.25","rasterId":4931,"resolution":0.00009259259341472414,"attributes":{"AcquisitionDate":"3/0/2024"}}"#;
    let c = terrain::parse_point(200, body, (40.0, -124.0), NOW).unwrap();
    assert_eq!(c.dataset.as_deref(), Some("3DEP raster 4931, resolution 1/3 arc-second (about 10 m), acquired 3/0/2024"));
}

#[test]
fn place_precision_url_and_origin_rules() {
    assert_eq!(terrain::lookup_coords(40.015_049, -105.270_549), Some((40.015, -105.2705)));
    assert_eq!(terrain::lookup_coords(f64::NAN, 0.0), None);
    let url = terrain::point_url((40.015, -105.2705));
    assert_eq!(url, "https://epqs.nationalmap.gov/v1/json?x=-105.2705&y=40.0150&wkid=4326&units=Meters&includeDate=true");
    assert!(terrain::url_allowed(&url));
    assert!(!terrain::url_allowed("https://epqs.nationalmap.gov.evil.example/v1/json?x=1"));
    assert!(!terrain::url_allowed("http://epqs.nationalmap.gov/v1/json?x=1"));
    assert!(!terrain::url_allowed("https://epqs.nationalmap.gov/v1/other"));
    assert_eq!(terrain::datum_for((61.2, -149.9)), VerticalDatum::Unknown, "Alaska");
    assert_eq!(terrain::datum_for((18.4, -66.1)), VerticalDatum::Unknown, "Puerto Rico");
    assert_eq!(terrain::datum_for((25.8, -80.2)), VerticalDatum::Navd88, "Miami");
    assert!(terrain::cache_key((40.015, -105.2705)).contains(":q4:40.0150,-105.2705:navd88"));
}

#[test]
fn lookup_caches_retries_and_falls_back_to_an_older_value() {
    let mut cache = MemCache::default();
    let (mut t, step) = TerrainLookup::start(BOULDER.0, BOULDER.1, NOW, 10_000, &cache).unwrap();
    let Step::Fetch(reqs) = step else { panic!() };
    assert_eq!((reqs[0].id, reqs[0].timeout_ms), (10_000, terrain::REQUEST_TIMEOUT_MS));
    // A timeout is retried with a new id; the slow first reply is then ignored.
    let Step::Fetch(retry) = t.on_response(resp(10_000, 0, vec![], Some(TransportError::Timeout)), NOW + 30_000, &mut cache)
    else {
        panic!()
    };
    assert_eq!(retry[0].id, 10_001);
    assert_eq!(
        t.on_response(resp(10_000, 200, fx("boulder_lidar_1m.json"), None), NOW + 31_000, &mut cache),
        Step::Fetch(vec![])
    );
    let Step::Done(res) = t.on_response(resp(10_001, 200, fx("boulder_lidar_1m.json"), None), NOW + 50_000, &mut cache) else {
        panic!()
    };
    assert_eq!((res.candidates.len(), res.retries, res.used_cache), (1, 1, false));

    // Within 180 days the cache answers without a request.
    let (_, step) = TerrainLookup::start(BOULDER.0, BOULDER.1, NOW + 86_400_000, 10_000, &cache).unwrap();
    let Step::Done(res) = step else { panic!("served from cache") };
    assert!(res.used_cache && res.candidates[0].value.si().is_some());

    // After expiry a refetch happens; offline then uses the older value instead of nothing.
    let later = NOW + 50_000 + terrain::CACHE_TTL_MS + 1;
    let (mut t, step) = TerrainLookup::start(BOULDER.0, BOULDER.1, later, 10_000, &cache).unwrap();
    let Step::Fetch(reqs) = step else { panic!("expired") };
    let Step::Done(res) = t.on_response(resp(reqs[0].id, 0, vec![], Some(TransportError::Offline)), later + 10, &mut cache)
    else {
        panic!()
    };
    assert!(res.used_cache && res.candidates.len() == 1);
    assert!(matches!(res.issues[0], ProviderIssue::Transport { .. }));

    // Outside coverage is reported as such, not filled from elsewhere.
    let (mut t, step) = TerrainLookup::start(51.5007, -0.1246, NOW, 10_000, &cache).unwrap();
    let Step::Fetch(reqs) = step else { panic!() };
    let Step::Done(res) = t.on_response(resp(reqs[0].id, 200, fx("london_outside.txt"), None), NOW + 27_000, &mut cache) else {
        panic!()
    };
    assert_eq!(res.issues, vec![ProviderIssue::TerrainNotCovered]);
    assert!(res.candidates.is_empty());
}

#[test]
fn terrain_is_its_own_field_compared_only_on_the_same_datum_and_redacted() {
    let t = terrain::parse_point(200, &fx("boulder_lidar_1m.json"), BOULDER, NOW).unwrap();
    let phone_msl = local::location_heights(None, None, Some(1625.0), Some(4.0), NOW).remove(0);
    let snap = resolve("s1", &[t.clone(), phone_msl.clone()], &[], None, vec![], &ResolverPolicy::default(), NOW);
    let f = snap.field(Field::ElevationTerrain).unwrap();
    assert_eq!(f.origin, Origin::Terrain);
    assert_eq!(f.reasons, vec![Reason::TerrainDataset]);
    assert_eq!(snap.field(Field::ElevationMsl).unwrap().origin, Origin::Here, "terrain never fills handset height");
    // NAVD88 against the phone's mean sea level: different references, no difference shown.
    assert!(snap.elevation_comparisons.iter().any(|c| matches!(c, ElevationComparison::DatumMismatch { .. })));
    assert!(!snap.elevation_comparisons.iter().any(|c| matches!(c, ElevationComparison::Compatible { .. })));
    // A second NAVD88 value is comparable.
    let mut other = t.clone();
    other.id = "other".into();
    other.value = Value::Si(1620.0);
    assert!(matches!(compare_elevations(&t, &other), Some(ElevationComparison::Compatible { .. })));
    // Redacted history keeps no terrain value (it reveals the place).
    let r = privacy::redact(&snap);
    assert!(r.field(Field::ElevationTerrain).unwrap().chosen.is_none());
    assert_eq!(attribution_for(terrain::PROVIDER), Some(terrain::ATTRIBUTION));
}
