//! National Weather Service (api.weather.gov) adapter: URL policy, response parsing,
//! and a bounded fetch planner. Transport is native; this module is pure.
//!
//! Pressure semantics, verified against raw METAR on 2026-10-07 (KDFW, KORD, KSEA,
//! KBOS): the API's `barometricPressure` equals the METAR altimeter setting and
//! `seaLevelPressure` equals the METAR SLP group. NWS observations therefore carry no
//! actual station pressure; neither value may fill the local actual-pressure field.

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::candidate::{Field, MeasurementCandidate, SourceKind, StationRef, Value};
use crate::time::parse_rfc3339_ms;
use crate::units::{Plausibility, plausibility, to_si};

pub const PROVIDER: &str = "nws";
pub const ADAPTER_VERSION: &str = "nws-adapter-v1";
pub const ORIGIN: &str = "https://api.weather.gov";
/// Identifiable User-Agent as NWS requests. Contact is the public project page; no
/// personal data.
pub const USER_AGENT: &str = "Squib/0.1 (https://github.com/digital-grease/squib)";
pub const ACCEPT: &str = "application/geo+json";
/// Shown wherever NWS values are displayed (A25). NWS data is US-government public
/// domain; attribution is a courtesy and a provenance aid.
pub const ATTRIBUTION: &str = "Weather observations: National Weather Service (api.weather.gov)";
/// Responses larger than this are rejected before parsing.
pub const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
/// Station candidates inspected per refresh (resource budget, not an NWS limit).
pub const MAX_STATIONS: usize = 5;
/// Observation requests in flight at once.
pub const MAX_CONCURRENT: usize = 2;
/// Point and station metadata cache lifetime.
pub const METADATA_TTL_MS: i64 = 24 * 3_600_000;

/// Coordinates sent to the provider are rounded to 2 decimal places (about 1 km):
/// enough for weather, less precise than the device fix.
pub fn lookup_coords(lat: f64, lon: f64) -> Option<(f64, f64)> {
    if !lat.is_finite() || !lon.is_finite() || !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    let r = |v: f64| (v * 100.0).round() / 100.0;
    Some((r(lat), r(lon)))
}

/// Only HTTPS URLs on the NWS API origin are fetched, including links the API returns.
pub fn url_allowed(url: &str) -> bool {
    url.strip_prefix(ORIGIN).is_some_and(|rest| rest.starts_with('/') && !rest.starts_with("//") && !rest.contains('@'))
}

pub fn points_url(lat: f64, lon: f64) -> String {
    format!("{ORIGIN}/points/{lat:.2},{lon:.2}")
}

pub fn latest_observation_url(station_id: &str) -> Option<String> {
    let ok = !station_id.is_empty() && station_id.len() <= 16 && station_id.chars().all(|c| c.is_ascii_alphanumeric());
    ok.then(|| format!("{ORIGIN}/stations/{station_id}/observations/latest"))
}

/// Typed provider failures; never collapsed into one generic error (docs/squib/03).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderIssue {
    /// The provider has no data for this point (e.g. outside the US).
    UnsupportedLocation,
    /// Transport failed (offline, timeout, TLS); `detail` is adapter text.
    Transport {
        url_kind: String,
        detail: String,
    },
    /// HTTP status other than success.
    Http {
        url_kind: String,
        status: u16,
    },
    RateLimited {
        retry_after_s: Option<u32>,
    },
    /// Response body was malformed, truncated, or too large.
    Malformed {
        url_kind: String,
        detail: String,
    },
    /// A returned link pointed outside the allowed origin.
    DisallowedUrl {
        url: String,
    },
    /// A field value was rejected (unknown unit, impossible value).
    RejectedValue {
        station_id: String,
        field: Field,
        reason: String,
    },
    /// The refresh deadline passed before all requests finished.
    DeadlineExceeded,
    /// No station reported near the place recently (METAR search came back empty).
    NoStationsNearby,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PointInfo {
    pub stations_url: String,
    pub grid_id: Option<String>,
    pub time_zone: Option<String>,
}

