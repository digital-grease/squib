package dev.digitalgrease.squib

import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.lifecycle.ViewModelProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.digitalgrease.squib.conditions.ConditionsController
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Plan phase 3.5 on an emulator: the heat card appears only when hot, with the NWS method. */
@RunWith(AndroidJUnit4::class)
class HeatCardFlowTest {
    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    private fun has(t: String) = rule.onAllNodes(hasText(t, substring = true)).fetchSemanticsNodes().isNotEmpty()
    private fun tag(t: String) = rule.onAllNodes(hasTestTag(t)).fetchSemanticsNodes().isNotEmpty()

    @Test
    fun heatCardShowsTheNwsCategoryWhenHotAndNotOtherwise() {
        val c = ViewModelProvider(rule.activity)[ConditionsController::class.java]
        rule.onNodeWithTag("tab_conditions").performClick()
        rule.waitOrDump { tag("conditions") }
        rule.runOnUiThread {
            c.setOverride("temperature", 288.15, 15.0, "degC")
            c.setOverride("relative_humidity", 0.5, 50.0, "%")
        }
        rule.waitOrDump { has("Air density (calculated)") }
        check(!tag("heat")) { "no heat card at 15 °C" }
        rule.runOnUiThread { c.setOverride("temperature", 308.15, 35.0, "degC") }
        rule.waitOrDump { tag("heat") }
        rule.onNodeWithTag("heat").performScrollTo()
        rule.waitOrDump { has("Danger") && has("Rothfusz") && has("Full sunshine can add up to 15 °F") }
        rule.runOnUiThread { listOf("temperature", "relative_humidity").forEach { c.clearOverride(it) } }
        rule.waitOrDump { !tag("heat") }
    }
}
