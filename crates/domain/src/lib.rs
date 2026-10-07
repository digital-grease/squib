//! Squib domain model.
//!
//! Pure types and calculations shared by the timing engine, the repository, and the
//! mobile facade. Nothing here reads a clock, opens a device, or draws randomness:
//! adapters inject time anchors, selected delays, and captured data.
//!
//! Timing values are integer nanoseconds (`i64`) or frame indices (`i64`). Detector
//! ranking scores are never probabilities, and timestamp precision is never measured
//! accuracy (docs/squib/04, 06, 09).

pub mod candidate;
pub mod config;
pub mod hash;
pub mod quality;
pub mod results;
pub mod revision;
pub mod state;

pub use candidate::*;
pub use config::*;
pub use quality::*;
pub use results::*;
pub use revision::*;
pub use state::*;

/// Schema version of persisted/exported domain records.
pub const DOMAIN_SCHEMA_VERSION: u32 = 1;

/// Nanoseconds per second.
pub const NS_PER_S: i64 = 1_000_000_000;
/// Nanoseconds per millisecond.
pub const NS_PER_MS: i64 = 1_000_000;
