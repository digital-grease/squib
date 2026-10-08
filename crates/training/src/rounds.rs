//! Round counts and optional cost (docs/squib/07 "Practice plan and low-cost value").
//!
//! The app proposes a count from accepted live events; the user confirms or adjusts
//! it. The confirmed count is independent of the detector count (A24). Par-only and
//! dry practice propose zero. Cost uses a user-entered per-round amount; nothing is
//! fetched.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoundCount {
    pub run_id: String,
    /// Suggested from accepted live events (0 for par-only/dry runs).
    pub proposed: u32,
    /// What the user confirmed; `None` until confirmed.
    pub confirmed: Option<u32>,
    pub confirmed_utc_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostSetting {
    /// Per-round cost in minor units (e.g. cents) to avoid float money.
    pub per_round_minor: i64,
    /// ISO 4217 code entered by the user.
    pub currency: String,
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum RoundError {
    #[error("round count must be at most 10000")]
    TooMany,
    #[error("cost must be 0-100000 minor units with a 3-letter currency code")]
    Cost,
}

pub fn propose(mode: &str, accepted_events: u32) -> u32 {
    if mode == "phone_live" { accepted_events } else { 0 }
}

pub fn confirm(rc: &RoundCount, n: u32, now: i64) -> Result<RoundCount, RoundError> {
    if n > 10_000 {
        return Err(RoundError::TooMany);
    }
    Ok(RoundCount { confirmed: Some(n), confirmed_utc_ms: Some(now), ..rc.clone() })
}

impl CostSetting {
    pub fn validate(&self) -> Result<(), RoundError> {
        let ok = (0..=100_000).contains(&self.per_round_minor)
            && self.currency.len() == 3
            && self.currency.chars().all(|c| c.is_ascii_uppercase());
        if ok { Ok(()) } else { Err(RoundError::Cost) }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoundSummary {
    pub confirmed_rounds: u64,
    /// Runs with a proposal but no confirmation: not added to the total.
    pub unconfirmed_runs: u32,
    pub cost_minor: Option<i64>,
    pub currency: Option<String>,
}

pub fn summarize(counts: &[RoundCount], cost: Option<&CostSetting>) -> RoundSummary {
    let confirmed_rounds: u64 = counts.iter().filter_map(|c| c.confirmed).map(u64::from).sum();
    RoundSummary {
        confirmed_rounds,
        unconfirmed_runs: counts.iter().filter(|c| c.confirmed.is_none() && c.proposed > 0).count() as u32,
        cost_minor: cost.map(|c| c.per_round_minor * confirmed_rounds as i64),
        currency: cost.map(|c| c.currency.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a24_confirmed_count_is_independent_of_detection() {
        let rc = RoundCount { run_id: "r".into(), proposed: propose("phone_live", 5), confirmed: None, confirmed_utc_ms: None };
        assert_eq!(rc.proposed, 5);
        let c = confirm(&rc, 6, 9).unwrap(); // a missed detection
        assert_eq!(c.confirmed, Some(6));
        assert_eq!(c.proposed, 5, "proposal preserved");
        assert_eq!(propose("par_only", 3), 0, "dry/par practice proposes no rounds");
        let cost = CostSetting { per_round_minor: 32, currency: "USD".into() };
        cost.validate().unwrap();
        let s =
            summarize(&[c, RoundCount { run_id: "x".into(), proposed: 4, confirmed: None, confirmed_utc_ms: None }], Some(&cost));
        assert_eq!(s.confirmed_rounds, 6);
        assert_eq!(s.unconfirmed_runs, 1, "unconfirmed proposals are not counted");
        assert_eq!(s.cost_minor, Some(192));
        assert!(CostSetting { per_round_minor: 32, currency: "usd".into() }.validate().is_err());
    }
}
