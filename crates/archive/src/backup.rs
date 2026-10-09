//! Private backup export and validated, transactional import.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, params_from_iter};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use squib_storage::{MIGRATIONS, Repository, SCHEMA_VERSION};

use crate::cell::{Cell, from_sql, to_sql};
use crate::{ArchiveError, Result};

pub const FORMAT: &str = "squib-private-backup";
pub const FORMAT_VERSION: u32 = 1;

/// How rows of a table are compared when an id already exists locally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Merge {
    /// Every column must match; otherwise it is a conflict.
    Exact,
    /// Keep the local row (profiles, settings); never a conflict.
    KeepLocal,
}

/// Tables in dependency order, with primary-key columns and merge policy.
/// `provider_cache` (re-fetchable) and `deletion_guard` are never exported.
const TABLES: &[(&str, &[&str], Merge)] = &[
    ("shooter_profile", &["id"], Merge::KeepLocal),
    ("session", &["id"], Merge::Exact),
    ("calibration_profile_version", &["id"], Merge::Exact),
    ("environment_snapshot", &["id"], Merge::Exact),
    ("drill", &["id"], Merge::Exact),
    ("drill_version", &["drill_id", "version"], Merge::Exact),
    ("run", &["id"], Merge::Exact),
    ("capture_epoch", &["id"], Merge::Exact),
    ("cue_observation", &["run_id", "cue_id"], Merge::Exact),
    ("detected_candidate", &["run_id", "epoch_id", "sequence"], Merge::Exact),
    ("quality_event", &["run_id", "seq"], Merge::Exact),
    ("energy_envelope", &["run_id", "epoch_id", "first_frame"], Merge::Exact),
    ("run_revision", &["run_id", "number"], Merge::Exact),
    ("score_revision", &["run_id", "number"], Merge::Exact),
    ("manual_string", &["run_id"], Merge::Exact),
    ("round_count", &["run_id"], Merge::Exact),
    ("attachment", &["id"], Merge::Exact),
    // Plans stay editable (notes, skip, checks), so a local copy wins over an older backup.
    ("day_plan", &["id"], Merge::KeepLocal),
    ("plan_item", &["id"], Merge::KeepLocal),
    ("plan_run", &["item_id", "run_id"], Merge::Exact),
    ("checklist_item", &["id"], Merge::KeepLocal),
    ("saved_place", &["id"], Merge::Exact),
    ("env_override", &["field"], Merge::KeepLocal),
    ("app_setting", &["key"], Merge::KeepLocal),
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Contains {
    /// Saved places, remembered places, or precise conditions provenance.
    pub location: bool,
    pub attachments: bool,
    /// Always false: Squib does not record audio.
    pub raw_audio: bool,
    pub notes: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub format_version: u32,
    pub schema_version: u32,
    pub app_version: String,
    pub created_utc_ms: i64,
    pub private: bool,
    pub encrypted: bool,
    pub contains: Contains,
    pub row_counts: BTreeMap<String, u64>,
    pub files: Vec<FileEntry>,
}

type Row = BTreeMap<String, Option<Cell>>;

fn sha(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

fn dump_table(conn: &Connection, table: &str, pk: &[&str]) -> Result<Vec<Row>> {
    let mut st = conn.prepare(&format!("SELECT * FROM {table} ORDER BY {}", pk.join(", ")))?;
    let cols: Vec<String> = st.column_names().iter().map(|c| c.to_string()).collect();
    let mut rows = st.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let mut row = Row::new();
        for (i, c) in cols.iter().enumerate() {
            row.insert(c.clone(), from_sql(r.get_ref(i)?));
        }
        out.push(row);
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExportOptions {
    pub app_version: String,
    pub created_utc_ms: i64,
    /// App-owned attachment directory; `None` excludes attachment files.
    pub attachment_root: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportSummary {
    pub bytes: u64,
    pub runs: u64,
    pub attachments: u64,
    pub contains: Contains,
}

/// Write a private backup to `out` from one consistent read snapshot.
pub fn export_private(repo: &mut Repository, out: &Path, opts: &ExportOptions) -> Result<ExportSummary> {
    let conn = repo.connection();
    let tx = conn.transaction()?; // one read snapshot for every table
    let mut tables: BTreeMap<&str, Vec<Row>> = BTreeMap::new();
    for (t, pk, _) in TABLES {
        tables.insert(t, dump_table(&tx, t, pk)?);
    }
    tx.finish()?;
    let count = |t: &str| tables.get(t).map(|v| v.len()).unwrap_or(0) as u64;
    let precise_snapshot =
        tables["environment_snapshot"].iter().any(|r| matches!(r.get("retention"), Some(Some(Cell::Text(v))) if v == "precise"));
    let remembered_place =
        tables["app_setting"].iter().any(|r| matches!(r.get("key"), Some(Some(Cell::Text(k))) if k == "last_place"));
    let include_files = opts.attachment_root.is_some();
    let contains = Contains {
        location: count("saved_place") > 0 || precise_snapshot || remembered_place,
        attachments: include_files && count("attachment") > 0,
        raw_audio: false,
        notes: true,
    };

    let file = std::fs::File::create(out)?;
    let mut zw = zip::ZipWriter::new(file);
    let fo = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut files = Vec::new();
    for (t, rows) in &tables {
        let body = serde_json::to_vec(rows).map_err(|e| ArchiveError::Io(e.to_string()))?;
        let path = format!("data/{t}.json");
        zw.start_file(&path, fo)?;
        zw.write_all(&body)?;
        files.push(FileEntry { sha256: sha(&body), bytes: body.len() as u64, path });
    }
    let mut n_att = 0;
    if let Some(root) = &opts.attachment_root {
        for r in &tables["attachment"] {
            let Some(Some(Cell::Text(rel))) = r.get("relative_path") else { continue };
            // Never read outside the attachment root, whatever the journal says.
            if !squib_storage::valid_attachment_path(rel) {
                return Err(ArchiveError::InvalidData { table: "attachment".into(), detail: format!("unsafe path {rel:?}") });
            }
            let body = std::fs::read(root.join(rel))?;
            let path = format!("attachments/{rel}");
            zw.start_file(&path, fo)?;
            zw.write_all(&body)?;
            files.push(FileEntry { sha256: sha(&body), bytes: body.len() as u64, path });
            n_att += 1;
        }
    }
    let manifest = Manifest {
        format: FORMAT.into(),
        format_version: FORMAT_VERSION,
        schema_version: SCHEMA_VERSION,
        app_version: opts.app_version.clone(),
        created_utc_ms: opts.created_utc_ms,
        private: true,
        encrypted: false,
        contains: contains.clone(),
        row_counts: tables.iter().map(|(k, v)| (k.to_string(), v.len() as u64)).collect(),
        files,
    };
    zw.start_file("manifest.json", fo)?;
    zw.write_all(&serde_json::to_vec_pretty(&manifest).map_err(|e| ArchiveError::Io(e.to_string()))?)?;
    zw.finish()?;
    Ok(ExportSummary { bytes: std::fs::metadata(out)?.len(), runs: count("run"), attachments: n_att, contains })
}

// ---- Import ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    pub max_archive_bytes: u64,
    pub max_entries: usize,
    pub max_entry_bytes: u64,
    pub max_total_bytes: u64,
    pub max_ratio: u64,
    pub max_rows_per_table: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_archive_bytes: 512 * 1024 * 1024,
            max_entries: 20_000,
            max_entry_bytes: 64 * 1024 * 1024,
            max_total_bytes: 1024 * 1024 * 1024,
            max_ratio: 200,
            max_rows_per_table: 2_000_000,
        }
    }
}

fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && !name.starts_with('/')
        && !name.contains('\\')
        && !name.contains('\0')
        && !name.contains(':')
        && name.split('/').all(|p| !p.is_empty() && p != "." && p != "..")
        && (name == "manifest.json" || name.starts_with("data/") || name.starts_with("attachments/"))
}

