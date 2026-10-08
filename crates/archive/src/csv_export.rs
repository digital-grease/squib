//! CSV analysis export (not a restore format). Explicit unit columns, source method,
//! review and outcome quality, and conditions origin categories. Text cells that a
//! spreadsheet could interpret as a formula are prefixed with an apostrophe (OWASP
//! CSV-injection guidance); a real CSV encoder handles quoting.

use squib_environment::Field;
use squib_storage::Repository;
use squib_training::analytics::RunRecord;

use crate::{Result, iso_utc};

/// Neutralize spreadsheet formula prefixes in text cells.
pub fn escape_text(s: &str) -> String {
    match s.chars().next() {
        Some('=' | '+' | '-' | '@' | '\t' | '\r') => format!("'{s}"),
        _ => s.to_string(),
    }
}

fn secs(ns: Option<i64>) -> String {
    ns.map(|v| format!("{:.3}", v as f64 / 1e9)).unwrap_or_default()
}

/// One row per run.
pub fn runs_csv(repo: &Repository) -> Result<String> {
    let records: Vec<RunRecord> = repo.analytics_records()?;
    let mut w = csv::Writer::from_writer(Vec::new());
    w.write_record([
        "run_id",
        "started_utc",
        "mode",
        "outcome",
        "review_state",
        "edited",
        "timing_method",
        "drill",
        "shots",
        "first_shot_s",
        "final_time_s",
        "splits_s",
        "hit_factor",
        "score_complete",
        "temperature_c",
        "temperature_origin",
        "wind_speed_m_s",
        "wind_origin",
    ])
    .map_err(io)?;
    for r in &records {
        let detail = repo.load_run(&r.run_id)?;
        let shots = detail.revisions.last().map(|v| v.content.accepted.len()).unwrap_or(0);
        let env = detail.environment.as_ref();
        let field = |f: Field| env.and_then(|s| s.field(f));
        let temp = field(Field::Temperature);
        let wind = field(Field::WindSpeed);
        let origin = |rf: Option<&squib_environment::ResolvedField>| {
            rf.map(|x| serde_json::to_value(x.origin).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default())
                .unwrap_or_default()
        };
        let drill = match (&r.drill_id, r.drill_version) {
            (Some(d), Some(v)) => {
                repo.load_drill_version(d, v)?.map(|dv| format!("{} v{v}", dv.recipe.title)).unwrap_or_default()
            }
            _ => String::new(),
        };
        w.write_record([
            escape_text(&r.run_id),
            iso_utc(r.created_utc_ms),
            escape_text(&r.mode),
            r.outcome.map(|o| o.as_str().to_string()).unwrap_or_default(),
            r.review_state.map(|o| o.as_str().to_string()).unwrap_or_default(),
            r.edited.to_string(),
            escape_text(&r.timing_method),
            escape_text(&drill),
            shots.to_string(),
            secs(r.first_ns),
            secs(r.final_ns),
            r.splits_ns.iter().map(|s| format!("{:.3}", *s as f64 / 1e9)).collect::<Vec<_>>().join(";"),
            r.hit_factor.map(|h| format!("{h:.4}")).unwrap_or_default(),
            r.score_complete.to_string(),
            temp.and_then(|t| t.chosen.as_ref())
                .and_then(|c| c.value.si())
                .map(|k| format!("{:.1}", k - 273.15))
                .unwrap_or_default(),
            origin(temp),
            wind.and_then(|t| t.chosen.as_ref()).and_then(|c| c.value.si()).map(|v| format!("{v:.1}")).unwrap_or_default(),
            origin(wind),
        ])
        .map_err(io)?;
    }
    let bytes = w.into_inner().map_err(|e| crate::ArchiveError::Io(e.to_string()))?;
    String::from_utf8(bytes).map_err(|e| crate::ArchiveError::Io(e.to_string()))
}

fn io(e: csv::Error) -> crate::ArchiveError {
    crate::ArchiveError::Io(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formula_prefixes_are_neutralized() {
        assert_eq!(escape_text("=HYPERLINK(\"x\")"), "'=HYPERLINK(\"x\")");
        assert_eq!(escape_text("+1"), "'+1");
        assert_eq!(escape_text("-2"), "'-2");
        assert_eq!(escape_text("@SUM(A1)"), "'@SUM(A1)");
        assert_eq!(escape_text("\tx"), "'\tx");
        assert_eq!(escape_text("Club timer"), "Club timer");
    }
}
