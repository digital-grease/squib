package net.digitalgrease.squib.audio

/**
 * Narrow JNI data plane (ADR-003). Copies a preallocated `ShortArray` region straight
 * into the Rust core's bounded queue; no per-call allocation on either side.
 *
 * Return codes: 0 ok, 1 queue overflow, 2 too large/copy failed, 3 closed handle,
 * 4 invalid arguments, 5 internal error.
 */
object PcmBridge {
    const val OK = 0
    const val OVERFLOW = 1
    const val CLOSED = 3

    init {
        System.loadLibrary("squib_mobile")
    }

    /** `anchorFrame < 0` means no anchor; `deliveredNs <= 0` means unknown. */
    @JvmStatic
    external fun nativePush(
        handle: Long,
        sequence: Long,
        firstFrame: Long,
        data: ShortArray,
        count: Int,
        anchorFrame: Long,
        anchorNs: Long,
        deliveredNs: Long,
        flags: Int,
    ): Int
}
