package net.digitalgrease.squib.core

import android.app.Application
import android.content.Context
import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioManager
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import java.util.TimeZone
import java.util.UUID
import java.util.concurrent.Executors
import kotlin.random.Random
import kotlinx.coroutines.Job
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import net.digitalgrease.squib.BuildConfig
import net.digitalgrease.squib.SquibApp
import net.digitalgrease.squib.audio.AudioCapture
import net.digitalgrease.squib.audio.CuePlayer
import net.digitalgrease.squib.audio.RouteInspector

/** Timer settings edited on the Timer screen. Times in milliseconds. */
data class TimerSettings(
    val mode: Mode = Mode.PAR_ONLY,
    val delayKind: DelayKind = DelayKind.RANDOM,
    val fixedMs: Int = 2000,
    val randomMinMs: Int = 2000,
    val randomMaxMs: Int = 4000,
    val parsText: String = "",
    val expectedCountText: String = "",
    val autoStop: Boolean = false,
    val manualThresholdDb: Float? = null,
)

enum class DelayKind { INSTANT, FIXED, RANDOM }

const val DELAY_MAX_MS = 30_000

/** Validation shared by the UI and arm(). Returns an error message or null. */
fun TimerSettings.validate(): String? {
    if (delayKind == DelayKind.FIXED && fixedMs !in 0..DELAY_MAX_MS) return "Fixed delay must be 0–30 s."
    if (delayKind == DelayKind.RANDOM) {
        if (randomMinMs < 0 || randomMaxMs < 0) return "Delays cannot be negative."
        if (randomMinMs > randomMaxMs) return "Minimum delay must not exceed maximum."
        if (randomMaxMs > DELAY_MAX_MS) return "Maximum delay is 30 s."
    }
    if (parsMs() == null) return "Par times: seconds separated by commas, e.g. 2.0, 3.5"
    if (autoStop && parsMs().isNullOrEmpty()) return "Auto stop needs at least one par time."
    if (expectedCountText.isNotBlank() && expectedCountText.trim().toIntOrNull()?.takeIf { it in 1..1000 } == null) {
        return "Expected count must be 1–1000."
    }
    return null
}

fun TimerSettings.parsMs(): List<Int>? {
    if (parsText.isBlank()) return emptyList()
    return parsText.split(',').map { it.trim() }.filter { it.isNotEmpty() }.map {
        val s = it.toDoubleOrNull() ?: return null
        if (s <= 0 || s > 600) return null
        (s * 1000).toInt()
    }
}

/**
 * Owns the engine control loop and platform adapters. All engine calls happen on one
 * control thread except cue playback confirmation, which the engine serializes itself.
 */
class SquibController(app: Application) : AndroidViewModel(app) {
    private val engine = (app as SquibApp).engine
    private val inspector = RouteInspector(app)
    private val audioManager = app.getSystemService(Context.AUDIO_SERVICE) as AudioManager
    private val control = Executors.newSingleThreadExecutor { Thread(it, "squib-control") }.asCoroutineDispatcher()
    private val cueThread = Executors.newSingleThreadExecutor { Thread(it, "squib-cue") }
    private val cuePlayer = CuePlayer(engine)
    private var capture: AudioCapture? = null
    private var captureRoute: net.digitalgrease.squib.core.RouteReport? = null
    private var loop: Job? = null

    private val _view = MutableStateFlow(engine.poll(System.nanoTime()))
    val view: StateFlow<EngineView> = _view.asStateFlow()
    private val _settings = MutableStateFlow(TimerSettings())
    val settings: StateFlow<TimerSettings> = _settings.asStateFlow()
    private val _preflight = MutableStateFlow<Preflight?>(null)
    val preflight: StateFlow<Preflight?> = _preflight.asStateFlow()
    private val _keepScreenOn = MutableStateFlow(false)
    val keepScreenOn: StateFlow<Boolean> = _keepScreenOn.asStateFlow()
    private val _message = MutableStateFlow<String?>(null)
    val message: StateFlow<String?> = _message.asStateFlow()
    private val _history = MutableStateFlow<List<RunListItem>>(emptyList())
    val history: StateFlow<List<RunListItem>> = _history.asStateFlow()
    private val _probe = MutableStateFlow<CalibrationView?>(null)
    val probe: StateFlow<CalibrationView?> = _probe.asStateFlow()
    private val _diagnostics = MutableStateFlow<CaptureDiagnostics?>(null)
    val diagnostics: StateFlow<CaptureDiagnostics?> = _diagnostics.asStateFlow()
    val recovered: List<RecoveredRunView> = engine.recoveredRuns()
    private var lastArm: ArmRequest? = null

