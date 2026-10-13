//! NWS active alerts for the place (plan phase 3.5, ADR-051). Part of weather lookup in
//! Automatic mode; sends the same rounded place (about 1 km) as the other NWS requests.
//!
//! Checked against the live API on 2026-10-11: `GET /alerts/active?point=lat,lon` returns a
//! GeoJSON FeatureCollection (empty when quiet), HTTP 400 "out of bounds" outside its
//! coverage, and `Cache-Control: max-age=5`. The nationwide list (about 2.4 MB) is never
//! fetched. It can carry test messages (`status: Test`), which are never shown, and
//! cancellations (`messageType: Cancel`), which are not active alerts.

use serde::Deserialize;

use crate::nws::{self, CachedDoc, DocCache, HttpRequest, HttpResponse, ProviderIssue};
use crate::time::parse_rfc3339_ms;

pub const MAX_BODY_BYTES: usize = 512 * 1024;
/// Alerts change quickly; the cache serves only an offline fallback, with its age shown.
pub const CACHE_TTL_MS: i64 = 60_000;
const MAX_ALERTS: usize = 10;

#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    pub id: String,
    pub event: String,
    /// CAP severity: Extreme, Severe, Moderate, Minor, Unknown.
    pub severity: String,
    pub urgency: String,
    pub certainty: String,
    pub headline: Option<String>,
    pub onset_utc_ms: Option<i64>,
    /// When the hazard ends, or when the message expires if no end is given.
    pub until_utc_ms: Option<i64>,
    pub sender: String,
    pub area: String,
    pub description: String,
    pub instruction: Option<String>,
}

pub fn alerts_url(lookup: (f64, f64)) -> String {
    format!("{}/alerts/active?point={:.2},{:.2}", nws::ORIGIN, lookup.0, lookup.1)
}

pub fn cache_key(lookup: (f64, f64)) -> String {
    format!("alerts:{:.2},{:.2}", lookup.0, lookup.1)
}

#[derive(Deserialize)]
struct Collection {
    features: Vec<Feature>,
}

#[derive(Deserialize)]
struct Feature {
    properties: Props,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Props {
    id: Option<String>,
    event: Option<String>,
    status: Option<String>,
    message_type: Option<String>,
    severity: Option<String>,
    urgency: Option<String>,
    certainty: Option<String>,
    headline: Option<String>,
    onset: Option<String>,
    effective: Option<String>,
    expires: Option<String>,
    ends: Option<String>,
    sender_name: Option<String>,
    area_desc: Option<String>,
    description: Option<String>,
    instruction: Option<String>,
}

fn clip(s: Option<String>, n: usize) -> Option<String> {
    s.map(|s| s.trim().chars().take(n).collect::<String>()).filter(|s| !s.is_empty())
}

fn severity_rank(s: &str) -> u8 {
    match s {
        "Extreme" => 0,
        "Severe" => 1,
        "Moderate" => 2,
        "Minor" => 3,
        _ => 4,
    }
}

/// Parse an alerts response: actual, non-cancelled alerts still in force at `now`, most
/// severe first. `Ok(None)` means the place is outside the service (US only).
pub fn parse_alerts(status: u16, body: &[u8], now_utc_ms: i64) -> Result<Option<Vec<Alert>>, ProviderIssue> {
    if status == 400 && std::str::from_utf8(body).is_ok_and(|b| b.contains("out of bounds")) {
        return Ok(None);
    }
    if status != 200 {
        return Err(ProviderIssue::Http { url_kind: "alerts".into(), status });
    }
    let c: Collection = serde_json::from_slice(body)
        .map_err(|e| ProviderIssue::Malformed { url_kind: "alerts".into(), detail: e.to_string().chars().take(80).collect() })?;
    let mut out: Vec<Alert> = c
        .features
        .into_iter()
        .map(|f| f.properties)
        .filter(|p| p.status.as_deref() == Some("Actual") && p.message_type.as_deref() != Some("Cancel"))
        .filter_map(|p| {
            let until = p.ends.as_deref().or(p.expires.as_deref()).and_then(parse_rfc3339_ms);
            if until.is_some_and(|u| u <= now_utc_ms) {
                return None;
            }
            // An onset after the end is contradictory: keep the alert, drop the onset.
            let onset = p
                .onset
                .as_deref()
                .or(p.effective.as_deref())
                .and_then(parse_rfc3339_ms)
                .filter(|o| until.is_none_or(|u| *o <= u));
            Some(Alert {
                id: clip(p.id, 200).unwrap_or_default(),
                event: clip(p.event, 80).unwrap_or_else(|| "Weather alert".into()),
                severity: clip(p.severity, 20).unwrap_or_else(|| "Unknown".into()),
                urgency: clip(p.urgency, 20).unwrap_or_else(|| "Unknown".into()),
                certainty: clip(p.certainty, 20).unwrap_or_else(|| "Unknown".into()),
                headline: clip(p.headline, 300),
                onset_utc_ms: onset,
                until_utc_ms: until,
                sender: clip(p.sender_name, 80).unwrap_or_else(|| "National Weather Service".into()),
                area: clip(p.area_desc, 300).unwrap_or_default(),
                description: clip(p.description, 2_000).unwrap_or_default(),
                instruction: clip(p.instruction, 1_000),
            })
        })
        .collect();
    out.sort_by(|a, b| severity_rank(&a.severity).cmp(&severity_rank(&b.severity)).then(a.onset_utc_ms.cmp(&b.onset_utc_ms)));
    out.truncate(MAX_ALERTS);
    Ok(Some(out))
}

/// Result of one alerts lookup.
#[derive(Debug, Clone, PartialEq)]
pub struct AlertsResult {
    /// `None`: outside the service (US only).
    pub alerts: Option<Vec<Alert>>,
    /// When the shown alerts were fetched (older when served from the offline cache).
    pub fetched_utc_ms: Option<i64>,
    pub issues: Vec<ProviderIssue>,
    pub retries: u32,
}

pub enum AlertsStep {
    Fetch(Vec<HttpRequest>),
    Done(AlertsResult),
}

/// One alerts request, retried under [`crate::retry`] within the weather deadline; when
/// it fails, the last saved alerts are shown with their age (still-active ones only).
#[derive(Debug, Clone)]
pub struct AlertsLookup {
    lookup: (f64, f64),
    started_utc_ms: i64,
    deadline_ms: i64,
    request_id: u32,
    attempt: u32,
    retries: u32,
    done: bool,
}

impl AlertsLookup {
    pub fn start(lat: f64, lon: f64, now_utc_ms: i64, deadline_ms: i64, first_id: u32) -> Option<(Self, AlertsStep)> {
        let lookup = nws::lookup_coords(lat, lon)?;
        let r =
            Self { lookup, started_utc_ms: now_utc_ms, deadline_ms, request_id: first_id, attempt: 1, retries: 0, done: false };
        let req = r.request(0);
        Some((r, AlertsStep::Fetch(vec![req])))
    }

