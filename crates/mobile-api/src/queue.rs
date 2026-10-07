//! Bounded PCM transfer queue (data plane).
//!
//! A fixed pool of preallocated blocks circulates between a free queue and a filled
//! queue. The producer (platform capture thread) never allocates, never blocks, and
//! never waits for DSP, storage, or UI: if no free block exists the push fails and a
//! sticky overflow is recorded. Old blocks are never dropped to make room, so an
//! overflowed epoch can never be reported as a complete capture (A22).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crossbeam_queue::ArrayQueue;
use squib_timing::clock::FrameAnchor;

#[derive(Debug)]
pub struct PcmBlock {
    pub sequence: u64,
    pub first_frame: i64,
    pub frame_count: u32,
    pub anchor: Option<FrameAnchor>,
    pub delivered_ns: Option<i64>,
    pub flags: u32,
    pub samples: Vec<i16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushResult {
    Ok,
    /// No free block: the queue is full. The epoch is no longer complete.
    Overflow,
    /// Block larger than the preallocated capacity.
    TooLarge,
}

impl PushResult {
    pub fn code(self) -> i32 {
        match self {
            PushResult::Ok => 0,
            PushResult::Overflow => 1,
            PushResult::TooLarge => 2,
        }
    }
}

pub struct PcmQueue {
    free: ArrayQueue<Box<PcmBlock>>,
    filled: ArrayQueue<Box<PcmBlock>>,
    max_frames: usize,
    overflowed: AtomicBool,
    overflow_count: AtomicU64,
    pushed: AtomicU64,
    high_water: AtomicU64,
}

impl PcmQueue {
    /// `blocks` × `max_frames` samples preallocated up front.
    pub fn new(blocks: usize, max_frames: usize) -> Self {
        let free = ArrayQueue::new(blocks);
        for _ in 0..blocks {
            let b = Box::new(PcmBlock {
                sequence: 0,
                first_frame: 0,
                frame_count: 0,
                anchor: None,
                delivered_ns: None,
                flags: 0,
                samples: vec![0; max_frames],
            });
            let _ = free.push(b);
        }
        Self {
            free,
            filled: ArrayQueue::new(blocks),
            max_frames,
            overflowed: AtomicBool::new(false),
            overflow_count: AtomicU64::new(0),
            pushed: AtomicU64::new(0),
            high_water: AtomicU64::new(0),
        }
    }

    /// Capacity sized to roughly `ms` of audio at `rate` in blocks of `block_frames`.
    pub fn for_duration(rate: u32, block_frames: usize, ms: u32) -> Self {
        let frames = (u64::from(rate) * u64::from(ms) / 1000) as usize;
        let blocks = frames.div_ceil(block_frames.max(1)).max(4);
        Self::new(blocks, block_frames)
    }

    pub fn max_frames(&self) -> usize {
        self.max_frames
    }

    pub fn capacity_blocks(&self) -> usize {
        self.free.capacity()
    }

    /// Producer side. `fill` copies samples into the provided preallocated slice and
    /// returns `false` if it could not (e.g. a JNI copy failure).
    #[allow(clippy::too_many_arguments)]
    pub fn push_with(
        &self,
        sequence: u64,
        first_frame: i64,
        frame_count: usize,
        anchor: Option<FrameAnchor>,
        delivered_ns: Option<i64>,
        flags: u32,
        fill: impl FnOnce(&mut [i16]) -> bool,
    ) -> PushResult {
        if frame_count > self.max_frames {
            return PushResult::TooLarge;
        }
        let Some(mut b) = self.free.pop() else {
            self.overflowed.store(true, Ordering::Release);
            self.overflow_count.fetch_add(1, Ordering::Relaxed);
            return PushResult::Overflow;
        };
        if !fill(&mut b.samples[..frame_count]) {
            let _ = self.free.push(b);
            return PushResult::TooLarge;
        }
        b.sequence = sequence;
        b.first_frame = first_frame;
        b.frame_count = frame_count as u32;
        b.anchor = anchor;
        b.delivered_ns = delivered_ns;
        b.flags = flags;
        // Cannot fail: total blocks in circulation equal both queues' capacity.
        let _ = self.filled.push(b);
        self.pushed.fetch_add(1, Ordering::Relaxed);
        let depth = self.filled.len() as u64;
        self.high_water.fetch_max(depth, Ordering::Relaxed);
        PushResult::Ok
    }

    pub fn push(
        &self,
        sequence: u64,
        first_frame: i64,
        samples: &[i16],
        anchor: Option<FrameAnchor>,
        delivered_ns: Option<i64>,
    ) -> PushResult {
        self.push_with(sequence, first_frame, samples.len(), anchor, delivered_ns, 0, |dst| {
            dst.copy_from_slice(samples);
            true
        })
    }

    /// Consumer side.
    pub fn pop(&self) -> Option<Box<PcmBlock>> {
        self.filled.pop()
    }

    pub fn is_empty(&self) -> bool {
        self.filled.is_empty()
    }

    /// Return a processed block to the pool.
    pub fn recycle(&self, b: Box<PcmBlock>) {
        let _ = self.free.push(b);
    }

    pub fn take_overflow(&self) -> bool {
        self.overflowed.swap(false, Ordering::AcqRel)
    }

    pub fn stats(&self) -> QueueStats {
        QueueStats {
            capacity_blocks: self.free.capacity() as u64,
            pushed: self.pushed.load(Ordering::Relaxed),
            overflows: self.overflow_count.load(Ordering::Relaxed),
            high_water_blocks: self.high_water.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QueueStats {
    pub capacity_blocks: u64,
    pub pushed: u64,
    pub overflows: u64,
    pub high_water_blocks: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overflow_is_reported_and_never_evicts_queued_blocks() {
        let q = PcmQueue::new(4, 480);
        let s = [1i16; 480];
        for i in 0..4 {
            assert_eq!(q.push(i, i as i64 * 480, &s, None, None), PushResult::Ok);
        }
        assert_eq!(q.push(4, 4 * 480, &s, None, None), PushResult::Overflow);
        assert!(q.take_overflow());
        // The four oldest blocks are intact and in order.
        for i in 0..4 {
            let b = q.pop().unwrap();
            assert_eq!(b.sequence, i);
            q.recycle(b);
        }
        assert_eq!(q.stats().overflows, 1);
        assert_eq!(q.push(5, 5 * 480, &[0; 481], None, None), PushResult::TooLarge);
    }

    #[test]
    fn sizing_targets_250ms() {
        let q = PcmQueue::for_duration(48_000, 480, 250);
        assert_eq!(q.capacity_blocks(), 25);
        let q = PcmQueue::for_duration(44_100, 441, 250);
        assert_eq!(q.capacity_blocks(), 25);
    }
}
