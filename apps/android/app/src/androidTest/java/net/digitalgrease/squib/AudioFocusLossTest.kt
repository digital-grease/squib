package net.digitalgrease.squib

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioManager
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** R23: another audio client taking focus mid-run interrupts the run instead of continuing silently. */
@RunWith(AndroidJUnit4::class)
class AudioFocusLossTest {
    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    @Test
    fun focusLossDuringRunInterruptsAndSaves() {
        rule.resetTimer()
        rule.onNodeWithTag("mode_par").performClick()
        rule.onNodeWithTag("delay_instant").performClick()
        rule.onNodeWithTag("arm").performScrollTo().performClick()
        rule.waitUntil(10_000) { rule.onAllNodes(hasTestTag("stop")).fetchSemanticsNodes().isNotEmpty() }

        val am = InstrumentationRegistry.getInstrumentation().targetContext.getSystemService(Context.AUDIO_SERVICE) as AudioManager
        val intruder = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
            .setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA).build())
            .setOnAudioFocusChangeListener { }
            .build()
        am.requestAudioFocus(intruder)
        try {
            rule.waitUntil(10_000) { rule.onAllNodes(hasText("Saved · Interrupted")).fetchSemanticsNodes().isNotEmpty() }
            rule.waitUntil(5_000) { rule.onAllNodes(hasText("Audio focus lost")).fetchSemanticsNodes().isNotEmpty() }
        } finally {
            am.abandonAudioFocusRequest(intruder)
        }
    }
}
