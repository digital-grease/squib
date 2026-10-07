package net.digitalgrease.squib.audio

import android.annotation.SuppressLint
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioRecord
import android.media.AudioRecordingConfiguration
import android.media.AudioTimestamp
import android.media.MediaRecorder
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.Process
import android.os.SystemClock

/**
 * Foreground AudioRecord capture on a dedicated thread (docs/squib/04 "Android capture").
 *
 * Frame domain: `firstFrame` counts frames read since `startRecording()`. Every
 * [ANCHOR_EVERY_BLOCKS] blocks the adapter polls `AudioRecord.getTimestamp` with
 * `TIMEBASE_MONOTONIC` (same clock as `System.nanoTime()`) and forwards
 * `(framePosition, nanoTime)` as an anchor. Whether `framePosition` shares the read
 * counter's origin is the M0 device experiment: the core rejects anchors that are
 * implausible (non-monotonic, wrong implied rate, beyond delivered + buffer).
 * Block arrival time is sent only as delivery metadata, never as a timestamp.
 *
 * The read loop never allocates per block, never blocks on the core (push is
 * non-blocking), and never touches storage or UI.
 */
class AudioCapture(
    private val inspector: RouteInspector,
    private val listener: Listener,
) {
    interface Listener {
        /** Capture opened with the actual negotiated format. Return the data-plane handle (0 = discard). */
        fun onOpened(info: OpenedInfo): Long
        fun onOpenFailed(reason: String)
        fun onError(reason: String)
        fun onRouteChanged()
        fun onSilenced()
    }

    data class OpenedInfo(
        val sampleRateHz: Int,
        val blockFrames: Int,
        val bufferFrames: Int,
        val input: String,
        val source: String,
        val effects: List<String>,
        val startedNs: Long,
    )

    @Volatile private var running = false
    private var thread: Thread? = null
    private var record: AudioRecord? = null
    private var recordingCallback: AudioManager.AudioRecordingCallback? = null
    @Volatile var overflows = 0
        private set

    @SuppressLint("MissingPermission") // Caller checks permission before starting.
    fun start(preferredRateHz: Int, audioManager: AudioManager) {
        if (running) return
        running = true
        thread = Thread({ loop(preferredRateHz, audioManager) }, "squib-capture").also { it.start() }
    }

    fun stop() {
        running = false
        thread?.join(500)
        thread = null
    }

    @SuppressLint("MissingPermission")
    private fun open(rate: Int, source: Int): AudioRecord? {
        val min = AudioRecord.getMinBufferSize(rate, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
        if (min <= 0) return null
        // ~200 ms platform buffer: room to absorb scheduling hiccups without loss.
        val bytes = maxOf(min * 2, rate / 5 * 2)
        val r = runCatching {
            AudioRecord(source, rate, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, bytes)
        }.getOrNull() ?: return null
        if (r.state != AudioRecord.STATE_INITIALIZED) {
            r.release()
            return null
        }
        return r
    }

    private fun loop(preferredRateHz: Int, audioManager: AudioManager) {
        Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_AUDIO)
        val unprocessed = inspector.unprocessedSupported()
        val sourceId = if (unprocessed) MediaRecorder.AudioSource.UNPROCESSED else MediaRecorder.AudioSource.VOICE_RECOGNITION
        val sourceName = if (unprocessed) "unprocessed" else "voice_recognition"
        val r = open(preferredRateHz, sourceId) ?: open(44_100, sourceId)
        if (r == null) {
            running = false
            listener.onOpenFailed("could not open AudioRecord at 48 kHz or 44.1 kHz")
            return
        }
        record = r
        val rate = r.sampleRate
        val block = rate / 100 // 10 ms blocks
        val buf = ShortArray(block)
        val ts = AudioTimestamp()
        try {
            r.startRecording()
            if (r.recordingState != AudioRecord.RECORDSTATE_RECORDING) {
                listener.onOpenFailed("AudioRecord did not start (microphone in use?)")
                return
            }
            val startedNs = System.nanoTime()
            val main = Handler(Looper.getMainLooper())
            if (Build.VERSION.SDK_INT >= 29) {
                val cb = object : AudioManager.AudioRecordingCallback() {
                    override fun onRecordingConfigChanged(configs: MutableList<AudioRecordingConfiguration>) {
                        val mine = configs.firstOrNull { it.clientAudioSessionId == r.audioSessionId }
                        if (mine != null && mine.isClientSilenced && running) listener.onSilenced()
                    }
                }
                recordingCallback = cb
                audioManager.registerAudioRecordingCallback(cb, main)
            }
            // Routed device is only meaningful after recording starts.
            var routed = r.routedDevice
            val t0 = SystemClock.uptimeMillis()
            while (routed == null && SystemClock.uptimeMillis() - t0 < 200) {
                Thread.sleep(5)
                routed = r.routedDevice
            }
            // Android delivers a routing callback when the initial route is established;
            // only a change away from the device captured here is a route change.
            val initialDeviceId = routed?.id
            r.addOnRoutingChangedListener({ rec ->
                val now = rec.routedDevice?.id
                if (running && now != null && now != initialDeviceId) listener.onRouteChanged()
            }, main)
            val info = OpenedInfo(
                sampleRateHz = rate,
                blockFrames = block,
                bufferFrames = r.bufferSizeInFrames,
                input = RouteInspector.inputName(routed),
                source = sourceName,
                effects = RouteInspector.activeEffects(r.audioSessionId),
                startedNs = startedNs,
            )
            val handle = listener.onOpened(info)
            if (handle == 0L) return
            var frames = 0L
            var seq = 0L
            while (running) {
                var got = 0
                while (got < block && running) {
                    val n = r.read(buf, got, block - got, AudioRecord.READ_BLOCKING)
                    if (n < 0) {
                        listener.onError("AudioRecord.read error $n")
                        return
                    }
                    got += n
                }
                if (!running) break
                val delivered = System.nanoTime()
                var anchorFrame = -1L
                var anchorNs = 0L
                if (seq % ANCHOR_EVERY_BLOCKS == 0L &&
                    r.getTimestamp(ts, AudioTimestamp.TIMEBASE_MONOTONIC) == AudioRecord.SUCCESS
                ) {
                    anchorFrame = ts.framePosition
                    anchorNs = ts.nanoTime
                }
                when (PcmBridge.nativePush(handle, seq, frames, buf, block, anchorFrame, anchorNs, delivered, 0)) {
                    PcmBridge.OK -> {}
                    PcmBridge.OVERFLOW -> overflows++ // core records the overflow and interrupts
                    PcmBridge.CLOSED -> return // engine closed this epoch
                    else -> {
                        listener.onError("data plane rejected block")
                        return
                    }
                }
                frames += block
                seq++
            }
        } catch (t: Throwable) {
            listener.onError("capture failed: ${t.message}")
        } finally {
            recordingCallback?.let { audioManager.unregisterAudioRecordingCallback(it) }
            recordingCallback = null
            runCatching { r.stop() }
            r.release()
            record = null
            running = false
        }
    }

    companion object {
        const val ANCHOR_EVERY_BLOCKS = 10L
    }
}
