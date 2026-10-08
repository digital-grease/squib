//! Squib range conditions (M2).
//!
//! Pure normalization and selection: no network client, location service, or clock.
//! Native adapters perform HTTP and sensor reads; this crate decides what to fetch,
//! validates what came back, and resolves each field with visible provenance.

pub mod candidate;
pub mod local;
pub mod nws;
pub mod privacy;
pub mod resolver;
pub mod time;
pub mod units;

pub use candidate::*;
pub use privacy::LocationRetention;
pub use resolver::*;