fn problem_type(body: &Json) -> Option<&str> {
    body.get("type").and_then(|t| t.as_str())
}

pub fn parse_points(status: u16, body: &[u8]) -> Result<PointInfo, ProviderIssue> {
    let json: Json = serde_json::from_slice(body)
        .map_err(|e| ProviderIssue::Malformed { url_kind: "points".into(), detail: e.to_string() })?;
    if status == 404 && problem_type(&json).is_some_and(|t| t.ends_with("/InvalidPoint")) {
        return Err(ProviderIssue::UnsupportedLocation);
    }
    if status != 200 {
        return Err(ProviderIssue::Http { url_kind: "points".into(), status });
    }
    let p =
        json.get("properties").ok_or(ProviderIssue::Malformed { url_kind: "points".into(), detail: "no properties".into() })?;
    let stations_url = p
        .get("observationStations")
        .and_then(|v| v.as_str())
        .ok_or(ProviderIssue::Malformed { url_kind: "points".into(), detail: "no observationStations".into() })?
        .to_string();
    if !url_allowed(&stations_url) {
        return Err(ProviderIssue::DisallowedUrl { url: stations_url });
    }
    Ok(PointInfo {
        stations_url,
        grid_id: p.get("gridId").and_then(|v| v.as_str()).map(String::from),
        time_zone: p.get("timeZone").and_then(|v| v.as_str()).map(String::from),
    })
}

/// Great-circle distance in metres (haversine, mean Earth radius).
pub fn distance_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r = 6_371_008.8;
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dp = (lat2 - lat1).to_radians();
    let dl = (lon2 - lon1).to_radians();
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * r * a.sqrt().asin()
}

/// Parse a station collection; returns up to `MAX_STATIONS` stations in provider order
/// with distances recomputed from the lookup place.
pub fn parse_stations(status: u16, body: &[u8], lookup: (f64, f64)) -> Result<Vec<StationRef>, ProviderIssue> {
    if status != 200 {
        return Err(ProviderIssue::Http { url_kind: "stations".into(), status });
    }
    let json: Json = serde_json::from_slice(body)
        .map_err(|e| ProviderIssue::Malformed { url_kind: "stations".into(), detail: e.to_string() })?;
    let features = json
        .get("features")
        .and_then(|f| f.as_array())
        .ok_or(ProviderIssue::Malformed { url_kind: "stations".into(), detail: "no features".into() })?;
    let mut out = Vec::new();
    for f in features {
        let p = f.get("properties");
        let id = p.and_then(|p| p.get("stationIdentifier")).and_then(|v| v.as_str());
        let coords = f.get("geometry").and_then(|g| g.get("coordinates")).and_then(|c| c.as_array());
        let (Some(id), Some(c)) = (id, coords) else { continue };
        let (Some(lon), Some(lat)) = (c.first().and_then(|v| v.as_f64()), c.get(1).and_then(|v| v.as_f64())) else { continue };
        if latest_observation_url(id).is_none() || !lat.is_finite() || !lon.is_finite() {
            continue;
        }
        let elevation_m = p
            .and_then(|p| p.get("elevation"))
            .and_then(|e| Some((e.get("value")?.as_f64()?, e.get("unitCode")?.as_str()?.to_string())))
            .and_then(|(v, u)| to_si(Field::ElevationMsl, v, &u).ok().map(|x| x.0));
        out.push(StationRef {
            station_id: id.to_string(),
            name: p.and_then(|p| p.get("name")).and_then(|v| v.as_str()).unwrap_or("").chars().take(80).collect(),
            lat,
            lon,
            elevation_m,
            distance_m: Some(distance_m(lookup.0, lookup.1, lat, lon)),
        });
        if out.len() == MAX_STATIONS {
            break;
        }
    }
    Ok(out)
}

