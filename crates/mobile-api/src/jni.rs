//! Narrow JNI data plane for PCM (ADR-003).
//!
//! `PcmBridge.nativePush` copies a `short[]` region straight into a preallocated queue
//! block with `GetShortArrayRegion`: no per-call heap allocation, no per-sample FFI
//! objects, no locks held across Java callbacks, and no blocking. Panics are caught and
//! never unwind into the JVM.
//!
//! Return codes: 0 ok, 1 queue overflow, 2 block too large / copy failed,
//! 3 unknown or closed handle, 4 invalid arguments, 5 internal panic.

use std::panic::{AssertUnwindSafe, catch_unwind};

use jni_sys::{JNIEnv, jclass, jint, jlong, jshortArray};
use squib_timing::clock::FrameAnchor;

use crate::queue::PushResult;
use crate::registry;

/// `anchor_frame < 0` or `anchor_ns <= 0` means no anchor; `delivered_ns <= 0` unknown.
///
/// # Safety
/// Called by the JVM with a valid `env` and a live `data` array reference.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "system" fn Java_net_digitalgrease_squib_audio_PcmBridge_nativePush(
    env: *mut JNIEnv,
    _class: jclass,
    handle: jlong,
    sequence: jlong,
    first_frame: jlong,
    data: jshortArray,
    count: jint,
    anchor_frame: jlong,
    anchor_ns: jlong,
    delivered_ns: jlong,
    flags: jint,
) -> jint {
    let r = catch_unwind(AssertUnwindSafe(|| {
        if env.is_null() || data.is_null() || count < 0 || sequence < 0 || first_frame < 0 {
            return 4;
        }
        let Some(sink) = registry::get(handle as u64) else { return 3 };
        let anchor = (anchor_frame >= 0 && anchor_ns > 0).then_some(FrameAnchor { frame: anchor_frame, mono_ns: anchor_ns });
        let delivered = (delivered_ns > 0).then_some(delivered_ns);
        let res = sink.queue.push_with(sequence as u64, first_frame, count as usize, anchor, delivered, flags as u32, |dst| {
            // SAFETY: env is a valid JNIEnv for this thread; dst has exactly `count`
            // elements; the JVM bounds-checks the region and raises an exception
            // (detected below) if the array is shorter than `count`.
            unsafe {
                let fns = &**env;
                (fns.v1_1.GetShortArrayRegion)(env, data, 0, count, dst.as_mut_ptr());
                if (fns.v1_2.ExceptionCheck)(env) {
                    (fns.v1_1.ExceptionClear)(env);
                    return false;
                }
            }
            true
        });
        if res == PushResult::Ok
            && let Some(t) = &sink.worker
        {
            t.unpark();
        }
        res.code()
    }));
    r.unwrap_or(5)
}
