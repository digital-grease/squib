//! Placing a video clip on the run timeline (M4 proof of concept, `camera-pts-v1`).
//!
//! The platform records frames stamped by the camera, keeps the first frame's camera
//! timestamp as file time zero, and samples the host clocks once in a capture-started
//! callback. This module decides which host clock the camera timestamps follow and how
//! much to trust that, without ever guessing silently:
//!
//! - `Measured`: the camera reports boot-time timestamps (Camera2 `REALTIME`) and a probe
//!   frame agrees with boot time; boot time is converted to monotonic time with offsets
//!   sampled when recording started and stopped.
//! - `Assumed`: the camera's clock is undocumented (`UNKNOWN`) but a probe frame agrees
//!   with one host clock. Plausible, not proven.
//! - `Unavailable`: nothing agrees, or the clocks moved during the clip. The video still
//!   plays; it just gets no timing markers.

use serde::{Deserialize, Serialize};

pub const VIDEO_MAPPING_METHOD: &str = "camera-pts-v1";

/// A frame's camera timestamp (start of exposure) may precede the host callback that
/// reports it by up to this much for the clocks to be considered the same.
pub const PROBE_WINDOW_NS: i64 = 500_000_000;

/// The capture-started callback may also arrive slightly before the exposure time it
/// reports: it fires "as capture begins" and pipelined camera HALs send it early. The
/// API 34 emulator's camera stamps frames about 31 ms after the callback's clock reads.
/// Up to three frames at 30 fps of lead is accepted; more means a different clock.
pub const PROBE_LEAD_NS: i64 = 100_000_000;

/// Boot time minus monotonic time changes only across device suspend. More movement than
/// this during a clip makes the conversion unreliable.
pub const MAX_OFFSET_DRIFT_NS: i64 = 2_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CameraClock {
    /// Camera2 `SENSOR_INFO_TIMESTAMP_SOURCE_REALTIME`: same base as boot time.
    Realtime,
    /// Camera2 `SENSOR_INFO_TIMESTAMP_SOURCE_UNKNOWN`: monotonic, base unspecified.
    Unknown,
}

