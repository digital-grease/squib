package net.digitalgrease.squib

import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Plan phase 2.3 on an emulator: par-only runs with screen (and, where the emulator has a
 * vibrator, vibration) cues are saved with their own start method and say so.
 */
@RunWith(AndroidJUnit4::class)
class CueChannelFlowTest {
    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    private fun tag(t: String) = rule.onAllNodes(hasTestTag(t)).fetchSemanticsNodes().isNotEmpty()
    private fun has(t: String) = rule.onAllNodes(hasText(t, substring = true)).fetchSemanticsNodes().isNotEmpty()

    private fun runOnce(cueTag: String, label: String, overlay: String?) {
        rule.onNodeWithTag(cueTag).performScrollTo().performClick()
        rule.waitOrDump { tag("cue_note") }
        rule.onNodeWithTag("arm").performScrollTo().performClick()
        if (overlay != null) rule.waitOrDump(10_000) { tag(overlay) }
        rule.waitOrDump(10_000) { has("Saved · Complete") }
        rule.waitOrDump { has(label) && has("Par cues played: 1/1") }
        rule.onNodeWithTag("new_setup").performScrollTo().performClick()
        rule.waitOrDump { tag("arm") }
    }

    @Test
    fun screenAndVibrationStartsAreSavedAsTheirOwnMethods() {
        rule.resetTimer()
        rule.onNodeWithTag("mode_par").performClick()
        rule.onNodeWithTag("delay_instant").performScrollTo().performClick()
        rule.onNodeWithTag("pars").performScrollTo().performTextReplacement("1.0")
        rule.waitOrDump { tag("auto_stop_switch") }
        rule.onNodeWithTag("auto_stop_switch").performScrollTo().performClick()

        // Live mode does not offer the choice: shot timing needs the sound cue.
        rule.onNodeWithTag("mode_live").performClick()
        rule.waitOrDump { !tag("cue_screen") }
        rule.onNodeWithTag("mode_par").performClick()
        rule.waitOrDump { tag("cue_screen") }

        runOnce("cue_screen", "screen cue frame", overlay = "screen_cue_start")
        val canVibrate = rule.onNodeWithTag("cue_vibration").fetchSemanticsNode().config
            .getOrElseNullable(SemanticsProperties.Disabled) { null } == null
        if (canVibrate) runOnce("cue_vibration", "vibration requested", overlay = null)
        android.util.Log.i("SquibTest", "vibration cue checked: $canVibrate")

        // Leave the shared timer on sound with no pars.
        rule.onNodeWithTag("cue_sound").performScrollTo().performClick()
        rule.onNodeWithTag("auto_stop_switch").performScrollTo().performClick()
        rule.onNodeWithTag("pars").performScrollTo().performTextReplacement("")
        rule.waitOrDump { !tag("cue_note") && !tag("auto_stop_switch") }
    }
}
