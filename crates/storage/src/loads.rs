//! Ballistics loads and saved range cards (plan phase 6.3 and 6.4, ADR-062).
//!
//! A load is versioned like equipment: editing appends a version and a saved card keeps
//! the version it used. A card stores the solver's full input and output with the
//! method that produced them, so a result can always be explained later.

use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use squib_ballistics::{DragModel, Load, Shot, Solution};

use crate::{Repository, Result, StorageError};

pub const MAX_LOAD_NAME: usize = 80;
pub const MAX_LOAD_NOTES: usize = 1000;
pub const MAX_CARD_TITLE: usize = 80;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadVersion {
    pub load_id: String,
    pub version: u32,
    pub name: String,
    /// The rifle it is for, when equipment profiles are kept.
    pub equipment_id: Option<String>,
    pub load: Load,
    /// Kept for later optional terms (spin drift); not used by the solver yet.
    pub bullet_diameter_m: Option<f64>,
    pub bullet_length_m: Option<f64>,
    /// Length of one turn; negative for a left-hand twist.
    pub twist_m: Option<f64>,
    pub notes: String,
    pub created_utc_ms: i64,
}

fn conflict(m: impl Into<String>) -> StorageError {
    StorageError::Conflict(m.into())
}

fn in_range(v: Option<f64>, lo: f64, hi: f64) -> bool {
    v.is_none_or(|x| x.is_finite() && (lo..=hi).contains(&x.abs()))
}

impl LoadVersion {
    pub fn validate(&self) -> Result<()> {
        let n = self.name.trim().chars().count();
        if n == 0 || n > MAX_LOAD_NAME {
            return Err(conflict(format!("name must be 1-{MAX_LOAD_NAME} characters")));
        }
        if self.notes.chars().count() > MAX_LOAD_NOTES {
            return Err(conflict(format!("notes must be at most {MAX_LOAD_NOTES} characters")));
        }
        squib_ballistics::validate_load(&self.load).map_err(|e| conflict(e.to_string()))?;
        if !in_range(self.bullet_diameter_m, 0.002, 0.03) {
            return Err(conflict("bullet diameter must be 2 to 30 mm"));
        }
        if !in_range(self.bullet_length_m, 0.005, 0.08) {
            return Err(conflict("bullet length must be 5 to 80 mm"));
        }
        if !in_range(self.twist_m, 0.1, 1.0) {
            return Err(conflict("twist must be one turn in 10 to 100 cm"));
        }
        Ok(())
    }
}

/// What a range card was computed from: the full solver input and the distances.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CardInputs {
    pub shot: Shot,
    pub ranges_m: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BallisticCard {
    pub id: String,
    pub load_id: String,
    pub load_version: u32,
    pub environment_snapshot_id: Option<String>,
    pub session_id: Option<String>,
    pub title: String,
    pub inputs: CardInputs,
    pub result: Solution,
    pub created_utc_ms: i64,
}

fn model_name(m: DragModel) -> &'static str {
    m.name()
}

fn model_from(s: &str) -> rusqlite::Result<DragModel> {
    match s {
        "G1" => Ok(DragModel::G1),
        "G7" => Ok(DragModel::G7),
        other => Err(rusqlite::Error::InvalidColumnType(0, format!("drag model {other}"), rusqlite::types::Type::Text)),
    }
}

const LOAD_COLUMNS: &str = "v.load_id, v.version, v.name, v.equipment_id, v.drag_model, v.bc_lb_in2, v.bullet_mass_kg, \
     v.muzzle_velocity_mps, v.sight_height_m, v.zero_range_m, v.bullet_diameter_m, v.bullet_length_m, v.twist_m, \
     v.notes, v.created_utc_ms";

fn load_row(r: &Row<'_>) -> rusqlite::Result<LoadVersion> {
    Ok(LoadVersion {
        load_id: r.get(0)?,
        version: r.get(1)?,
        name: r.get(2)?,
        equipment_id: r.get(3)?,
        load: Load {
            model: model_from(&r.get::<_, String>(4)?)?,
            bc_lb_in2: r.get(5)?,
            mass_kg: r.get(6)?,
            muzzle_velocity_mps: r.get(7)?,
            sight_height_m: r.get(8)?,
            zero_range_m: r.get(9)?,
        },
        bullet_diameter_m: r.get(10)?,
        bullet_length_m: r.get(11)?,
        twist_m: r.get(12)?,
        notes: r.get(13)?,
        created_utc_ms: r.get(14)?,
    })
}

