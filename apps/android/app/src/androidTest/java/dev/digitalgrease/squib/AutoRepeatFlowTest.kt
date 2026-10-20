package dev.digitalgrease.squib

import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Plan phase 2.2 on an emulator, par-only: with automatic repeat on, a complete string shows
 * a countdown with Cancel, then the next string arms by itself; Cancel ends the chain.
 */
@RunWith(AndroidJUnit4::class)
class AutoRepeatFlowTest {
    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    private fun tag(t: String) = rule.onAllNodes(hasTestTag(t)).fetchSemanticsNodes().isNotEmpty()
    private fun has(t: String) = rule.onAllNodes(hasText(t, substring = true)).fetchSemanticsNodes().isNotEmpty()

    @Test
    fun nextStringArmsAfterAVisibleCountdownAndCancelEndsTheChain() {
        rule.resetTimer()
        rule.onNodeWithTag("tab_practice").performClick()
        rule.waitOrDump { has("Repeated par") }
        rule.onNodeWithTag("start_starter-2").performScrollTo().performClick()
        rule.waitOrDump { has("string 1 of 5") }
        rule.onNodeWithTag("mode_par").performClick()
        rule.onNodeWithTag("delay_instant").performScrollTo().performClick()
        rule.onNodeWithTag("auto_stop_switch").performScrollTo().performClick()
        check(!tag("auto_repeat_status")) { "nothing is pending before the first string" }
        rule.onNodeWithTag("auto_repeat_switch").performScrollTo().performClick()
        rule.waitOrDump { has("at least 5 s") }
        rule.onNodeWithTag("arm").performScrollTo().performClick()

        // String 1 stops itself 1 s after the 1.5 s par, then the 20 s rest counts down.
        rule.waitOrDump(15_000) { tag("auto_repeat_status") }
        rule.waitOrDump { has("String 2 of 5 arms automatically in") }
        rule.onNodeWithTag("auto_repeat_cancel").assertExists()
        // String 2 arms by itself and saves.
        rule.waitOrDump(30_000) { has("string 2 of 5") && !tag("auto_repeat_status") }
        rule.waitOrDump(15_000) { has("String 3 of 5 arms automatically in") }

        rule.onNodeWithTag("auto_repeat_cancel").performScrollTo().performClick()
        rule.waitOrDump { !tag("auto_repeat_status") }
        Thread.sleep(1_500)
        check(has("string 2 of 5") && has("Saved · Complete")) { "cancel keeps the result and arms nothing" }
        rule.onNodeWithTag("repeat").assertExists()

        // Leave the shared timer without a drill for other tests.
        rule.onNodeWithTag("new_setup").performScrollTo().performClick()
        rule.waitOrDump { tag("drill_banner") && has("Clear") }
        rule.onNodeWithText("Clear").performClick()
        rule.onNodeWithTag("auto_stop_switch").performScrollTo().performClick()
        rule.onNodeWithTag("pars").performScrollTo().performTextReplacement("")
        rule.waitOrDump { !tag("drill_banner") }
    }
}
