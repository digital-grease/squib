package net.digitalgrease.squib.audio

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioTimestamp
import android.media.AudioTrack
import net.digitalgrease.squib.core.CueType
import net.digitalgrease.squib.core.SquibEngine

/**
 * Plays the core's deterministic cue templates and reports a render reference.
 *
 * Uses media-volume playback (`USAGE_MEDIA`): the app never raises volume or bypasses
 * mute/DND (A23). The render reference is the platform playback timestamp mapped back
 * to the cue's first frame; when the platform gives no timestamp the reference is
 * `null` and the core labels the start `requested_only`.
 */
class CuePlayer(private val engine: SquibEngine) {
    private val rate = AudioTrack.getNativeOutputSampleRate(AudioManager.STREAM_MUSIC).coerceIn(8_000, 192_000)
    private val tracks = HashMap<CueType, Pair<AudioTrack, Int>>()

    private fun track(cue: CueType): Pair<AudioTrack, Int> = tracks.getOrPut(cue) {
        val pcm = engine.cuePcm(cue, rate.toUInt()).toShortArray()
        val attrs = AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_MEDIA)
            .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
            .build()
        val fmt = AudioFormat.Builder()
            .setSampleRate(rate)
            .setChannelMask(AudioFormat.CHANNEL_OUT_MONO)
            .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
            .build()
        val t = AudioTrack.Builder()
            .setAudioAttributes(attrs)
            .setAudioFormat(fmt)
            .setTransferMode(AudioTrack.MODE_STATIC)
            .setBufferSizeInBytes(pcm.size * 2)
            .build()
        t.write(pcm, 0, pcm.size)
        t to pcm.size
    }

    /** Preload templates so the first cue does not pay setup cost. */
    fun prepare() {
        track(CueType.START)
        track(CueType.PAR)
    }

    /**
     * Start playback now. Blocks the calling (control) thread for up to ~300 ms polling
     * the playback timestamp; returns the monotonic time of the cue's first frame or null.
     */
    fun play(cue: CueType): Long? {
        val (t, frames) = track(cue)
        runCatching {
            t.stop()
            t.reloadStaticData()
        }
        t.play()
        val ts = AudioTimestamp()
        val deadline = System.nanoTime() + 300_000_000L
        while (System.nanoTime() < deadline) {
            if (t.getTimestamp(ts) && ts.framePosition > 0 && ts.framePosition <= frames) {
                return ts.nanoTime - ts.framePosition * 1_000_000_000L / rate
            }
            Thread.sleep(2)
        }
        return null
    }

    /** Actual output device of the last playback, for route reporting. */
    fun routedOutput(): String = when (tracks.values.firstOrNull()?.first?.routedDevice?.type) {
        android.media.AudioDeviceInfo.TYPE_BUILTIN_SPEAKER -> "builtin_speaker"
        android.media.AudioDeviceInfo.TYPE_BLUETOOTH_A2DP, android.media.AudioDeviceInfo.TYPE_BLUETOOTH_SCO -> "bluetooth"
        android.media.AudioDeviceInfo.TYPE_WIRED_HEADPHONES, android.media.AudioDeviceInfo.TYPE_WIRED_HEADSET -> "wired_headphones"
        null -> "unknown"
        else -> "other"
    }

    fun release() {
        tracks.values.forEach { it.first.release() }
        tracks.clear()
    }
}
