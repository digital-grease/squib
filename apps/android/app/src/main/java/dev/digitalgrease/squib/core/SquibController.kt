package dev.digitalgrease.squib.core

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
import dev.digitalgrease.squib.BuildConfig
import dev.digitalgrease.squib.SquibApp
import dev.digitalgrease.squib.audio.AudioCapture
import dev.digitalgrease.squib.audio.CuePlayer
import dev.digitalgrease.squib.audio.RouteInspector
import dev.digitalgrease.squib.video.ClipRecorder
import android.graphics.SurfaceTexture
import java.io.File
import kotlinx.coroutines.Dispatchers

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
    /** Live mode: stop about 1 s after this many accepted shots (null: off). */
    val autoStopCount: Int? = null,
    val manualThresholdDb: Float? = null,
    /** Drill this timer setup came from (pinned per run). */
    val drillId: String? = null,
    val drillVersion: UInt? = null,
    val drillTitle: String? = null,
    val drillRepeats: Int = 1,
    val drillRestS: Int = 0,
    /** 1-based string number within the current set. */
    val stringIndex: Int = 1,
    /** Day-plan agenda item the runs count toward. */
    val planItemId: String? = null,
    val planTitle: String? = null,
    /** Opt-in video clip (video only, no sound) recorded with each run. */
    val video: Boolean = false,
    /** Keep a diagnostic audio recording of the next live run only (turns itself off). */
    val diagnostic: Boolean = false,
    /** Drill sets: arm the next string after a visible, cancelable countdown. Off per set. */
    val autoRepeat: Boolean = false,
    /** Par-only start and par cues by sound, vibration, or on screen. Live runs use sound. */
    val cueChannel: CueChannel = CueChannel.SOUND,
)

/** A screen cue waiting to be drawn; `seq` makes each cue distinct. */
data class ScreenCue(val cueId: UInt, val cue: CueType, val seq: Long)

/** Shortest gap between screen cues (core `SCREEN_CUE_MIN_SPACING_MS`). */
const val SCREEN_CUE_MIN_SPACING_MS = 350

/** The next string of a set is about to arm itself; shown with a Cancel button. */
data class AutoRepeatCountdown(val nextString: Int, val strings: Int, val secondsLeft: Int)

/** Shortest countdown before an automatic repeat, even when the drill has no rest. */
const val AUTO_REPEAT_MIN_S = 5

/**
 * Seconds before the next string arms automatically, or null when it must not: only
 * after a complete, saved string that is not the set's last (plan phase 2.2).
 */
