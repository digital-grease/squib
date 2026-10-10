//! Worldwide station observations: METAR reports from the NOAA Aviation Weather Center
//! Data API (aviationweather.gov). Pure like `nws`: URL policy, parsing, and a one-request
//! fetch planner; transport is native.
//!
//! Values are parsed from the raw METAR text, not the API's convenience fields, because
//! the JSON rounds the altimeter setting to whole hPa even when the report states inches
//! of mercury (`A2991` = 29.91 inHg became `1013`), and converts m/s winds to knots.
//! Verified against live responses on 2026-10-10. The JSON supplies only station
//! metadata (position, elevation, name) and the observation time.
//!
//! Pressure semantics follow the report: `Q`/`A` groups are the altimeter setting (QNH),
//! `SLPnnn` remarks are sea-level pressure, and a `QFE` remark (used in some countries)
//! is the actual pressure at the station's elevation, which stays a remote-station value
//! and never fills the local actual-pressure field.

use serde_json::Value as Json;

use crate::candidate::{Field, MeasurementCandidate, SourceKind, StationRef, Value};
use crate::nws::{CachedDoc, DocCache, HttpRequest, HttpResponse, ProviderIssue, RefreshResult, Step, distance_m, lookup_coords};
use crate::time::parse_rfc3339_ms;
use crate::units::{Plausibility, plausibility, to_si};

pub const PROVIDER: &str = "aviationweather";
pub const ADAPTER_VERSION: &str = "metar-adapter-v1";
pub const ORIGIN: &str = "https://aviationweather.gov";
pub const ACCEPT: &str = "application/json";
pub const ATTRIBUTION: &str = "Weather observations: METAR reports via NOAA Aviation Weather Center (aviationweather.gov)";
pub const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
/// Stations kept per refresh, nearest first (resource budget).
pub const MAX_STATIONS: usize = 5;
/// Half-height of the search box in degrees of latitude (about 110 km).
pub const SEARCH_HALF_DEG: f64 = 1.0;
/// Reports older than this are not requested; stations report hourly or more often.
pub const LOOKBACK_HOURS: u32 = 3;
/// Transformation label for humidity derived from temperature and dew point.
pub const RH_TRANSFORM: &str = "derived:rh_from_dewpoint_magnus_ae1996";

/// Only HTTPS URLs on the Aviation Weather Center origin's data API are fetched.
pub fn url_allowed(url: &str) -> bool {
    url.strip_prefix(ORIGIN).is_some_and(|rest| rest.starts_with("/api/data/") && !rest.contains('@'))
}

/// Search box around the (already rounded) lookup place, clamped to valid coordinates.
/// Near the antimeridian the box is cut at ±180 rather than wrapped.
pub fn search_box(lookup: (f64, f64)) -> (f64, f64, f64, f64) {
    let (lat, lon) = lookup;
    let half_lon = (SEARCH_HALF_DEG / lat.to_radians().cos().max(0.2)).min(5.0);
    let r = |v: f64| (v * 100.0).round() / 100.0;
    (
        r((lat - SEARCH_HALF_DEG).max(-90.0)),
        r((lon - half_lon).max(-180.0)),
        r((lat + SEARCH_HALF_DEG).min(90.0)),
        r((lon + half_lon).min(180.0)),
    )
}

pub fn search_url(lookup: (f64, f64)) -> String {
    let (s, w, n, e) = search_box(lookup);
    format!("{ORIGIN}/api/data/metar?bbox={s:.2},{w:.2},{n:.2},{e:.2}&format=json&hours={LOOKBACK_HOURS}")
}

pub fn cache_key(lookup: (f64, f64)) -> String {
    format!("metar:{:.2},{:.2}", lookup.0, lookup.1)
}

