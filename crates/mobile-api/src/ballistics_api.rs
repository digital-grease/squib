//! Ballistics facade (plan phase 6, ADR-061, ADR-062). Loads: versioned cartridge and
//! rifle records for the solver, in SI units (the app converts for display), with one
//! remembered as the active load for the ballistics screen.

use squib_ballistics::{DragModel, Load};
use squib_storage::LoadVersion;

use crate::engine::SquibEngine;
use crate::ffi::SquibError;

pub const SETTING_ACTIVE_LOAD: &str = "active_load";

/// What the shooter enters for a load. SI units; `drag_model` is "G1" or "G7".
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct LoadInput {
    pub name: String,
    pub equipment_id: Option<String>,
    pub drag_model: String,
    pub bc_lb_in2: f64,
    pub bullet_mass_kg: f64,
    pub muzzle_velocity_mps: f64,
    pub sight_height_m: f64,
    pub zero_range_m: f64,
    pub bullet_diameter_m: Option<f64>,
    pub bullet_length_m: Option<f64>,
    /// Length of one turn; negative for a left-hand twist.
    pub twist_m: Option<f64>,
    pub notes: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct LoadView {
    pub id: String,
    pub version: u32,
    pub input: LoadInput,
    /// Name of the linked equipment item, as it is now.
    pub equipment_name: Option<String>,
    pub active: bool,
    pub created_utc_ms: i64,
}

fn parse_model(s: &str) -> Result<DragModel, SquibError> {
    match s.trim().to_ascii_uppercase().as_str() {
        "G1" => Ok(DragModel::G1),
        "G7" => Ok(DragModel::G7),
        other => Err(SquibError::Invalid(format!("drag model must be G1 or G7, not {other:?}"))),
    }
}

fn to_version(id: String, version: u32, i: LoadInput, now_utc_ms: i64) -> Result<LoadVersion, SquibError> {
    Ok(LoadVersion {
        load_id: id,
        version,
        name: i.name,
        equipment_id: i.equipment_id.filter(|e| !e.is_empty()),
        load: Load {
            model: parse_model(&i.drag_model)?,
            bc_lb_in2: i.bc_lb_in2,
            mass_kg: i.bullet_mass_kg,
            muzzle_velocity_mps: i.muzzle_velocity_mps,
            sight_height_m: i.sight_height_m,
            zero_range_m: i.zero_range_m,
        },
        bullet_diameter_m: i.bullet_diameter_m,
        bullet_length_m: i.bullet_length_m,
        twist_m: i.twist_m,
        notes: i.notes,
        created_utc_ms: now_utc_ms,
    })
}

pub(crate) fn load_view(repo: &squib_storage::Repository, v: LoadVersion, active: Option<&str>) -> LoadView {
    let equipment_name = v
        .equipment_id
        .as_deref()
        .and_then(|e| repo.list_equipment().ok()?.into_iter().find(|x| x.equipment_id == e))
        .map(|e| e.name);
    LoadView {
        active: active == Some(v.load_id.as_str()),
        id: v.load_id,
        version: v.version,
        equipment_name,
        created_utc_ms: v.created_utc_ms,
        input: LoadInput {
            name: v.name,
            equipment_id: v.equipment_id,
            drag_model: v.load.model.name().into(),
            bc_lb_in2: v.load.bc_lb_in2,
            bullet_mass_kg: v.load.mass_kg,
            muzzle_velocity_mps: v.load.muzzle_velocity_mps,
            sight_height_m: v.load.sight_height_m,
            zero_range_m: v.load.zero_range_m,
            bullet_diameter_m: v.bullet_diameter_m,
            bullet_length_m: v.bullet_length_m,
            twist_m: v.twist_m,
            notes: v.notes,
        },
    }
}

#[uniffi::export]
impl SquibEngine {
    /// Current version of every load not archived, by name.
    pub fn list_loads(&self) -> Result<Vec<LoadView>, SquibError> {
        let repo = self.read_repo();
        let active = repo.get_setting(SETTING_ACTIVE_LOAD)?;
        Ok(repo.list_loads()?.into_iter().map(|v| load_view(&repo, v, active.as_deref())).collect())
    }

    /// Every version of one load, oldest first.
    pub fn load_history(&self, id: String) -> Result<Vec<LoadView>, SquibError> {
        let repo = self.read_repo();
        Ok(repo.load_history(&id)?.into_iter().map(|v| load_view(&repo, v, None)).collect())
    }

    /// Save a load: with no `id`, a new load (version 1); otherwise the next version of
    /// that load. Saved range cards keep the version they used.
    pub fn save_load(&self, id: Option<String>, input: LoadInput, now_utc_ms: i64) -> Result<LoadView, SquibError> {
        let (id, version) = match id {
            None => (uuid::Uuid::new_v4().to_string(), 1),
            Some(id) => {
                let latest = self.read_repo().load_history(&id)?.last().map(|v| v.version);
                let latest = latest.ok_or_else(|| SquibError::NotFound(format!("load {id}")))?;
                (id, latest + 1)
            }
        };
        let v = to_version(id.clone(), version, input, now_utc_ms)?;
        let saved = v.clone();
        self.store_actor().exec(move |repo| repo.save_load_version(&saved))?;
        let repo = self.read_repo();
        let active = repo.get_setting(SETTING_ACTIVE_LOAD)?;
        Ok(load_view(&repo, v, active.as_deref()))
    }

    pub fn archive_load(&self, id: String) -> Result<(), SquibError> {
        self.store_actor().exec(move |repo| {
            repo.archive_load(&id)?;
            if repo.get_setting(SETTING_ACTIVE_LOAD)?.as_deref() == Some(id.as_str()) {
                repo.set_setting(SETTING_ACTIVE_LOAD, "")?;
            }
            Ok(())
        })?;
        Ok(())
    }

    /// Remember the load the ballistics screen opens with (`None` to clear).
    pub fn set_active_load(&self, id: Option<String>) -> Result<(), SquibError> {
        if let Some(i) = &id
            && !self.read_repo().list_loads()?.iter().any(|v| &v.load_id == i)
        {
            return Err(SquibError::NotFound(format!("load {i}")));
        }
        let value = id.unwrap_or_default();
        self.store_actor().exec(move |repo| repo.set_setting(SETTING_ACTIVE_LOAD, &value))?;
        Ok(())
    }
}
