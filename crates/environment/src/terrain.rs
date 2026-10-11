//! Terrain (ground) elevation from the USGS Elevation Point Query Service (plan phase
//! 3.3, ADR-049). Opt-in, US only.
//!
//! Facts checked against the live service and USGS documentation on 2026-10-10:
//! - Values are interpolated from the 3D Elevation Program (1 m lidar where available,
//!   otherwise the 1/3 arc-second seamless DEM). "All 3DEP products are public domain."
//! - Vertical datum: NAVD88 over the conterminous United States; USGS says the reference
//!   "will vary" elsewhere (Hawaii, Puerto Rico, Alaska), so those are `Unknown` here.
//! - USGS states an overall RMSE of 0.53 m; this is the service's, not a per-point bound.
//! - The value is a JSON string; `resolution` is metres for lidar tiles and degrees for
//!   the seamless DEM; outside coverage the service answers 200 with a plain-text
//!   "Call failed" body; open water returns exactly 0; responses can take 15 to 30 s.
//!
//! The place is sent to about 10 m (four decimal places), unlike weather lookups, which is
//! why this has its own switch. Terrain is ground height, not handset height (docs/05).

use serde::Deserialize;

use crate::candidate::{Field, MeasurementCandidate, SourceKind, Value, VerticalDatum};
use crate::nws::{CachedDoc, DocCache, HttpRequest, HttpResponse, ProviderIssue, RefreshResult, Step};
use crate::units::{Plausibility, plausibility};

pub const PROVIDER: &str = "usgs_epqs";
pub const ATTRIBUTION: &str = "Terrain elevation: U.S. Geological Survey, 3D Elevation Program (public domain)";
pub const ORIGIN: &str = "https://epqs.nationalmap.gov";
pub const ACCEPT: &str = "application/json";
pub const MAX_BODY_BYTES: usize = 16 * 1024;
pub const ADAPTER_VERSION: &str = "usgs-epqs-v1";
/// The service can take 15 to 30 s; terrain has its own budget, apart from weather.
pub const DEADLINE_MS: i64 = 60_000;
pub const REQUEST_TIMEOUT_MS: u32 = 30_000;
/// Terrain changes rarely, but datasets are updated; refetch after this.
pub const CACHE_TTL_MS: i64 = 180 * 24 * 3_600_000;
/// USGS's stated overall accuracy of the service.
pub const SERVICE_RMSE_M: f64 = 0.53;

/// About 10 m: four decimal places of latitude and longitude.
pub fn lookup_coords(lat: f64, lon: f64) -> Option<(f64, f64)> {
    if !lat.is_finite() || !lon.is_finite() || !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    let r = |v: f64| (v * 10_000.0).round() / 10_000.0;
    Some((r(lat), r(lon)))
}

/// Conterminous United States, by bounding box. Inside it 3DEP is NAVD88; outside it
/// (Alaska, Hawaii, territories) the datum is not stated, so it is treated as unknown.
pub fn datum_for(lookup: (f64, f64)) -> VerticalDatum {
    let (lat, lon) = lookup;
    if (24.4..=49.4).contains(&lat) && (-124.9..=-66.8).contains(&lon) { VerticalDatum::Navd88 } else { VerticalDatum::Unknown }
}

pub fn point_url(lookup: (f64, f64)) -> String {
    format!("{ORIGIN}/v1/json?x={:.4}&y={:.4}&wkid=4326&units=Meters&includeDate=true", lookup.1, lookup.0)
}

pub fn url_allowed(url: &str) -> bool {
    url.strip_prefix(ORIGIN).is_some_and(|rest| rest.starts_with("/v1/json?") && !rest.contains('@'))
}

pub fn cache_key(lookup: (f64, f64)) -> String {
    let datum = match datum_for(lookup) {
        VerticalDatum::Navd88 => "navd88",
        _ => "unknown",
    };
    format!("terrain:{ADAPTER_VERSION}:q4:{:.4},{:.4}:{datum}", lookup.0, lookup.1)
}