// ---- Raw METAR parsing ---------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WindDir {
    Degrees(f64),
    Variable,
    Calm,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Wind {
    pub dir: WindDir,
    pub speed: f64,
    pub gust: Option<f64>,
    /// `kt`, `m_s-1`, or `km_h-1`.
    pub unit: &'static str,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawMetar {
    pub temp_c: Option<f64>,
    pub dewpoint_c: Option<f64>,
    /// True when temperatures came from the 0.1 °C `T` remark group.
    pub tenths: bool,
    pub wind: Option<Wind>,
    /// Altimeter setting (QNH) and its unit as reported: `hPa` or `inHg`.
    pub altimeter: Option<(f64, &'static str)>,
    pub sea_level_hpa: Option<f64>,
    /// Actual station pressure from a QFE remark, with unit `hPa` or `mmHg`.
    pub qfe: Option<(f64, &'static str)>,
}

fn digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn temp_part(s: &str) -> Option<f64> {
    let (neg, d) = match s.strip_prefix('M') {
        Some(d) => (true, d),
        None => (false, s),
    };
    (d.len() == 2 && digits(d)).then(|| {
        let v: f64 = d.parse().unwrap_or(0.0);
        if neg { -v } else { v }
    })
}

fn parse_wind(tok: &str) -> Option<Wind> {
    let (body, unit) = if let Some(b) = tok.strip_suffix("KT") {
        (b, "kt")
    } else if let Some(b) = tok.strip_suffix("MPS") {
        (b, "m_s-1")
    } else if let Some(b) = tok.strip_suffix("KMH") {
        (b, "km_h-1")
    } else {
        return None;
    };
    if body.len() < 5 {
        return None;
    }
    let (dir, rest) = body.split_at(3);
    let (spd, gust) = match rest.split_once('G') {
        Some((s, g)) => (s, Some(g)),
        None => (rest, None),
    };
    if !(2..=3).contains(&spd.len()) || !digits(spd) || gust.is_some_and(|g| !(2..=3).contains(&g.len()) || !digits(g)) {
        return None;
    }
    let speed: f64 = spd.parse().ok()?;
    let dir = match dir {
        "VRB" => WindDir::Variable,
        d if digits(d) => {
            let deg: f64 = d.parse().ok()?;
            if speed == 0.0 && deg == 0.0 { WindDir::Calm } else { WindDir::Degrees(deg) }
        }
        _ => return None,
    };
    Some(Wind { dir, speed, gust: gust.and_then(|g| g.parse().ok()), unit })
}

/// Parse the groups Squib uses. Unknown groups are skipped; trend groups (`TEMPO`,
/// `BECMG`, `NOSIG`) end the observed part so forecast values are never read.
pub fn parse_raw(raw: &str) -> RawMetar {
    let mut m = RawMetar::default();
    let mut in_remarks = false;
    let mut in_trend = false;
    for tok in raw.split_whitespace() {
        match tok {
            "RMK" => {
                in_remarks = true;
                continue;
            }
            "TEMPO" | "BECMG" | "NOSIG" => {
                in_trend = true;
                continue;
            }
            _ => {}
        }
        if in_remarks {
            if let Some(v) = tok.strip_prefix("SLP").filter(|v| v.len() == 3 && digits(v)) {
                let n: f64 = v.parse().unwrap_or(0.0) / 10.0;
                m.sea_level_hpa = Some(if n < 50.0 { 1000.0 + n } else { 900.0 + n });
            } else if let Some(v) = tok.strip_prefix('T').filter(|v| v.len() == 8 && digits(v)) {
                let t = |sign: &str, d: &str| {
                    let x: f64 = d.parse::<f64>().unwrap_or(0.0) / 10.0;
                    if sign == "1" { -x } else { x }
                };
                m.temp_c = Some(t(&v[0..1], &v[1..4]));
                m.dewpoint_c = Some(t(&v[4..5], &v[5..8]));
                m.tenths = true;
            } else if let Some(v) = tok.strip_prefix("QFE") {
                let (mm, hpa) = match v.split_once('/') {
                    Some((a, b)) => (a, Some(b)),
                    None => (v, None),
                };
                if let Some(h) = hpa.filter(|h| (3..=4).contains(&h.len()) && digits(h)) {
                    m.qfe = h.parse().ok().map(|x| (x, "hPa"));
                } else if (3..=4).contains(&mm.len()) && digits(mm) {
                    m.qfe = mm.parse().ok().map(|x| (x, "mmHg"));
                }
            }
            continue;
        }
        if in_trend {
            continue;
        }
        if m.wind.is_none()
            && let Some(w) = parse_wind(tok)
        {
            m.wind = Some(w);
        } else if let Some(v) = tok.strip_prefix('Q').filter(|v| v.len() == 4 && digits(v)) {
            m.altimeter = v.parse().ok().map(|x| (x, "hPa"));
        } else if let Some(v) = tok.strip_prefix('A').filter(|v| v.len() == 4 && digits(v)) {
            m.altimeter = v.parse::<f64>().ok().map(|x| (x / 100.0, "inHg"));
        } else if let Some((a, b)) = tok.split_once('/')
            && !m.tenths
            && m.temp_c.is_none()
            && let Some(t) = temp_part(a)
        {
            m.temp_c = Some(t);
            m.dewpoint_c = temp_part(b);
        }
    }
    m
}

/// Saturation vapour pressure over water, hPa (Magnus form, Alduchov and Eskridge 1996).
fn es_hpa(t_c: f64) -> f64 {
    6.1094 * (17.625 * t_c / (t_c + 243.04)).exp()
}

/// Relative humidity (fraction) from temperature and dew point. Rounded report values
/// can put dew point a little above temperature; that is saturation, not over 100 %.
pub fn rh_from_dewpoint(t_c: f64, td_c: f64) -> f64 {
    (es_hpa(td_c) / es_hpa(t_c)).clamp(0.0, 1.0)
}

// ---- Collection parsing --------------------------------------------------------------

/// Stations kept, their candidates, and per-value issues from one search response.
pub type Collection = (Vec<StationRef>, Vec<MeasurementCandidate>, Vec<ProviderIssue>);

struct Report {
    station: StationRef,
    observed: Option<i64>,
    raw: String,
    qc: Option<String>,
}

/// Parse a bbox response: newest report per station, nearest `MAX_STATIONS` first.
pub fn parse_collection(status: u16, body: &[u8], lookup: (f64, f64), fetched_utc_ms: i64) -> Result<Collection, ProviderIssue> {
    if status == 204 || (status == 200 && body.iter().all(u8::is_ascii_whitespace)) {
        return Err(ProviderIssue::NoStationsNearby);
    }
    if status != 200 {
        return Err(ProviderIssue::Http { url_kind: "metar".into(), status });
    }
    let json: Json =
        serde_json::from_slice(body).map_err(|e| ProviderIssue::Malformed { url_kind: "metar".into(), detail: e.to_string() })?;
    let arr = json.as_array().ok_or(ProviderIssue::Malformed { url_kind: "metar".into(), detail: "not a list".into() })?;
    let mut reports: Vec<Report> = Vec::new();
    for o in arr {
        let id = o.get("icaoId").and_then(|v| v.as_str()).unwrap_or("");
        let (Some(lat), Some(lon), Some(raw)) = (
            o.get("lat").and_then(|v| v.as_f64()),
            o.get("lon").and_then(|v| v.as_f64()),
            o.get("rawOb").and_then(|v| v.as_str()),
        ) else {
            continue;
        };
        if id.is_empty() || id.len() > 8 || !id.chars().all(|c| c.is_ascii_alphanumeric()) || !lat.is_finite() || !lon.is_finite()
        {
            continue;
        }
        // Observation time: the API's epoch seconds, else its RFC 3339 report time. Never fetch time.
        let observed = o
            .get("obsTime")
            .and_then(|v| v.as_i64())
            .map(|s| s * 1000)
            .or_else(|| o.get("reportTime").and_then(|v| v.as_str()).and_then(parse_rfc3339_ms));
        if let Some(prev) = reports.iter_mut().find(|r| r.station.station_id == id) {
            if observed > prev.observed {
                prev.observed = observed;
                prev.raw = raw.to_string();
            }
            continue;
        }
        reports.push(Report {
            station: StationRef {
                station_id: id.to_string(),
                name: o.get("name").and_then(|v| v.as_str()).unwrap_or("").chars().take(80).collect(),
                lat,
                lon,
                elevation_m: o.get("elev").and_then(|v| v.as_f64()).filter(|e| e.is_finite()),
                distance_m: Some(distance_m(lookup.0, lookup.1, lat, lon)),
            },
            observed,
            raw: raw.to_string(),
            qc: o.get("qcField").and_then(|v| v.as_i64()).map(|q| format!("qcField={q}")),
        });
    }
    if reports.is_empty() {
        return Err(ProviderIssue::NoStationsNearby);
    }
    reports.sort_by(|a, b| a.station.distance_m.partial_cmp(&b.station.distance_m).unwrap_or(std::cmp::Ordering::Equal));
    reports.truncate(MAX_STATIONS);
    let mut cands = Vec::new();
    let mut issues = Vec::new();
    for r in &reports {
        observation_candidates(r, fetched_utc_ms, &mut cands, &mut issues);
    }
    Ok((reports.into_iter().map(|r| r.station).collect(), cands, issues))
}

fn observation_candidates(r: &Report, fetched: i64, out: &mut Vec<MeasurementCandidate>, issues: &mut Vec<ProviderIssue>) {
    let m = parse_raw(&r.raw);
    let base = |field: Field, original: Option<f64>, unit: &str| MeasurementCandidate {
        id: format!(
            "{PROVIDER}:{}:{}:{}",
            r.station.station_id,
            field.as_str(),
            r.observed.map(|t| t.to_string()).unwrap_or_else(|| "unknown".into())
        ),
        field,
        value: Value::Missing,
        original_value: original,
        original_unit: Some(unit.to_string()),
        source: SourceKind::StationObservation,
        provider: PROVIDER.into(),
        station: Some(r.station.clone()),
        observed_utc_ms: r.observed,
        fetched_utc_ms: fetched,
        qc: r.qc.clone(),
        datum: None,
        accuracy: None,
        accuracy_meaning: None,
        transforms: vec![],
        adapter_version: ADAPTER_VERSION.into(),
    };
    // Values that are explicit states rather than conversions are collected separately.
    let mut states: Vec<MeasurementCandidate> = Vec::new();
    let mut push = |field: Field, raw: f64, unit: &str, extra: Option<&str>| match to_si(field, raw, unit) {
        Ok((si, t)) if plausibility(field, si) != Plausibility::Invalid => {
            let mut c = base(field, Some(raw), unit);
            c.value = Value::Si(si);
            c.transforms = t.into_iter().chain(extra.map(String::from)).collect();
            out.push(c);
        }
        Ok((si, _)) => issues.push(ProviderIssue::RejectedValue {
            station_id: r.station.station_id.clone(),
            field,
            reason: format!("implausible {si}"),
        }),
        Err(e) => issues.push(ProviderIssue::RejectedValue {
            station_id: r.station.station_id.clone(),
            field,
            reason: format!("{e:?}"),
        }),
    };
    if let Some(t) = m.temp_c {
        push(Field::Temperature, t, "degC", m.tenths.then_some("source:metar_t_group"));
    }
    if let (Some(t), Some(td)) = (m.temp_c, m.dewpoint_c) {
        let rh = rh_from_dewpoint(t, td);
        let mut c = base(Field::RelativeHumidity, Some(td), "degC_dewpoint");
        c.value = Value::Si(rh);
        c.transforms = vec![RH_TRANSFORM.into()];
        states.push(c);
    }
    if let Some(w) = &m.wind {
        push(Field::WindSpeed, w.speed, w.unit, None);
        if let Some(g) = w.gust {
            push(Field::WindGust, g, w.unit, None);
        }
        match w.dir {
            WindDir::Degrees(d) => push(Field::WindDirection, d, "deg", None),
            WindDir::Variable => {
                let mut c = base(Field::WindDirection, None, "VRB");
                c.value = Value::Variable;
                states.push(c);
            }
            WindDir::Calm => {
                let mut c = base(Field::WindDirection, Some(0.0), "deg");
                c.value = Value::Calm;
                c.transforms.push("calm:speed_zero".into());
                states.push(c);
            }
        }
    }
    if let Some((v, u)) = m.altimeter {
        push(Field::AltimeterSetting, v, u, None);
    }
    if let Some(v) = m.sea_level_hpa {
        push(Field::SeaLevelPressure, v, "hPa", None);
    }
    if let Some((v, u)) = m.qfe {
        push(Field::RemoteStationPressure, v, u, Some("source:metar_qfe_remark"));
    }
    out.extend(states);
}

// ---- Fetch planner -------------------------------------------------------------------

/// One request (the search box); cached copy used when offline or rate limited.
#[derive(Debug, Clone)]
pub struct MetarRefresh {
    lookup: (f64, f64),
    started_utc_ms: i64,
    deadline_ms: i64,
    request_id: u32,
    done: bool,
}

fn finish(res: RefreshResult) -> Step {
    Step::Done(res)
}

impl MetarRefresh {
    pub fn start(lat: f64, lon: f64, now_utc_ms: i64, deadline_ms: i64, cache: &dyn DocCache) -> Option<(Self, Step)> {
        let lookup = lookup_coords(lat, lon)?;
        let r = Self { lookup, started_utc_ms: now_utc_ms, deadline_ms, request_id: 1, done: false };
        let etag = cache.get(&cache_key(lookup)).and_then(|d| d.etag);
        let req = HttpRequest { id: r.request_id, url: search_url(lookup), if_none_match: etag };
        Some((r, Step::Fetch(vec![req])))
    }

    pub fn lookup(&self) -> (f64, f64) {
        self.lookup
    }

    fn with_cached(&self, cache: &dyn DocCache, mut res: RefreshResult) -> RefreshResult {
        if let Some(doc) = cache.get(&cache_key(self.lookup))
            && let Ok((st, c, _)) = parse_collection(200, &doc.body, self.lookup, doc.fetched_utc_ms)
        {
            res.used_cache = true;
            res.stations = st;
            res.candidates = c;
        }
        res
    }

    pub fn on_response(&mut self, resp: HttpResponse, now_utc_ms: i64, cache: &mut dyn DocCache) -> Step {
        if self.done || resp.id != self.request_id {
            return Step::Fetch(vec![]);
        }
        self.done = true;
        let mut res = RefreshResult { candidates: vec![], issues: vec![], stations: vec![], used_cache: false };
        if now_utc_ms - self.started_utc_ms > self.deadline_ms {
            res.issues.push(ProviderIssue::DeadlineExceeded);
            return finish(self.with_cached(cache, res));
        }
        if let Some(e) = &resp.error {
            res.issues.push(ProviderIssue::Transport { url_kind: "metar".into(), detail: format!("{e:?}") });
            return finish(self.with_cached(cache, res));
        }
        if resp.status == 429 || (resp.status == 503 && resp.retry_after_s.is_some()) {
            res.issues.push(ProviderIssue::RateLimited { retry_after_s: resp.retry_after_s });
            return finish(self.with_cached(cache, res));
        }
        if resp.body.len() > MAX_BODY_BYTES {
            res.issues.push(ProviderIssue::Malformed { url_kind: "metar".into(), detail: "body too large".into() });
            return finish(self.with_cached(cache, res));
        }
        if resp.status == 304 {
            return finish(self.with_cached(cache, res));
        }
        match parse_collection(resp.status, &resp.body, self.lookup, now_utc_ms) {
            Ok((st, c, issues)) => {
                cache.put(CachedDoc {
                    key: cache_key(self.lookup),
                    body: resp.body,
                    etag: resp.etag,
                    fetched_utc_ms: now_utc_ms,
                    expires_utc_ms: now_utc_ms + 60_000.max(i64::from(resp.max_age_s.unwrap_or(0)) * 1000),
                });
                res.stations = st;
                res.candidates = c;
                res.issues = issues;
                finish(res)
            }
            Err(ProviderIssue::NoStationsNearby) => {
                res.issues.push(ProviderIssue::NoStationsNearby);
                finish(res)
            }
            Err(e) => {
                res.issues.push(e);
                finish(self.with_cached(cache, res))
            }
        }
    }
}