    init {
        cueThread.execute { runCatching { cuePlayer.prepare() } }
        refreshPreflight()
        refreshHistory()
    }

    fun updateSettings(f: (TimerSettings) -> TimerSettings) {
        _settings.value = f(_settings.value)
        refreshPreflight()
    }

    fun refreshPreflight() {
        viewModelScope.launch(control) {
            val route = inspector.expectedRoute()
            _preflight.value = engine.preflight(_settings.value.mode, route, inspector.expectedRateHz().toUInt())
        }
    }

    fun refreshHistory() {
        viewModelScope.launch(control) {
            _history.value = runCatching { engine.listRuns(200u) }.getOrElse {
                _message.value = "Could not read history: ${it.message}"
                emptyList()
            }
        }
    }

    fun clearMessage() {
        _message.value = null
    }

    private fun tz(): Int = TimeZone.getDefault().getOffset(System.currentTimeMillis()) / 60_000

    fun arm() {
        val s = _settings.value
        s.validate()?.let {
            _message.value = it
            return
        }
        val delay = when (s.delayKind) {
            DelayKind.INSTANT -> StartDelay.Instant
            DelayKind.FIXED -> StartDelay.Fixed(s.fixedMs.toUInt())
            DelayKind.RANDOM -> StartDelay.Random(s.randomMinMs.toUInt(), s.randomMaxMs.toUInt())
        }
        val pf = _preflight.value
        val req = ArmRequest(
            mode = s.mode,
            delay = delay,
            unitRandom = Random.nextDouble(),
            parsMs = s.parsMs().orEmpty().map { it.toUInt() },
            autoStopGraceMs = if (s.autoStop) 1000u else null,
            expectedCount = s.expectedCountText.trim().toIntOrNull()?.toUInt(),
            thresholdDb = s.manualThresholdDb,
            calibrationId = pf?.calibrationId,
            route = if (s.mode == Mode.PHONE_LIVE) inspector.expectedRoute() else null,
            nowUtcMs = System.currentTimeMillis(),
            tzOffsetMin = tz(),
            appBuild = "${BuildConfig.VERSION_NAME}+${BuildConfig.VERSION_CODE}",
            expectedRateHz = inspector.expectedRateHz().toUInt(),
        )
        armWith(req)
    }

    /** Repeat reuses the configuration with a fresh random delay and a new run id. */
    fun repeat() {
        val r = lastArm ?: return arm()
        armWith(r.copy(unitRandom = Random.nextDouble(), nowUtcMs = System.currentTimeMillis()))
    }

    private fun armWith(req: ArmRequest) {
        viewModelScope.launch(control) {
            if (req.mode == Mode.PHONE_LIVE) {
                val pf = engine.preflight(req.mode, inspector.expectedRoute(), inspector.expectedRateHz().toUInt())
                _preflight.value = pf
                if (!pf.canArm) {
                    _message.value = pf.messages.firstOrNull() ?: "This route cannot be used for live timing."
                    return@launch
                }
            }
            if (!requestFocus()) {
                _message.value = "Another app is using audio (for example a call), so cues may not be heard. Try again when it stops."
                return@launch
            }
            try {
                lastArm = req
                apply(engine.arm(UUID.randomUUID().toString(), req, System.nanoTime()))
                startLoop()
            } catch (e: SquibException) {
                _message.value = e.message
            }
        }
    }

