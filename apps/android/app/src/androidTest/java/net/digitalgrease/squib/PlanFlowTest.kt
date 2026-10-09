package net.digitalgrease.squib

import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** M4 on an emulator: day plan agenda, a planned drill run counted from the timer, checklist, coach bar. */
@RunWith(AndroidJUnit4::class)
class PlanFlowTest {
    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    private fun has(text: String) = rule.onAllNodes(hasText(text, substring = true)).fetchSemanticsNodes().isNotEmpty()
    private fun tag(t: String) = rule.onAllNodes(hasTestTag(t)).fetchSemanticsNodes().isNotEmpty()

    @Test
    fun planAgendaDrillRunAndChecklist() {
        rule.resetTimer()
        val title = "Plan ${System.currentTimeMillis() % 100000}"
        rule.onNodeWithTag("tab_practice").performClick()
        rule.waitUntil(5_000) { tag("new_plan") }
        rule.onNodeWithTag("new_plan").performClick()
        rule.onNodeWithTag("plan_title").performTextInput(title)
        rule.onNodeWithTag("plan_create").performClick()
        rule.waitUntil(10_000) { tag("plan_screen") }
        // A new plan gets the default checklist.
        rule.waitUntil(5_000) { has("Eye protection") }

        // Stage item with notes and a time.
        rule.onNodeWithTag("add_item").performScrollTo().performClick()
        rule.onNodeWithTag("item_kind_stage").performClick()
        rule.onNodeWithTag("item_title").performTextInput("Stage 1")
        rule.onNodeWithTag("item_time").performTextInput("23:59")
        rule.onNodeWithTag("item_save").performScrollTo().performClick()
        rule.waitUntil(5_000) { has("Stage · Stage 1") }
        rule.waitUntil(5_000) { tag("next_up") }

        // Drill item: Single par, two strings.
        rule.onNodeWithTag("add_item").performScrollTo().performClick()
        rule.onNodeWithTag("pick_starter-1").performClick()
        rule.onNodeWithTag("item_save").performScrollTo().performClick()
        rule.waitUntil(5_000) { has("0 of 1 strings done") }

        // Skip only marks the item.
        rule.onNodeWithTag("skip_Stage 1").performScrollTo().performClick()
        rule.waitUntil(5_000) { has("Skipped") }

        // Check a checklist item.
        rule.onNodeWithTag("check_Eye protection").performScrollTo().performClick()

        // Start the drill from the plan: the timer shows the plan and shooter.
        rule.onNodeWithTag("run_item_Single par").performScrollTo().performClick()
        rule.waitUntil(5_000) { tag("plan_banner") }
        rule.onNodeWithTag("shooter_banner").assertExists()
        rule.onNodeWithTag("delay_instant").performScrollTo().performClick()
        rule.onNodeWithTag("arm").performScrollTo().performClick()
        rule.waitUntil(15_000) { tag("stop") }
        rule.onNodeWithTag("stop").performClick()
        rule.waitUntil(15_000) { tag("new_setup") }

        // Back on the plan, the completed run counts toward the item.
        rule.onNodeWithTag("tab_practice").performClick()
        rule.waitUntil(10_000) { has("1 of 1 strings done") }
        rule.waitUntil(5_000) { has("Done") }
        rule.onNodeWithTag("tab_timer").performClick()
        rule.resetTimer()
    }
}
