//! Equipment profiles facade (plan 1.2): versioned local labels, an active choice used
//! for new runs (locked while a run is active), and names for history and review.

use squib_storage::{EQUIPMENT_CATEGORIES, EquipmentVersion, equipment_ref, parse_equipment_ref};

use crate::engine::SquibEngine;
use crate::ffi::SquibError;

pub const SETTING_ACTIVE_EQUIPMENT: &str = "active_equipment";

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct EquipmentView {
    pub id: String,
    pub version: u32,
    pub name: String,
    /// pistol, rifle, pcc, shotgun, rimfire, or other.
    pub category: String,
    pub notes: String,
    pub active: bool,
}

fn view(v: EquipmentVersion, active: &Option<String>) -> EquipmentView {
    EquipmentView {
        active: active.as_deref() == Some(v.equipment_id.as_str()),
        id: v.equipment_id,
        version: v.version,
        name: v.name,
        category: v.category,
        notes: v.notes,
    }
}

impl SquibEngine {
    /// The active equipment's current version reference for a new run, if any.
    pub(crate) fn active_equipment_ref(&self) -> Option<String> {
        let repo = self.read_repo();
        let id = repo.get_setting(SETTING_ACTIVE_EQUIPMENT).ok().flatten()?;
        let v = repo.list_equipment().ok()?.into_iter().find(|e| e.equipment_id == id)?;
        Some(equipment_ref(&v.equipment_id, v.version))
    }
}

/// "Name (category)" for a run's pinned equipment reference, as it was then. Takes the
/// caller's repository so it never re-locks the read connection.
pub(crate) fn equipment_label(repo: &squib_storage::Repository, reference: Option<&str>) -> Option<String> {
    let (id, v) = parse_equipment_ref(reference?)?;
    let e = repo.load_equipment_version(&id, v).ok()??;
    Some(format!("{} ({})", e.name, e.category))
}

#[uniffi::export]
impl SquibEngine {
    pub fn equipment_categories(&self) -> Vec<String> {
        EQUIPMENT_CATEGORIES.iter().map(|c| c.to_string()).collect()
    }

    pub fn list_equipment(&self) -> Result<Vec<EquipmentView>, SquibError> {
        let repo = self.read_repo();
        let active = repo.get_setting(SETTING_ACTIVE_EQUIPMENT)?;
        Ok(repo.list_equipment()?.into_iter().map(|v| view(v, &active)).collect())
    }

    pub fn add_equipment(
        &self,
        name: String,
        category: String,
        notes: String,
        now_utc_ms: i64,
    ) -> Result<EquipmentView, SquibError> {
        let v = EquipmentVersion {
            equipment_id: uuid::Uuid::new_v4().to_string(),
            version: 1,
            name,
            category,
            notes,
            created_utc_ms: now_utc_ms,
        };
        let saved = v.clone();
        self.store_actor().exec(move |repo| repo.save_equipment_version(&saved))?;
        Ok(view(v, &None))
    }

    /// Editing creates the next version; runs keep the version they used.
    pub fn edit_equipment(
        &self,
        id: String,
        name: String,
        category: String,
        notes: String,
        now_utc_ms: i64,
    ) -> Result<EquipmentView, SquibError> {
        let latest = self
            .read_repo()
            .list_equipment()?
            .into_iter()
            .find(|e| e.equipment_id == id)
            .ok_or_else(|| SquibError::NotFound(format!("equipment {id}")))?;
        let v =
            EquipmentVersion { equipment_id: id, version: latest.version + 1, name, category, notes, created_utc_ms: now_utc_ms };
        let saved = v.clone();
        self.store_actor().exec(move |repo| repo.save_equipment_version(&saved))?;
        let active = self.read_repo().get_setting(SETTING_ACTIVE_EQUIPMENT)?;
        Ok(view(v, &active))
    }

    pub fn archive_equipment(&self, id: String) -> Result<(), SquibError> {
        self.store_actor().exec(move |repo| {
            repo.archive_equipment(&id)?;
            if repo.get_setting(SETTING_ACTIVE_EQUIPMENT)?.as_deref() == Some(id.as_str()) {
                repo.set_setting(SETTING_ACTIVE_EQUIPMENT, "")?;
            }
            Ok(())
        })?;
        Ok(())
    }

    /// Choose the equipment for new runs (`None` for no equipment). Refused during a run.
    pub fn set_active_equipment(&self, id: Option<String>) -> Result<(), SquibError> {
        if self.run_active() {
            return Err(SquibError::Rejected("finish or cancel the current run before changing equipment".into()));
        }
        if let Some(i) = &id
            && !self.read_repo().list_equipment()?.iter().any(|e| &e.equipment_id == i)
        {
            return Err(SquibError::NotFound(format!("equipment {i}")));
        }
        let value = id.unwrap_or_default();
        self.store_actor().exec(move |repo| repo.set_setting(SETTING_ACTIVE_EQUIPMENT, &value))?;
        Ok(())
    }
}