    fun cancel() = command { engine.cancel(UUID.randomUUID().toString(), System.nanoTime()) }

    fun stop() = command { engine.stop(UUID.randomUUID().toString(), System.nanoTime()) }

    fun retrySave() = command { engine.retrySave(UUID.randomUUID().toString(), System.nanoTime()) }

    fun dismissResult() = command { engine.reset() }

    private fun command(f: () -> EngineView) {
        viewModelScope.launch(control) {
            try {
                apply(f())
                startLoop()
            } catch (e: SquibException) {
                _message.value = e.message
            }
        }
    }

    /** Activity left the foreground: an active run is interrupted (ADR-006). */
    fun onForegroundLost() {
        viewModelScope.launch(control) {
            if (_view.value.active) apply(engine.lifecycle(LifecycleEvent.FOREGROUND_LOST, System.nanoTime()))
            if (_probe.value != null) apply(engine.lifecycle(LifecycleEvent.FOREGROUND_LOST, System.nanoTime()))
        }
    }

    private fun startLoop() {
        if (loop?.isActive == true) return
        loop = viewModelScope.launch(control) {
            while (isActive) {
                val now = System.nanoTime()
                val v = if (_probe.value != null) {
                    val p = engine.probeStatus(now)
                    _probe.value = p
                    handleEffects(p.effects)
                    engine.poll(now)
                } else {
                    engine.poll(now)
                }
                apply(v)
                _diagnostics.value = engine.captureDiagnostics() ?: _diagnostics.value
                val idle = !v.active && v.phase !in setOf("completing", "interrupted", "save_pending") && _probe.value == null &&
                    capture == null
                if (idle) {
                    abandonFocus()
                    refreshHistory()
                    break
                }
                val wait = v.nextDeadlineNs?.let { ((it - System.nanoTime()) / 1_000_000).coerceIn(1, 15) } ?: 15
                delay(wait)
            }
        }
    }

    private fun apply(v: EngineView) {
        _view.value = v
        handleEffects(v.effects)
    }

    private fun handleEffects(effects: List<NativeEffect>) {
        for (e in effects) {
            when (e) {
                is NativeEffect.StartCapture -> startCapture(e.preferredRateHz.toInt())
                is NativeEffect.StopCapture -> {
                    capture?.let { c -> Thread { c.stop() }.start() }
                    capture = null
                }
                is NativeEffect.PlayCue -> {
                    val id = e.cueId
                    val cue = e.cue
                    cueThread.execute {
                        val render = runCatching { cuePlayer.play(cue) }.getOrNull()
                        engine.cueRendered(id, render, System.nanoTime())
                    }
                }
                is NativeEffect.KeepScreenOn -> _keepScreenOn.value = e.on
            }
        }
    }

    private fun startCapture(rate: Int) {
        if (!inspector.micPermission()) {
            val v = engine.captureFailed("microphone permission denied", System.nanoTime())
            apply(v)
            return
        }
        val expected = inspector.expectedRoute()
        val c = AudioCapture(inspector, object : AudioCapture.Listener {
            override fun onOpened(info: AudioCapture.OpenedInfo): Long {
                val route = inspector.report(info.input, cuePlayer.routedOutput().let {
                    if (it == "unknown") expected.outputDevice else it
                }, info.source, info.effects)
                captureRoute = route
                val meta = CaptureMeta(
                    sampleRateHz = info.sampleRateHz.toUInt(),
                    blockFrames = info.blockFrames.toUInt(),
                    platformBufferFrames = info.bufferFrames.toUInt(),
                    route = route,
                    clockDomain = "CLOCK_MONOTONIC",
                    startedMonoNs = info.startedNs,
                )
                return try {
                    engine.captureStarted(meta, System.nanoTime()).toLong()
                } catch (e: SquibException) {
                    _message.value = e.message
                    0L
                }
            }

            override fun onOpenFailed(reason: String) {
                viewModelScope.launch(control) { apply(engine.captureFailed(reason, System.nanoTime())) }
            }

            override fun onError(reason: String) {
                viewModelScope.launch(control) { apply(engine.captureError(reason, System.nanoTime())) }
            }

            override fun onRouteChanged() {
                viewModelScope.launch(control) { apply(engine.lifecycle(LifecycleEvent.ROUTE_CHANGED, System.nanoTime())) }
            }

            override fun onSilenced() {
                viewModelScope.launch(control) { apply(engine.lifecycle(LifecycleEvent.MIC_SILENCED, System.nanoTime())) }
            }
        })
        capture = c
        c.start(rate, audioManager)
    }

