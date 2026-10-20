package dev.digitalgrease.squib

import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.performTextInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Plan phase 1.2 on an emulator: add equipment, select it on the Timer, run, see it on the run. */
@RunWith(AndroidJUnit4::class)
class EquipmentFlowTest {
    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    private fun has(t: String) = rule.onAllNodes(hasText(t, substring = true)).fetchSemanticsNodes().isNotEmpty()
    private fun tag(t: String) = rule.onAllNodes(hasTestTag(t)).fetchSemanticsNodes().isNotEmpty()

    @Test
    fun equipmentIsSelectedOnTimerAndShownOnTheRun() {
        val name = "Test pistol ${System.currentTimeMillis() % 10000}"
        rule.resetTimer()
        rule.onNodeWithTag("tab_practice").performClick()
        rule.waitUntil(5_000) { tag("equipment_add") }
        rule.onNodeWithTag("equipment_add").performScrollTo().performClick()
        rule.onNodeWithTag("equipment_name").performScrollTo().performTextInput(name)
        rule.waitForIdle()
        rule.onNodeWithTag("equipment_save").performSemanticsAction(SemanticsActions.OnClick)
        rule.waitUntil(5_000) { has("$name · Pistol") }

        rule.onNodeWithTag("tab_timer").performClick()
        rule.waitUntil(5_000) { tag("equipment_switch") }
        rule.onNodeWithTag("equipment_switch").performClick()
        rule.onNodeWithTag("pick_equipment_$name").performClick()
        rule.waitUntil(5_000) { has("Equipment: $name (Pistol)") }
        rule.onNodeWithTag("equipment_switch").performClick()

        rule.onNodeWithTag("mode_par").performClick()
        rule.onNodeWithTag("delay_instant").performScrollTo().performClick()
        rule.onNodeWithTag("arm").performScrollTo().performClick()
        rule.waitUntil(15_000) { tag("stop") }
        // Locked while running.
        check(!rule.onNodeWithTag("equipment_switch").fetchSemanticsNode().config.contains(SemanticsActions.OnClick) ||
            rule.onNodeWithTag("equipment_switch").fetchSemanticsNode().config.contains(androidx.compose.ui.semantics.SemanticsProperties.Disabled))
        rule.onNodeWithTag("stop").performScrollTo().performClick()
        rule.waitUntil(15_000) { tag("new_setup") }

        rule.onNodeWithTag("tab_history").performClick()
        rule.waitUntil(10_000) { has("Equipment: $name (pistol)") }

        // Leave no active equipment for later tests.
        rule.onNodeWithTag("tab_practice").performClick()
        rule.waitUntil(5_000) { has("$name · Pistol") }
        rule.onNodeWithTag("equipment_archive_$name").performScrollTo().performClick()
        rule.waitUntil(5_000) { !has("$name · Pistol") }
        rule.onNodeWithTag("tab_timer").performClick()
        rule.resetTimer()
    }
}
