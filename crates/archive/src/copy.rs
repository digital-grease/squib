//! Import as copy (docs/squib/06 "Import contract": "Import-as-copy generates a full
//! internal ID/reference remap while preserving original-source identifiers as
//! provenance"). Conflicting archives are still rejected by a normal import; a copy is
//! the explicit way to keep both.
//!
//! - Journal records always get new ids: sessions, runs and everything under them,
//!   capture epochs, conditions snapshots, attachments (with new file paths), day plans,
//!   their items and checklists, and saved places.
//! - Reference data (drills, equipment, calibration profiles) keeps its id unless the
//!   same id exists locally with different content; then it is copied too.
//! - Shooter profiles, settings, overrides, and the checklist template are never
//!   remapped: a copy lands under the existing profiles.
//!
//! Ids are replaced wherever they occur: id and reference columns, `"{id}#{version}"`
//! references, and inside JSON columns. Hashes that cover ids (run configuration,
//! review and score revision chains) are recomputed; staging then re-verifies them.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{Connection, OptionalExtension};
use serde_json::Value as Json;
use squib_domain::RunConfig;
use squib_domain::hash::content_hash;
use squib_domain::revision::RevisionContent;
use squib_training::scoring::ScoreEntry;

use crate::backup::{Loaded, Row};
use crate::cell::Cell;
use crate::{ArchiveError, Result};

/// Journal entities whose ids are always replaced in a copy: (table, id column).
const ALWAYS: &[(&str, &str)] = &[
    ("session", "id"),
    ("run", "id"),
    ("capture_epoch", "id"),
    ("environment_snapshot", "id"),
    ("attachment", "id"),
    ("day_plan", "id"),
    ("plan_item", "id"),
    ("saved_place", "id"),
];

/// (entity table, id column, optional (version table, its id column)).
type ReferenceTable = (&'static str, &'static str, Option<(&'static str, &'static str)>);

/// Reference data copied only when the same id holds different content locally.
const REFERENCE: &[ReferenceTable] = &[
    ("drill", "id", Some(("drill_version", "drill_id"))),
    ("equipment", "id", Some(("equipment_version", "equipment_id"))),
    ("calibration_profile_version", "id", None),
];

fn text<'a>(row: &'a Row, col: &str) -> Option<&'a str> {
    match row.get(col) {
        Some(Some(Cell::Text(s))) => Some(s.as_str()),
        _ => None,
    }
}

fn set_text(row: &mut Row, col: &str, v: String) {
    row.insert(col.to_string(), Some(Cell::Text(v)));
}

