//! Ballistics facade (plan phase 6, ADR-061 to ADR-063). Loads: versioned cartridge and
//! rifle records for the solver, in SI units (the app converts for display), with one
//! remembered as the active load. Range cards: solved here from a load, air (current
//! conditions or entered), wind and shot angle; saving solves again in the core, pins the
//! conditions used, and stores the full input, output and method. Experimental.

use squib_ballistics::{Atmosphere, DragModel, Earth, Load, Shot, Spin, Wind};
use squib_environment::EnvironmentSnapshot;
use squib_storage::{BallisticCard, CardInputs, LoadVersion};

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

/// Air at the firing point for the ballistics screen.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AirView {
    pub available: bool,
    pub density_kg_m3: Option<f64>,
    pub temperature_k: Option<f64>,
    /// Each input with where it came from, as on the Conditions screen.
    pub inputs: Vec<String>,
    pub uses_older_data: bool,
    /// Why there is no air, when there is none.
    pub reason: Option<String>,
    /// The conditions' place to the nearest whole degree, to suggest for Coriolis; none
    /// when the snapshot's precise place was removed.
    pub latitude_deg: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum AirChoice {
    /// The current conditions (pinned as a snapshot when the card is saved).
    Conditions,
    /// Station pressure as read (not corrected to sea level), temperature and humidity.
    Manual { pressure_pa: f64, temperature_k: f64, relative_humidity: f64 },
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CardRequest {
    pub load_id: String,
    pub air: AirChoice,
    pub wind_mps: f64,
    /// Where the wind comes from, clockwise from the target: 0 head, 90 from the right.
    pub wind_from_deg: f64,
    pub look_angle_deg: f64,
    pub ranges_m: Vec<f64>,
    /// Add spin drift (needs the load's twist, bullet diameter and bullet length).
    pub spin_drift: bool,
    /// Add Coriolis for this latitude and shot direction.
    pub coriolis: Option<CoriolisInput>,
}

#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct CoriolisInput {
    /// Positive north of the equator.
    pub latitude_deg: f64,
    /// Direction of the shot, clockwise from true north.
    pub azimuth_deg: f64,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct RangeRow {
    pub range_m: f64,
    pub drop_m: f64,
    /// Spin drift included.
    pub windage_m: f64,
    /// The part of the windage that is spin drift.
    pub spin_drift_m: f64,
    /// Corrections: positive means dial or hold up, and right.
    pub elevation_mil: f64,
    pub elevation_moa: f64,
    pub windage_mil: f64,
    pub windage_moa: f64,
    pub velocity_mps: f64,
    pub mach: f64,
    pub time_s: f64,
    pub energy_j: f64,
    /// Below Mach 1.2, where the model is least reliable.
    pub transonic: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CardView {
    /// Set once saved.
    pub id: Option<String>,
    pub title: String,
    /// The load version the card was computed with.
    pub load: LoadView,
    pub air_from_conditions: bool,
    pub density_kg_m3: f64,
    pub temperature_k: f64,
    pub air_inputs: Vec<String>,
    pub uses_older_data: bool,
    pub wind_mps: f64,
    pub wind_from_deg: f64,
    pub look_angle_deg: f64,
    pub spin_drift: bool,
    /// Miller stability factor, with spin drift.
    pub stability: Option<f64>,
    pub coriolis: Option<CoriolisInput>,
    pub zero_elevation_mil: f64,
    pub rows: Vec<RangeRow>,
    pub unreached_m: Vec<f64>,
    pub method: String,
    pub created_utc_ms: Option<i64>,
}

fn rejected(e: impl ToString) -> SquibError {
    SquibError::Rejected(e.to_string())
}

/// Air from a resolved snapshot, with the same derivation as the Conditions screen.
fn air_of(s: &EnvironmentSnapshot) -> AirView {
    let (derived, reason) = crate::conditions::derived_of(s);
    let temperature_k = s.field(squib_environment::Field::Temperature).and_then(|f| f.chosen.as_ref()).and_then(|c| c.value.si());
    match (derived, temperature_k) {
        (Some(d), Some(t)) => AirView {
            available: true,
            density_kg_m3: Some(d.density_kg_m3),
            temperature_k: Some(t),
            inputs: d.inputs,
            uses_older_data: d.uses_older_data,
            reason: None,
            latitude_deg: latitude_of(s),
        },
        (_, _) => AirView {
            available: false,
            density_kg_m3: None,
            temperature_k: None,
            inputs: vec![],
            uses_older_data: false,
            reason: Some(reason.unwrap_or_else(|| "Needs temperature.".into())),
            latitude_deg: latitude_of(s),
        },
    }
}

fn latitude_of(s: &EnvironmentSnapshot) -> Option<f64> {
    s.reference.as_ref().filter(|_| !s.redacted).map(|r| r.lat.round()).filter(|l| l.is_finite() && l.abs() <= 90.0)
}

fn card_view(repo: &squib_storage::Repository, c: &BallisticCard, lv: LoadVersion, air: &AirView) -> CardView {
    let shot = &c.inputs.shot;
    let wind = shot.wind.unwrap_or(Wind { speed_mps: 0.0, from_deg: 0.0 });
    CardView {
        id: Some(c.id.clone()).filter(|i| !i.is_empty()),
        title: c.title.clone(),
        load: load_view(repo, lv, None),
        air_from_conditions: c.environment_snapshot_id.is_some() || air.available && !air.inputs.is_empty(),
        density_kg_m3: shot.air.density_kg_m3,
        temperature_k: shot.air.temperature_k,
        air_inputs: air.inputs.clone(),
        uses_older_data: air.uses_older_data,
        wind_mps: wind.speed_mps,
        wind_from_deg: wind.from_deg,
        look_angle_deg: shot.look_angle_deg,
        spin_drift: shot.spin.is_some(),
        stability: c.result.stability,
        coriolis: shot.coriolis.map(|e| CoriolisInput { latitude_deg: e.latitude_deg, azimuth_deg: e.azimuth_deg }),
        zero_elevation_mil: c.result.zero_elevation_rad * 1000.0,
        rows: c
            .result
            .points
            .iter()
            .map(|p| RangeRow {
                range_m: p.range_m,
                drop_m: p.drop_m,
                windage_m: p.windage_m,
                spin_drift_m: p.spin_drift_m,
                elevation_mil: p.elevation_mil,
                elevation_moa: p.elevation_moa,
                windage_mil: p.windage_mil,
                windage_moa: p.windage_moa,
                velocity_mps: p.velocity_mps,
                mach: p.mach,
                time_s: p.time_s,
                energy_j: p.energy_j,
                transonic: p.transonic,
            })
            .collect(),
        unreached_m: c.result.unreached_m.clone(),
        method: c.result.method.clone(),
        created_utc_ms: (c.created_utc_ms > 0).then_some(c.created_utc_ms),
    }
}

impl SquibEngine {
    /// Solve a request. Returns the card (unsaved), the load version, the air shown, and
    /// the snapshot the air came from (to pin on save).
    fn solve_request(
        &self,
        req: &CardRequest,
        now_utc_ms: i64,
    ) -> Result<(BallisticCard, LoadVersion, AirView, Option<EnvironmentSnapshot>), SquibError> {
        let lv = self
            .read_repo()
            .list_loads()?
            .into_iter()
            .find(|v| v.load_id == req.load_id)
            .ok_or_else(|| SquibError::NotFound(format!("load {}", req.load_id)))?;
        let (atmosphere, air, snapshot) = match req.air {
            AirChoice::Conditions => {
                let snap = self
                    .current_snapshot(now_utc_ms)
                    .ok_or_else(|| rejected("no current conditions: refresh them or enter the air yourself"))?;
                let air = air_of(&snap);
                let (Some(d), Some(t)) = (air.density_kg_m3, air.temperature_k) else {
                    return Err(rejected(format!(
                        "current conditions cannot give air density: {}",
                        air.reason.clone().unwrap_or_default()
                    )));
                };
                (Atmosphere { density_kg_m3: d, temperature_k: t }, air, Some(snap))
            }
            AirChoice::Manual { pressure_pa, temperature_k, relative_humidity } => {
                if !(pressure_pa.is_finite() && (30_000.0..=110_000.0).contains(&pressure_pa)) {
                    return Err(rejected("pressure must be 300 to 1100 hPa"));
                }
                if !(relative_humidity.is_finite() && (0.0..=1.0).contains(&relative_humidity)) {
                    return Err(rejected("humidity must be 0 to 100 %"));
                }
                if !temperature_k.is_finite() {
                    return Err(rejected("temperature must be a number"));
                }
                let a = Atmosphere::from_station(pressure_pa, temperature_k - 273.15, relative_humidity);
                let air = AirView {
                    available: true,
                    density_kg_m3: Some(a.density_kg_m3),
                    temperature_k: Some(a.temperature_k),
                    inputs: vec![],
                    uses_older_data: false,
                    reason: None,
                    latitude_deg: None,
                };
                (a, air, None)
            }
        };
        let shot = Shot {
            load: lv.load.clone(),
            air: atmosphere,
            zero_air: None,
            wind: (req.wind_mps != 0.0).then_some(Wind { speed_mps: req.wind_mps, from_deg: req.wind_from_deg }),
            look_angle_deg: req.look_angle_deg,
            spin: if req.spin_drift {
                match (lv.twist_m, lv.bullet_diameter_m, lv.bullet_length_m) {
                    (Some(twist_m), Some(diameter_m), Some(length_m)) => Some(Spin { twist_m, diameter_m, length_m }),
                    _ => return Err(rejected("spin drift needs the load's twist, bullet diameter and bullet length")),
                }
            } else {
                None
            },
            coriolis: req.coriolis.map(|c| Earth { latitude_deg: c.latitude_deg, azimuth_deg: c.azimuth_deg.rem_euclid(360.0) }),
        };
        let result = squib_ballistics::solve(&shot, &req.ranges_m).map_err(rejected)?;
        let card = BallisticCard {
            id: String::new(),
            load_id: lv.load_id.clone(),
            load_version: lv.version,
            environment_snapshot_id: None,
            session_id: None,
            title: String::new(),
            inputs: CardInputs { shot, ranges_m: req.ranges_m.clone() },
            result,
            created_utc_ms: 0,
        };
        Ok((card, lv, air, snapshot))
    }
}

#[uniffi::export]
impl SquibEngine {
    /// Air density and temperature from the current conditions, or why there are none.
    pub fn ballistics_air(&self, now_utc_ms: i64) -> AirView {
        match self.current_snapshot(now_utc_ms) {
            Some(s) => air_of(&s),
            None => AirView {
                available: false,
                density_kg_m3: None,
                temperature_k: None,
                inputs: vec![],
                uses_older_data: false,
                reason: Some("No current conditions. Refresh them on the Conditions screen, or enter the air.".into()),
                latitude_deg: None,
            },
        }
    }

    /// Solve a range card without saving it.
    pub fn solve_card(&self, req: CardRequest, now_utc_ms: i64) -> Result<CardView, SquibError> {
        let (card, lv, air, _) = self.solve_request(&req, now_utc_ms)?;
        Ok(card_view(&self.read_repo(), &card, lv, &air))
    }

    /// Solve again and save: the card records the full input and output and the method;
    /// air from conditions is pinned as a snapshot; the card joins the current visit if
    /// one is under way.
    pub fn save_card(&self, req: CardRequest, title: String, now_utc_ms: i64) -> Result<CardView, SquibError> {
        let (mut card, lv, air, snapshot) = self.solve_request(&req, now_utc_ms)?;
        card.id = uuid::Uuid::new_v4().to_string();
        card.title = title;
        card.created_utc_ms = now_utc_ms;
        let retention = self.retention();
        let shooter = self.active_shooter();
        let to_save = card.clone();
        let saved = self.store_actor().exec(move |repo| {
            let mut c = to_save;
            if let Some(s) = snapshot {
                c.environment_snapshot_id = Some(repo.insert_snapshot(&s, retention)?.id);
            }
            c.session_id = repo.current_session(&shooter, now_utc_ms)?;
            repo.save_ballistic_card(&c)?;
            Ok(c)
        })?;
        Ok(card_view(&self.read_repo(), &saved, lv, &air))
    }

    /// Saved cards, newest first, each with the load version it used.
    pub fn list_cards(&self, limit: u32) -> Result<Vec<CardView>, SquibError> {
        let repo = self.read_repo();
        let mut out = Vec::new();
        for c in repo.list_ballistic_cards(limit)? {
            let Some(lv) = repo.load_load_version(&c.load_id, c.load_version)? else { continue };
            let air = match &c.environment_snapshot_id {
                Some(id) => repo.load_snapshot(id)?.map(|s| air_of(&s)).unwrap_or_else(|| manual_air(&c)),
                None => manual_air(&c),
            };
            out.push(card_view(&repo, &c, lv, &air));
        }
        Ok(out)
    }

    pub fn delete_card(&self, id: String) -> Result<(), SquibError> {
        self.store_actor().exec(move |repo| repo.delete_ballistic_card(&id))?;
        Ok(())
    }
}

fn manual_air(c: &BallisticCard) -> AirView {
    AirView {
        available: true,
        density_kg_m3: Some(c.inputs.shot.air.density_kg_m3),
        temperature_k: Some(c.inputs.shot.air.temperature_k),
        inputs: vec![],
        uses_older_data: false,
        reason: None,
        latitude_deg: None,
    }
}
