package net.digitalgrease.squib.video

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.ImageFormat
import android.graphics.SurfaceTexture
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CaptureRequest
import android.media.ImageReader
import android.os.Handler
import android.os.HandlerThread
import android.os.SystemClock
import android.util.Size
import android.view.Surface
import java.util.concurrent.atomic.AtomicBoolean
import net.digitalgrease.squib.core.decodeQrLuma

/**
 * Reads QR codes with the back camera (plan phase 4.1). Frames go to the core's decoder as
 * greyscale (the Y plane), at most about five a second; nothing is saved or sent. The
 * camera opens only while scanning and closes when a code is found or scanning stops.
 */
class QrScanner(private val context: Context) {
    private val thread = HandlerThread("squib-qr").also { it.start() }
    private val handler = Handler(thread.looper)
    private var camera: CameraDevice? = null
    private var session: CameraCaptureSession? = null
    private var reader: ImageReader? = null
    private val busy = AtomicBoolean(false)
    @Volatile private var lastDecodeMs = 0L
    @Volatile private var active = false

    /** Start scanning; [onText] is called (on a background thread) with each code's text. */
    @SuppressLint("MissingPermission") // CAMERA is requested at the point of use.
    fun start(preview: SurfaceTexture?, onError: (String) -> Unit, onText: (String) -> Unit) {
        val mgr = context.getSystemService(Context.CAMERA_SERVICE) as CameraManager
        val id = mgr.cameraIdList.firstOrNull {
            mgr.getCameraCharacteristics(it).get(CameraCharacteristics.LENS_FACING) == CameraCharacteristics.LENS_FACING_BACK
        } ?: mgr.cameraIdList.firstOrNull() ?: return onError("No camera is available.")
        val sizes = mgr.getCameraCharacteristics(id).get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)
            ?.getOutputSizes(ImageFormat.YUV_420_888).orEmpty()
        // About VGA to 720p: enough for a phone-screen QR at arm's length, cheap to decode.
        val size = sizes.filter { it.width in 640..1280 && it.height in 480..960 }.minByOrNull { it.width * it.height }
            ?: sizes.minByOrNull { kotlin.math.abs(it.width * it.height - 640 * 480) } ?: Size(640, 480)
        val r = ImageReader.newInstance(size.width, size.height, ImageFormat.YUV_420_888, 2)
        reader = r
        active = true
        r.setOnImageAvailableListener({ ir ->
            val img = ir.acquireLatestImage() ?: return@setOnImageAvailableListener
            try {
                val now = SystemClock.elapsedRealtime()
                if (!active || now - lastDecodeMs < 200 || !busy.compareAndSet(false, true)) return@setOnImageAvailableListener
                lastDecodeMs = now
                val plane = img.planes[0]
                val buf = plane.buffer
                val bytes = ByteArray(buf.remaining()).also { buf.get(it) }
                val text = try {
                    decodeQrLuma(img.width.toUInt(), img.height.toUInt(), plane.rowStride.toUInt(), bytes)
                } finally {
                    busy.set(false)
                }
                if (text != null && active) onText(text)
            } finally {
                img.close()
            }
        }, handler)
        val previewSurface = preview?.let { it.setDefaultBufferSize(size.width, size.height); Surface(it) }
        mgr.openCamera(id, object : CameraDevice.StateCallback() {
            override fun onOpened(device: CameraDevice) {
                if (!active) return device.close()
                camera = device
                val targets = listOfNotNull(r.surface, previewSurface)
                @Suppress("DEPRECATION") // SessionConfiguration needs API 28; minSdk is 26.
                device.createCaptureSession(targets, object : CameraCaptureSession.StateCallback() {
                    override fun onConfigured(s: CameraCaptureSession) {
                        session = s
                        val req = device.createCaptureRequest(CameraDevice.TEMPLATE_PREVIEW).apply {
                            targets.forEach { addTarget(it) }
                            set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_PICTURE)
                        }.build()
                        runCatching { s.setRepeatingRequest(req, null, handler) }.onFailure { onError("The camera could not start.") }
                    }

                    override fun onConfigureFailed(s: CameraCaptureSession) = onError("The camera could not start.")
                }, handler)
            }

            override fun onDisconnected(device: CameraDevice) {
                device.close()
                camera = null
            }

            override fun onError(device: CameraDevice, error: Int) {
                device.close()
                camera = null
                onError("The camera stopped (error $error).")
            }
        }, handler)
    }

    fun stop() {
        active = false
        handler.post {
            runCatching { session?.close() }
            runCatching { camera?.close() }
            runCatching { reader?.close() }
            session = null
            camera = null
            reader = null
        }
    }

    fun release() {
        stop()
        thread.quitSafely()
    }
}
