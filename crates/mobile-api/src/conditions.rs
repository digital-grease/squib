//! Range conditions facade (M2). The platform performs HTTP and sensor reads; the
//! core decides what to fetch, validates responses, resolves fields, and stores
//! privacy-filtered snapshots. Nothing here runs on the audio path.

use std::sync::MutexGuard;

use squib_environment::metar::{self, MetarRefresh};
use squib_environment::nws::{self, CachedDoc, DocCache, HttpResponse, NwsRefresh, Step, TransportError};
use squib_environment::privacy::LocationRetention;
use squib_environment::{
    ElevationComparison, EnvironmentSnapshot, Field, MeasurementCandidate, Origin, Override, ReferencePlace, ResolvedField,
    ResolverPolicy, Value, local, resolve,
};
use squib_storage::{
    Repository, SETTING_LAST_PLACE, SETTING_LOCATION_RETENTION, SETTING_WEATHER_ENABLED, SETTING_WEATHER_PROVIDER, SavedPlace,
};

use crate::engine::SquibEngine;
use crate::ffi::SquibError;
use crate::store::StoreActor;

/// Total time budget for one refresh.
pub const REFRESH_DEADLINE_MS: i64 = 20_000;

/// A refresh in progress with one provider.
pub enum Refresh {
    Nws(NwsRefresh),
    Metar(MetarRefresh),
}

impl Refresh {
    fn provider(&self) -> &'static str {
        match self {
            Refresh::Nws(_) => nws::PROVIDER,
            Refresh::Metar(_) => metar::PROVIDER,
        }
    }
}

#[derive(Default)]
pub struct CondState {
    pub place: Option<ReferencePlace>,
    pub device: Vec<MeasurementCandidate>,
    pub remote: Vec<MeasurementCandidate>,
    pub issues: Vec<nws::ProviderIssue>,
    pub refresh: Option<Refresh>,
    pub last_refresh_utc_ms: Option<i64>,
    pub used_cache: bool,
    pub allow_older: bool,
}

