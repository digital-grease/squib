//! Resolver policy v1 (docs/squib/05): field-by-field selection, no numeric blending.
//!
//! Order per field: active manual override → fresh local sensor → best-ranked fresh
//! remote observation → (model estimates: none enabled in M2) → older data only when
//! the user explicitly allows it → Unavailable. Ranking penalties are tuning
//! parameters, not probabilities. Alternatives and disagreement are preserved.

use serde::{Deserialize, Serialize};

use crate::candidate::{Field, MeasurementCandidate, Origin, SourceKind, Value, VerticalDatum};
use crate::nws::ProviderIssue;

pub const RESOLVER_POLICY_VERSION: &str = "resolver-v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolverPolicy {
    /// Station fields become Stale after this observation age.
    pub stale_after_ms: i64,
    /// Station fields are not selected automatically beyond this age.
    pub expired_after_ms: i64,
    /// Local sensor readings older than this are not "here, now".
    pub local_max_age_ms: i64,
    /// Ranking penalty per km of distance.
    pub w_distance_per_km: f64,
    /// Ranking penalty per 100 m of elevation difference (when both are known).
    pub w_elevation_per_100m: f64,
    /// Ranking penalty per 30 min of observation age.
    pub w_age_per_30min: f64,
    /// QC codes that exclude a value from automatic selection (MADIS: X failed, B bad).
    pub excluded_qc: Vec<String>,
    /// User chose "use older data": expired station values may be selected, labelled.
    pub allow_older: bool,
}

impl Default for ResolverPolicy {
    fn default() -> Self {
        Self {
            stale_after_ms: 45 * 60_000,
            expired_after_ms: 3 * 3_600_000,
            local_max_age_ms: 15 * 60_000,
            w_distance_per_km: 0.1,
            w_elevation_per_100m: 1.0,
            w_age_per_30min: 1.0,
            excluded_qc: vec!["X".into(), "B".into()],
            allow_older: false,
        }
    }
}

/// The place conditions are resolved for. Coordinates are the rounded lookup values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReferencePlace {
    pub lat: f64,
    pub lon: f64,
    pub label: Option<String>,
    /// `gps`, `manual`, or `saved_place`.
    pub source: String,
    /// Display elevation used for station elevation penalties, when known (MSL, m).
    pub elevation_msl_m: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Override {
    pub candidate: MeasurementCandidate,
    pub set_utc_ms: i64,
    /// `None` until cleared by the user.
    pub expires_utc_ms: Option<i64>,
}

