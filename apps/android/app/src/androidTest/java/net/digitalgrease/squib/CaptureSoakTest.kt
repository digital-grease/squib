package net.digitalgrease.squib

import android.Manifest
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.lifecycle.ViewModelProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import net.digitalgrease.squib.core.SquibController
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Doc 09 capture-duration gate on an emulator (plan phase 2.5): the real AudioRecord path,
 * data plane and DSP worker stay open for the requested time with no silent discontinuity.
 * Uses the sensitivity probe, which keeps capture open and fails on any capture fault.
 * Excluded from the default suite (it is long and grants the microphone). Run it with
 *   ./gradlew :app:connectedDebugAndroidTest \
 *     -Pandroid.testInstrumentationRunnerArguments.notAnnotation=org.junit.Ignore \
 *     -Pandroid.testInstrumentationRunnerArguments.annotation=net.digitalgrease.squib.Soak \
 *     -Pandroid.testInstrumentationRunnerArguments.soakMinutes=30
 * Emulator evidence only.
 */
@Soak
@RunWith(AndroidJUnit4::class)
class CaptureSoakTest {
    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    private fun shell(cmd: String) = AuditDevice.shell(cmd)

    private fun report(key: String, value: String) {
        android.util.Log.i("SquibPerf", "$key=$value")
        InstrumentationRegistry.getInstrumentation().sendStatus(0, android.os.Bundle().apply { putString(key, value) })
    }

    @Test
    fun captureStaysContinuousForTheRequestedMinutes() {
        val minutes = InstrumentationRegistry.getArguments().getString("soakMinutes")?.toIntOrNull()
        check(minutes != null && minutes > 0) { "give the soak length with the soakMinutes argument" }
        val inst = InstrumentationRegistry.getInstrumentation()
        inst.uiAutomation.grantRuntimePermission(inst.targetContext.packageName, Manifest.permission.RECORD_AUDIO)
        // The activity must stay in the foreground: leaving it interrupts capture by design.
        shell("svc power stayon true")
        try {
            val c = ViewModelProvider(rule.activity)[SquibController::class.java]
            rule.runOnUiThread { c.startCalibration() }
            val t0 = System.nanoTime()
            val end = t0 + minutes!! * 60_000_000_000L
            var lastFrames = 0L
            while (System.nanoTime() < end) {
                Thread.sleep(10_000)
                val p = c.probe.value
                check(p != null && p.stage != "failed") { "probe ended: ${p?.stage} ${p?.message}" }
                val d = c.diagnostics.value ?: continue
                check(d.queueOverflows == 0uL) { "queue overflowed ${d.queueOverflows} times" }
                check(d.processedFrames >= lastFrames) { "processed frames went backwards" }
                lastFrames = d.processedFrames
            }
            val elapsedNs = System.nanoTime() - t0
            val d = c.diagnostics.value!!
            val p = c.probe.value!!
            check(p.stage != "failed") { p.message }
            val blockFrames = d.processedFrames.toDouble() / d.blocks.toDouble()
            val rate = blockFrames * 1e9 / d.blockDurationNs.toDouble()
            val expected = elapsedNs / 1e9 * rate
            val shortfallS = (expected - d.processedFrames) / rate
            report("device", "${android.os.Build.MANUFACTURER} ${android.os.Build.MODEL} sdk ${android.os.Build.VERSION.SDK_INT}")
            report("soak", "$minutes min, stage ${p.stage}, ${d.blocks} blocks, rate %.0f Hz, processed vs wall clock %.3f s short, ".format(java.util.Locale.US, rate, shortfallS) +
                "overflows ${d.queueOverflows}, queue high water ${d.queueHighWaterBlocks}/${d.queueCapacityBlocks}, timestamps ${d.timestampQuality}, " +
                "DSP p50 ${d.dspP50Ns / 1000u} us p99 ${d.dspP99Ns / 1000u} us max ${d.dspMaxNs / 1000u} us")
            // Capture starts after the probe is requested and delivers in blocks, so a small
            // shortfall is expected; anything over a second would be a silent gap.
            check(shortfallS in -0.5..1.0) { "processed frames differ from wall clock by $shortfallS s" }
            rule.runOnUiThread { c.finishProbe(false, null) }
        } finally {
            shell("svc power stayon false")
        }
    }
}

/** Long opt-in instrumented tests, excluded from the default suite. */
@Retention(AnnotationRetention.RUNTIME)
@Target(AnnotationTarget.CLASS)
annotation class Soak

