package dev.digitalgrease.squib

import android.database.sqlite.SQLiteDatabase
import androidx.lifecycle.ViewModelProvider
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.digitalgrease.squib.core.CueChannel
import dev.digitalgrease.squib.core.DelayKind
import dev.digitalgrease.squib.core.Mode
import dev.digitalgrease.squib.core.SquibController
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Doc 09 UI-response gate on an emulator (plan phase 2.5): p95 event-to-display < 150 ms.
 * Each sample is a screen cue: from the time the cue was due (its scheduled time) to the
 * frame that drew it, through the control loop, state flow, Compose and the draw pass.
 * Panel scan-out after that frame is not included. The activity runs without the Compose
 * test rule, whose virtual frame clock would make the timing meaningless. Emulator only.
 */
@RunWith(AndroidJUnit4::class)
class UiLatencyTest {
    private fun report(key: String, value: String) {
        android.util.Log.i("SquibPerf", "$key=$value")
        InstrumentationRegistry.getInstrumentation().sendStatus(0, android.os.Bundle().apply { putString(key, value) })
    }

    private fun waitFor(ms: Long, what: String, cond: () -> Boolean) {
        val end = System.currentTimeMillis() + ms
        while (!cond()) {
            check(System.currentTimeMillis() < end) { "timed out waiting for $what" }
            Thread.sleep(50)
        }
    }

    @Test
    fun screenCuesReachTheDisplayWithinTheGate() {
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            lateinit var c: SquibController
            lateinit var dbPath: String
            scenario.onActivity {
                c = ViewModelProvider(it)[SquibController::class.java]
                dbPath = it.getDatabasePath("journal.db").absolutePath
            }
            if (c.view.value.phase != "ready") scenario.onActivity { c.dismissResult() }
            waitFor(5_000, "ready") { c.view.value.phase == "ready" }
            scenario.onActivity {
                c.updateSettings { s ->
                    s.copy(mode = Mode.PAR_ONLY, delayKind = DelayKind.INSTANT, cueChannel = CueChannel.SCREEN,
                        parsText = (1..10).joinToString(", ") { i -> "%.1f".format(java.util.Locale.US, 0.5 + 0.5 * i) }, autoStop = true)
                }
            }
            val runs = mutableListOf<String>()
            repeat(5) {
                scenario.onActivity { c.arm() }
                waitFor(20_000, "a saved run") { c.view.value.phase == "saved" }
                check(c.view.value.outcome == "complete") { "run ended ${c.view.value.outcome}: ${c.view.value.interruptReason}" }
                runs += c.view.value.runId!!
                scenario.onActivity { c.dismissResult() }
                waitFor(5_000, "ready") { c.view.value.phase == "ready" }
            }
            scenario.onActivity { c.updateSettings { s -> s.copy(cueChannel = CueChannel.SOUND, parsText = "", autoStop = false) } }

            val ms = mutableListOf<Double>()
            var missed = 0
            SQLiteDatabase.openDatabase(dbPath, null, SQLiteDatabase.OPEN_READONLY).use { db ->
                for (r in runs) {
                    db.rawQuery("SELECT requested_mono_ns, render_mono_ns, missed FROM cue_observation WHERE run_id = ?", arrayOf(r)).use { q ->
                        while (q.moveToNext()) {
                            if (q.getInt(2) == 1 || q.isNull(1)) { missed++; continue }
                            ms += (q.getLong(1) - q.getLong(0)) / 1e6
                        }
                    }
                }
            }
            val s = ms.sorted()
            fun pct(p: Double) = s[((s.size - 1) * p).toInt()]
            report("device", "${android.os.Build.MANUFACTURER} ${android.os.Build.MODEL} sdk ${android.os.Build.VERSION.SDK_INT}")
            report("ui_samples", "${s.size} drawn, $missed missed or not drawn")
            report("ui_event_to_frame_ms", "p50 %.1f p95 %.1f max %.1f min %.1f".format(java.util.Locale.US, pct(0.5), pct(0.95), s.last(), s.first()))
            check(s.size == 55 && missed == 0) { "expected 55 drawn cues, got ${s.size} and $missed missed" }
            check(pct(0.95) < 150.0) { "p95 ${pct(0.95)} ms exceeds the 150 ms gate" }
        }
    }
}