fun TimerSettings.autoRepeatDelayS(phase: String, outcome: String?): Int? =
    if (autoRepeat && drillId != null && stringIndex < drillRepeats && phase == "saved" && outcome == "complete") {
        maxOf(drillRestS, AUTO_REPEAT_MIN_S)
    } else {
        null
    }

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
    if (mode == Mode.PHONE_LIVE && autoStopCount != null) {
        if (autoStopCount !in 1..1000) return "Shot count for automatic stop must be 1-1000."
        if (autoStop) return "Choose one automatic stop: after the last par or after a shot count."
    }
    // Live timing always uses the sound cue it listens for; the choice applies to par-only.
    if (mode == Mode.PAR_ONLY && cueChannel == CueChannel.SCREEN) {
        val starts = listOf(0) + parsMs().orEmpty()
        if (starts.zipWithNext().any { (a, b) -> b - a < SCREEN_CUE_MIN_SPACING_MS }) {
            return "Screen cues need at least 0.35 s between them so the screen changes at most about three times a second."
        }
    }
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
    private val vibrator = dev.digitalgrease.squib.audio.CueVibrator(app)
    /** Whether this phone can give vibration cues. */
    val canVibrate: Boolean = vibrator.available()
    private val _screenCue = MutableStateFlow<ScreenCue?>(null)
    /** Screen cue for the Timer to draw; it reports the frame with [screenCueShown]. */
    val screenCue: StateFlow<ScreenCue?> = _screenCue.asStateFlow()
    private var screenSeq = 0L
    private var capture: AudioCapture? = null
    private var captureRoute: dev.digitalgrease.squib.core.RouteReport? = null
    private var loop: Job? = null

    private val _view = MutableStateFlow(engine.poll(System.nanoTime()))
    val view: StateFlow<EngineView> = _view.asStateFlow()
    private val _settings = MutableStateFlow(TimerSettings())
    val settings: StateFlow<TimerSettings> = _settings.asStateFlow()
    private val _preflight = MutableStateFlow<Preflight?>(null)
    val preflight: StateFlow<Preflight?> = _preflight.asStateFlow()
    private val _autoRepeat = MutableStateFlow<AutoRepeatCountdown?>(null)
    val autoRepeat: StateFlow<AutoRepeatCountdown?> = _autoRepeat.asStateFlow()
    private var autoRepeatJob: Job? = null
    /** Run whose result has already been considered for an automatic repeat. */
    @Volatile private var autoRepeatRun: String? = null
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

    // ---- Video (M4 proof of concept) ----
    private val recorder = ClipRecorder(app)
    private val attachmentRoot = File(app.filesDir, "attachments")
    /** Relative path of the clip being recorded and the run it belongs to. */
    @Volatile private var videoClip: String? = null
    private var videoRunId: String? = null
    private var videoFinish: Job? = null
    @Volatile private var preview: SurfaceTexture? = null

    /** The Timer's preview surface; recording works without one. */
    fun setPreview(t: SurfaceTexture?) {
        preview = t
    }

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
            _preflight.value = engine.preflight(
                _settings.value.mode, route.copy(cameraRecording = _settings.value.video), inspector.expectedRateHz().toUInt(),
            )
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
        stopCountdown()
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
            autoStopCount = s.autoStopCount?.takeIf { s.mode == Mode.PHONE_LIVE }?.toUInt(),
            expectedCount = s.expectedCountText.trim().toIntOrNull()?.toUInt(),
            thresholdDb = s.manualThresholdDb,
            calibrationId = pf?.calibrationId,
            route = if (s.mode == Mode.PHONE_LIVE) inspector.expectedRoute().copy(cameraRecording = s.video) else null,
            nowUtcMs = System.currentTimeMillis(),
            tzOffsetMin = tz(),
            appBuild = "${BuildConfig.VERSION_NAME}+${BuildConfig.VERSION_CODE}",
            expectedRateHz = inspector.expectedRateHz().toUInt(),
            drillId = s.drillId,
            drillVersion = s.drillVersion,
            planItemId = s.planItemId,
            video = s.video,
            cueChannel = if (s.mode == Mode.PAR_ONLY) s.cueChannel else CueChannel.SOUND,
            diagnosticRoot = if (s.diagnostic && s.mode == Mode.PHONE_LIVE) attachmentRoot.absolutePath else null,
        )
        // One run only: the switch turns itself off once this run is armed.
        if (s.diagnostic) _settings.value = s.copy(diagnostic = false)
        armWith(req)
    }

    /** Repeat reuses the configuration with a fresh random delay and a new run id. */
    fun repeat() {
        stopCountdown()
        val s = _settings.value
        if (s.drillId != null) {
            _settings.value = s.copy(stringIndex = if (s.stringIndex >= s.drillRepeats) 1 else s.stringIndex + 1)
        }
        val r = lastArm ?: return arm()
        // Repeat never re-records audio: diagnostic recording is chosen per run.
        armWith(r.copy(unitRandom = Random.nextDouble(), nowUtcMs = System.currentTimeMillis(), diagnosticRoot = null))
    }

    /** Load a drill's recipe into the timer. Each string is a separate run. */
    fun startDrill(d: DrillView, planItemId: String? = null, planTitle: String? = null, strings: Int? = null) {
        val i = d.input
        val delay = i.delay
        _settings.value = TimerSettings(
            video = _settings.value.video,
            // A cue preference outlasts the drill (live runs still use sound).
            cueChannel = _settings.value.cueChannel,
            mode = i.mode,
            delayKind = when (delay) {
                is StartDelay.Instant -> DelayKind.INSTANT
                is StartDelay.Fixed -> DelayKind.FIXED
                is StartDelay.Random -> DelayKind.RANDOM
            },
            fixedMs = (delay as? StartDelay.Fixed)?.ms?.toInt() ?: 2000,
            randomMinMs = (delay as? StartDelay.Random)?.minMs?.toInt() ?: 2000,
            randomMaxMs = (delay as? StartDelay.Random)?.maxMs?.toInt() ?: 4000,
            parsText = i.parsMs.joinToString(", ") { (it.toInt() / 1000.0).toString() },
            expectedCountText = i.expectedCount?.toString() ?: "",
            drillId = d.drillId,
            drillVersion = d.version,
            drillTitle = i.title,
            drillRepeats = strings ?: i.repeats.toInt(),
            drillRestS = i.restS.toInt(),
            stringIndex = 1,
            planItemId = planItemId,
            planTitle = planTitle,
        )
        lastArm = null
        stopCountdown()
        viewModelScope.launch(control) { if (!_view.value.active) runCatching { apply(engine.reset()) } }
        refreshPreflight()
    }

    fun clearDrill() {
        cancelAutoRepeat(null)
        _settings.value = _settings.value.copy(drillId = null, drillVersion = null, drillTitle = null, drillRepeats = 1, drillRestS = 0, stringIndex = 1, planItemId = null, planTitle = null)
    }

    private fun armWith(req: ArmRequest) {
        viewModelScope.launch(control) {
            if (req.mode == Mode.PHONE_LIVE) {
                val pf = engine.preflight(req.mode, inspector.expectedRoute().copy(cameraRecording = req.video), inspector.expectedRateHz().toUInt())
                _preflight.value = pf
                if (!pf.canArm) {
                    armFailed(pf.messages.firstOrNull() ?: "This route cannot be used for live timing.")
                    return@launch
                }
            }
            if (!requestFocus()) {
                armFailed("Another app is using audio (for example a call), so cues may not be heard. Try again when it stops.")
                return@launch
            }
            // Video starts first, so any change the camera makes to the audio path happens
            // before the microphone opens rather than mid-run.
            if (req.video && !startVideo()) {
                abandonFocus()
                _settings.value = _settings.value.copy(autoRepeat = false)
                return@launch
            }
            try {
                lastArm = req
                val v = engine.arm(UUID.randomUUID().toString(), req, System.nanoTime())
                if (req.video) videoRunId = v.runId
                apply(v)
                startLoop()
            } catch (e: SquibException) {
                armFailed(e.message)
                if (req.video) finishVideo(keep = false)
            }
        }
    }

    fun cancel() = command { engine.cancel(UUID.randomUUID().toString(), System.nanoTime()) }

    fun stop() = command { engine.stop(UUID.randomUUID().toString(), System.nanoTime()) }

    fun retrySave() = command { engine.retrySave(UUID.randomUUID().toString(), System.nanoTime()) }

    fun dismissResult() {
        cancelAutoRepeat(null)
        command { engine.reset() }
    }

    private fun armFailed(message: String?) {
        _message.value = message
        // A string that could not arm ends an automatic chain; Repeat still works by hand.
        _settings.value = _settings.value.copy(autoRepeat = false)
    }

    /** Stop a pending automatic repeat and turn it off for the rest of this set. */
    fun cancelAutoRepeat(reason: String? = null) {
        val pending = _autoRepeat.value != null
        stopCountdown()
        if (reason != null && !pending) return
        if (_settings.value.autoRepeat) _settings.value = _settings.value.copy(autoRepeat = false)
        if (reason != null) _message.value = reason
    }

    @Volatile private var timerVisible = false

    /** The Timer drew a screen cue in the frame starting at `frameNs` (System.nanoTime base). */
    fun screenCueShown(c: ScreenCue, frameNs: Long) {
        cueThread.execute { engine.cueRendered(c.cueId, frameNs, System.nanoTime()) }
    }

    fun screenCueDone(c: ScreenCue) {
        _screenCue.compareAndSet(c, null)
    }

    /** The Timer screen came on or went off screen; a pending automatic repeat is cancelled. */
    fun onTimerVisible(visible: Boolean) {
        timerVisible = visible
        if (!visible) cancelAutoRepeat("Automatic repeat was cancelled because you left the Timer.")
    }

    private fun stopCountdown() {
        autoRepeatJob?.cancel()
        autoRepeatJob = null
        _autoRepeat.value = null
    }

    /** After a string ends: count down to the next one, or end the chain (plan phase 2.2). */
    private fun considerAutoRepeat(v: EngineView) {
        val s = _settings.value
        if (!s.autoRepeat || v.phase !in setOf("saved", "cancelled", "failed")) return
        val run = v.runId ?: return
        if (run == autoRepeatRun) return
        autoRepeatRun = run
        val wait = s.autoRepeatDelayS(v.phase, v.outcome)
        if (wait != null && !timerVisible) {
            _settings.value = s.copy(autoRepeat = false)
            _message.value = "Automatic repeat stopped because the Timer was not on screen. Press Repeat to go on."
            return
        }
        if (wait == null) {
            _settings.value = s.copy(autoRepeat = false)
            if (s.stringIndex < s.drillRepeats) {
                _message.value = "Automatic repeat stopped because that string did not complete. Press Repeat to go on."
            }
            return
        }
        autoRepeatJob?.cancel()
        autoRepeatJob = viewModelScope.launch {
            val end = android.os.SystemClock.elapsedRealtime() + wait * 1000L
            while (isActive) {
                val leftMs = end - android.os.SystemClock.elapsedRealtime()
                if (leftMs <= 0) break
                _autoRepeat.value = AutoRepeatCountdown(s.stringIndex + 1, s.drillRepeats, ((leftMs + 999) / 1000).toInt())
                delay(minOf(leftMs, 250L))
            }
            if (isActive && _settings.value.autoRepeat) repeat()
        }
    }

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
        cancelAutoRepeat("Automatic repeat was cancelled because Squib left the screen.")
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
        if (_settings.value.autoRepeat && v.phase in setOf("saved", "cancelled", "failed") && v.runId != autoRepeatRun) {
            viewModelScope.launch { considerAutoRepeat(v) }
        }
        if (videoClip != null && !v.active && v.phase != "ready") {
            // Cancelled runs record no timing and failed starts have no run: their clips are discarded.
            finishVideo(keep = v.phase !in setOf("cancelled", "failed"))
        }
    }

    private suspend fun startVideo(): Boolean {
        videoFinish?.join()
        val rel = "videos/${UUID.randomUUID()}.mp4"
        val err = withContext(Dispatchers.IO) { recorder.start(File(attachmentRoot, rel), preview) }
        if (err != null) {
            _message.value = "Video could not start: $err Turn video off to time without it."
            return false
        }
        videoClip = rel
        return true
    }

    private fun finishVideo(keep: Boolean) {
        val rel = videoClip ?: return
        val runId = videoRunId
        videoClip = null
        videoRunId = null
        videoFinish = viewModelScope.launch(Dispatchers.IO) {
            val r = recorder.stop()
            if (r == null) return@launch
            if (!keep || runId == null) {
                r.file.delete()
                return@launch
            }
            try {
                engine.registerVideo(runId, attachmentRoot.absolutePath, rel, r.meta, System.currentTimeMillis())
            } catch (e: SquibException) {
                r.file.delete()
                _message.value = "The video clip could not be saved: ${e.message}"
            }
        }
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
                    when (e.channel) {
                        CueChannel.SOUND -> cueThread.execute {
                            val render = runCatching { cuePlayer.play(cue) }.getOrNull()
                            engine.cueRendered(id, render, System.nanoTime())
                        }
                        // A cue that was not given is not reported, so the core times out
                        // the start instead of trusting a vibration or screen that never happened.
                        CueChannel.VIBRATION -> cueThread.execute {
                            runCatching { vibrator.vibrate(cue) }.getOrNull()?.let { engine.cueRendered(id, it, System.nanoTime()) }
                        }
                        CueChannel.SCREEN -> if (timerVisible) _screenCue.value = ScreenCue(id, cue, ++screenSeq)
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
                }, info.source, info.effects).copy(cameraRecording = videoClip != null)
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

    fun coreVersions(): List<String> = dev.digitalgrease.squib.core.coreVersions()

    override fun onCleared() {
        recorder.release()
        abandonFocus()
        capture?.stop()
        cuePlayer.release()
        cueThread.shutdown()
        control.close()
    }
}
