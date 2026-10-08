//! Location-retention redaction (docs/squib/08 "Location privacy").
//!
//! With precise retention off (the default), a snapshot saved with a run keeps values,
//! units, origin categories, QC codes, reason codes, and times, and removes lookup and
//! station coordinates, station identities and names, place labels, fine elevation,
//! and per-station issue identities. History then shows "precise provenance not
//! retained". Exact replay is intentionally limited by this choice.

use serde::{Deserialize, Serialize};

use crate::candidate::{Field, MeasurementCandidate};
use crate::nws::ProviderIssue;
use crate::resolver::{EnvironmentSnapshot, Reason};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocationRetention {
    /// Default: precise provenance is not stored with runs.
    Redacted,
    /// User opted in: full provenance is stored (private backups only).
    Precise,
}

impl LocationRetention {
    pub fn as_str(self) -> &'static str {
        match self {
            LocationRetention::Redacted => "redacted",
            LocationRetention::Precise => "precise",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "redacted" => Some(LocationRetention::Redacted),
            "precise" => Some(LocationRetention::Precise),
            _ => None,
        }
    }
}

fn scrub(c: &mut MeasurementCandidate) {
    c.station = None;
    // Candidate ids embed station identifiers.
    c.id = format!("{}:redacted:{}", c.provider, c.field.as_str());
}

pub fn redact(s: &EnvironmentSnapshot) -> EnvironmentSnapshot {
    let mut r = s.clone();
    r.reference = None;
    r.redacted = true;
    r.elevation_comparisons.clear();
    for f in &mut r.fields {
        if matches!(f.field, Field::ElevationMsl | Field::ElevationEllipsoid) && f.chosen.is_some() {
            f.chosen = None;
            f.alternatives.clear();
            f.reasons.push(Reason::PreciseProvenanceNotRetained);
            continue;
        }
        if let Some(c) = f.chosen.as_mut() {
            scrub(c);
        }
        for a in &mut f.alternatives {
            scrub(a);
        }
    }
    for i in &mut r.issues {
        match i {
            ProviderIssue::RejectedValue { station_id, .. } => *station_id = "redacted".into(),
            ProviderIssue::DisallowedUrl { url } => *url = "redacted".into(),
            _ => {}
        }
    }
    r
}

pub fn apply(s: &EnvironmentSnapshot, retention: LocationRetention) -> EnvironmentSnapshot {
    match retention {
        LocationRetention::Precise => s.clone(),
        LocationRetention::Redacted => redact(s),
    }
}
