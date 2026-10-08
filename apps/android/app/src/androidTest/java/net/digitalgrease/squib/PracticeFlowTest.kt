package net.digitalgrease.squib

import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** M3 on an emulator: manual result (A20), generic score (A15), analytics denominators (A16), drill start. */
@RunWith(AndroidJUnit4::class)
class PracticeFlowTest {
    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    private fun has(text: String) = rule.onAllNodes(hasText(text, substring = true)).fetchSemanticsNodes().isNotEmpty()

    @Test
    fun manualResultScoreAndAnalytics() {
        rule.onNodeWithTag("tab_practice").performClick()
        rule.waitUntil(5_000) { has("Timed string with score") }
        rule.onNodeWithTag("manual_entry").performClick()
        rule.onNodeWithTag("manual_label").performTextInput("Club timer")
        rule.onNodeWithTag("manual_times").performTextInput("1.20 1.65 2.10")
        rule.onNodeWithTag("manual_save").performClick()
        // Review opens for the new run with manual events.
        rule.waitUntil(10_000) { rule.onAllNodes(hasTestTag("review_screen")).fetchSemanticsNodes().isNotEmpty() }
        rule.waitUntil(5_000) { has("1.20 s") }
        rule.waitUntil(5_000) { rule.onAllNodes(hasTestTag("score_status")).fetchSemanticsNodes().isNotEmpty() }
        rule.onNodeWithText("Points and hit factor").performScrollTo().performClick()
        rule.waitUntil(5_000) { has("Incomplete") }
        rule.onNodeWithTag("inc_A").performScrollTo().performClick()
        rule.onNodeWithTag("inc_A").performClick()
        rule.onNodeWithTag("score_complete").performScrollTo().performClick()
        rule.onNodeWithTag("save_score").performScrollTo().performClick()
        rule.waitUntil(5_000) { has("HF (10 pts / 2.10 s)") }

        rule.onNodeWithTag("tab_history").performClick()
        rule.waitUntil(5_000) { rule.onAllNodes(hasTestTag("analytics_count")).fetchSemanticsNodes().isNotEmpty() }
        // Manual results are excluded unless chosen, and the exclusion is stated.
        rule.waitUntil(5_000) { has("manual results") }
        rule.onNodeWithText("Include manual").performClick()
        rule.waitUntil(5_000) { has("too few for a trend") }
    }

    @Test
    fun startingADrillLoadsItIntoTheTimer() {
        rule.onNodeWithTag("tab_practice").performClick()
        rule.waitUntil(5_000) { has("Repeated par") }
        rule.onNodeWithTag("start_starter-2").performScrollTo().performClick()
        rule.waitUntil(5_000) { rule.onAllNodes(hasTestTag("drill_banner")).fetchSemanticsNodes().isNotEmpty() }
        rule.waitUntil(5_000) { has("string 1 of 5") }
    }
}