impl Override {
    pub fn active(&self, now: i64) -> bool {
        self.expires_utc_ms.is_none_or(|e| now < e)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    Fresh,
    Stale,
    /// Older than the automatic limit; used only because the user allowed it.
    Expired,
    /// Observation time unknown.
    UnknownAge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    ManualOverride,
    LocalSensor,
    RankedNearbyObservation,
    OlderDataAllowed,
    NoCandidates,
    AllMissing,
    AllExcludedByQc,
    AllExpired,
    /// Local actual pressure needs a local sensor or a manual actual-pressure entry.
    LocalPressureRequiresLocalSource,
    PreciseProvenanceNotRetained,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedField {
    pub field: Field,
    pub chosen: Option<MeasurementCandidate>,
    pub origin: Origin,
    pub freshness: Option<Freshness>,
    pub reasons: Vec<Reason>,
    /// Up to three other eligible candidates, best first.
    pub alternatives: Vec<MeasurementCandidate>,
    /// An eligible alternative differs materially from the chosen value.
    pub disagreement: bool,
}

/// Result of comparing two elevation candidates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ElevationComparison {
    /// Same reference: `a − b` in metres.
    Compatible {
        a: String,
        b: String,
        difference_m: f64,
    },
    /// Different references (ellipsoid vs MSL): not comparable without a geoid model.
    DatumMismatch {
        a: String,
        b: String,
    },
    UnknownDatum {
        a: String,
        b: String,
    },
}

pub fn compare_elevations(a: &MeasurementCandidate, b: &MeasurementCandidate) -> Option<ElevationComparison> {
    let (va, vb) = (a.value.si()?, b.value.si()?);
    let (da, db) = (a.datum.unwrap_or(VerticalDatum::Unknown), b.datum.unwrap_or(VerticalDatum::Unknown));
    let (ia, ib) = (a.id.clone(), b.id.clone());
    Some(if da == VerticalDatum::Unknown || db == VerticalDatum::Unknown {
        ElevationComparison::UnknownDatum { a: ia, b: ib }
    } else if (da.is_msl() && db.is_msl()) || (da == db) {
        ElevationComparison::Compatible { a: ia, b: ib, difference_m: va - vb }
    } else {
        ElevationComparison::DatumMismatch { a: ia, b: ib }
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnvironmentSnapshot {
    pub id: String,
    pub created_utc_ms: i64,
    pub policy_version: String,
    pub reference: Option<ReferencePlace>,
    pub fields: Vec<ResolvedField>,
    pub elevation_comparisons: Vec<ElevationComparison>,
    pub issues: Vec<ProviderIssue>,
    /// Precise provenance (coordinates, station identities, fine elevation) removed.
    pub redacted: bool,
    /// Attribution lines for providers that contributed.
    pub attribution: Vec<String>,
}

impl EnvironmentSnapshot {
    pub fn field(&self, f: Field) -> Option<&ResolvedField> {
        self.fields.iter().find(|r| r.field == f)
    }
}

fn excluded(c: &MeasurementCandidate, p: &ResolverPolicy) -> bool {
    c.qc.as_deref().is_some_and(|q| p.excluded_qc.iter().any(|x| x == q))
}

fn usable(c: &MeasurementCandidate) -> bool {
    !matches!(c.value, Value::Missing)
}

fn penalty(c: &MeasurementCandidate, p: &ResolverPolicy, reference: Option<&ReferencePlace>, now: i64) -> f64 {
    let mut s = 0.0;
    if let Some(st) = &c.station {
        if let Some(d) = st.distance_m {
            s += d / 1000.0 * p.w_distance_per_km;
        }
        if let (Some(se), Some(re)) = (st.elevation_m, reference.and_then(|r| r.elevation_msl_m)) {
            s += (se - re).abs() / 100.0 * p.w_elevation_per_100m;
        }
    }
    if let Some(age) = c.age_ms(now) {
        s += age.max(0) as f64 / 1_800_000.0 * p.w_age_per_30min;
    }
    s
}

/// Values differ enough to flag (a display aid, not a validity test).
fn disagrees(field: Field, a: Value, b: Value) -> bool {
    match (a, b) {
        (Value::Si(x), Value::Si(y)) => {
            let d = (x - y).abs();
            match field {
                Field::Temperature => d > 5.0,
                Field::RelativeHumidity => d > 0.2,
                Field::WindSpeed | Field::WindGust => d > 5.0,
                // Circular difference: 359° vs 1° is 2°, never 358°.
                Field::WindDirection => {
                    let c = d % 360.0;
                    c.min(360.0 - c) > 90.0
                }
                Field::ElevationMsl | Field::ElevationEllipsoid => d > 50.0,
                _ => d > 300.0,
            }
        }
        _ => false,
    }
}

fn freshness(c: &MeasurementCandidate, p: &ResolverPolicy, now: i64) -> Freshness {
    match c.age_ms(now) {
        None => Freshness::UnknownAge,
        Some(a) if a > p.expired_after_ms => Freshness::Expired,
        Some(a) if a > p.stale_after_ms => Freshness::Stale,
        Some(_) => Freshness::Fresh,
    }
}

pub fn resolve_field(
    field: Field,
    candidates: &[MeasurementCandidate],
    overrides: &[Override],
    reference: Option<&ReferencePlace>,
    p: &ResolverPolicy,
    now: i64,
) -> ResolvedField {
    let all: Vec<&MeasurementCandidate> = candidates.iter().filter(|c| c.field == field).collect();
    let unavailable = |reasons: Vec<Reason>, alts: Vec<MeasurementCandidate>| ResolvedField {
        field,
        chosen: None,
        origin: Origin::Unavailable,
        freshness: None,
        reasons,
        alternatives: alts,
        disagreement: false,
    };
    let local = |c: &&MeasurementCandidate| matches!(c.source, SourceKind::PhoneSensor | SourceKind::ExternalSensor);
    let remote_ranked = {
        let mut v: Vec<&MeasurementCandidate> =
            all.iter().copied().filter(|c| c.source == SourceKind::StationObservation && usable(c) && !excluded(c, p)).collect();
        v.sort_by(|a, b| penalty(a, p, reference, now).total_cmp(&penalty(b, p, reference, now)).then_with(|| a.id.cmp(&b.id)));
        v
    };
    let alts = |chosen_id: Option<&str>| -> Vec<MeasurementCandidate> {
        remote_ranked.iter().filter(|c| Some(c.id.as_str()) != chosen_id).take(3).map(|c| (*c).clone()).collect()
    };

    // 1. Manual override (automatic candidates are kept as alternatives, not erased).
    if let Some(o) = overrides.iter().find(|o| o.candidate.field == field && o.active(now)) {
        let alternatives = alts(None);
        return ResolvedField {
            field,
            disagreement: alternatives.iter().any(|a| disagrees(field, o.candidate.value, a.value)),
            chosen: Some(o.candidate.clone()),
            origin: Origin::Manual,
            freshness: Some(Freshness::Fresh),
            reasons: vec![Reason::ManualOverride],
            alternatives,
        };
    }

    // 2. Fresh local sensor (newest first).
    let mut locals: Vec<&MeasurementCandidate> = all
        .iter()
        .copied()
        .filter(local)
        .filter(|c| usable(c) && c.age_ms(now).is_some_and(|a| a <= p.local_max_age_ms))
        .collect();
    locals.sort_by_key(|c| std::cmp::Reverse(c.observed_utc_ms));
    if let Some(c) = locals.first() {
        let alternatives = alts(None);
        return ResolvedField {
            field,
            disagreement: alternatives.iter().any(|a| disagrees(field, c.value, a.value)),
            chosen: Some((*c).clone()),
            origin: Origin::Here,
            freshness: Some(Freshness::Fresh),
            reasons: vec![Reason::LocalSensor],
            alternatives,
        };
    }

    // Local actual pressure never falls back to a remote or reduced value (A10);
    // elevation of "here" never comes from a station.
    if field == Field::LocalPressure {
        return unavailable(vec![Reason::LocalPressureRequiresLocalSource], vec![]);
    }
    if field.is_elevation() {
        return unavailable(vec![Reason::NoCandidates], vec![]);
    }

    // 3. Ranked remote observation within the freshness limit (or older if allowed).
    let eligible: Vec<&&MeasurementCandidate> = remote_ranked
        .iter()
        .filter(|c| matches!(freshness(c, p, now), Freshness::Fresh | Freshness::Stale) || p.allow_older)
        .collect();
    if let Some(c) = eligible.first() {
        let f = freshness(c, p, now);
        let alternatives = alts(Some(&c.id));
        let mut reasons = vec![Reason::RankedNearbyObservation];
        if f == Freshness::Expired {
            reasons.push(Reason::OlderDataAllowed);
        }
        return ResolvedField {
            field,
            disagreement: eligible.iter().skip(1).any(|a| disagrees(field, c.value, a.value)),
            chosen: Some((**c).clone()),
            origin: Origin::NearbyObservation,
            freshness: Some(f),
            reasons,
            alternatives,
        };
    }

    // 4. Model estimates: no model provider is enabled in this policy version.
    // 5. Unavailable, with the reason.
    let reason = if all.is_empty() {
        Reason::NoCandidates
    } else if all.iter().all(|c| !usable(c)) {
        Reason::AllMissing
    } else if all.iter().filter(|c| usable(c)).all(|c| excluded(c, p)) {
        Reason::AllExcludedByQc
    } else {
        Reason::AllExpired
    };
    unavailable(vec![reason], alts(None))
}

#[allow(clippy::too_many_arguments)]
pub fn resolve(
    id: &str,
    candidates: &[MeasurementCandidate],
    overrides: &[Override],
    reference: Option<ReferencePlace>,
    issues: Vec<ProviderIssue>,
    p: &ResolverPolicy,
    now: i64,
) -> EnvironmentSnapshot {
    let fields: Vec<ResolvedField> =
        Field::ALL.iter().map(|&f| resolve_field(f, candidates, overrides, reference.as_ref(), p, now)).collect();
    // Compare every pair of available elevation readings transparently.
    let elev: Vec<&MeasurementCandidate> = candidates
        .iter()
        .chain(overrides.iter().filter(|o| o.active(now)).map(|o| &o.candidate))
        .filter(|c| c.field.is_elevation() && c.value.si().is_some())
        .collect();
    let mut elevation_comparisons = Vec::new();
    for i in 0..elev.len() {
        for j in i + 1..elev.len() {
            if let Some(c) = compare_elevations(elev[i], elev[j]) {
                elevation_comparisons.push(c);
            }
        }
    }
    let nws_used = fields.iter().any(|f| f.chosen.as_ref().is_some_and(|c| c.provider == crate::nws::PROVIDER));
    EnvironmentSnapshot {
        id: id.to_string(),
        created_utc_ms: now,
        policy_version: RESOLVER_POLICY_VERSION.into(),
        reference,
        fields,
        elevation_comparisons,
        issues,
        redacted: false,
        attribution: if nws_used { vec![crate::nws::ATTRIBUTION.to_string()] } else { vec![] },
    }
}