/// Parse a latest-observation document into candidates. Nulls stay missing; units are
/// read from each `unitCode`; QC codes are kept verbatim.
pub fn parse_observation(
    status: u16,
    body: &[u8],
    station: &StationRef,
    fetched_utc_ms: i64,
) -> Result<(Vec<MeasurementCandidate>, Vec<ProviderIssue>), ProviderIssue> {
    if status != 200 {
        return Err(ProviderIssue::Http { url_kind: "observation".into(), status });
    }
    let json: Json = serde_json::from_slice(body)
        .map_err(|e| ProviderIssue::Malformed { url_kind: "observation".into(), detail: e.to_string() })?;
    let p = json
        .get("properties")
        .ok_or(ProviderIssue::Malformed { url_kind: "observation".into(), detail: "no properties".into() })?;
    // Unknown observation time stays unknown; it is never replaced by fetch time.
    let observed = p.get("timestamp").and_then(|v| v.as_str()).and_then(parse_rfc3339_ms);
    let mut out = Vec::new();
    let mut issues = Vec::new();
    let qv = |key: &str| -> Option<(Option<f64>, String, Option<String>)> {
        let q = p.get(key)?;
        Some((
            q.get("value").and_then(|v| v.as_f64()),
            q.get("unitCode").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            q.get("qualityControl").and_then(|v| v.as_str()).map(String::from),
        ))
    };
    let make = |field: Field, key: &str, out: &mut Vec<MeasurementCandidate>, issues: &mut Vec<ProviderIssue>| -> Option<f64> {
        let (raw, unit, qc) = qv(key)?;
        let base = MeasurementCandidate {
            id: format!(
                "{PROVIDER}:{}:{}:{}",
                station.station_id,
                field.as_str(),
                observed.map(|t| t.to_string()).unwrap_or_else(|| "unknown".into())
            ),
            field,
            value: Value::Missing,
            original_value: raw,
            original_unit: Some(unit.clone()),
            source: SourceKind::StationObservation,
            provider: PROVIDER.into(),
            station: Some(station.clone()),
            observed_utc_ms: observed,
            fetched_utc_ms,
            qc,
            datum: None,
            accuracy: None,
            accuracy_meaning: None,
            transforms: vec![],
            adapter_version: ADAPTER_VERSION.into(),
        };
        let Some(raw) = raw else {
            out.push(base);
            return None;
        };
        match to_si(field, raw, &unit) {
            Ok((si, transform)) => {
                if plausibility(field, si) == Plausibility::Invalid {
                    issues.push(ProviderIssue::RejectedValue {
                        station_id: station.station_id.clone(),
                        field,
                        reason: format!("implausible {si}"),
                    });
                    return None;
                }
                let mut c = base;
                c.value = Value::Si(si);
                c.transforms = transform.into_iter().collect();
                out.push(c);
                Some(si)
            }
            Err(e) => {
                issues.push(ProviderIssue::RejectedValue {
                    station_id: station.station_id.clone(),
                    field,
                    reason: format!("{e:?}"),
                });
                None
            }
        }
    };
    make(Field::Temperature, "temperature", &mut out, &mut issues);
    make(Field::RelativeHumidity, "relativeHumidity", &mut out, &mut issues);
    let speed = make(Field::WindSpeed, "windSpeed", &mut out, &mut issues);
    make(Field::WindGust, "windGust", &mut out, &mut issues);
    make(Field::WindDirection, "windDirection", &mut out, &mut issues);
    make(Field::AltimeterSetting, "barometricPressure", &mut out, &mut issues);
    make(Field::SeaLevelPressure, "seaLevelPressure", &mut out, &mut issues);
    // Calm wind reports direction 0: that is "no direction", not north.
    if speed == Some(0.0)
        && let Some(d) = out.iter_mut().find(|c| c.field == Field::WindDirection)
    {
        d.value = Value::Calm;
        d.transforms.push("calm:speed_zero".into());
    }
    Ok((out, issues))
}

