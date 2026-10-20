package dev.digitalgrease.squib

import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onFirst
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Plan phase 1.3 on an emulator: a two-string manual drill with a total shows its set result. */
@RunWith(AndroidJUnit4::class)
class SetResultFlowTest {
    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    private fun has(t: String) = rule.onAllNodes(hasText(t, substring = true)).fetchSemanticsNodes().isNotEmpty()
    private fun tag(t: String) = rule.onAllNodes(hasTestTag(t)).fetchSemanticsNodes().isNotEmpty()

    private fun enterResult(title: String, times: String) {
        rule.onNodeWithTag("tab_practice").performClick()
        rule.waitUntil(5_000) { has(title) }
        rule.onNodeWithTag("enter_result_$title").performScrollTo().performClick()
        rule.waitUntil(5_000) { tag("manual_times") }
        rule.onNodeWithTag("manual_label").performTextInput("Club timer")
        rule.onNodeWithTag("manual_times").performTextInput(times)
        rule.waitForIdle()
        rule.onNodeWithTag("manual_save").performSemanticsAction(SemanticsActions.OnClick)
        rule.waitUntil(10_000) { tag("review_screen") }
    }

    @Test
    fun twoStringTotalIsShownForTheVisit() {
        val title = "Pair ${System.currentTimeMillis() % 10000}"
        rule.resetTimer()
        rule.onNodeWithTag("tab_practice").performClick()
        rule.waitUntil(5_000) { has("New drill") }
        rule.onAllNodes(hasText("New drill")).onFirst().performClick()
        rule.onNodeWithTag("drill_title").performTextInput(title)
        rule.onAllNodes(hasText("Manual result")).onFirst().performClick()
        rule.onNodeWithTag("drill_strings").performScrollTo().performTextReplacement("2")
        rule.waitUntil(5_000) { tag("set_sum") }
        rule.onNodeWithTag("set_sum").performScrollTo().performClick()
        rule.waitForIdle()
        rule.onNodeWithTag("drill_save").performScrollTo().performSemanticsAction(SemanticsActions.OnClick)
        rule.waitUntil(5_000) { has("set: total") }

        enterResult(title, "0.80 1.20")
        enterResult(title, "1.10 1.90")

        rule.onNodeWithTag("tab_history").performClick()
        rule.waitUntil(10_000) { has("run(s),") }
        rule.onAllNodes(hasText("In progress", substring = true)).onFirst().performClick()
        rule.waitUntil(5_000) { tag("session_screen") }
        rule.waitUntil(10_000) { has("$title · Total of 2 strings") }
        rule.waitUntil(5_000) { has("3.10 s") }
    }
}
