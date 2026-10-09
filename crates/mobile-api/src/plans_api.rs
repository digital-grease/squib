//! M4 facade: day plans (practice or match agenda), stage notes, checklists, and
//! coach rotation. Schedule times are local wall-clock labels for display; they are
//! never used for run timing.

use squib_storage::{DayPlan, PlanItem, SETTING_COACH_MODE};

use crate::engine::SquibEngine;
use crate::ffi::SquibError;
use crate::training_api::{DrillView, ShooterView, drill_view};

// ---- FFI types ------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PlanSummaryView {
    pub id: String,
    pub title: String,
    pub date_local: String,
    pub kind: String,
    pub items: u32,
    pub done_items: u32,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PlanItemInput {
    /// `drill`, `stage`, or `event`.
    pub kind: String,
    /// Empty for a drill item uses the drill's title.
    pub title: String,
    /// `HH:MM` local time, if scheduled.
    pub time_local: Option<String>,
    pub drill_id: Option<String>,
    pub drill_version: Option<u32>,
    pub target_strings: Option<u32>,
    pub notes: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PlanItemView {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub time_local: Option<String>,
    pub drill_id: Option<String>,
    pub drill_version: Option<u32>,
    pub target_strings: Option<u32>,
    pub notes: String,
    pub skipped: bool,
    /// Completed runs linked to this item.
    pub completed: u32,
    /// Cancelled, interrupted, or failed runs linked to this item.
    pub aborted: u32,
    /// Drill items: target reached. Other items are never marked done automatically.
    pub done: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ChecklistView {
    pub id: String,
    pub text: String,
    pub checked: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct NextUpView {
    pub item_id: String,
    pub title: String,
    pub time_local: String,
    /// Minutes from the supplied local time; negative means it is overdue.
    pub minutes_until: i32,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PlanView {
    pub id: String,
    pub title: String,
    pub date_local: String,
    pub kind: String,
    pub notes: String,
    pub items: Vec<PlanItemView>,
    pub checklist: Vec<ChecklistView>,
    pub next_up: Option<NextUpView>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CoachView {
    pub enabled: bool,
    /// Rotation order; only current, non-archived shooters.
    pub squad: Vec<ShooterView>,
    /// Who shoots after the active shooter, when the rotation has someone else.
    pub next: Option<ShooterView>,
}

// ---- Pure helpers --------------------------------------------------------------------

fn minutes(hhmm: &str) -> Option<i32> {
    let (h, m) = hhmm.split_once(':')?;
    Some(h.parse::<i32>().ok()? * 60 + m.parse::<i32>().ok()?)
}

/// First scheduled, unfinished item at or after `now_min - 30` (a late item stays
/// "next" for half an hour so a running-behind day still shows it).
pub fn next_up(items: &[PlanItemView], now_min: i32) -> Option<NextUpView> {
    items
        .iter()
        .filter(|i| !i.skipped && !i.done)
        .filter_map(|i| Some((i, minutes(i.time_local.as_deref()?)?)))
        .filter(|(_, t)| *t >= now_min - 30)
        .min_by_key(|(_, t)| *t)
        .map(|(i, t)| NextUpView {
            item_id: i.id.clone(),
            title: i.title.clone(),
            time_local: i.time_local.clone().unwrap_or_default(),
            minutes_until: t - now_min,
        })
}

/// The shooter after `active` in `squad`, wrapping; the first one if `active` is not in it.
pub fn next_in_rotation(squad: &[String], active: &str) -> Option<String> {
    let next = match squad.iter().position(|s| s == active) {
        Some(i) => squad.get((i + 1) % squad.len())?,
        None => squad.first()?,
    };
    (next != active).then(|| next.clone())
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn inval(e: impl ToString) -> SquibError {
    SquibError::Invalid(e.to_string())
}

impl SquibEngine {
    fn item_views(&self, plan_id: &str) -> Result<Vec<PlanItemView>, SquibError> {
        let repo = self.read_repo();
        repo.plan_items(plan_id)?
            .into_iter()
            .map(|i| {
                let p = repo.item_progress(&i.id)?;
                let done = i.kind == "drill" && p.completed >= i.target_strings.unwrap_or(1);
                Ok(PlanItemView {
                    id: i.id,
                    kind: i.kind,
                    title: i.title,
                    time_local: i.time_local,
                    drill_id: i.drill_id,
                    drill_version: i.drill_version,
                    target_strings: i.target_strings,
                    notes: i.notes,
                    skipped: i.skipped,
                    completed: p.completed,
                    aborted: p.aborted,
                    done,
                })
            })
            .collect()
    }

    fn checklist_views(&self, plan_id: Option<&str>) -> Result<Vec<ChecklistView>, SquibError> {
        Ok(self
            .read_repo()
            .checklist(plan_id)?
            .into_iter()
            .map(|c| ChecklistView { id: c.id, text: c.text, checked: c.checked })
            .collect())
    }
}

#[uniffi::export]
impl SquibEngine {
    // ---- Plans ---------------------------------------------------------------------------

    pub fn list_plans(&self) -> Result<Vec<PlanSummaryView>, SquibError> {
        let plans = self.read_repo().list_plans()?;
        plans
            .into_iter()
            .map(|p| {
                let items = self.item_views(&p.id)?;
                Ok(PlanSummaryView {
                    done_items: items.iter().filter(|i| i.done).count() as u32,
                    items: items.len() as u32,
                    id: p.id,
                    title: p.title,
                    date_local: p.date_local,
                    kind: p.kind,
                })
            })
            .collect()
    }

    pub fn create_plan(
        &self,
        title: String,
        date_local: String,
        kind: String,
        copy_checklist: bool,
        now_utc_ms: i64,
    ) -> Result<String, SquibError> {
        let p = DayPlan { id: new_id(), title, date_local, kind, notes: String::new(), created_utc_ms: now_utc_ms };
        let id = p.id.clone();
        self.store_actor().exec(move |repo| repo.create_plan(&p, copy_checklist))?;
        Ok(id)
    }

    /// `now_local_min`: minutes since local midnight, for the "next up" line (display only).
    pub fn load_plan(&self, plan_id: String, now_local_min: Option<u32>) -> Result<PlanView, SquibError> {
        let p = self
            .read_repo()
            .list_plans()?
            .into_iter()
            .find(|p| p.id == plan_id)
            .ok_or_else(|| SquibError::NotFound(format!("plan {plan_id}")))?;
        let items = self.item_views(&plan_id)?;
        let next_up = now_local_min.and_then(|m| next_up(&items, m as i32));
        Ok(PlanView {
            checklist: self.checklist_views(Some(&plan_id))?,
            items,
            next_up,
            id: p.id,
            title: p.title,
            date_local: p.date_local,
            kind: p.kind,
            notes: p.notes,
        })
    }

    pub fn set_plan_notes(&self, plan_id: String, notes: String) -> Result<(), SquibError> {
        self.store_actor().exec(move |repo| repo.update_plan_notes(&plan_id, &notes))?;
        Ok(())
    }

    /// Hide a plan. Runs made from it stay in history.
    pub fn archive_plan(&self, plan_id: String) -> Result<(), SquibError> {
        self.store_actor().exec(move |repo| repo.archive_plan(&plan_id))?;
        Ok(())
    }

    // ---- Agenda items --------------------------------------------------------------------

    pub fn add_plan_item(&self, plan_id: String, input: PlanItemInput) -> Result<String, SquibError> {
        let mut title = input.title.trim().to_string();
        if input.kind == "drill" && title.is_empty() {
            let (Some(d), Some(v)) = (&input.drill_id, input.drill_version) else {
                return Err(inval("choose a drill"));
            };
            let dv = self.read_repo().load_drill_version(d, v)?.ok_or_else(|| SquibError::NotFound(format!("drill {d}")))?;
            title = dv.recipe.title;
        }
        if input.target_strings.is_some_and(|n| !(1..=50).contains(&n)) {
            return Err(inval("strings must be 1-50"));
        }
        let item = PlanItem {
            id: new_id(),
            plan_id,
            ordinal: 0,
            kind: input.kind,
            title,
            time_local: input.time_local.filter(|t| !t.trim().is_empty()),
            drill_id: input.drill_id,
            drill_version: input.drill_version,
            target_strings: input.target_strings,
            notes: input.notes,
            skipped: false,
        };
        let id = item.id.clone();
        self.store_actor().exec(move |repo| repo.add_plan_item(&item))?;
        Ok(id)
    }

    pub fn set_item_notes(&self, item_id: String, notes: String) -> Result<(), SquibError> {
        self.store_actor().exec(move |repo| repo.update_item_notes(&item_id, &notes))?;
        Ok(())
    }

    /// Skipping records only that the item was skipped; it never creates results.
    pub fn set_item_skipped(&self, item_id: String, skipped: bool) -> Result<(), SquibError> {
        self.store_actor().exec(move |repo| repo.set_item_skipped(&item_id, skipped))?;
        Ok(())
    }

    pub fn move_plan_item(&self, item_id: String, up: bool) -> Result<(), SquibError> {
        self.store_actor().exec(move |repo| repo.move_item(&item_id, up))?;
        Ok(())
    }

    /// Remove an item from the agenda; its runs stay in history.
    pub fn delete_plan_item(&self, item_id: String) -> Result<(), SquibError> {
        self.store_actor().exec(move |repo| repo.delete_item(&item_id))?;
        Ok(())
    }

    /// A specific drill version, so an agenda item runs the version it was planned with.
    pub fn drill_version(&self, drill_id: String, version: u32) -> Result<DrillView, SquibError> {
        let v = self.read_repo().load_drill_version(&drill_id, version)?;
        v.as_ref().map(drill_view).ok_or_else(|| SquibError::NotFound(format!("drill {drill_id} version {version}")))
    }

    /// Count an existing run (for example a manual entry) toward an agenda item.
    pub fn link_run_to_item(&self, item_id: String, run_id: String) -> Result<(), SquibError> {
        self.store_actor().exec(move |repo| repo.link_run(&item_id, &run_id))?;
        Ok(())
    }

    // ---- Checklists ------------------------------------------------------------------------

    /// The template copied into new plans.
    pub fn checklist_template(&self) -> Result<Vec<ChecklistView>, SquibError> {
        self.checklist_views(None)
    }

    /// `plan_id` `None` adds to the template.
    pub fn add_checklist_item(&self, plan_id: Option<String>, text: String) -> Result<(), SquibError> {
        self.store_actor().exec(move |repo| repo.add_checklist_item(&new_id(), plan_id.as_deref(), &text))?;
        Ok(())
    }

    pub fn set_checklist_checked(&self, id: String, checked: bool) -> Result<(), SquibError> {
        self.store_actor().exec(move |repo| repo.set_checked(&id, checked))?;
        Ok(())
    }

    pub fn delete_checklist_item(&self, id: String) -> Result<(), SquibError> {
        self.store_actor().exec(move |repo| repo.delete_checklist_item(&id))?;
        Ok(())
    }

    // ---- Coach rotation --------------------------------------------------------------------

    pub fn coach(&self) -> CoachView {
        let shooters = self.list_shooters();
        let (enabled, ids) = {
            let repo = self.read_repo();
            let enabled = repo.get_setting(SETTING_COACH_MODE).ok().flatten().as_deref() == Some("on");
            (enabled, repo.squad().unwrap_or_default())
        };
        let view = |id: &str| shooters.iter().find(|s| s.id == id).cloned();
        let active = shooters.iter().find(|s| s.active).map(|s| s.id.clone()).unwrap_or_default();
        CoachView {
            enabled,
            squad: ids.iter().filter_map(|i| view(i)).collect(),
            next: if enabled { next_in_rotation(&ids, &active).and_then(|i| view(&i)) } else { None },
        }
    }

    pub fn set_coach(&self, enabled: bool, squad_ids: Vec<String>) -> Result<CoachView, SquibError> {
        self.store_actor().exec(move |repo| {
            repo.set_squad(&squad_ids)?;
            repo.set_setting(SETTING_COACH_MODE, if enabled { "on" } else { "off" })
        })?;
        Ok(self.coach())
    }

    /// Make the next shooter in the rotation active (refused while a run is in progress).
    pub fn advance_shooter(&self) -> Result<CoachView, SquibError> {
        let next = self.coach().next.ok_or_else(|| SquibError::Rejected("no next shooter in the rotation".into()))?;
        self.set_active_shooter(next.id)?;
        Ok(self.coach())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, time: Option<&str>, done: bool, skipped: bool) -> PlanItemView {
        PlanItemView {
            id: id.into(),
            kind: "event".into(),
            title: id.into(),
            time_local: time.map(Into::into),
            drill_id: None,
            drill_version: None,
            target_strings: None,
            notes: String::new(),
            skipped,
            completed: 0,
            aborted: 0,
            done,
        }
    }

    #[test]
    fn next_up_picks_earliest_unfinished_scheduled_item() {
        let items = [
            item("a", Some("08:00"), false, false),
            item("b", Some("09:15"), false, false),
            item("c", Some("09:00"), false, true),
            item("d", None, false, false),
            item("e", Some("08:50"), true, false),
        ];
        let n = next_up(&items, 8 * 60 + 40).unwrap();
        assert_eq!((n.item_id.as_str(), n.minutes_until), ("b", 35));
        // A slightly late item stays next, reported as overdue.
        let n = next_up(&items, 8 * 60 + 10).unwrap();
        assert_eq!((n.item_id.as_str(), n.minutes_until), ("a", -10));
        assert!(next_up(&items, 23 * 60).is_none());
    }

    #[test]
    fn rotation_wraps_and_skips_self() {
        let s: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
        assert_eq!(next_in_rotation(&s, "a").as_deref(), Some("b"));
        assert_eq!(next_in_rotation(&s, "c").as_deref(), Some("a"));
        assert_eq!(next_in_rotation(&s, "z").as_deref(), Some("a"));
        assert_eq!(next_in_rotation(&["a".to_string()], "a"), None);
        assert_eq!(next_in_rotation(&[], "a"), None);
    }
}