// ---- Fetch planner -----------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HttpRequest {
    pub id: u32,
    pub url: String,
    /// Cache validator from an earlier response, when known.
    pub if_none_match: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TransportError {
    Offline,
    Timeout,
    TooLarge,
    Tls,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HttpResponse {
    pub id: u32,
    pub status: u16,
    pub body: Vec<u8>,
    pub etag: Option<String>,
    pub max_age_s: Option<u32>,
    pub retry_after_s: Option<u32>,
    pub error: Option<TransportError>,
}

/// Cached provider document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedDoc {
    pub key: String,
    pub body: Vec<u8>,
    pub etag: Option<String>,
    pub fetched_utc_ms: i64,
    pub expires_utc_ms: i64,
}

/// Cache access supplied by the engine (backed by SQLite).
pub trait DocCache {
    fn get(&self, key: &str) -> Option<CachedDoc>;
    fn put(&mut self, doc: CachedDoc);
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RefreshResult {
    pub candidates: Vec<MeasurementCandidate>,
    pub issues: Vec<ProviderIssue>,
    pub stations: Vec<StationRef>,
    /// True when at least one value came from the cache instead of a fresh response.
    pub used_cache: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Perform these requests (at most `MAX_CONCURRENT` outstanding), then call `on_response`.
    Fetch(Vec<HttpRequest>),
    Done(RefreshResult),
}

#[derive(Debug, Clone, PartialEq)]
enum Phase {
    Points,
    Stations(String),
    Observations,
    Done,
}

/// Bounded refresh: points → stations → up to five latest observations, two at a time.
/// No hidden retries: a failed request is reported and cached data fills in when present.
#[derive(Debug, Clone)]
pub struct NwsRefresh {
    lookup: (f64, f64),
    started_utc_ms: i64,
    deadline_ms: i64,
    phase: Phase,
    next_id: u32,
    outstanding: Vec<(u32, String, Option<usize>)>,
    stations: Vec<StationRef>,
    queue: Vec<usize>,
    result: RefreshResult,
}

pub fn cache_key_points(lookup: (f64, f64)) -> String {
    format!("points:{:.2},{:.2}", lookup.0, lookup.1)
}

pub fn cache_key_stations(url: &str) -> String {
    format!("stations:{url}")
}

pub fn cache_key_obs(station_id: &str) -> String {
    format!("obs:{station_id}")
}

impl NwsRefresh {
    /// `lat`/`lon` are rounded for the lookup; returns `None` for invalid coordinates.
    pub fn start(lat: f64, lon: f64, now_utc_ms: i64, deadline_ms: i64, cache: &dyn DocCache) -> Option<(Self, Step)> {
        let lookup = lookup_coords(lat, lon)?;
        let mut r = Self {
            lookup,
            started_utc_ms: now_utc_ms,
            deadline_ms,
            phase: Phase::Points,
            next_id: 1,
            outstanding: vec![],
            stations: vec![],
            queue: vec![],
            result: RefreshResult { candidates: vec![], issues: vec![], stations: vec![], used_cache: false },
        };
        let key = cache_key_points(lookup);
        if let Some(doc) = cache.get(&key).filter(|d| d.expires_utc_ms > now_utc_ms)
            && let Ok(info) = parse_points(200, &doc.body)
        {
            let step = r.after_points(info, now_utc_ms, cache);
            return Some((r, step));
        }
        let req = r.request(points_url(lookup.0, lookup.1), None, None);
        Some((r, Step::Fetch(vec![req])))
    }

    pub fn lookup(&self) -> (f64, f64) {
        self.lookup
    }

    fn request(&mut self, url: String, station: Option<usize>, etag: Option<String>) -> HttpRequest {
        let id = self.next_id;
        self.next_id += 1;
        self.outstanding.push((id, url.clone(), station));
        HttpRequest { id, url, if_none_match: etag }
    }