// ---- FFI types ----------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PlaceInput {
    pub lat: f64,
    pub lon: f64,
    pub label: Option<String>,
    /// `gps`, `manual`, or `saved_place`.
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DeviceReadings {
    /// Android barometer reading, hPa, and its time (UTC ms).
    pub barometer_hpa: Option<f64>,
    pub barometer_utc_ms: Option<i64>,
    /// `Location.getAltitude()`: WGS84 ellipsoid height.
    pub ellipsoid_m: Option<f64>,
    pub ellipsoid_accuracy_m: Option<f64>,
    /// `Location.getMslAltitudeMeters()` when the platform supplies it.
    pub msl_m: Option<f64>,
    pub msl_accuracy_m: Option<f64>,
    pub location_utc_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct HttpRequestFfi {
    pub id: u32,
    pub url: String,
    pub user_agent: String,
    pub accept: String,
    pub if_none_match: Option<String>,
    pub max_body_bytes: u32,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct HttpResponseFfi {
    pub id: u32,
    pub status: u16,
    pub body: Vec<u8>,
    pub etag: Option<String>,
    pub max_age_s: Option<u32>,
    pub retry_after_s: Option<u32>,
    /// `offline`, `timeout`, `too_large`, `tls`, or free text; `None` on success.
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ConditionsStep {
    pub requests: Vec<HttpRequestFfi>,
    pub done: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AlternativeView {
    pub value_si: Option<f64>,
    pub value_state: String,
    pub source_label: String,
    pub age_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct FieldView {
    pub field: String,
    pub label: String,
    /// here / nearby_observation / model_estimate / manual / unavailable.
    pub origin: String,
    /// SI value when numeric (K, fraction, m/s, degrees, Pa, m).
    pub value_si: Option<f64>,
    /// si / calm / variable / missing.
    pub value_state: String,
    pub si_unit: String,
    /// fresh / stale / expired / unknown_age.
    pub freshness: Option<String>,
    /// From the observation time, never the fetch time.
    pub age_ms: Option<i64>,
    pub observed_utc_ms: Option<i64>,
    pub fetched_utc_ms: Option<i64>,
    pub station_id: Option<String>,
    pub station_name: Option<String>,
    pub station_distance_m: Option<f64>,
    pub station_elevation_m: Option<f64>,
    pub qc: Option<String>,
    pub original_value: Option<f64>,
    pub original_unit: Option<String>,
    pub datum: Option<String>,
    pub accuracy: Option<f64>,
    pub reasons: Vec<String>,
    pub alternatives: Vec<AlternativeView>,
    pub disagreement: bool,
    pub overridden: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PlaceView {
    pub lat: f64,
    pub lon: f64,
    pub label: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ConditionsView {
    pub weather_enabled: bool,
    pub precise_retention: bool,
    pub place: Option<PlaceView>,
    pub fields: Vec<FieldView>,
    pub elevation_notes: Vec<String>,
    pub issues: Vec<String>,
    pub attribution: Vec<String>,
    pub last_refresh_utc_ms: Option<i64>,
    pub used_cache: bool,
    pub refreshing: bool,
    pub allow_older: bool,
    /// The snapshot is redacted (history view of a run).
    pub redacted: bool,
    /// `auto` or `metar`; see `set_weather_provider`.
    pub weather_provider: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SavedPlaceView {
    pub id: String,
    pub label: String,
    pub lat: f64,
    pub lon: f64,
}

// ---- Helpers ------------------------------------------------------------------------

pub fn field_label(f: Field) -> &'static str {
    match f {
        Field::Temperature => "Temperature",
        Field::RelativeHumidity => "Humidity",
        Field::WindSpeed => "Wind",
        Field::WindGust => "Gusts",
        Field::WindDirection => "Wind direction",
        Field::LocalPressure => "Pressure here (actual)",
        Field::RemoteStationPressure => "Station pressure (remote)",
        Field::AltimeterSetting => "Altimeter setting",
        Field::SeaLevelPressure => "Sea-level pressure",
        Field::ElevationMsl => "Elevation (above sea level)",
        Field::ElevationEllipsoid => "GPS height (ellipsoid)",
    }
}

fn reason_text(r: squib_environment::Reason) -> &'static str {
    use squib_environment::Reason::*;
    match r {
        ManualOverride => "Your entry",
        LocalSensor => "Measured by this phone",
        RankedNearbyObservation => "Nearest suitable station",
        OlderDataAllowed => "Older data, used because you allowed it",
        NoCandidates => "No source available",
        AllMissing => "Stations did not report this",
        AllExcludedByQc => "Station values failed quality checks",
        AllExpired => "Station data is too old",
        LocalPressureRequiresLocalSource => "Needs a phone barometer or your entry; station values are not local pressure",
        PreciseProvenanceNotRetained => "Precise provenance not retained",
    }
}

fn state(v: Value) -> (Option<f64>, String) {
    match v {
        Value::Si(x) => (Some(x), "si".into()),
        Value::Calm => (None, "calm".into()),
        Value::Variable => (None, "variable".into()),
        Value::Missing => (None, "missing".into()),
    }
}

fn origin_str(o: Origin) -> String {
    serde_json::to_value(o).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
}

fn source_label(c: &MeasurementCandidate) -> String {
    match &c.station {
        Some(s) => format!("{} ({})", s.station_id, s.name),
        None => match c.provider.as_str() {
            "android_barometer" => "Phone barometer".into(),
            "android_location" => "Phone location".into(),
            "manual" => "Your entry".into(),
            "nws" => "Nearby station".into(),
            other => other.into(),
        },
    }
}

pub fn field_view(r: &ResolvedField, now_utc_ms: i64) -> FieldView {
    let c = r.chosen.as_ref();
    let (value_si, value_state) = c.map(|c| state(c.value)).unwrap_or((None, "missing".into()));
    FieldView {
        field: r.field.as_str().into(),
        label: field_label(r.field).into(),
        origin: origin_str(r.origin),
        value_si,
        value_state,
        si_unit: r.field.si_unit().into(),
        freshness: r.freshness.and_then(|f| serde_json::to_value(f).ok()).and_then(|v| v.as_str().map(String::from)),
        age_ms: c.and_then(|c| c.age_ms(now_utc_ms)),
        observed_utc_ms: c.and_then(|c| c.observed_utc_ms),
        fetched_utc_ms: c.map(|c| c.fetched_utc_ms),
        station_id: c.and_then(|c| c.station.as_ref().map(|s| s.station_id.clone())),
        station_name: c.and_then(|c| c.station.as_ref().map(|s| s.name.clone())),
        station_distance_m: c.and_then(|c| c.station.as_ref().and_then(|s| s.distance_m)),
        station_elevation_m: c.and_then(|c| c.station.as_ref().and_then(|s| s.elevation_m)),
        qc: c.and_then(|c| c.qc.clone()),
        original_value: c.and_then(|c| c.original_value),
        original_unit: c.and_then(|c| c.original_unit.clone()),
        datum: c.and_then(|c| c.datum).and_then(|d| serde_json::to_value(d).ok()).and_then(|v| v.as_str().map(String::from)),
        accuracy: c.and_then(|c| c.accuracy),
        reasons: r.reasons.iter().map(|x| reason_text(*x).to_string()).collect(),
        alternatives: r
            .alternatives
            .iter()
            .map(|a| {
                let (v, s) = state(a.value);
                AlternativeView { value_si: v, value_state: s, source_label: source_label(a), age_ms: a.age_ms(now_utc_ms) }
            })
            .collect(),
        disagreement: r.disagreement,
        overridden: r.origin == Origin::Manual,
    }
}

fn elevation_note(e: &ElevationComparison) -> String {
    match e {
        ElevationComparison::Compatible { difference_m, .. } => {
            format!("Elevation sources agree within {:.0} m (same reference).", difference_m.abs())
        }
        ElevationComparison::DatumMismatch { .. } => {
            "GPS ellipsoid height and sea-level elevation use different references; comparison unavailable.".into()
        }
        ElevationComparison::UnknownDatum { .. } => {
            "An elevation source has an unknown reference; comparison unavailable.".into()
        }
    }
}

fn issue_text(i: &nws::ProviderIssue) -> String {
    use nws::ProviderIssue::*;
    match i {
        UnsupportedLocation => {
            "The National Weather Service has no data for this place (outside the US?). Choose worldwide airport reports or enter values manually."
                .into()
        }
        NoStationsNearby => {
            "No airport weather station reported within about 110 km in the last 3 hours. Enter values manually.".into()
        }
        Transport { detail, .. } => format!("Could not reach the weather service ({detail}). Showing saved data if any."),
        Http { status, url_kind } => format!("Weather service error {status} ({url_kind})."),
        RateLimited { retry_after_s } => match retry_after_s {
            Some(s) => format!("Weather service asked to wait {s} s."),
            None => "Weather service is rate limiting requests.".into(),
        },
        Malformed { url_kind, .. } => format!("Weather service sent an unreadable {url_kind} response."),
        DisallowedUrl { .. } => "Weather service returned a link Squib does not follow.".into(),
        RejectedValue { field, reason, .. } => format!("Ignored a {} value ({reason}).", field_label(*field).to_lowercase()),
        DeadlineExceeded => "Weather refresh took too long; partial results shown.".into(),
    }
}

pub fn conditions_view_of(s: &EnvironmentSnapshot, now_utc_ms: i64) -> Vec<FieldView> {
    s.fields.iter().map(|f| field_view(f, now_utc_ms)).collect()
}

/// Cache adapter: reads on the read connection, writes through the single writer.
struct EngineCache<'a> {
    read: &'a Repository,
    store: &'a StoreActor,
    provider: &'static str,
}

impl DocCache for EngineCache<'_> {
    fn get(&self, key: &str) -> Option<CachedDoc> {
        self.read.cache_get(self.provider, key).ok().flatten()
    }
    fn put(&mut self, doc: CachedDoc) {
        let provider = self.provider;
        let _ = self.store.exec(move |repo| repo.cache_put(provider, &doc));
    }
}

fn to_step(step: Step, provider: &'static str, st: &mut CondState, now_utc_ms: i64) -> ConditionsStep {
    let (accept, max_body, allowed): (&str, usize, fn(&str) -> bool) = if provider == metar::PROVIDER {
        (metar::ACCEPT, metar::MAX_BODY_BYTES, metar::url_allowed)
    } else {
        (nws::ACCEPT, nws::MAX_BODY_BYTES, nws::url_allowed)
    };
    match step {
        Step::Fetch(reqs) => ConditionsStep {
            requests: reqs
                .into_iter()
                // Defence in depth: planners only build allowed URLs.
                .filter(|r| allowed(&r.url))
                .map(|r| HttpRequestFfi {
                    id: r.id,
                    url: r.url,
                    user_agent: nws::USER_AGENT.into(),
                    accept: accept.into(),
                    if_none_match: r.if_none_match,
                    max_body_bytes: max_body as u32,
                })
                .collect(),
            done: false,
        },
        Step::Done(res) => {
            st.refresh = None;
            st.remote = res.candidates;
            st.issues = res.issues;
            st.used_cache = res.used_cache;
            st.last_refresh_utc_ms = Some(now_utc_ms);
            ConditionsStep { requests: vec![], done: true }
        }
    }
}

impl SquibEngine {
    pub(crate) fn weather_provider(&self) -> String {
        self.read_repo()
            .get_setting(SETTING_WEATHER_PROVIDER)
            .ok()
            .flatten()
            .filter(|p| p == "metar")
            .unwrap_or_else(|| "auto".into())
    }

    fn settings_bool(&self, key: &str) -> bool {
        self.read_repo().get_setting(key).ok().flatten().as_deref() == Some("true")
    }

    pub(crate) fn retention(&self) -> LocationRetention {
        self.read_repo()
            .get_setting(SETTING_LOCATION_RETENTION)
            .ok()
            .flatten()
            .and_then(|v| LocationRetention::parse(&v))
            .unwrap_or(LocationRetention::Redacted)
    }

    fn resolve_now(&self, st: &CondState, now_utc_ms: i64) -> EnvironmentSnapshot {
        let overrides = self.read_repo().list_overrides().unwrap_or_default();
        let mut cands = st.remote.clone();
        cands.extend(st.device.iter().cloned());
        let mut place = st.place.clone();
        if let Some(p) = place.as_mut() {
            // Station elevation penalties use a compatible (MSL) local elevation only.
            let msl = overrides
                .iter()
                .find(|o| o.candidate.field == Field::ElevationMsl && o.active(now_utc_ms))
                .map(|o| &o.candidate)
                .or_else(|| st.device.iter().find(|c| c.field == Field::ElevationMsl))
                .and_then(|c| c.value.si());
            p.elevation_msl_m = msl;
        }
        let policy = ResolverPolicy { allow_older: st.allow_older, ..ResolverPolicy::default() };
        resolve(&uuid::Uuid::new_v4().to_string(), &cands, &overrides, place, st.issues.clone(), &policy, now_utc_ms)
    }

    /// Pin current conditions for a run being armed: resolve, apply the retention
    /// policy, store immutably. `None` when no conditions source has been used.
    pub(crate) fn pin_conditions(&self, now_utc_ms: i64) -> Result<Option<String>, SquibError> {
        let snap = {
            let st = self.cond();
            let has_overrides = !self.read_repo().list_overrides().unwrap_or_default().is_empty();
            if st.remote.is_empty() && st.device.is_empty() && !has_overrides {
                return Ok(None);
            }
            self.resolve_now(&st, now_utc_ms)
        };
        let retention = self.retention();
        let stored = self.store_actor().exec(move |repo| repo.insert_snapshot(&snap, retention))?;
        Ok(Some(stored.id))
    }

    /// Restore the last place at startup (see `SETTING_LAST_PLACE` policy).
    pub(crate) fn restore_place(&self) {
        let saved = self.read_repo().get_setting(SETTING_LAST_PLACE).ok().flatten();
        if let Some(p) = saved.and_then(|j| serde_json::from_str::<ReferencePlace>(&j).ok()) {
            self.cond().place = Some(p);
        }
    }

    fn cond(&self) -> MutexGuard<'_, CondState> {
        self.cond_state().lock().unwrap_or_else(|p| p.into_inner())
    }
}

#[uniffi::export]
impl SquibEngine {
    pub fn conditions_view(&self, now_utc_ms: i64) -> ConditionsView {
        let st = self.cond();
        let snap = self.resolve_now(&st, now_utc_ms);
        ConditionsView {
            weather_enabled: self.settings_bool(SETTING_WEATHER_ENABLED),
            precise_retention: self.retention() == LocationRetention::Precise,
            place: st.place.as_ref().map(|p| PlaceView {
                lat: p.lat,
                lon: p.lon,
                label: p.label.clone(),
                source: p.source.clone(),
            }),
            fields: conditions_view_of(&snap, now_utc_ms),
            elevation_notes: snap.elevation_comparisons.iter().map(elevation_note).collect(),
            issues: st.issues.iter().map(issue_text).collect(),
            attribution: snap.attribution.clone(),
            last_refresh_utc_ms: st.last_refresh_utc_ms,
            used_cache: st.used_cache,
            refreshing: st.refresh.is_some(),
            allow_older: st.allow_older,
            redacted: false,
            weather_provider: self.weather_provider(),
        }
    }

    /// `auto`: the National Weather Service where it has data, METAR airport reports
    /// elsewhere. `metar`: METAR everywhere. Both receive only the rounded place.
    pub fn set_weather_provider(&self, provider: String) -> Result<(), SquibError> {
        if !matches!(provider.as_str(), "auto" | "metar") {
            return Err(SquibError::Invalid("weather provider must be auto or metar".into()));
        }
        self.store_actor().exec(move |repo| repo.set_setting(SETTING_WEATHER_PROVIDER, &provider))?;
        let mut st = self.cond();
        st.refresh = None;
        st.remote.clear();
        st.issues.clear();
        st.last_refresh_utc_ms = None;
        Ok(())
    }

    /// Weather lookup sends the (rounded) place to the provider: off until enabled.
    pub fn set_weather_enabled(&self, enabled: bool) -> Result<(), SquibError> {
        let v = if enabled { "true" } else { "false" };
        self.store_actor().exec(move |repo| repo.set_setting(SETTING_WEATHER_ENABLED, v))?;
        if !enabled {
            self.cond().refresh = None;
        }
        Ok(())
    }

    /// Opt in to storing precise location provenance with runs (default off).
    pub fn set_precise_retention(&self, precise: bool) -> Result<(), SquibError> {
        let v = if precise { LocationRetention::Precise } else { LocationRetention::Redacted }.as_str();
        self.store_actor().exec(move |repo| repo.set_setting(SETTING_LOCATION_RETENTION, v))?;
        Ok(())
    }

    pub fn set_allow_older(&self, allow: bool) {
        self.cond().allow_older = allow;
    }

    pub fn set_place(&self, place: PlaceInput) -> Result<(), SquibError> {
        let Some((lat, lon)) = nws::lookup_coords(place.lat, place.lon) else {
            return Err(SquibError::Invalid("latitude must be -90..90 and longitude -180..180".into()));
        };
        if !matches!(place.source.as_str(), "gps" | "manual" | "saved_place") {
            return Err(SquibError::Invalid("unknown place source".into()));
        }
        let new_place = ReferencePlace {
            lat,
            lon,
            label: place.label.map(|l| l.chars().take(80).collect()),
            source: place.source,
            elevation_msl_m: None,
        };
        // Remember user-chosen places; a GPS place only with precise retention on.
        let remember = new_place.source != "gps" || self.retention() == LocationRetention::Precise;
        let json = if remember { serde_json::to_string(&new_place).unwrap_or_default() } else { String::new() };
        self.store_actor().exec(move |repo| repo.set_setting(SETTING_LAST_PLACE, &json))?;
        let mut st = self.cond();
        let moved = st.place.as_ref().is_none_or(|p| nws::distance_m(p.lat, p.lon, lat, lon) > 1000.0);
        st.place = Some(new_place);
        if moved {
            // Remote values belong to the old place.
            st.remote.clear();
            st.issues.clear();
            st.last_refresh_utc_ms = None;
        }
        Ok(())
    }

    pub fn set_device_readings(&self, r: DeviceReadings) {
        let mut out = Vec::new();
        if let (Some(hpa), Some(t)) = (r.barometer_hpa, r.barometer_utc_ms) {
            out.extend(local::barometer(hpa, t));
        }
        if let Some(t) = r.location_utc_ms {
            out.extend(local::location_heights(r.ellipsoid_m, r.ellipsoid_accuracy_m, r.msl_m, r.msl_accuracy_m, t));
        }
        self.cond().device = out;
    }

    /// Begin a weather refresh for the current place. Returns the first requests.
    pub fn conditions_refresh(&self, now_utc_ms: i64) -> Result<ConditionsStep, SquibError> {
        if !self.settings_bool(SETTING_WEATHER_ENABLED) {
            return Err(SquibError::Rejected("weather lookup is off".into()));
        }
        let mut st = self.cond();
        let Some(place) = st.place.clone() else { return Err(SquibError::Rejected("choose a place first".into())) };
        let metar_only = self.weather_provider() == "metar";
        let read = self.read_repo();
        let started = if metar_only {
            let cache = EngineCache { read: &read, store: self.store_actor(), provider: metar::PROVIDER };
            MetarRefresh::start(place.lat, place.lon, now_utc_ms, REFRESH_DEADLINE_MS, &cache)
                .map(|(r, s)| (Refresh::Metar(r), s))
        } else {
            let cache = EngineCache { read: &read, store: self.store_actor(), provider: nws::PROVIDER };
            NwsRefresh::start(place.lat, place.lon, now_utc_ms, REFRESH_DEADLINE_MS, &cache).map(|(r, s)| (Refresh::Nws(r), s))
        };
        let Some((r, step)) = started else {
            return Err(SquibError::Invalid("invalid place".into()));
        };
        let provider = r.provider();
        st.refresh = Some(r);
        Ok(to_step(step, provider, &mut st, now_utc_ms))
    }

    pub fn conditions_response(&self, resp: HttpResponseFfi, now_utc_ms: i64) -> ConditionsStep {
        let mut st = self.cond();
        let Some(mut r) = st.refresh.take() else { return ConditionsStep { requests: vec![], done: true } };
        let error = resp.error.as_deref().map(|e| match e {
            "offline" => TransportError::Offline,
            "timeout" => TransportError::Timeout,
            "too_large" => TransportError::TooLarge,
            "tls" => TransportError::Tls,
            other => TransportError::Other(other.chars().take(120).collect()),
        });
        let resp = HttpResponse {
            id: resp.id,
            status: resp.status,
            body: resp.body,
            etag: resp.etag,
            max_age_s: resp.max_age_s,
            retry_after_s: resp.retry_after_s,
            error,
        };
        let read = self.read_repo();
        let provider = r.provider();
        let mut cache = EngineCache { read: &read, store: self.store_actor(), provider };
        let step = match &mut r {
            Refresh::Nws(n) => n.on_response(resp, now_utc_ms, &mut cache),
            Refresh::Metar(m) => m.on_response(resp, now_utc_ms, &mut cache),
        };
        // Auto: where NWS has no data (outside the US), continue with METAR reports.
        if let (Refresh::Nws(n), Step::Done(res)) = (&r, &step)
            && res.issues.contains(&nws::ProviderIssue::UnsupportedLocation)
            && let Some((m, next)) = MetarRefresh::start(
                n.lookup().0,
                n.lookup().1,
                now_utc_ms,
                REFRESH_DEADLINE_MS,
                &EngineCache { read: &read, store: self.store_actor(), provider: metar::PROVIDER },
            )
        {
            drop(read);
            st.refresh = Some(Refresh::Metar(m));
            return to_step(next, metar::PROVIDER, &mut st, now_utc_ms);
        }
        drop(read);
        if matches!(step, Step::Fetch(_)) {
            st.refresh = Some(r);
        }
        to_step(step, provider, &mut st, now_utc_ms)
    }

    /// Manual value for one field, in SI. `entered_*` preserves what the user typed.
    pub fn set_override(
        &self,
        field: String,
        value_si: f64,
        entered_value: f64,
        entered_unit: String,
        now_utc_ms: i64,
    ) -> Result<(), SquibError> {
        let f = Field::parse(&field).ok_or_else(|| SquibError::Invalid(format!("unknown field {field}")))?;
        let c = local::manual(f, value_si, entered_value, &entered_unit, now_utc_ms)
            .ok_or_else(|| SquibError::Invalid("value is outside the physically possible range".into()))?;
        let o = Override { candidate: c, set_utc_ms: now_utc_ms, expires_utc_ms: None };
        self.store_actor().exec(move |repo| repo.set_override(&o))?;
        Ok(())
    }

    /// Calm or variable wind direction as a manual entry.
    pub fn set_direction_state_override(&self, calm: bool, now_utc_ms: i64) -> Result<(), SquibError> {
        let o =
            Override { candidate: local::manual_direction_state(calm, now_utc_ms), set_utc_ms: now_utc_ms, expires_utc_ms: None };
        self.store_actor().exec(move |repo| repo.set_override(&o))?;
        Ok(())
    }

    pub fn clear_override(&self, field: String) -> Result<(), SquibError> {
        let f = Field::parse(&field).ok_or_else(|| SquibError::Invalid(format!("unknown field {field}")))?;
        self.store_actor().exec(move |repo| repo.clear_override(f))?;
        Ok(())
    }

    pub fn clear_weather_cache(&self) -> Result<u32, SquibError> {
        let n = self.store_actor().exec(|repo| repo.cache_clear())?;
        let mut st = self.cond();
        st.remote.clear();
        st.issues.clear();
        st.last_refresh_utc_ms = None;
        Ok(n as u32)
    }

    pub fn add_saved_place(&self, label: String, lat: f64, lon: f64, now_utc_ms: i64) -> Result<SavedPlaceView, SquibError> {
        if nws::lookup_coords(lat, lon).is_none() {
            return Err(SquibError::Invalid("latitude must be -90..90 and longitude -180..180".into()));
        }
        let p = SavedPlace { id: uuid::Uuid::new_v4().to_string(), label, lat, lon, created_utc_ms: now_utc_ms };
        let v = SavedPlaceView { id: p.id.clone(), label: p.label.trim().into(), lat, lon };
        self.store_actor().exec(move |repo| repo.add_place(&p))?;
        Ok(v)
    }

    pub fn list_saved_places(&self) -> Vec<SavedPlaceView> {
        self.read_repo()
            .list_places()
            .unwrap_or_default()
            .into_iter()
            .map(|p| SavedPlaceView { id: p.id, label: p.label, lat: p.lat, lon: p.lon })
            .collect()
    }

    pub fn delete_saved_place(&self, id: String) -> Result<(), SquibError> {
        self.store_actor().exec(move |repo| repo.delete_place(&id))?;
        Ok(())
    }

    /// Conditions pinned to a run, as stored (redacted unless precise retention was on).
    pub fn run_conditions(&self, run_id: String) -> Result<Option<ConditionsView>, SquibError> {
        let d = self.read_repo().load_run(&run_id)?;
        Ok(d.environment.map(|s| ConditionsView {
            weather_enabled: true,
            precise_retention: !s.redacted,
            place: s.reference.as_ref().map(|p| PlaceView {
                lat: p.lat,
                lon: p.lon,
                label: p.label.clone(),
                source: p.source.clone(),
            }),
            // Ages are shown as of when the snapshot was pinned.
            fields: conditions_view_of(&s, s.created_utc_ms),
            elevation_notes: s.elevation_comparisons.iter().map(elevation_note).collect(),
            issues: s.issues.iter().map(issue_text).collect(),
            attribution: s.attribution.clone(),
            last_refresh_utc_ms: Some(s.created_utc_ms),
            used_cache: false,
            refreshing: false,
            allow_older: false,
            redacted: s.redacted,
            weather_provider: String::new(),
        }))
    }
}
