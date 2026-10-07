//! Audio block headers and capture-integrity checks (docs/squib/04 `AudioBlock`).

use serde::{Deserialize, Serialize};
use squib_domain::{QualityEvent, QualityKind, Severity};

use crate::clock::FrameAnchor;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleFormat {
    Pcm16,
    /// Normalized float; non-finite values are replaced by zero and reported.
    F32,
}

/// Negotiated (actual, not requested) capture format of an epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpochFormat {
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub format: SampleFormat,
    /// Platform capture buffer size in frames, when known. Used to detect overrun:
    /// an anchor beyond `frames delivered + capacity` means frames were lost.
    pub platform_buffer_frames: Option<u32>,
}

/// Adapter-reported flag: the platform reported an overrun for this block.
pub const FLAG_ADAPTER_OVERRUN: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockHeader {
    /// Strictly increasing within the epoch, starting at 0.
    pub sequence: u64,
    /// Index of the first frame in the adapter's documented frame domain.
    pub first_frame: i64,
    pub frame_count: u32,
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub format: SampleFormat,
    /// Optional platform timestamp anchor polled near this block.
    pub anchor: Option<FrameAnchor>,
    /// Monotonic time the reader received this block. Delivery metadata only.
    pub delivered_ns: Option<i64>,
    pub flags: u32,
}

/// Validates block ordering and continuity. Any discontinuity is an integrity event:
/// the epoch can no longer support a complete run.
#[derive(Debug, Clone)]
pub struct IntegrityChecker {
    fmt: EpochFormat,
    next_sequence: u64,
    next_frame: Option<i64>,
}

impl IntegrityChecker {
    pub fn new(fmt: EpochFormat) -> Self {
        Self { fmt, next_sequence: 0, next_frame: None }
    }

    /// End (exclusive) of the last contiguous frame delivered.
    pub fn delivered_end(&self) -> Option<i64> {
        self.next_frame
    }

    pub fn check(&mut self, h: &BlockHeader, out: &mut Vec<QualityEvent>) {
        if h.sample_rate_hz != self.fmt.sample_rate_hz || h.channels != self.fmt.channels || h.format != self.fmt.format {
            out.push(
                QualityEvent::new(
                    QualityKind::FormatChanged,
                    Severity::Integrity,
                    format!(
                        "expected {} Hz/{} ch/{:?}, got {} Hz/{} ch/{:?}",
                        self.fmt.sample_rate_hz, self.fmt.channels, self.fmt.format, h.sample_rate_hz, h.channels, h.format
                    ),
                )
                .frames(h.first_frame, h.first_frame + i64::from(h.frame_count)),
            );
        }
        if let Some(expect) = self.next_frame
            && h.first_frame != expect
        {
            let (kind, detail) = if h.first_frame > expect {
                (QualityKind::CaptureGap, format!("{} frames missing", h.first_frame - expect))
            } else {
                (QualityKind::SequenceDiscontinuity, format!("block overlaps by {} frames", expect - h.first_frame))
            };
            out.push(
                QualityEvent::new(kind, Severity::Integrity, detail).frames(expect.min(h.first_frame), expect.max(h.first_frame)),
            );
        }
        // Frame continuity is reported first: it names what was lost.
        if h.sequence != self.next_sequence {
            out.push(QualityEvent::new(
                QualityKind::SequenceDiscontinuity,
                Severity::Integrity,
                format!("expected block {}, got {}", self.next_sequence, h.sequence),
            ));
        }
        if h.flags & FLAG_ADAPTER_OVERRUN != 0 {
            out.push(
                QualityEvent::new(QualityKind::CaptureOverrun, Severity::Integrity, "platform reported overrun")
                    .frames(h.first_frame, h.first_frame + i64::from(h.frame_count)),
            );
        }
        self.next_sequence = h.sequence.wrapping_add(1);
        self.next_frame = Some(h.first_frame + i64::from(h.frame_count));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt() -> EpochFormat {
        EpochFormat { sample_rate_hz: 48_000, channels: 1, format: SampleFormat::Pcm16, platform_buffer_frames: None }
    }

    fn hdr(seq: u64, first: i64, n: u32) -> BlockHeader {
        BlockHeader {
            sequence: seq,
            first_frame: first,
            frame_count: n,
            sample_rate_hz: 48_000,
            channels: 1,
            format: SampleFormat::Pcm16,
            anchor: None,
            delivered_ns: None,
            flags: 0,
        }
    }

    #[test]
    fn contiguous_blocks_with_varying_sizes_are_clean() {
        let mut c = IntegrityChecker::new(fmt());
        let mut out = vec![];
        c.check(&hdr(0, 0, 480), &mut out);
        c.check(&hdr(1, 480, 17), &mut out);
        c.check(&hdr(2, 497, 1024), &mut out);
        assert!(out.is_empty());
        assert_eq!(c.delivered_end(), Some(1521));
    }

    #[test]
    fn gaps_reorders_and_format_changes_are_integrity_events() {
        let mut c = IntegrityChecker::new(fmt());
        let mut out = vec![];
        c.check(&hdr(0, 0, 480), &mut out);
        c.check(&hdr(2, 960, 480), &mut out); // dropped block 1
        assert!(out.iter().any(|e| e.kind == QualityKind::CaptureGap && e.interrupts()));
        assert!(out.iter().any(|e| e.kind == QualityKind::SequenceDiscontinuity));
        out.clear();
        c.check(&hdr(3, 1000, 480), &mut out); // overlaps (reordered/duplicated frames)
        assert!(out.iter().any(|e| e.kind == QualityKind::SequenceDiscontinuity));
        out.clear();
        let mut h = hdr(4, 1480, 480);
        h.sample_rate_hz = 44_100;
        c.check(&h, &mut out);
        assert!(out.iter().any(|e| e.kind == QualityKind::FormatChanged && e.interrupts()));
        out.clear();
        let mut h = hdr(5, 1960, 480);
        h.flags = FLAG_ADAPTER_OVERRUN;
        c.check(&h, &mut out);
        assert!(out.iter().any(|e| e.kind == QualityKind::CaptureOverrun));
    }
}