#[derive(Deserialize)]
struct Point {
    value: serde_json::Value,
    #[serde(rename = "rasterId")]
    raster_id: Option<i64>,
    resolution: Option<f64>,
    attributes: Option<Attributes>,
}

#[derive(Deserialize)]
struct Attributes {
    #[serde(rename = "AcquisitionDate")]
    acquisition_date: Option<String>,
}

/// Lidar tiles report resolution in metres, the seamless DEM in degrees.
fn resolution_text(r: f64) -> String {
    if r >= 0.5 {
        format!("{r:.0} m")
    } else {
        let arcsec = r * 3_600.0;
        if (arcsec - 1.0 / 3.0).abs() < 0.01 { "1/3 arc-second (about 10 m)".into() } else { format!("{arcsec:.2} arc-second") }
    }
}

/// Parse one point response. The acquisition date is kept verbatim: the service has been
/// seen to return impossible dates such as "3/0/2024".
pub fn parse_point(status: u16, body: &[u8], lookup: (f64, f64), now_utc_ms: i64) -> Result<MeasurementCandidate, ProviderIssue> {
    if status != 200 {
        return Err(ProviderIssue::Http { url_kind: "terrain".into(), status });
    }
    let text = std::str::from_utf8(body).unwrap_or("");
    if text.trim_start().starts_with("Call failed") {
        return Err(ProviderIssue::TerrainNotCovered);
    }
    let p: Point = serde_json::from_slice(body)
        .map_err(|e| ProviderIssue::Malformed { url_kind: "terrain".into(), detail: e.to_string().chars().take(80).collect() })?;
    let v = match &p.value {
        serde_json::Value::String(s) => s.trim().parse::<f64>().ok(),
        serde_json::Value::Number(n) => n.as_f64(),
        _ => None,
    }
    .filter(|v| v.is_finite())
    .ok_or_else(|| ProviderIssue::Malformed { url_kind: "terrain".into(), detail: "value is not a number".into() })?;
    // Open water reads exactly 0; the documented no-data value is -1000000.
    if v == 0.0 || v <= -1_000.0 {
        return Err(ProviderIssue::TerrainNoData);
    }
    if plausibility(Field::ElevationTerrain, v) == Plausibility::Invalid {
        return Err(ProviderIssue::Malformed { url_kind: "terrain".into(), detail: format!("implausible elevation {v}") });
    }
    let mut dataset = format!("3DEP raster {}", p.raster_id.map(|r| r.to_string()).unwrap_or_else(|| "unknown".into()));
    if let Some(r) = p.resolution.filter(|r| r.is_finite() && *r > 0.0) {
        dataset += &format!(", resolution {}", resolution_text(r));
    }
    if let Some(d) = p.attributes.and_then(|a| a.acquisition_date).filter(|d| !d.trim().is_empty()) {
        dataset += &format!(", acquired {}", d.chars().take(20).collect::<String>());
    }
    Ok(MeasurementCandidate {
        id: format!("{PROVIDER}:elevation_terrain:{}", p.raster_id.unwrap_or(0)),
        field: Field::ElevationTerrain,
        value: Value::Si(v),
        original_value: Some(v),
        original_unit: Some("m".into()),
        source: SourceKind::TerrainDataset,
        provider: PROVIDER.into(),
        station: None,
        // A dataset value has no observation time.
        observed_utc_ms: None,
        fetched_utc_ms: now_utc_ms,
        qc: None,
        datum: Some(datum_for(lookup)),
        accuracy: Some(SERVICE_RMSE_M),
        accuracy_meaning: Some("USGS stated overall RMSE of the service, not a bound for this point".into()),
        transforms: vec!["string->m".into()],
        adapter_version: ADAPTER_VERSION.into(),
        dataset: Some(dataset),
    })
}

/// One terrain lookup: cache first, then one request, retried under [`crate::retry`].
#[derive(Debug, Clone)]
pub struct TerrainLookup {
    lookup: (f64, f64),
    started_utc_ms: i64,
    request_id: u32,
    attempt: u32,
    retries: u32,
    done: bool,
}