    // ---- Audio focus -------------------------------------------------------------------

    /**
     * Transient focus for the duration of a run. Losing it (a call, another app taking
     * audio) interrupts the run: cues may be inaudible and the microphone may be shared.
     * Ducking requests from others are ignored; Squib never changes volume.
     */
    private val focusRequest = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT)
        .setAudioAttributes(
            AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_MEDIA)
                .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                .build(),
        )
        .setWillPauseWhenDucked(false)
        .setOnAudioFocusChangeListener { change ->
            if (change == AudioManager.AUDIOFOCUS_LOSS || change == AudioManager.AUDIOFOCUS_LOSS_TRANSIENT) {
                viewModelScope.launch(control) {
                    if (_view.value.active) apply(engine.lifecycle(LifecycleEvent.AUDIO_FOCUS_LOST, System.nanoTime()))
                }
            }
        }
        .build()
    private var hasFocus = false

    private fun requestFocus(): Boolean {
        if (hasFocus) return true
        hasFocus = audioManager.requestAudioFocus(focusRequest) == AudioManager.AUDIOFOCUS_REQUEST_GRANTED
        return hasFocus
    }

    private fun abandonFocus() {
        if (hasFocus) {
            audioManager.abandonAudioFocusRequest(focusRequest)
            hasFocus = false
        }
    }

    // ---- Review ----------------------------------------------------------------------

    suspend fun loadReview(runId: String): Result<ReviewView> = withContext(control) {
        runCatching { engine.loadReview(runId) }
    }

    suspend fun runConditions(runId: String): ConditionsView? = withContext(control) {
        runCatching { engine.runConditions(runId) }.getOrNull()
    }

    suspend fun applyReview(runId: String, base: UInt, actions: List<ReviewAction>, reason: String?): Result<ReviewView> =
        withContext(control) {
            runCatching { engine.applyReview(runId, base, actions, reason, System.currentTimeMillis()) }
                .also { refreshHistory() }
        }

    // ---- Sensitivity setup and cue test ----------------------------------------------

    fun startCalibration() = startProbe { engine.startCalibration(inspector.expectedRoute(), System.nanoTime()) }

    fun startCueTest() = startProbe { engine.startCueTest(inspector.expectedRoute(), System.nanoTime()) }

    private fun startProbe(f: () -> CalibrationView) {
        viewModelScope.launch(control) {
            try {
                val p = f()
                _probe.value = p
                handleEffects(p.effects)
                startLoop()
            } catch (e: SquibException) {
                _message.value = e.message
            }
        }
    }

    fun finishProbe(save: Boolean, label: String?) {
        viewModelScope.launch(control) {
            try {
                val p = engine.finishProbe(save, label, System.currentTimeMillis(), System.nanoTime())
                handleEffects(p.effects)
                _probe.value = null
                _lastProbeResult.value = p
                refreshPreflight()
            } catch (e: SquibException) {
                _message.value = e.message
            }
        }
    }

    private val _lastProbeResult = MutableStateFlow<CalibrationView?>(null)
    val lastProbeResult: StateFlow<CalibrationView?> = _lastProbeResult.asStateFlow()

    fun coreVersions(): List<String> = net.digitalgrease.squib.core.coreVersions()

    override fun onCleared() {
        abandonFocus()
        capture?.stop()
        cuePlayer.release()
        cueThread.shutdown()
        control.close()
    }
}
