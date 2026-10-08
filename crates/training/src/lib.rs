//! Squib training: versioned drill recipes, generic practice scoring, comparable
//! analytics, manual timer strings, and round counts (docs/squib/07).
//!
//! Pure data and calculation. Recipes are declarative data, never code. Scoring never
//! touches detector observations; analytics always reports its denominator and
//! inclusion policy.

pub mod analytics;
pub mod drill;
pub mod manual;
pub mod rounds;
pub mod scoring;
