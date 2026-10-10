//! Squib timing engine: clock mapping, capture integrity, cue matching, onset
//! detection, calibration, and the run state machine.
//!
//! The same code runs on device and in the desktop replay tool (docs/squib/04).

pub mod block;
pub mod calibration;
pub mod clock;
pub mod cue;
pub mod detector;
pub mod envelope;
pub mod pipeline;
pub mod replay;
pub mod run;
pub mod synth;
pub mod video;
