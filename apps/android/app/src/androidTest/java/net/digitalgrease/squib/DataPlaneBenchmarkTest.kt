package net.digitalgrease.squib

import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import net.digitalgrease.squib.audio.PcmBridge
import net.digitalgrease.squib.core.benchCloseSink
import net.digitalgrease.squib.core.benchDrain
import net.digitalgrease.squib.core.benchOpenSink
import net.digitalgrease.squib.core.pushPcmUniffi
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith

/**
 * M0 task 6: compare the narrow JNI copy path with generated UniFFI marshaling for
 * 10 ms PCM16 blocks (480 frames). Results are reported to instrumentation status and
 * logcat; they are evidence only for the device/emulator that ran them.
 */
@RunWith(AndroidJUnit4::class)
class DataPlaneBenchmarkTest {
    private fun report(key: String, value: String) {
        Log.i("SquibBench", "$key=$value")
        val b = android.os.Bundle()
        b.putString(key, value)
        InstrumentationRegistry.getInstrumentation().sendStatus(0, b)
    }

    private fun percentile(ns: LongArray, p: Double): Long {
        val s = ns.sorted()
        return s[((s.size - 1) * p).toInt()]
    }

    @Test
    fun jniVersusUniffiPushCost() {
        val n = 5_000
        val block = 480
        val data = ShortArray(block) { (it % 100).toShort() }
        val handle = benchOpenSink(block.toUInt(), 64u).toLong()
        try {
            // Warm up both paths.
            repeat(500) { i ->
                PcmBridge.nativePush(handle, i.toLong(), i.toLong() * block, data, block, -1, 0, 0, 0)
                if (i % 32 == 31) benchDrain(handle.toULong())
            }
            benchDrain(handle.toULong())
            val jni = LongArray(n)
            for (i in 0 until n) {
                val t0 = System.nanoTime()
                val code = PcmBridge.nativePush(handle, i.toLong(), i.toLong() * block, data, block, -1, 0, 0, 0)
                jni[i] = System.nanoTime() - t0
                assertEquals(0, code)
                if (i % 32 == 31) benchDrain(handle.toULong())
            }
            benchDrain(handle.toULong())
            val uni = LongArray(n)
            for (i in 0 until n) {
                val t0 = System.nanoTime()
                // Generated binding requires a boxed List<Short> per call.
                val list = data.asList()
                val code = pushPcmUniffi(handle.toULong(), i.toULong(), i.toLong() * block, list, -1, 0)
                uni[i] = System.nanoTime() - t0
                assertEquals(0, code)
                if (i % 32 == 31) benchDrain(handle.toULong())
            }
            report("device", "${android.os.Build.MANUFACTURER} ${android.os.Build.MODEL} sdk ${android.os.Build.VERSION.SDK_INT}")
            report("jni_p50_ns", percentile(jni, 0.5).toString())
            report("jni_p99_ns", percentile(jni, 0.99).toString())
            report("uniffi_p50_ns", percentile(uni, 0.5).toString())
            report("uniffi_p99_ns", percentile(uni, 0.99).toString())
            report("block_duration_ns", "10000000")
            // Closed handles are rejected, not dereferenced.
            benchCloseSink(handle.toULong())
            assertEquals(PcmBridge.CLOSED, PcmBridge.nativePush(handle, 0, 0, data, block, -1, 0, 0, 0))
            // Count larger than the array is rejected without crashing.
            val h2 = benchOpenSink(block.toUInt(), 8u).toLong()
            assertEquals(2, PcmBridge.nativePush(h2, 0, 0, ShortArray(10), block, -1, 0, 0, 0))
            benchCloseSink(h2.toULong())
        } finally {
            benchCloseSink(handle.toULong())
        }
    }
}