/// Archive contents after structural validation; nothing written anywhere yet.
struct Loaded {
    manifest: Manifest,
    tables: BTreeMap<String, Vec<Row>>,
    attachments: BTreeMap<String, Vec<u8>>,
}

fn load(path: &Path, limits: &Limits) -> Result<Loaded> {
    if std::fs::metadata(path)?.len() > limits.max_archive_bytes {
        return Err(ArchiveError::TooLarge);
    }
    let mut za = zip::ZipArchive::new(std::fs::File::open(path)?)?;
    if za.len() > limits.max_entries {
        return Err(ArchiveError::TooManyEntries);
    }
    let mut total = 0u64;
    let mut contents: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for i in 0..za.len() {
        let mut f = za.by_index(i)?;
        let name = f.name().to_string();
        let is_symlink = f.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000);
        if !safe_name(&name) || f.is_dir() || is_symlink || f.size() > limits.max_entry_bytes || contents.contains_key(&name) {
            return Err(ArchiveError::BadEntry(name));
        }
        if f.size() > 1024 && f.size() / f.compressed_size().max(1) > limits.max_ratio {
            return Err(ArchiveError::CompressionRatio(name));
        }
        total += f.size();
        if total > limits.max_total_bytes {
            return Err(ArchiveError::TooLarge);
        }
        // Read at most the declared size + 1 to catch lying headers.
        let declared = f.size();
        let mut buf = Vec::with_capacity(declared as usize);
        (&mut f).take(declared + 1).read_to_end(&mut buf)?;
        if buf.len() as u64 != declared {
            return Err(ArchiveError::BadEntry(name));
        }
        contents.insert(name, buf);
    }
    let mbytes = contents.remove("manifest.json").ok_or(ArchiveError::NotSquib("no manifest".into()))?;
    let manifest: Manifest = serde_json::from_slice(&mbytes).map_err(|e| ArchiveError::NotSquib(e.to_string()))?;
    if manifest.format != FORMAT || manifest.format_version != FORMAT_VERSION {
        return Err(ArchiveError::NotSquib(format!("{} v{}", manifest.format, manifest.format_version)));
    }
    if manifest.schema_version > SCHEMA_VERSION {
        return Err(ArchiveError::NewerSchema { found: manifest.schema_version, supported: SCHEMA_VERSION });
    }
    if manifest.schema_version == 0 {
        return Err(ArchiveError::NotSquib("schema 0".into()));
    }
    // Inventory: every file listed, hashed, and nothing unlisted.
    let listed: BTreeSet<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
    for name in contents.keys() {
        if !listed.contains(name.as_str()) {
            return Err(ArchiveError::Inventory(name.clone()));
        }
    }
    for f in &manifest.files {
        let body = contents.get(&f.path).ok_or_else(|| ArchiveError::Inventory(f.path.clone()))?;
        if sha(body) != f.sha256 || body.len() as u64 != f.bytes {
            return Err(ArchiveError::HashMismatch(f.path.clone()));
        }
    }
    let known: BTreeSet<&str> = TABLES.iter().map(|t| t.0).collect();
    let mut tables = BTreeMap::new();
    let mut attachments = BTreeMap::new();
    for (name, body) in contents {
        if let Some(t) = name.strip_prefix("data/").and_then(|n| n.strip_suffix(".json")) {
            if !known.contains(t) {
                return Err(ArchiveError::InvalidData { table: t.into(), detail: "unknown table".into() });
            }
            let rows: Vec<Row> = serde_json::from_slice(&body)
                .map_err(|e| ArchiveError::InvalidData { table: t.into(), detail: e.to_string() })?;
            if rows.len() > limits.max_rows_per_table {
                return Err(ArchiveError::InvalidData { table: t.into(), detail: "too many rows".into() });
            }
            tables.insert(t.to_string(), rows);
        } else if let Some(rel) = name.strip_prefix("attachments/") {
            attachments.insert(rel.to_string(), body);
        } else {
            return Err(ArchiveError::BadEntry(name));
        }
    }
    Ok(Loaded { manifest, tables, attachments })
}

fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let mut st = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    Ok(st.query_map([], |r| r.get::<_, String>(1))?.collect::<std::result::Result<_, _>>()?)
}

/// Build a staging database at the archive's schema version, insert its rows, then
/// migrate it to the current schema and verify integrity and revision hashes.
fn stage(l: &Loaded, dir: &Path) -> Result<PathBuf> {
    let path = dir.join(format!("squib-import-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    {
        let mut conn = Connection::open(&path)?;
        conn.pragma_update(None, "foreign_keys", "OFF")?;
        let tx = conn.transaction()?;
        for (v, sql) in MIGRATIONS.iter().take(l.manifest.schema_version as usize) {
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", v)?;
        }
        for (t, _, _) in TABLES {
            let Some(rows) = l.tables.get(*t) else { continue };
            let cols = table_columns(&tx, t)?;
            for row in rows {
                // Attachment paths later reach the filesystem: validate before storing.
                if *t == "attachment" {
                    match row.get("relative_path") {
                        Some(Some(Cell::Text(p))) if squib_storage::valid_attachment_path(p) => {}
                        _ => {
                            return Err(ArchiveError::InvalidData {
                                table: "attachment".into(),
                                detail: "unsafe relative_path".into(),
                            });
                        }
                    }
                }
                // Column names come from the archive: accept only real columns.
                for k in row.keys() {
                    if !cols.contains(k) {
                        return Err(ArchiveError::InvalidData { table: t.to_string(), detail: format!("unknown column {k:?}") });
                    }
                }
                let names: Vec<&String> = row.keys().collect();
                let sql = format!(
                    "INSERT INTO {t} ({}) VALUES ({})",
                    names.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join(", "),
                    vec!["?"; names.len()].join(", ")
                );
                let vals: Vec<rusqlite::types::Value> = row
                    .values()
                    .map(to_sql)
                    .collect::<std::result::Result<_, _>>()
                    .map_err(|e| ArchiveError::InvalidData { table: t.to_string(), detail: e })?;
                tx.execute(&sql, params_from_iter(vals))
                    .map_err(|e| ArchiveError::InvalidData { table: t.to_string(), detail: e.to_string() })?;
            }
        }
        tx.commit()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut st = conn.prepare("PRAGMA foreign_key_check")?;
        if st.exists([])? {
            return Err(ArchiveError::InvalidData { table: "*".into(), detail: "broken references".into() });
        }
    }
    // Migrate with the app's own migrations, then verify every run and revision hash.
    // Any failure validating staged data means the archive's data is invalid.
    let invalid = |table: &str| {
        let table = table.to_string();
        move |e: squib_storage::StorageError| ArchiveError::InvalidData { table: table.clone(), detail: e.to_string() }
    };
    let repo = Repository::open(&path).map_err(invalid("*"))?;
    repo.quick_check().map_err(invalid("*"))?;
    for s in repo.list_runs(u32::MAX).map_err(invalid("run"))? {
        repo.load_run(&s.row.run_id).map_err(invalid("run"))?;
    }
    for d in repo.list_drills().map_err(invalid("drill_version"))? {
        d.recipe.validate().map_err(|e| ArchiveError::InvalidData { table: "drill_version".into(), detail: e.to_string() })?;
    }
    for (rel, body) in &l.attachments {
        let listed =
            l.tables.get("attachment").into_iter().flatten().find_map(|r| match (r.get("relative_path"), r.get("sha256")) {
                (Some(Some(Cell::Text(p))), Some(Some(Cell::Text(h)))) if p == rel => Some(h.clone()),
                _ => None,
            });
        match listed {
            Some(h) if h == sha(body) => {}
            _ => return Err(ArchiveError::Inventory(format!("attachments/{rel}"))),
        }
    }
    drop(repo);
    Ok(path)
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ImportReport {
    pub inserted: BTreeMap<String, u64>,
    pub skipped_identical: BTreeMap<String, u64>,
    pub kept_local: BTreeMap<String, u64>,
    pub conflicts: Vec<String>,
    pub attachments_restored: u64,
    pub schema_version: u32,
    pub contains_location: bool,
}

fn merge(conn: &mut Connection, staging: &Path, commit: bool) -> Result<ImportReport> {
    conn.execute("ATTACH DATABASE ?1 AS s", [staging.to_string_lossy()])?;
    let result = (|| -> Result<ImportReport> {
        let tx = conn.transaction()?;
        let mut rep = ImportReport { schema_version: SCHEMA_VERSION, ..Default::default() };
        for (t, pk, policy) in TABLES {
            let cols = table_columns(&tx, t)?;
            let q = |c: &String| format!("\"{c}\"");
            let on = pk.iter().map(|k| format!("m.\"{k}\" = x.\"{k}\"")).collect::<Vec<_>>().join(" AND ");
            let same = cols.iter().map(|c| format!("m.{0} IS x.{0}", q(c))).collect::<Vec<_>>().join(" AND ");
            let total: u64 = tx.query_row(&format!("SELECT COUNT(*) FROM s.{t}"), [], |r| r.get::<_, i64>(0))? as u64;
            let existing: u64 = tx
                .query_row(&format!("SELECT COUNT(*) FROM s.{t} x JOIN main.{t} m ON {on}"), [], |r| r.get::<_, i64>(0))?
                as u64;
            let identical: u64 =
                tx.query_row(&format!("SELECT COUNT(*) FROM s.{t} x JOIN main.{t} m ON {on} WHERE {same}"), [], |r| {
                    r.get::<_, i64>(0)
                })? as u64;
            if existing > identical {
                match policy {
                    Merge::Exact => {
                        let first = pk[0];
                        let mut st = tx.prepare(&format!(
                            "SELECT CAST(x.\"{first}\" AS TEXT) FROM s.{t} x JOIN main.{t} m ON {on} WHERE NOT ({same}) LIMIT 20"
                        ))?;
                        let ids: Vec<String> = st.query_map([], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
                        rep.conflicts.extend(ids.into_iter().map(|id| format!("{t}:{id}")));
                    }
                    Merge::KeepLocal => {
                        rep.kept_local.insert(t.to_string(), existing - identical);
                    }
                }
            }
            let collist = cols.iter().map(q).collect::<Vec<_>>().join(", ");
            let n = tx.execute(
                &format!(
                    "INSERT INTO main.{t} ({collist}) SELECT {collist} FROM s.{t} x WHERE NOT EXISTS (SELECT 1 FROM main.{t} m WHERE {on})"
                ),
                [],
            )? as u64;
            rep.inserted.insert(t.to_string(), n);
            rep.skipped_identical.insert(t.to_string(), identical);
            debug_assert_eq!(n + existing, total);
        }
        if !rep.conflicts.is_empty() || !commit {
            tx.rollback()?;
        } else {
            tx.commit()?;
        }
        Ok(rep)
    })();
    conn.execute("DETACH DATABASE s", [])?;
    result
}

/// Remove the staging database and any migration backups made of it.
fn cleanup(staging: &Path) {
    let _ = std::fs::remove_file(staging);
    for v in 1..=SCHEMA_VERSION {
        let _ = std::fs::remove_file(staging.with_extension(format!("pre-v{v}.bak")));
    }
    for ext in ["db-wal", "db-shm"] {
        let _ = std::fs::remove_file(staging.with_extension(ext));
    }
}

/// Validate and stage an archive and report what an import would do. Nothing is
/// written to the journal.
pub fn preview(repo: &mut Repository, archive: &Path, scratch_dir: &Path, limits: &Limits) -> Result<ImportReport> {
    let l = load(archive, limits)?;
    let staging =
        stage(&l, scratch_dir).inspect_err(|_| cleanup(&scratch_dir.join(format!("squib-import-{}.db", std::process::id()))))?;
    let r = merge(repo.connection(), &staging, false);
    cleanup(&staging);
    let mut r = r?;
    r.schema_version = l.manifest.schema_version;
    r.contains_location = l.manifest.contains.location;
    Ok(r)
}

/// Import an archive. All-or-nothing: on any validation failure or conflict the
/// journal is unchanged. Attachment files are written only after the journal commit
/// and never overwrite an existing different file.
pub fn import(
    repo: &mut Repository,
    archive: &Path,
    scratch_dir: &Path,
    attachment_root: Option<&Path>,
    limits: &Limits,
) -> Result<ImportReport> {
    let l = load(archive, limits)?;
    let staging =
        stage(&l, scratch_dir).inspect_err(|_| cleanup(&scratch_dir.join(format!("squib-import-{}.db", std::process::id()))))?;
    let r = merge(repo.connection(), &staging, true);
    cleanup(&staging);
    let mut r = r?;
    if !r.conflicts.is_empty() {
        return Err(ArchiveError::Conflicts(r.conflicts.len()));
    }
    r.schema_version = l.manifest.schema_version;
    r.contains_location = l.manifest.contains.location;
    if let Some(root) = attachment_root {
        for (rel, body) in &l.attachments {
            let dest = root.join(rel);
            if dest.exists() {
                continue; // same path implies same row; hash was checked in staging
            }
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let tmp = dest.with_extension("partial");
            std::fs::write(&tmp, body)?;
            std::fs::rename(&tmp, &dest)?;
            r.attachments_restored += 1;
        }
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_names_are_constrained() {
        for ok in ["manifest.json", "data/run.json", "attachments/photos/a.jpg"] {
            assert!(safe_name(ok), "{ok}");
        }
        for bad in ["../x", "/etc/passwd", "data/../../x", "attachments/a\\b", "C:/x", "other/x", "data//x", "data/./x", ""] {
            assert!(!safe_name(bad), "{bad}");
        }
    }
}
