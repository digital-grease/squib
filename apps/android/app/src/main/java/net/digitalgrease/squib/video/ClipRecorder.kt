package net.digitalgrease.squib.video

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.SurfaceTexture
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CaptureRequest
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.media.MediaMuxer
import android.os.Handler
import android.os.HandlerThread
import android.os.SystemClock
import android.util.Range
import android.util.Size
import android.view.Surface
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import net.digitalgrease.squib.core.VideoMeta

/**
 * Records a video-only clip (no audio track, so the timing microphone is never shared)
 * with Camera2 into a MediaCodec surface encoder and MediaMuxer.
 *
 * Timing evidence kept for the core's `camera-pts-v1` mapping:
 * - The encoder input surface carries each frame's camera timestamp, so the smallest
 *   encoded presentation time is the camera time of file position zero.
 * - One capture-started callback reads the frame's camera timestamp next to both host
 *   clocks, so the core can tell which clock the camera follows.
 * - Boot-minus-monotonic offsets are sampled at start and stop to detect device sleep.
 *
 * Nothing here decides alignment; the core does, and labels it.
 */
class ClipRecorder(private val context: Context) {
    data class Result(val file: File, val meta: VideoMeta)

    private val thread = HandlerThread("squib-camera").also { it.start() }
    private val handler = Handler(thread.looper)
    private var camera: CameraDevice? = null
    private var session: CameraCaptureSession? = null
    private var encoder: MediaCodec? = null
    private var inputSurface: Surface? = null
    private var muxer: MediaMuxer? = null
    private var drain: Thread? = null
    private var file: File? = null
    private var size = Size(1280, 720)
    private var clock = "unknown"
    private val stopped = AtomicBoolean(false)
    private var result: Result? = null

    @Volatile private var track = -1
    @Volatile private var minPtsUs = Long.MAX_VALUE
    @Volatile private var maxPtsUs = Long.MIN_VALUE
    @Volatile private var frames = 0
    @Volatile private var probe: LongArray? = null
    private var startOffsetNs = 0L
    private var firstFrame = CountDownLatch(1)

    val recording: Boolean get() = camera != null && !stopped.get()

    private fun bootMinusMono(): Long {
        // Read in both orders and average to bound the gap between the two reads.
        val m1 = System.nanoTime()
        val b = SystemClock.elapsedRealtimeNanos()
        val m2 = System.nanoTime()
        return b - (m1 + (m2 - m1) / 2)
    }

    /**
     * Opens the back camera and starts recording to [out]. Blocks until the first frame
     * is encoded (or fails within [timeoutMs]); returns an error message or null.
     */
    @SuppressLint("MissingPermission") // Callers request CAMERA at the point of use.
    fun start(out: File, preview: SurfaceTexture?, timeoutMs: Long = 5_000): String? {
        val mgr = context.getSystemService(Context.CAMERA_SERVICE) as CameraManager
        val id = mgr.cameraIdList.firstOrNull {
            mgr.getCameraCharacteristics(it).get(CameraCharacteristics.LENS_FACING) == CameraCharacteristics.LENS_FACING_BACK
        } ?: mgr.cameraIdList.firstOrNull() ?: return "No camera is available."
        val ch = mgr.getCameraCharacteristics(id)
        clock = if (ch.get(CameraCharacteristics.SENSOR_INFO_TIMESTAMP_SOURCE) ==
            CameraCharacteristics.SENSOR_INFO_TIMESTAMP_SOURCE_REALTIME
        ) "realtime" else "unknown"
        val sizes = ch.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)?.getOutputSizes(MediaCodec::class.java).orEmpty()
        size = sizes.firstOrNull { it.width == 1280 && it.height == 720 }
            ?: sizes.filter { it.width <= 1280 }.maxByOrNull { it.width * it.height }
            ?: return "The camera has no video size this app can record."
        val fpsRange = ch.get(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES)
            ?.firstOrNull { it.lower == 30 && it.upper == 30 } ?: Range(15, 30)

        out.parentFile?.mkdirs()
        file = out
        result = null
        stopped.set(false)
        firstFrame = CountDownLatch(1)
        startOffsetNs = bootMinusMono()