impl TerrainLookup {
    /// `first_id` keeps request ids apart from weather requests in the same refresh.
    pub fn start(lat: f64, lon: f64, now_utc_ms: i64, first_id: u32, cache: &dyn DocCache) -> Option<(Self, Step)> {
        let lookup = lookup_coords(lat, lon)?;
        let mut r = Self { lookup, started_utc_ms: now_utc_ms, request_id: first_id, attempt: 1, retries: 0, done: false };
        if let Some(doc) = cache.get(&cache_key(lookup)).filter(|d| d.expires_utc_ms > now_utc_ms)
            && let Ok(c) = parse_point(200, &doc.body, lookup, doc.fetched_utc_ms)
        {
            r.done = true;
            return Some((r, Step::Done(RefreshResult { candidates: vec![c], used_cache: true, ..empty() })));
        }
        let req = r.request(0);
        Some((r, Step::Fetch(vec![req])))
    }

    fn request(&self, delay_ms: u32) -> HttpRequest {
        HttpRequest {
            id: self.request_id,
            url: point_url(self.lookup),
            if_none_match: None,
            delay_ms,
            timeout_ms: REQUEST_TIMEOUT_MS,
        }
    }

    pub fn owns(&self, id: u32) -> bool {
        !self.done && id == self.request_id
    }

    /// An earlier value (even expired) when this lookup fails: terrain rarely changes.
    fn cached(&self, cache: &dyn DocCache, mut res: RefreshResult) -> RefreshResult {
        if let Some(doc) = cache.get(&cache_key(self.lookup))
            && let Ok(c) = parse_point(200, &doc.body, self.lookup, doc.fetched_utc_ms)
        {
            res.used_cache = true;
            res.candidates = vec![c];
        }
        res
    }

    pub fn on_response(&mut self, resp: HttpResponse, now_utc_ms: i64, cache: &mut dyn DocCache) -> Step {
        if !self.owns(resp.id) {
            return Step::Fetch(vec![]);
        }
        let elapsed = now_utc_ms - self.started_utc_ms;
        if elapsed <= DEADLINE_MS
            && let Some(delay) = crate::retry::next_delay_ms(&resp, self.attempt, elapsed, DEADLINE_MS, resp.unit_random)
        {
            self.request_id += 1;
            self.attempt += 1;
            self.retries += 1;
            return Step::Fetch(vec![self.request(delay)]);
        }
        self.done = true;
        let mut res = RefreshResult { retries: self.retries, ..empty() };
        if elapsed > DEADLINE_MS {
            res.issues.push(ProviderIssue::DeadlineExceeded);
            return Step::Done(self.cached(cache, res));
        }
        if let Some(e) = &resp.error {
            res.issues.push(ProviderIssue::Transport { url_kind: "terrain".into(), detail: format!("{e:?}") });
            return Step::Done(self.cached(cache, res));
        }
        if resp.body.len() > MAX_BODY_BYTES {
            res.issues.push(ProviderIssue::Malformed { url_kind: "terrain".into(), detail: "body too large".into() });
            return Step::Done(self.cached(cache, res));
        }
        match parse_point(resp.status, &resp.body, self.lookup, now_utc_ms) {
            Ok(c) => {
                cache.put(CachedDoc {
                    key: cache_key(self.lookup),
                    body: resp.body,
                    etag: None,
                    fetched_utc_ms: now_utc_ms,
                    expires_utc_ms: now_utc_ms + CACHE_TTL_MS,
                });
                res.candidates = vec![c];
                Step::Done(res)
            }
            // Coverage answers are facts about the place, not failures to paper over.
            Err(e @ (ProviderIssue::TerrainNotCovered | ProviderIssue::TerrainNoData)) => {
                res.issues.push(e);
                Step::Done(res)
            }
            Err(e) => {
                res.issues.push(e);
                Step::Done(self.cached(cache, res))
            }
        }
    }
}

fn empty() -> RefreshResult {
    RefreshResult { candidates: vec![], issues: vec![], stations: vec![], used_cache: false, retries: 0 }
}
