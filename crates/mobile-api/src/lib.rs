//! Squib mobile facade.
//!
//! - Control plane: UniFFI-exported `SquibEngine` (Kotlin now, Swift later).
//! - Data plane: `PcmBridge.nativePush` JNI entry point copying PCM into a bounded,
//!   preallocated queue consumed by a dedicated DSP thread (see `jni.rs`).
//! - A UniFFI `push_pcm_uniffi` path exists only to benchmark generated marshaling
//!   against the JNI path (M0 task 6); the app does not use it.

pub mod conditions;
pub mod dsp;
pub mod engine;
pub mod ffi;
pub mod jni;
pub mod queue;
pub mod registry;
pub mod store;

pub use conditions::*;
pub use engine::SquibEngine;
pub use ffi::*;

uniffi::setup_scaffolding!();

/// Rust-side data-plane push with the same semantics and return codes as the JNI
/// entry point (used by desktop tests and non-JVM hosts).
pub fn data_plane_push(
    handle: u64,
    sequence: u64,
    first_frame: i64,
    samples: &[i16],
    anchor: Option<squib_timing::clock::FrameAnchor>,
    delivered_ns: Option<i64>,
    flags: u32,
) -> i32 {
    let Some(sink) = registry::get(handle) else { return 3 };
    let r = sink.queue.push_with(sequence, first_frame, samples.len(), anchor, delivered_ns, flags, |d| {
        d.copy_from_slice(samples);
        true
    });
    if r == queue::PushResult::Ok
        && let Some(t) = &sink.worker
    {
        t.unpark();
    }
    r.code()
}

/// Benchmark-only: PCM through generated UniFFI marshaling (allocates per call).
/// Returns the same codes as the JNI path.
#[uniffi::export]
pub fn push_pcm_uniffi(
    handle: u64,
    sequence: u64,
    first_frame: i64,
    samples: Vec<i16>,
    anchor_frame: i64,
    anchor_ns: i64,
) -> i32 {
    let Some(sink) = registry::get(handle) else { return 3 };
    let anchor = (anchor_frame >= 0 && anchor_ns > 0)
        .then_some(squib_timing::clock::FrameAnchor { frame: anchor_frame, mono_ns: anchor_ns });
    let r = sink.queue.push(sequence, first_frame, &samples, anchor, None).code();
    if let Some(t) = &sink.worker {
        t.unpark();
    }
    r
}

/// Benchmark-only: register a sink whose blocks are discarded by the caller via
/// `bench_drain`. Lets the platform measure transfer cost without DSP.
#[uniffi::export]
pub fn bench_open_sink(block_frames: u32, blocks: u32) -> u64 {
    let q = std::sync::Arc::new(queue::PcmQueue::new(blocks.clamp(4, 4096) as usize, block_frames.clamp(1, 16_384) as usize));
    registry::register(q, None)
}

/// Benchmark-only: recycle all queued blocks; returns how many were drained.
#[uniffi::export]
pub fn bench_drain(handle: u64) -> u32 {
    let Some(sink) = registry::get(handle) else { return 0 };
    let mut n = 0;
    while let Some(b) = sink.queue.pop() {
        sink.queue.recycle(b);
        n += 1;
    }
    n
}

#[uniffi::export]
pub fn bench_close_sink(handle: u64) {
    registry::unregister(handle);
}

/// Library versions for diagnostics/reporting.
#[uniffi::export]
pub fn core_versions() -> Vec<String> {
    vec![
        format!("core {}", env!("CARGO_PKG_VERSION")),
        format!("detector {}", squib_domain::DETECTOR_ALGORITHM_VERSION),
        format!("mapping {}", squib_timing::clock::MAPPING_POLICY_VERSION),
        format!("envelope {}", squib_timing::envelope::ENVELOPE_VERSION),
        format!("schema {}", squib_storage::SCHEMA_VERSION),
    ]
}