    fn after_points(&mut self, info: PointInfo, now: i64, cache: &dyn DocCache) -> Step {
        let key = cache_key_stations(&info.stations_url);
        if let Some(doc) = cache.get(&key).filter(|d| d.expires_utc_ms > now)
            && let Ok(st) = parse_stations(200, &doc.body, self.lookup)
        {
            return self.begin_observations(st, cache);
        }
        self.phase = Phase::Stations(info.stations_url.clone());
        let url = format!("{}?limit={}", info.stations_url, MAX_STATIONS * 2);
        Step::Fetch(vec![self.request(url, None, None)])
    }

    fn begin_observations(&mut self, stations: Vec<StationRef>, cache: &dyn DocCache) -> Step {
        self.stations = stations.clone();
        self.result.stations = stations;
        self.phase = Phase::Observations;
        self.queue = (0..self.stations.len()).rev().collect();
        self.pump(cache)
    }

    fn pump(&mut self, cache: &dyn DocCache) -> Step {
        let mut reqs = Vec::new();
        while self.outstanding.len() < MAX_CONCURRENT {
            let Some(i) = self.queue.pop() else { break };
            let id = self.stations[i].station_id.clone();
            let Some(url) = latest_observation_url(&id) else { continue };
            let etag = cache.get(&cache_key_obs(&id)).and_then(|d| d.etag);
            reqs.push(self.request(url, Some(i), etag));
        }
        if reqs.is_empty() && self.outstanding.is_empty() {
            self.phase = Phase::Done;
            return Step::Done(std::mem::replace(
                &mut self.result,
                RefreshResult { candidates: vec![], issues: vec![], stations: vec![], used_cache: false },
            ));
        }
        Step::Fetch(reqs)
    }

    fn cache_obs_fallback(&mut self, i: usize, cache: &dyn DocCache) {
        let st = self.stations[i].clone();
        if let Some(doc) = cache.get(&cache_key_obs(&st.station_id))
            && let Ok((c, _)) = parse_observation(200, &doc.body, &st, doc.fetched_utc_ms)
        {
            self.result.used_cache = true;
            self.result.candidates.extend(c);
        }
    }

    /// Network unavailable: use any cached documents regardless of expiry. Values keep
    /// their original observation times, so their age shows how old they are.
    fn offline_fallback(&mut self, cache: &dyn DocCache) -> Step {
        let stations = cache
            .get(&cache_key_points(self.lookup))
            .and_then(|d| parse_points(200, &d.body).ok())
            .and_then(|info| cache.get(&cache_key_stations(&info.stations_url)))
            .and_then(|d| parse_stations(200, &d.body, self.lookup).ok())
            .unwrap_or_default();
        self.stations = stations.clone();
        self.result.stations = stations;
        for i in 0..self.stations.len() {
            self.cache_obs_fallback(i, cache);
        }
        self.phase = Phase::Done;
        Step::Done(std::mem::replace(&mut self.result, empty()))
    }