impl Repository {
    /// Save version 1 of a new load, or the next version of an existing one. Any other
    /// version number is refused, so history cannot be rewritten.
    pub fn save_load_version(&mut self, v: &LoadVersion) -> Result<()> {
        v.validate()?;
        let tx = self.conn.transaction()?;
        if let Some(eq) = &v.equipment_id {
            let exists = tx.query_row("SELECT 1 FROM equipment WHERE id = ?1", params![eq], |_| Ok(())).optional()?.is_some();
            if !exists {
                return Err(conflict(format!("equipment {eq} does not exist")));
            }
        }
        let latest: Option<u32> = tx
            .query_row("SELECT MAX(version) FROM load_version WHERE load_id = ?1", params![v.load_id], |r| r.get(0))
            .optional()?
            .flatten();
        match latest {
            None if v.version == 1 => {
                tx.execute("INSERT INTO load(id, created_utc_ms) VALUES (?1, ?2)", params![v.load_id, v.created_utc_ms])?;
            }
            Some(l) if v.version == l + 1 => {}
            _ => return Err(conflict("load versions must be sequential")),
        }
        let l = &v.load;
        tx.execute(
            "INSERT INTO load_version(load_id, version, name, equipment_id, drag_model, bc_lb_in2, bullet_mass_kg,
                 muzzle_velocity_mps, sight_height_m, zero_range_m, bullet_diameter_m, bullet_length_m, twist_m, notes,
                 created_utc_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                v.load_id,
                v.version,
                v.name.trim(),
                v.equipment_id,
                model_name(l.model),
                l.bc_lb_in2,
                l.mass_kg,
                l.muzzle_velocity_mps,
                l.sight_height_m,
                l.zero_range_m,
                v.bullet_diameter_m,
                v.bullet_length_m,
                v.twist_m,
                v.notes,
                v.created_utc_ms
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn load_load_version(&self, id: &str, version: u32) -> Result<Option<LoadVersion>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {LOAD_COLUMNS} FROM load_version v WHERE v.load_id = ?1 AND v.version = ?2"),
                params![id, version],
                load_row,
            )
            .optional()?)
    }

    /// Latest version of every load that is not archived, by name.
    pub fn list_loads(&self) -> Result<Vec<LoadVersion>> {
        let mut st = self.conn.prepare(&format!(
            "SELECT {LOAD_COLUMNS} FROM load_version v JOIN load l ON l.id = v.load_id
             WHERE l.archived = 0
               AND v.version = (SELECT MAX(version) FROM load_version x WHERE x.load_id = v.load_id)
             ORDER BY v.name COLLATE NOCASE"
        ))?;
        Ok(st.query_map([], load_row)?.collect::<std::result::Result<_, _>>()?)
    }

    /// Every version of one load, oldest first.
    pub fn load_history(&self, id: &str) -> Result<Vec<LoadVersion>> {
        let mut st =
            self.conn.prepare(&format!("SELECT {LOAD_COLUMNS} FROM load_version v WHERE v.load_id = ?1 ORDER BY v.version"))?;
        Ok(st.query_map(params![id], load_row)?.collect::<std::result::Result<_, _>>()?)
    }

    /// Hide a load from selection. Saved cards keep their reference.
    pub fn archive_load(&self, id: &str) -> Result<()> {
        let n = self.conn.execute("UPDATE load SET archived = 1 WHERE id = ?1", params![id])?;
        if n == 0 {
            return Err(StorageError::NotFound(format!("load {id}")));
        }
        Ok(())
    }

    /// Save a range card. Its load must be exactly the stored load version, and its
    /// result must carry a method, so a card can never claim inputs it was not made from.
    pub fn save_ballistic_card(&mut self, c: &BallisticCard) -> Result<()> {
        if c.title.chars().count() > MAX_CARD_TITLE {
            return Err(conflict(format!("title must be at most {MAX_CARD_TITLE} characters")));
        }
        let lv = self
            .load_load_version(&c.load_id, c.load_version)?
            .ok_or_else(|| conflict(format!("load {} version {} does not exist", c.load_id, c.load_version)))?;
        if lv.load != c.inputs.shot.load {
            return Err(conflict("the card's load does not match the saved load version"));
        }
        if let Some(s) = c.inputs.shot.spin {
            let from_load = (lv.twist_m, lv.bullet_diameter_m, lv.bullet_length_m);
            if from_load != (Some(s.twist_m), Some(s.diameter_m), Some(s.length_m)) {
                return Err(conflict("the card's spin drift inputs do not match the saved load version"));
            }
        }
        if c.result.method.trim().is_empty() || c.result.method.chars().count() > squib_ballistics::MAX_METHOD_CHARS {
            return Err(conflict("a card must record its method"));
        }
        if c.result.points.len() > squib_ballistics::MAX_POINTS || c.inputs.ranges_m.len() > squib_ballistics::MAX_POINTS {
            return Err(conflict("too many distances"));
        }
        let inputs = serde_json::to_string(&c.inputs).map_err(|e| conflict(e.to_string()))?;
        let result = serde_json::to_string(&c.result).map_err(|e| conflict(e.to_string()))?;
        self.conn.execute(
            "INSERT INTO ballistic_card(id, load_id, load_version, environment_snapshot_id, session_id, title, inputs_json,
                 result_json, method, created_utc_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                c.id,
                c.load_id,
                c.load_version,
                c.environment_snapshot_id,
                c.session_id,
                c.title.trim(),
                inputs,
                result,
                c.result.method,
                c.created_utc_ms
            ],
        )?;
        Ok(())
    }

    fn card_row(r: &Row<'_>) -> rusqlite::Result<BallisticCard> {
        let bad = |i: usize, e: serde_json::Error| {
            rusqlite::Error::FromSqlConversionFailure(i, rusqlite::types::Type::Text, Box::new(e))
        };
        Ok(BallisticCard {
            id: r.get(0)?,
            load_id: r.get(1)?,
            load_version: r.get(2)?,
            environment_snapshot_id: r.get(3)?,
            session_id: r.get(4)?,
            title: r.get(5)?,
            inputs: serde_json::from_str(&r.get::<_, String>(6)?).map_err(|e| bad(6, e))?,
            result: serde_json::from_str(&r.get::<_, String>(7)?).map_err(|e| bad(7, e))?,
            created_utc_ms: r.get(8)?,
        })
    }

    pub fn ballistic_card(&self, id: &str) -> Result<Option<BallisticCard>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, load_id, load_version, environment_snapshot_id, session_id, title, inputs_json, result_json,
                     created_utc_ms FROM ballistic_card WHERE id = ?1",
                params![id],
                Self::card_row,
            )
            .optional()?)
    }

    /// Saved cards, newest first.
    pub fn list_ballistic_cards(&self, limit: u32) -> Result<Vec<BallisticCard>> {
        let mut st = self.conn.prepare(
            "SELECT id, load_id, load_version, environment_snapshot_id, session_id, title, inputs_json, result_json,
                 created_utc_ms FROM ballistic_card ORDER BY created_utc_ms DESC, id LIMIT ?1",
        )?;
        Ok(st.query_map(params![limit], Self::card_row)?.collect::<std::result::Result<_, _>>()?)
    }

    /// Delete one saved card (its conditions snapshot too, when nothing else uses it).
    pub fn delete_ballistic_card(&mut self, id: &str) -> Result<()> {
        let snapshot: Option<Option<String>> = self
            .conn
            .query_row("SELECT environment_snapshot_id FROM ballistic_card WHERE id = ?1", params![id], |r| r.get(0))
            .optional()?;
        let Some(snapshot) = snapshot else { return Err(StorageError::NotFound(format!("card {id}"))) };
        let id = id.to_string();
        self.with_guard(move |tx, rep| {
            rep.cards += tx.execute("DELETE FROM ballistic_card WHERE id = ?1", params![id])? as u32;
            if let Some(s) = snapshot {
                tx.execute(
                    "DELETE FROM environment_snapshot WHERE id = ?1 AND NOT EXISTS (SELECT 1 FROM run WHERE environment_snapshot_id = ?1)
                       AND NOT EXISTS (SELECT 1 FROM ballistic_card WHERE environment_snapshot_id = ?1)",
                    params![s],
                )?;
            }
            Ok(())
        })?;
        Ok(())
    }
}