    fn request(&self, delay_ms: u32) -> HttpRequest {
        HttpRequest { id: self.request_id, url: alerts_url(self.lookup), if_none_match: None, delay_ms, timeout_ms: 0 }
    }

    pub fn owns(&self, id: u32) -> bool {
        !self.done && id == self.request_id
    }

    fn cached(&self, cache: &dyn DocCache, now: i64, issues: Vec<ProviderIssue>, retries: u32) -> AlertsResult {
        let doc = cache.get(&cache_key(self.lookup));
        let alerts = doc.as_ref().and_then(|d| parse_alerts(200, &d.body, now).ok().flatten());
        AlertsResult { fetched_utc_ms: alerts.as_ref().and(doc.map(|d| d.fetched_utc_ms)), alerts, issues, retries }
    }

    pub fn on_response(&mut self, resp: HttpResponse, now_utc_ms: i64, cache: &mut dyn DocCache) -> AlertsStep {
        if !self.owns(resp.id) {
            return AlertsStep::Fetch(vec![]);
        }
        let elapsed = now_utc_ms - self.started_utc_ms;
        if elapsed <= self.deadline_ms
            && let Some(delay) = crate::retry::next_delay_ms(&resp, self.attempt, elapsed, self.deadline_ms, resp.unit_random)
        {
            self.request_id += 1;
            self.attempt += 1;
            self.retries += 1;
            return AlertsStep::Fetch(vec![self.request(delay)]);
        }
        self.done = true;
        if let Some(e) = &resp.error {
            let issue = ProviderIssue::Transport { url_kind: "alerts".into(), detail: format!("{e:?}") };
            return AlertsStep::Done(self.cached(cache, now_utc_ms, vec![issue], self.retries));
        }
        if resp.body.len() > MAX_BODY_BYTES {
            let issue = ProviderIssue::Malformed { url_kind: "alerts".into(), detail: "body too large".into() };
            return AlertsStep::Done(self.cached(cache, now_utc_ms, vec![issue], self.retries));
        }
        match parse_alerts(resp.status, &resp.body, now_utc_ms) {
            Ok(alerts) => {
                if alerts.is_some() {
                    cache.put(CachedDoc {
                        key: cache_key(self.lookup),
                        body: resp.body,
                        etag: resp.etag,
                        fetched_utc_ms: now_utc_ms,
                        expires_utc_ms: now_utc_ms + CACHE_TTL_MS,
                    });
                }
                AlertsStep::Done(AlertsResult { alerts, fetched_utc_ms: Some(now_utc_ms), issues: vec![], retries: self.retries })
            }
            Err(e) => AlertsStep::Done(self.cached(cache, now_utc_ms, vec![e], self.retries)),
        }
    }
}