    /// Feed one response. Unknown ids are ignored.
    pub fn on_response(&mut self, resp: HttpResponse, now_utc_ms: i64, cache: &mut dyn DocCache) -> Step {
        let Some(pos) = self.outstanding.iter().position(|o| o.0 == resp.id) else { return Step::Fetch(vec![]) };
        let (_, _url, station) = self.outstanding.remove(pos);
        if now_utc_ms - self.started_utc_ms > self.deadline_ms {
            self.result.issues.push(ProviderIssue::DeadlineExceeded);
            self.queue.clear();
            for (_, _, st) in std::mem::take(&mut self.outstanding) {
                if let Some(i) = st {
                    self.cache_obs_fallback(i, cache);
                }
            }
            if let Some(i) = station {
                self.cache_obs_fallback(i, cache);
            }
            return self.pump(cache);
        }
        let kind = match &self.phase {
            Phase::Points => "points",
            Phase::Stations(_) => "stations",
            _ => "observation",
        };
        if let Some(e) = &resp.error {
            self.result.issues.push(ProviderIssue::Transport { url_kind: kind.into(), detail: format!("{e:?}") });
        } else if resp.status == 429 || (resp.status == 503 && resp.retry_after_s.is_some()) {
            self.result.issues.push(ProviderIssue::RateLimited { retry_after_s: resp.retry_after_s });
        } else if resp.body.len() > MAX_BODY_BYTES {
            self.result.issues.push(ProviderIssue::Malformed { url_kind: kind.into(), detail: "body too large".into() });
        }
        let ok = resp.error.is_none() && resp.body.len() <= MAX_BODY_BYTES && resp.status != 429;
        let ttl = |max_age: Option<u32>, floor: i64| now_utc_ms + floor.max(i64::from(max_age.unwrap_or(0)) * 1000);
        match self.phase.clone() {
            Phase::Points => {
                if !ok {
                    return self.offline_fallback(cache);
                }
                match parse_points(resp.status, &resp.body) {
                    Ok(info) => {
                        cache.put(CachedDoc {
                            key: cache_key_points(self.lookup),
                            body: resp.body,
                            etag: resp.etag,
                            fetched_utc_ms: now_utc_ms,
                            expires_utc_ms: ttl(None, METADATA_TTL_MS),
                        });
                        self.after_points(info, now_utc_ms, cache)
                    }
                    Err(e) => {
                        let unsupported = e == ProviderIssue::UnsupportedLocation;
                        self.result.issues.push(e);
                        if unsupported {
                            self.phase = Phase::Done;
                            Step::Done(std::mem::replace(&mut self.result, empty()))
                        } else {
                            self.offline_fallback(cache)
                        }
                    }
                }
            }
            Phase::Stations(stations_url) => {
                if !ok {
                    return self.offline_fallback(cache);
                }
                match parse_stations(resp.status, &resp.body, self.lookup) {
                    Ok(st) => {
                        cache.put(CachedDoc {
                            key: cache_key_stations(&stations_url),
                            body: resp.body,
                            etag: resp.etag,
                            fetched_utc_ms: now_utc_ms,
                            expires_utc_ms: ttl(None, METADATA_TTL_MS),
                        });
                        self.begin_observations(st, cache)
                    }
                    Err(e) => {
                        let unsupported = e == ProviderIssue::UnsupportedLocation;
                        self.result.issues.push(e);
                        if unsupported {
                            self.phase = Phase::Done;
                            Step::Done(std::mem::replace(&mut self.result, empty()))
                        } else {
                            self.offline_fallback(cache)
                        }
                    }
                }
            }
            Phase::Observations => {
                let Some(i) = station else { return self.pump(cache) };
                let st = self.stations[i].clone();
                if ok && resp.status == 304 {
                    // Not modified: the cached document is current.
                    self.cache_obs_fallback(i, cache);
                } else if ok {
                    match parse_observation(resp.status, &resp.body, &st, now_utc_ms) {
                        Ok((c, issues)) => {
                            cache.put(CachedDoc {
                                key: cache_key_obs(&st.station_id),
                                body: resp.body,
                                etag: resp.etag,
                                fetched_utc_ms: now_utc_ms,
                                expires_utc_ms: ttl(resp.max_age_s, 60_000),
                            });
                            self.result.candidates.extend(c);
                            self.result.issues.extend(issues);
                        }
                        Err(e) => {
                            self.result.issues.push(e);
                            self.cache_obs_fallback(i, cache);
                        }
                    }
                } else {
                    self.cache_obs_fallback(i, cache);
                }
                self.pump(cache)
            }
            Phase::Done => Step::Fetch(vec![]),
        }
    }
}

fn empty() -> RefreshResult {
    RefreshResult { candidates: vec![], issues: vec![], stations: vec![], used_cache: false }
}