        val fmt = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, size.width, size.height).apply {
            setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
            setInteger(MediaFormat.KEY_BIT_RATE, 3_000_000)
            setInteger(MediaFormat.KEY_FRAME_RATE, 30)
            setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 1)
        }
        val enc = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_VIDEO_AVC)
        enc.configure(fmt, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        val input = enc.createInputSurface()
        enc.start()
        encoder = enc
        inputSurface = input
        // No setLocation: the file carries no location metadata.
        muxer = MediaMuxer(out.path, MediaMuxer.OutputFormat.MUXER_OUTPUT_MPEG_4)
        drain = Thread({ drainLoop(enc) }, "squib-video-drain").also { it.start() }

        val previewSurface = preview?.let { it.setDefaultBufferSize(size.width, size.height); Surface(it) }
        val opened = CountDownLatch(1)
        var error: String? = null
        mgr.openCamera(id, object : CameraDevice.StateCallback() {
            override fun onOpened(device: CameraDevice) {
                camera = device
                val targets = listOfNotNull(input, previewSurface)
                @Suppress("DEPRECATION") // SessionConfiguration needs API 28; minSdk is 26.
                device.createCaptureSession(targets, object : CameraCaptureSession.StateCallback() {
                    override fun onConfigured(s: CameraCaptureSession) {
                        session = s
                        val req = device.createCaptureRequest(CameraDevice.TEMPLATE_RECORD).apply {
                            targets.forEach { addTarget(it) }
                            set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, fpsRange)
                        }.build()
                        s.setRepeatingRequest(req, object : CameraCaptureSession.CaptureCallback() {
                            override fun onCaptureStarted(s: CameraCaptureSession, r: CaptureRequest, timestamp: Long, frame: Long) {
                                if (probe == null) {
                                    val mono = System.nanoTime()
                                    val boot = SystemClock.elapsedRealtimeNanos()
                                    probe = longArrayOf(timestamp, mono, boot)
                                }
                            }
                        }, handler)
                        opened.countDown()
                    }

                    override fun onConfigureFailed(s: CameraCaptureSession) {
                        error = "The camera could not be configured for recording."
                        opened.countDown()
                    }
                }, handler)
            }

            override fun onDisconnected(device: CameraDevice) {
                // Another app or the system took the camera (for example when Squib leaves
                // the foreground). Keep what was recorded.
                error = error ?: "The camera was disconnected."
                opened.countDown()
                Thread { stop() }.start()
            }

            override fun onError(device: CameraDevice, code: Int) {
                error = "Camera error $code."
                opened.countDown()
                Thread { stop() }.start()
            }
        }, handler)

        if (!opened.await(timeoutMs, TimeUnit.MILLISECONDS)) error = "The camera did not open in time."
        if (error == null && !firstFrame.await(timeoutMs, TimeUnit.MILLISECONDS)) error = "The camera produced no frames."
        if (error != null) {
            stop()?.file?.delete()
            result = null
            out.delete()
        }
        return error
    }

    private fun drainLoop(enc: MediaCodec) {
        val info = MediaCodec.BufferInfo()
        while (true) {
            val idx = try {
                enc.dequeueOutputBuffer(info, 10_000)
            } catch (e: IllegalStateException) {
                return
            }
            when {
                idx == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> {
                    val m = muxer ?: return
                    track = m.addTrack(enc.outputFormat)
                    m.start()
                }
                idx >= 0 -> {
                    val buf = enc.getOutputBuffer(idx)
                    val config = info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0
                    if (buf != null && info.size > 0 && !config && track >= 0) {
                        buf.position(info.offset).limit(info.offset + info.size)
                        muxer?.writeSampleData(track, buf, info)
                        minPtsUs = minOf(minPtsUs, info.presentationTimeUs)
                        maxPtsUs = maxOf(maxPtsUs, info.presentationTimeUs)
                        frames++
                        firstFrame.countDown()
                        if ((maxPtsUs - minPtsUs) / 1000 >= MAX_CLIP_MS) handler.post { session?.stopRepeating() }
                    }
                    enc.releaseOutputBuffer(idx, false)
                    if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) return
                }
            }
        }
    }

    /**
     * Stops and finalizes the file. Returns null if nothing usable was recorded. Safe to
     * call again (for example after a camera disconnect already stopped it): later calls
     * return the same result.
     */
    @Synchronized
    fun stop(): Result? {
        if (!stopped.compareAndSet(false, true)) return result
        val endOffset = bootMinusMono()
        runCatching { session?.stopRepeating() }
        runCatching { session?.close() }
        runCatching { camera?.close() }
        session = null
        camera = null
        val enc = encoder
        runCatching { enc?.signalEndOfInputStream() }
        drain?.join(3_000)
        drain = null
        runCatching { enc?.stop() }
        runCatching { enc?.release() }
        encoder = null
        runCatching { inputSurface?.release() }
        inputSurface = null
        val started = track >= 0
        runCatching { if (started) muxer?.stop() }
        runCatching { muxer?.release() }
        muxer = null
        track = -1
        val f = file ?: return null
        val p = probe
        probe = null
        if (!started || frames == 0 || p == null) {
            f.delete()
            return null
        }
        val fps = if (frames > 1 && maxPtsUs > minPtsUs) ((frames - 1) * 1_000_000.0 / (maxPtsUs - minPtsUs)).toFloat() else 30f
        // The last frame is shown for one frame period.
        val durationMs = (maxPtsUs - minPtsUs) / 1000 + (1000 / fps.coerceIn(1f, 240f)).toLong()
        val meta = VideoMeta(
            mime = "video/mp4",
            width = size.width.toUInt(),
            height = size.height.toUInt(),
            frameRate = fps.coerceIn(1f, 240f),
            durationMs = durationMs,
            firstFrameCameraNs = minPtsUs * 1000,
            cameraClock = clock,
            probeCameraNs = p[0],
            probeMonoNs = p[1],
            probeBootNs = p[2],
            startBootMinusMonoNs = startOffsetNs,
            endBootMinusMonoNs = endOffset,
        )
        // Raw clock evidence for device qualification (timing numbers only).
        android.util.Log.i(
            "SquibVideo",
            "clock=$clock frames=$frames fps=$fps first=${meta.firstFrameCameraNs} probe=${p[0]},${p[1]},${p[2]} " +
                "bootMinusMono=$startOffsetNs..$endOffset",
        )
        minPtsUs = Long.MAX_VALUE
        maxPtsUs = Long.MIN_VALUE
        frames = 0
        return Result(f, meta).also { result = it }
    }

    fun release() {
        stop()
        thread.quitSafely()
    }

    companion object {
        /** Matches the core's recording limit. */
        const val MAX_CLIP_MS = 120_000L
    }
}