impl CameraClock {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "realtime" => Some(Self::Realtime),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockObservations {
    pub camera_clock: CameraClock,
    pub first_frame_camera_ns: i64,
    pub probe_camera_ns: i64,
    pub probe_mono_ns: i64,
    pub probe_boot_ns: i64,
    pub start_boot_minus_mono_ns: i64,
    pub end_boot_minus_mono_ns: i64,
    pub frame_period_ns: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum MappingStatus {
    Measured,
    Assumed,
    Unavailable { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoMapping {
    pub status: MappingStatus,
    /// Monotonic time of file time zero, when a mapping exists.
    pub first_frame_mono_ns: Option<i64>,
    /// At least one frame period: a frame shows an interval, not an instant.
    pub uncertainty_ns: i64,
}

/// `delta_ns` is host clock minus camera timestamp, both read for the same frame.
fn within_probe(delta_ns: i64) -> bool {
    (-PROBE_LEAD_NS..=PROBE_WINDOW_NS).contains(&delta_ns)
}

pub fn map_clip(o: &ClockObservations) -> VideoMapping {
    let unavailable = |reason: &str| VideoMapping {
        status: MappingStatus::Unavailable { reason: reason.into() },
        first_frame_mono_ns: None,
        uncertainty_ns: o.frame_period_ns,
    };
    if o.frame_period_ns <= 0 {
        return unavailable("the clip has no frame rate");
    }
    if (o.end_boot_minus_mono_ns - o.start_boot_minus_mono_ns).abs() > MAX_OFFSET_DRIFT_NS {
        return unavailable("the phone's clocks moved during recording (it may have slept)");
    }
    let boot_offset = o.start_boot_minus_mono_ns + (o.end_boot_minus_mono_ns - o.start_boot_minus_mono_ns) / 2;
    let mono_agrees = within_probe(o.probe_mono_ns - o.probe_camera_ns);
    let boot_agrees = within_probe(o.probe_boot_ns - o.probe_camera_ns);
    // `offset` converts a camera timestamp to monotonic time: mono = camera - offset.
    let (status, offset) = match o.camera_clock {
        CameraClock::Realtime if boot_agrees => (MappingStatus::Measured, boot_offset),
        CameraClock::Realtime => return unavailable("camera timestamps did not match boot time as the camera reported"),
        CameraClock::Unknown if mono_agrees => (MappingStatus::Assumed, 0),
        CameraClock::Unknown if boot_agrees => (MappingStatus::Assumed, boot_offset),
        CameraClock::Unknown => return unavailable("camera timestamps did not match any phone clock"),
    };
    VideoMapping { status, first_frame_mono_ns: Some(o.first_frame_camera_ns - offset), uncertainty_ns: o.frame_period_ns }
}

impl VideoMapping {
    /// File position (ms from the first frame) of a monotonic instant, if mapped.
    pub fn file_ms(&self, mono_ns: i64) -> Option<i64> {
        self.first_frame_mono_ns.map(|f| (mono_ns - f).div_euclid(1_000_000))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: i64 = 1_000_000;

    fn obs(clock: CameraClock) -> ClockObservations {
        // Monotonic 1_000 s, boot 1_500 s: the phone slept 500 s since boot.
        ClockObservations {
            camera_clock: clock,
            first_frame_camera_ns: 0,
            probe_camera_ns: 0,
            probe_mono_ns: 0,
            probe_boot_ns: 0,
            start_boot_minus_mono_ns: 500_000 * MS,
            end_boot_minus_mono_ns: 500_000 * MS,
            frame_period_ns: 33 * MS,
        }
    }

    #[test]
    fn realtime_camera_is_converted_with_the_measured_offset() {
        let mut o = obs(CameraClock::Realtime);
        o.first_frame_camera_ns = 1_500_000 * MS + 40 * MS; // boot clock
        o.probe_camera_ns = 1_500_000 * MS + 40 * MS;
        o.probe_boot_ns = o.probe_camera_ns + 30 * MS;
        o.probe_mono_ns = o.probe_boot_ns - 500_000 * MS;
        let m = map_clip(&o);
        assert_eq!(m.status, MappingStatus::Measured);
        assert_eq!(m.first_frame_mono_ns, Some(1_000_000 * MS + 40 * MS));
        // A shot 2.5 s after the first frame lands at 2500 ms in the file.
        assert_eq!(m.file_ms(1_000_000 * MS + 40 * MS + 2_500 * MS), Some(2_500));
        assert_eq!(m.file_ms(1_000_000 * MS), Some(-40), "before the clip is negative, not clamped");
    }

    #[test]
    fn unknown_camera_clock_is_only_assumed_and_picks_the_agreeing_clock() {
        let mut o = obs(CameraClock::Unknown);
        o.first_frame_camera_ns = 1_000_000 * MS;
        o.probe_camera_ns = 1_000_000 * MS;
        o.probe_mono_ns = o.probe_camera_ns + 20 * MS;
        o.probe_boot_ns = o.probe_mono_ns + 500_000 * MS;
        let m = map_clip(&o);
        assert_eq!((m.status.clone(), m.first_frame_mono_ns), (MappingStatus::Assumed, Some(1_000_000 * MS)));

        // Same camera values, but they follow boot time instead.
        o.first_frame_camera_ns = 1_500_000 * MS;
        o.probe_camera_ns = 1_500_000 * MS;
        o.probe_boot_ns = o.probe_camera_ns + 20 * MS;
        o.probe_mono_ns = o.probe_boot_ns - 500_000 * MS;
        let m = map_clip(&o);
        assert_eq!((m.status, m.first_frame_mono_ns), (MappingStatus::Assumed, Some(1_000_000 * MS)));
    }

    #[test]
    fn emulator_camera_stamped_slightly_after_its_callback_is_accepted() {
        // Recorded on the API 34 emulator (emulated back camera, REALTIME source).
        let o = ClockObservations {
            camera_clock: CameraClock::Realtime,
            first_frame_camera_ns: 726_785_364_000,
            probe_camera_ns: 726_785_364_387,
            probe_mono_ns: 726_754_012_327,
            probe_boot_ns: 726_754_012_738,
            start_boot_minus_mono_ns: 296,
            end_boot_minus_mono_ns: 722,
            frame_period_ns: 35_533_000,
        };
        let m = map_clip(&o);
        assert_eq!(m.status, MappingStatus::Measured);
        assert_eq!(m.first_frame_mono_ns, Some(726_785_364_000 - 509));
    }

    #[test]
    fn disagreement_or_clock_movement_makes_the_mapping_unavailable() {
        let mut o = obs(CameraClock::Unknown);
        o.probe_camera_ns = 7 * MS;
        o.probe_mono_ns = 9_000 * MS; // 9 s later: not the same clock
        o.probe_boot_ns = 509_000 * MS;
        assert!(matches!(map_clip(&o).status, MappingStatus::Unavailable { .. }));
        assert_eq!(map_clip(&o).file_ms(0), None);

        let mut o = obs(CameraClock::Realtime);
        o.probe_boot_ns = o.probe_camera_ns - 150 * MS; // camera far ahead of the callback
        assert!(matches!(map_clip(&o).status, MappingStatus::Unavailable { .. }));

        let mut o = obs(CameraClock::Realtime);
        o.probe_boot_ns = 10 * MS;
        o.end_boot_minus_mono_ns += 3 * MS; // slept during the clip
        assert!(matches!(map_clip(&o).status, MappingStatus::Unavailable { .. }));
    }
}
