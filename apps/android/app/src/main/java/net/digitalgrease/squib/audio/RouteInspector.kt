package net.digitalgrease.squib.audio

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.media.AudioDeviceInfo
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioRecord
import android.media.audiofx.AcousticEchoCanceler
import android.media.audiofx.AutomaticGainControl
import android.media.audiofx.NoiseSuppressor
import android.os.Build
import net.digitalgrease.squib.core.RouteReport

/**
 * Reports the audio route. Before capture this is the *expected* route from connected
 * devices; once capture/playback start, the adapters report the actually routed device.
 * A platform preference request is never treated as evidence of routing.
 */
class RouteInspector(private val context: Context) {
    private val am = context.getSystemService(Context.AUDIO_SERVICE) as AudioManager

    fun micPermission(): Boolean =
        context.checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED

    fun unprocessedSupported(): Boolean =
        am.getProperty(AudioManager.PROPERTY_SUPPORT_AUDIO_SOURCE_UNPROCESSED) == "true"

    fun mediaVolume(): Float {
        val max = am.getStreamMaxVolume(AudioManager.STREAM_MUSIC).coerceAtLeast(1)
        return am.getStreamVolume(AudioManager.STREAM_MUSIC).toFloat() / max
    }

    /** 48 kHz preferred; 44.1 kHz accepted when the device cannot open 48 kHz. */
    fun expectedRateHz(): Int {
        val ok = AudioRecord.getMinBufferSize(48_000, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
        return if (ok > 0) 48_000 else 44_100
    }

    fun expectedRoute(): RouteReport {
        val inputs = am.getDevices(AudioManager.GET_DEVICES_INPUTS).map { it.type }
        val outputs = am.getDevices(AudioManager.GET_DEVICES_OUTPUTS).map { it.type }
        val input = when {
            inputs.any { it == AudioDeviceInfo.TYPE_BLUETOOTH_SCO } -> "bluetooth_sco"
            inputs.any { it == AudioDeviceInfo.TYPE_USB_DEVICE || it == AudioDeviceInfo.TYPE_USB_HEADSET } -> "usb"
            inputs.any { it == AudioDeviceInfo.TYPE_WIRED_HEADSET } -> "wired_headset"
            inputs.any { it == AudioDeviceInfo.TYPE_BUILTIN_MIC } -> "builtin_mic"
            else -> "unknown"
        }
        val output = when {
            outputs.any { it == AudioDeviceInfo.TYPE_BLUETOOTH_A2DP || it == AudioDeviceInfo.TYPE_BLUETOOTH_SCO } -> "bluetooth"
            outputs.any { it == AudioDeviceInfo.TYPE_WIRED_HEADPHONES || it == AudioDeviceInfo.TYPE_WIRED_HEADSET } -> "wired_headphones"
            outputs.any { it == AudioDeviceInfo.TYPE_USB_DEVICE || it == AudioDeviceInfo.TYPE_USB_HEADSET } -> "usb"
            outputs.any { it == AudioDeviceInfo.TYPE_BUILTIN_SPEAKER } -> "builtin_speaker"
            else -> "unknown"
        }
        return report(input, output, if (unprocessedSupported()) "unprocessed" else "voice_recognition", emptyList())
    }

    fun report(input: String, output: String, source: String, effects: List<String>) = RouteReport(
        inputDevice = input,
        outputDevice = output,
        audioSource = source,
        unprocessedSupported = unprocessedSupported(),
        effects = effects,
        osBuild = "${Build.VERSION.RELEASE}/${Build.VERSION.SDK_INT}/${Build.ID}",
        deviceModel = "${Build.MANUFACTURER} ${Build.MODEL}",
        mediaVolume = mediaVolume(),
        micPermission = micPermission(),
    )

    companion object {
        fun inputName(d: AudioDeviceInfo?): String = when (d?.type) {
            AudioDeviceInfo.TYPE_BUILTIN_MIC -> "builtin_mic"
            AudioDeviceInfo.TYPE_BLUETOOTH_SCO -> "bluetooth_sco"
            AudioDeviceInfo.TYPE_USB_DEVICE, AudioDeviceInfo.TYPE_USB_HEADSET -> "usb"
            AudioDeviceInfo.TYPE_WIRED_HEADSET -> "wired_headset"
            null -> "unknown"
            else -> "type_${d.type}"
        }

        /** Effects enabled by default on this capture session. Inspect only; never toggled. */
        fun activeEffects(sessionId: Int): List<String> {
            val out = mutableListOf<String>()
            fun probe(name: String, available: Boolean, create: () -> android.media.audiofx.AudioEffect?) {
                if (!available) return
                val e = runCatching { create() }.getOrNull() ?: return
                if (runCatching { e.enabled }.getOrDefault(false)) out += name
                e.release()
            }
            probe("aec", AcousticEchoCanceler.isAvailable()) { AcousticEchoCanceler.create(sessionId) }
            probe("agc", AutomaticGainControl.isAvailable()) { AutomaticGainControl.create(sessionId) }
            probe("ns", NoiseSuppressor.isAvailable()) { NoiseSuppressor.create(sessionId) }
            return out
        }
    }
}
