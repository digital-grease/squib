package net.digitalgrease.squib.audio

import android.content.Context
import android.media.AudioAttributes
import android.os.Build
import android.os.VibrationAttributes
import android.os.VibrationEffect
import android.os.Vibrator
import android.os.VibratorManager
import net.digitalgrease.squib.core.CueType

/**
 * Vibration start and par cues (plan phase 2.3). Uses media vibration attributes, so the
 * phone's own vibration settings and Do Not Disturb apply; Squib never overrides them.
 * The start time it reports is when the vibrator was asked to start: the motor's own
 * response time is not measured, and the run says so.
 */
class CueVibrator(context: Context) {
    private val vibrator: Vibrator = if (Build.VERSION.SDK_INT >= 31) {
        (context.getSystemService(Context.VIBRATOR_MANAGER_SERVICE) as VibratorManager).defaultVibrator
    } else {
        @Suppress("DEPRECATION")
        context.getSystemService(Context.VIBRATOR_SERVICE) as Vibrator
    }

    fun available(): Boolean = vibrator.hasVibrator()

    /** Vibrate now; returns the monotonic time of the request, or null without a vibrator. */
    fun vibrate(cue: CueType): Long? {
        if (!available()) return null
        // Same lengths as the sound cues, so par spacing rules are unchanged.
        val effect = VibrationEffect.createOneShot(if (cue == CueType.START) 150L else 120L, VibrationEffect.DEFAULT_AMPLITUDE)
        val requested = System.nanoTime()
        if (Build.VERSION.SDK_INT >= 33) {
            vibrator.vibrate(effect, VibrationAttributes.createForUsage(VibrationAttributes.USAGE_MEDIA))
        } else {
            @Suppress("DEPRECATION")
            vibrator.vibrate(effect, AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA).build())
        }
        return requested
    }
}
