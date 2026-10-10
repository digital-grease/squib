package net.digitalgrease.squib

import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onFirst
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Plan phase 1.1 on an emulator: a visit groups runs, keeps notes, ends, and deletes alone. */
@RunWith(AndroidJUnit4::class)
class SessionFlowTest {
    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    private fun has(t: String) = rule.onAllNodes(hasText(t, substring = true)).fetchSemanticsNodes().isNotEmpty()
    private fun tag(t: String) = rule.onAllNodes(hasTestTag(t)).fetchSemanticsNodes().isNotEmpty()

    /** Wait, and on timeout log the visible text so a failure shows what was on screen. */
    private fun waitFor(ms: Long = 5_000, cond: () -> Boolean) {
        try {
            rule.waitUntil(ms, cond)
        } catch (e: Throwable) {
            val texts = rule.onAllNodes(androidx.compose.ui.test.isRoot()).fetchSemanticsNodes().flatMap { n ->
                fun walk(x: androidx.compose.ui.semantics.SemanticsNode): List<String> =
                    (x.config.getOrNull(androidx.compose.ui.semantics.SemanticsProperties.Text)?.map { it.text } ?: emptyList()) +
                        (x.config.getOrNull(androidx.compose.ui.semantics.SemanticsProperties.EditableText)?.let { listOf("[edit:" + it.text + "]") } ?: emptyList()) +
                        x.children.flatMap(::walk)
                walk(n)
            }
            android.util.Log.e("SquibSessionTest", "screen: " + texts.joinToString(" | "))
            throw e
        }
    }

    private fun manualRun(times: String) {
        rule.onNodeWithTag("tab_practice").performClick()
        rule.waitUntil(5_000) { tag("manual_entry") }
        rule.onNodeWithTag("manual_entry").performScrollTo().performClick()
        rule.onNodeWithTag("manual_label").performTextInput("Club timer")
        rule.onNodeWithTag("manual_times").performTextInput(times)
        rule.onNodeWithTag("manual_save").performClick()
        rule.waitUntil(10_000) { tag("review_screen") }
    }

    @Test
    fun visitGroupsRunsKeepsNotesEndsAndDeletes() {
        rule.resetTimer()
        // End whatever visit earlier tests left open, so this test owns a fresh one.
        rule.onNodeWithTag("tab_history").performClick()
        waitFor { tag("history") }
        // Visits load asynchronously: wait until they are shown (or there are none).
        waitFor(10_000) { has("run(s),") || has("No runs yet") }
        if (has("In progress")) {
            rule.onAllNodes(hasText("In progress", substring = true)).onFirst().performClick()
            waitFor { tag("session_end") }
            rule.onNodeWithTag("session_end").performScrollTo().performClick()
            waitFor { has("Ended") }
            rule.onAllNodes(hasText("‹ History")).onFirst().performClick()
        }
        manualRun("1.10 1.60")
        manualRun("1.20 1.75 2.30")

        rule.onNodeWithTag("tab_history").performClick()
        waitFor { has("2 run(s), 2 complete") }
        rule.onAllNodes(hasText("2 run(s), 2 complete", substring = true)).onFirst().performClick()
        waitFor { tag("session_screen") }
        rule.onNodeWithTag("session_notes").performScrollTo().performTextReplacement("Bay 4, windy")
        // The keyboard resizes the screen as it opens; invoke the click action directly so a
        // moving layout cannot swallow the tap.
        rule.waitForIdle()
        rule.onNodeWithTag("session_notes_save").performSemanticsAction(androidx.compose.ui.semantics.SemanticsActions.OnClick)
        waitFor { !tag("session_notes_save") }
        rule.onNodeWithTag("session_end").performScrollTo().performClick()
        waitFor { has("Ended") }

        rule.onNodeWithTag("session_delete").performScrollTo().performClick()
        rule.onNodeWithTag("session_delete_confirm").performClick()
        waitFor { tag("history") }
        waitFor { !has("Bay 4, windy") }
    }
}
