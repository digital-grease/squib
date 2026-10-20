package dev.digitalgrease.squib.data

import android.app.Application
import android.media.AudioManager
import android.media.ToneGenerator
import android.os.SystemClock
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import java.util.Locale
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/** A running or finished walkthrough countdown for one plan. Times are `elapsedRealtime`. */
data class Walkthrough(
    val planId: String,
    val planTitle: String,
    val endAtMs: Long,
    val beep: Boolean,
    val leftMs: Long,
) {
    val finished: Boolean get() = leftMs == 0L
    /** Minutes and seconds left, rounded up so 0:00 means time is up. */
    val label: String get() = ((leftMs + 999) / 1000).let { String.format(Locale.US, "%d:%02d", it / 60, it % 60) }
}

/**
 * The countdown's rules, separate from Android so a test can drive the clock. One
 * walkthrough at a time; starting another replaces it.
 */
class WalkthroughClock(private val now: () -> Long) {
    var current: Walkthrough? = null
        private set

    fun start(planId: String, planTitle: String, minutes: Int, beep: Boolean): Walkthrough {
        val ms = minutes.coerceIn(1, 60) * 60_000L
        return Walkthrough(planId, planTitle, now() + ms, beep, ms).also { current = it }
    }

    /** Recompute the time left. Returns true once, on the update where time runs out. */
    fun tick(): Boolean {
        val w = current ?: return false
        if (w.finished) return false
        val left = (w.endAtMs - now()).coerceAtLeast(0)
        current = w.copy(leftMs = left)
        return left == 0L
    }

    fun stop() {
        current = null
    }
}

/**
 * Holds the walkthrough countdown for the whole app (plan phase 2.6), so it keeps running
 * when you leave the plan screen; other screens show it in a banner. It lives as long as the
 * app's process: Android may end a backgrounded app, and then the countdown and its end
 * beep are lost. The beep uses media volume and never overrides mute or Do Not Disturb.
 */
class WalkthroughController(app: Application) : AndroidViewModel(app) {
    private val clock = WalkthroughClock { SystemClock.elapsedRealtime() }
    private val _state = MutableStateFlow<Walkthrough?>(null)
    val state: StateFlow<Walkthrough?> = _state.asStateFlow()
    private var ticker: Job? = null

    fun start(planId: String, planTitle: String, minutes: Int, beep: Boolean) {
        _state.value = clock.start(planId, planTitle, minutes, beep)
        ticker?.cancel()
        ticker = viewModelScope.launch {
            while (true) {
                val endedNow = clock.tick()
                _state.value = clock.current
                if (endedNow) {
                    if (clock.current?.beep == true) beep()
                    break
                }
                if (clock.current == null) break
                delay(200)
            }
        }
    }

    fun stop() {
        ticker?.cancel()
        clock.stop()
        _state.value = null
    }

    private suspend fun beep() {
        runCatching {
            val g = ToneGenerator(AudioManager.STREAM_MUSIC, 90)
            g.startTone(ToneGenerator.TONE_PROP_BEEP2, 800)
            delay(1000)
            g.release()
        }
    }
}
