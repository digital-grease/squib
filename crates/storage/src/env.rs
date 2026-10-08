//! Repository operations for range conditions (schema 2).

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use squib_environment::nws::CachedDoc;
use squib_environment::privacy::{LocationRetention, apply};
use squib_environment::{EnvironmentSnapshot, Field, Override};

use crate::{Repository, Result, StorageError};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedPlace {
    pub id: String,
    pub label: String,
    pub lat: f64,
    pub lon: f64,
    pub created_utc_ms: i64,
}

pub const SETTING_WEATHER_ENABLED: &str = "weather_lookup_enabled";
pub const SETTING_LOCATION_RETENTION: &str = "location_retention";
pub const SETTING_UNITS: &str = "display_units";
/// Last chosen place, stored only for user-entered or saved places, or for GPS places
/// when precise retention is on.
pub const SETTING_LAST_PLACE: &str = "last_place";

impl Repository {
    /// Store a snapshot with the retention policy applied before it reaches disk.
    /// Returns what was stored.
    pub fn insert_snapshot(&self, s: &EnvironmentSnapshot, retention: LocationRetention) -> Result<EnvironmentSnapshot> {
        let stored = apply(s, retention);
        self.conn.execute(
            "INSERT INTO environment_snapshot(id, created_utc_ms, policy_version, retention, content_json)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![stored.id, stored.created_utc_ms, stored.policy_version, retention.as_str(), serde_json::to_string(&stored)?],
        )?;
        Ok(stored)
    }

    pub fn load_snapshot(&self, id: &str) -> Result<Option<EnvironmentSnapshot>> {
        let json: Option<String> = self
            .conn
            .query_row("SELECT content_json FROM environment_snapshot WHERE id = ?1", params![id], |r| r.get(0))
            .optional()?;
        json.map(|j| serde_json::from_str(&j).map_err(StorageError::from)).transpose()
    }

    pub fn cache_get(&self, provider: &str, key: &str) -> Result<Option<CachedDoc>> {
        Ok(self
            .conn
            .query_row(
                "SELECT fetched_utc_ms, expires_utc_ms, etag, body FROM provider_cache WHERE provider = ?1 AND cache_key = ?2",
                params![provider, key],
                |r| {
                    Ok(CachedDoc {
                        key: key.to_string(),
                        fetched_utc_ms: r.get(0)?,
                        expires_utc_ms: r.get(1)?,
                        etag: r.get(2)?,
                        body: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn cache_put(&self, provider: &str, d: &CachedDoc) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO provider_cache(provider, cache_key, fetched_utc_ms, expires_utc_ms, etag, body)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![provider, d.key, d.fetched_utc_ms, d.expires_utc_ms, d.etag, d.body],
        )?;
        Ok(())
    }

    /// Clear cached provider documents. Journal data and snapshots are untouched.
    pub fn cache_clear(&self) -> Result<usize> {
        Ok(self.conn.execute("DELETE FROM provider_cache", [])?)
    }

    pub fn set_override(&self, o: &Override) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO env_override(field, candidate_json, set_utc_ms, expires_utc_ms) VALUES (?1, ?2, ?3, ?4)",
            params![o.candidate.field.as_str(), serde_json::to_string(&o.candidate)?, o.set_utc_ms, o.expires_utc_ms],
        )?;
        Ok(())
    }

    pub fn clear_override(&self, field: Field) -> Result<()> {
        self.conn.execute("DELETE FROM env_override WHERE field = ?1", params![field.as_str()])?;
        Ok(())
    }

    pub fn list_overrides(&self) -> Result<Vec<Override>> {
        let mut st = self.conn.prepare("SELECT candidate_json, set_utc_ms, expires_utc_ms FROM env_override ORDER BY field")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, Option<i64>>(2)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (j, set, exp) = row?;
            out.push(Override { candidate: serde_json::from_str(&j)?, set_utc_ms: set, expires_utc_ms: exp });
        }
        Ok(out)
    }

    pub fn add_place(&self, p: &SavedPlace) -> Result<()> {
        if p.label.trim().is_empty() || p.label.len() > 80 {
            return Err(StorageError::Conflict("place label must be 1-80 characters".into()));
        }
        self.conn.execute(
            "INSERT INTO saved_place(id, label, lat, lon, created_utc_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![p.id, p.label.trim(), p.lat, p.lon, p.created_utc_ms],
        )?;
        Ok(())
    }

    pub fn list_places(&self) -> Result<Vec<SavedPlace>> {
        let mut st = self.conn.prepare("SELECT id, label, lat, lon, created_utc_ms FROM saved_place ORDER BY label")?;
        let rows = st.query_map([], |r| {
            Ok(SavedPlace { id: r.get(0)?, label: r.get(1)?, lat: r.get(2)?, lon: r.get(3)?, created_utc_ms: r.get(4)? })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn delete_place(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM saved_place WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT value FROM app_setting WHERE key = ?1", params![key], |r| r.get(0)).optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute("INSERT OR REPLACE INTO app_setting(key, value) VALUES (?1, ?2)", params![key, value])?;
        Ok(())
    }
}