/// Does any archive row of `table` for `id` differ from the local row with the same key?
fn conflicts_locally(local: &Connection, l: &Loaded, table: &str, id_col: &str, id: &str) -> Result<bool> {
    for row in l.tables.get(table).into_iter().flatten().filter(|r| text(r, id_col) == Some(id)) {
        let cols: Vec<&String> = row.keys().collect();
        let mut conds = Vec::new();
        let mut vals: Vec<rusqlite::types::Value> = Vec::new();
        // Match the primary key: the id column, plus `version` for version tables.
        conds.push(format!("\"{id_col}\" = ?"));
        vals.push(rusqlite::types::Value::Text(id.to_string()));
        if let Some(Some(Cell::Int(v))) = row.get("version") {
            conds.push("\"version\" = ?".into());
            vals.push(rusqlite::types::Value::Integer(v.parse().unwrap_or(-1)));
        }
        let exists: Option<i64> = local
            .query_row(
                &format!("SELECT 1 FROM {table} WHERE {}", conds.join(" AND ")),
                rusqlite::params_from_iter(vals.clone()),
                |r| r.get(0),
            )
            .optional()?;
        if exists.is_none() {
            continue;
        }
        for c in cols {
            let v = crate::cell::to_sql(row.get(c).unwrap_or(&None))
                .map_err(|e| ArchiveError::InvalidData { table: table.into(), detail: e })?;
            let mut all = vals.clone();
            all.push(v);
            let same: Option<i64> = local
                .query_row(
                    &format!("SELECT 1 FROM {table} WHERE {} AND \"{c}\" IS ?", conds.join(" AND ")),
                    rusqlite::params_from_iter(all),
                    |r| r.get(0),
                )
                .optional()?;
            if same.is_none() {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn remap_str(s: &str, map: &BTreeMap<String, String>) -> Option<String> {
    if let Some(n) = map.get(s) {
        return Some(n.clone());
    }
    // `"{id}#{version}"` references (drills, equipment).
    if let Some((id, v)) = s.rsplit_once('#')
        && v.chars().all(|c| c.is_ascii_digit())
        && let Some(n) = map.get(id)
    {
        return Some(format!("{n}#{v}"));
    }
    None
}

/// Replace ids inside a JSON value; returns whether anything changed.
fn remap_json(v: &mut Json, map: &BTreeMap<String, String>) -> bool {
    match v {
        Json::String(s) => match remap_str(s, map) {
            Some(n) => {
                *s = n;
                true
            }
            None => false,
        },
        Json::Array(a) => a.iter_mut().fold(false, |acc, x| remap_json(x, map) | acc),
        Json::Object(o) => o.values_mut().fold(false, |acc, x| remap_json(x, map) | acc),
        _ => false,
    }
}

/// New path for a copied attachment: same directory and extension, new file name.
fn new_path(old: &str, new_id: &str) -> String {
    let (dir, file) = old.rsplit_once('/').map(|(d, f)| (format!("{d}/"), f)).unwrap_or((String::new(), old));
    let ext = file.rsplit_once('.').map(|(_, e)| format!(".{e}")).unwrap_or_default();
    format!("{dir}{new_id}{ext}")
}

/// Rewrite `l` in place as a copy. Returns how many records were given new ids.
pub(crate) fn remap_for_copy(l: &mut Loaded, local: &Connection, now_utc_ms: i64) -> Result<usize> {
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    let mut entity_of: BTreeMap<String, &'static str> = BTreeMap::new();
    let fresh = || uuid::Uuid::new_v4().to_string();
    for (table, col) in ALWAYS {
        for row in l.tables.get(*table).into_iter().flatten() {
            if let Some(id) = text(row, col) {
                map.entry(id.to_string()).or_insert_with(fresh);
                entity_of.insert(id.to_string(), table);
            }
        }
    }
    // Plan checklists move with their plan; the template (no plan) is a preference.
    for row in l.tables.get("checklist_item").into_iter().flatten() {
        if text(row, "plan_id").is_some()
            && let Some(id) = text(row, "id")
        {
            map.entry(id.to_string()).or_insert_with(fresh);
            entity_of.insert(id.to_string(), "checklist_item");
        }
    }
    for (table, col, versions) in REFERENCE {
        let ids: BTreeSet<String> =
            l.tables.get(*table).into_iter().flatten().filter_map(|r| text(r, col).map(String::from)).collect();
        for id in ids {
            let differs = conflicts_locally(local, l, table, col, &id)?
                || match versions {
                    Some((vt, vc)) => conflicts_locally(local, l, vt, vc, &id)?,
                    None => false,
                };
            if differs {
                map.insert(id.clone(), fresh());
                entity_of.insert(id, table);
            }
        }
    }

    // Attachment paths are unique: copies get new file names, and their files follow.
    let mut path_map: BTreeMap<String, String> = BTreeMap::new();
    for row in l.tables.get_mut("attachment").into_iter().flatten() {
        if let (Some(id), Some(p)) = (text(row, "id").map(String::from), text(row, "relative_path").map(String::from))
            && let Some(n) = map.get(&id)
        {
            let np = new_path(&p, n);
            path_map.insert(p, np.clone());
            set_text(row, "relative_path", np);
        }
    }
    l.attachments =
        std::mem::take(&mut l.attachments).into_iter().map(|(k, v)| (path_map.get(&k).cloned().unwrap_or(k), v)).collect();

    // Replace ids in every text cell; JSON columns are walked structurally.
    for (table, rows) in l.tables.iter_mut() {
        if table == "import_provenance" {
            continue;
        }
        for row in rows.iter_mut() {
            for (col, cell) in row.iter_mut() {
                let Some(Cell::Text(s)) = cell else { continue };
                if col.ends_with("_json") {
                    // Rewrite only when an id changed: re-serializing alone would reorder keys and
                    // make unchanged rows look different from their local copies.
                    if let Ok(mut j) = serde_json::from_str::<Json>(s)
                        && remap_json(&mut j, &map)
                    {
                        *s = serde_json::to_string(&j).map_err(|e| ArchiveError::Io(e.to_string()))?;
                    }
                } else if col != "relative_path"
                    && let Some(n) = remap_str(s, &map)
                {
                    *s = n;
                }
            }
        }
    }

    // Recompute hashes that cover ids.
    let bad = |t: &str, e: String| ArchiveError::InvalidData { table: t.into(), detail: e };
    for row in l.tables.get_mut("run").into_iter().flatten() {
        let cfg: RunConfig =
            serde_json::from_str(text(row, "config_json").unwrap_or("")).map_err(|e| bad("run", e.to_string()))?;
        set_text(row, "config_hash", cfg.config_hash());
    }
    rechain(l, "run_revision", |row, parent| {
        let mut c: RevisionContent =
            serde_json::from_str(text(row, "content_json").unwrap_or("")).map_err(|e| bad("run_revision", e.to_string()))?;
        c.parent_hash = parent.clone();
        let h = content_hash(&c);
        set_text(row, "content_json", serde_json::to_string(&c).map_err(|e| bad("run_revision", e.to_string()))?);
        Ok(h)
    })?;
    rechain(l, "score_revision", |row, parent| {
        let entry: ScoreEntry =
            serde_json::from_str(text(row, "entry_json").unwrap_or("")).map_err(|e| bad("score_revision", e.to_string()))?;
        let run_id = text(row, "run_id").unwrap_or("").to_string();
        let number: u32 = match row.get("number") {
            Some(Some(Cell::Int(v))) => v.parse().unwrap_or(0),
            _ => 0,
        };
        let editor = text(row, "editor").unwrap_or("").to_string();
        let created: i64 = match row.get("created_utc_ms") {
            Some(Some(Cell::Int(v))) => v.parse().unwrap_or(0),
            _ => 0,
        };
        Ok(content_hash(&(run_id.as_str(), number, parent, &entry, editor.as_str(), created)))
    })?;

    let n = map.len();
    let prov = l.tables.entry("import_provenance".into()).or_default();
    for (old, new) in &map {
        let mut row = Row::new();
        set_text(&mut row, "entity", entity_of.get(old).copied().unwrap_or("unknown").to_string());
        set_text(&mut row, "new_id", new.clone());
        set_text(&mut row, "original_id", old.clone());
        row.insert("archive_created_utc_ms".into(), Some(Cell::Int(l.manifest.created_utc_ms.to_string())));
        row.insert("imported_utc_ms".into(), Some(Cell::Int(now_utc_ms.to_string())));
        prov.push(row);
    }
    Ok(n)
}

/// Recompute a per-run hash chain ordered by `number`, updating `parent_hash` and
/// `content_hash`. `hash(row, parent)` returns the row's new content hash.
fn rechain(l: &mut Loaded, table: &str, mut hash: impl FnMut(&mut Row, &Option<String>) -> Result<String>) -> Result<()> {
    let Some(rows) = l.tables.get_mut(table) else { return Ok(()) };
    let num = |r: &Row| match r.get("number") {
        Some(Some(Cell::Int(v))) => v.parse::<i64>().unwrap_or(0),
        _ => 0,
    };
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by_key(|i| (text(&rows[*i], "run_id").unwrap_or("").to_string(), num(&rows[*i])));
    let mut last: Option<(String, String)> = None;
    for i in order {
        let run = text(&rows[i], "run_id").unwrap_or("").to_string();
        let parent = match (&last, rows[i].get("parent_hash")) {
            (Some((r, h)), Some(Some(_))) if *r == run => Some(h.clone()),
            _ => None,
        };
        let h = hash(&mut rows[i], &parent)?;
        match &parent {
            Some(p) => set_text(&mut rows[i], "parent_hash", p.clone()),
            None => {
                rows[i].insert("parent_hash".into(), None);
            }
        }
        set_text(&mut rows[i], "content_hash", h.clone());
        last = Some((run, h));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_and_paths_are_rewritten_precisely() {
        let map: BTreeMap<String, String> =
            [("d1".to_string(), "n1".to_string()), ("run-a".to_string(), "run-b".to_string())].into();
        assert_eq!(remap_str("d1", &map).as_deref(), Some("n1"));
        assert_eq!(remap_str("d1#3", &map).as_deref(), Some("n1#3"), "versioned references");
        assert_eq!(remap_str("d1#x", &map), None, "not a version suffix");
        assert_eq!(remap_str("d10", &map), None, "no partial matches");
        let mut j: Json = serde_json::json!({"run_id": "run-a", "nested": [{"drill_version_id": "d1#2"}], "name": "d1 notes"});
        assert!(remap_json(&mut j, &map));
        assert_eq!(j["run_id"], "run-b");
        assert_eq!(j["nested"][0]["drill_version_id"], "n1#2");
        assert_eq!(j["name"], "d1 notes", "free text containing an id is left alone");
        let mut untouched: Json = serde_json::json!({"a": "x"});
        assert!(!remap_json(&mut untouched, &map));
        assert_eq!(new_path("photos/abc.jpg", "new"), "photos/new.jpg");
        assert_eq!(new_path("diagnostics/run-a.wav", "new"), "diagnostics/new.wav");
        assert_eq!(new_path("plain", "new"), "new");
    }
}
