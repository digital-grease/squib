package net.digitalgrease.squib

import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
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
 * Plan phase 2.1 settings on an emulator: the shot-count stop is offered in live mode only
 * and is exclusive with the after-last-par stop. The emulator cannot hear its own start cue,
 * so the stop itself is covered by the engine simulation (`engine_flows`), not here.
 */
@RunWith(AndroidJUnit4::class)
class AutoStopSettingsTest {
    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    private fun tag(t: String) = rule.onAllNodes(hasTestTag(t)).fetchSemanticsNodes().isNotEmpty()
    private fun has(t: String) = rule.onAllNodes(hasText(t, substring = true)).fetchSemanticsNodes().isNotEmpty()
    private fun node(t: String) = rule.onNodeWithTag(t).performScrollTo()

    @Test
    fun countStopIsLiveOnlyAndExclusiveWithParStop() {
        rule.resetTimer()
        rule.onNodeWithTag("mode_par").performClick()
        rule.waitOrDump { !tag("auto_stop_count_switch") }

        rule.onNodeWithTag("mode_live").performClick()
        rule.waitOrDump { tag("auto_stop_count_switch") }
        node("pars").performTextReplacement("2.0")
        rule.waitOrDump { tag("auto_stop_switch") }
        if (rule.onAllNodes(hasTestTag("auto_stop_count")).fetchSemanticsNodes().isEmpty()) {
            node("auto_stop_count_switch").performClick()
        }
        rule.waitOrDump { tag("auto_stop_count") }
        node("auto_stop_count").performTextReplacement("5")
        rule.waitOrDump { has("accepts shot 5") }
        node("auto_stop_switch").assertIsOff()

        // Only one automatic stop at a time, in either direction.
        node("auto_stop_switch").performClick()
        rule.waitOrDump { !tag("auto_stop_count") }
        node("auto_stop_count_switch").assertIsOff()
        node("auto_stop_count_switch").performClick()
        rule.waitOrDump { tag("auto_stop_count") }
        node("auto_stop_switch").assertIsOff()
        node("auto_stop_count_switch").assertIsOn()

        // Leave the shared settings as other tests expect them.
        node("auto_stop_count_switch").performClick()
        node("pars").performTextReplacement("")
        rule.onNodeWithTag("mode_par").performScrollTo().performClick()
        rule.waitOrDump { !tag("auto_stop_count_switch") }
    }
}
