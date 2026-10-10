//! Versioned equipment profiles (docs/squib/06 `EquipmentProfileVersion`, plan 1.2).
//! A run references `"{equipment_id}#{version}"` in its immutable configuration.

use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use squib_domain::RunConfig;

use crate::{Repository, Result, StorageError};

pub const EQUIPMENT_CATEGORIES: [&str; 6] = ["pistol", "rifle", "pcc", "shotgun", "rimfire", "other"];
pub const MAX_EQUIPMENT_NAME: usize = 80;
pub const MAX_EQUIPMENT_NOTES: usize = 1000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EquipmentVersion {
    pub equipment_id: String,
    pub version: u32,
    pub name: String,
    pub category: String,
    pub notes: String,
    pub created_utc_ms: i64,
}

pub fn equipment_ref(id: &str, version: u32) -> String {
    format!("{id}#{version}")
}

pub fn parse_equipment_ref(r: &str) -> Option<(String, u32)> {
    let (id, v) = r.rsplit_once('#')?;
    Some((id.to_string(), v.parse().ok().filter(|v| *v >= 1)?))
}

impl EquipmentVersion {
    pub fn validate(&self) -> Result<()> {
        let name = self.name.trim();
        if name.is_empty() || name.chars().count() > MAX_EQUIPMENT_NAME {
            return Err(StorageError::Conflict(format!("name must be 1-{MAX_EQUIPMENT_NAME} characters")));
        }
        if !EQUIPMENT_CATEGORIES.contains(&self.category.as_str()) {
            return Err(StorageError::Conflict(format!("category must be one of {}", EQUIPMENT_CATEGORIES.join(", "))));
        }
        if self.notes.chars().count() > MAX_EQUIPMENT_NOTES {
            return Err(StorageError::Conflict(format!("notes must be at most {MAX_EQUIPMENT_NOTES} characters")));
        }
        Ok(())
    }
}

impl Repository {
    /// Save version 1 of a new item, or the next version of an existing one. Any other
    /// version number is refused, so history cannot be rewritten.
    pub fn save_equipment_version(&mut self, v: &EquipmentVersion) -> Result<()> {
        v.validate()?;
        let tx = self.conn.transaction()?;
        let latest: Option<u32> = tx
            .query_row("SELECT MAX(version) FROM equipment_version WHERE equipment_id = ?1", params![v.equipment_id], |r| {
                r.get(0)
            })
            .optional()?
            .flatten();
        match latest {
            None if v.version == 1 => {
                tx.execute(
                    "INSERT INTO equipment(id, created_utc_ms) VALUES (?1, ?2)",
                    params![v.equipment_id, v.created_utc_ms],
                )?;
            }
            Some(l) if v.version == l + 1 => {}
            _ => return Err(StorageError::Conflict("equipment versions must be sequential".into())),
        }
        tx.execute(
            "INSERT INTO equipment_version(equipment_id, version, name, category, notes, created_utc_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![v.equipment_id, v.version, v.name.trim(), v.category, v.notes, v.created_utc_ms],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn load_equipment_version(&self, id: &str, version: u32) -> Result<Option<EquipmentVersion>> {
        Ok(self
            .conn
            .query_row(
                "SELECT name, category, notes, created_utc_ms FROM equipment_version WHERE equipment_id = ?1 AND version = ?2",
                params![id, version],
                |r| {
                    Ok(EquipmentVersion {
                        equipment_id: id.to_string(),
                        version,
                        name: r.get(0)?,
                        category: r.get(1)?,
                        notes: r.get(2)?,
                        created_utc_ms: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    /// Latest version of every item that is not archived, by name.
    pub fn list_equipment(&self) -> Result<Vec<EquipmentVersion>> {
        let mut st = self.conn.prepare(
            "SELECT v.equipment_id, v.version, v.name, v.category, v.notes, v.created_utc_ms
             FROM equipment_version v JOIN equipment e ON e.id = v.equipment_id
             WHERE e.archived = 0
               AND v.version = (SELECT MAX(version) FROM equipment_version x WHERE x.equipment_id = v.equipment_id)
             ORDER BY v.name COLLATE NOCASE",
        )?;
        let rows = st.query_map([], |r| {
            Ok(EquipmentVersion {
                equipment_id: r.get(0)?,
                version: r.get(1)?,
                name: r.get(2)?,
                category: r.get(3)?,
                notes: r.get(4)?,
                created_utc_ms: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Hide an item from selection. Runs that used it keep their reference.
    pub fn archive_equipment(&self, id: &str) -> Result<()> {
        let n = self.conn.execute("UPDATE equipment SET archived = 1 WHERE id = ?1", params![id])?;
        if n == 0 {
            return Err(StorageError::NotFound(format!("equipment {id}")));
        }
        Ok(())
    }

    pub(crate) fn check_equipment_ref(tx: &Transaction<'_>, cfg: &RunConfig) -> Result<Option<(String, u32)>> {
        let Some(r) = cfg.equipment_version_id.as_deref() else { return Ok(None) };
        let (id, v) = parse_equipment_ref(r).ok_or_else(|| StorageError::Conflict(format!("bad equipment reference {r}")))?;
        let exists = tx
            .query_row("SELECT 1 FROM equipment_version WHERE equipment_id = ?1 AND version = ?2", params![id, v], |_| Ok(()))
            .optional()?
            .is_some();
        if !exists {
            return Err(StorageError::Conflict(format!("equipment {id} version {v} does not exist")));
        }
        Ok(Some((id, v)))
    }
}
