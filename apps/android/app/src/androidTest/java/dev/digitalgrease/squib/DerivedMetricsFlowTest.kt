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

/** Plan phase 3.4 on an emulator: the density card needs actual pressure and names its method. */
@RunWith(AndroidJUnit4::class)
class DerivedMetricsFlowTest {
    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    private fun has(t: String) = rule.onAllNodes(hasText(t, substring = true)).fetchSemanticsNodes().isNotEmpty()
    private fun tag(t: String) = rule.onAllNodes(hasTestTag(t)).fetchSemanticsNodes().isNotEmpty()

    @Test
    fun densityAltitudeAppearsOnlyWithActualPressure() {
        val c = ViewModelProvider(rule.activity)[ConditionsController::class.java]
        rule.onNodeWithTag("tab_conditions").performClick()
        rule.waitOrDump { tag("conditions") }
        rule.runOnUiThread {
            listOf("local_pressure", "temperature", "relative_humidity").forEach { c.clearOverride(it) }
            c.setOverride("temperature", 303.15, 30.0, "degC")
            c.setOverride("relative_humidity", 0.2, 20.0, "%")
        }
        rule.waitOrDump { tag("derived_unavailable") && has("Needs pressure here (actual)") }
        rule.runOnUiThread { c.setOverride("local_pressure", 83_500.0, 835.0, "hPa") }
        rule.waitOrDump { has("Density altitude") && has("Pressure altitude") }
        rule.onNodeWithTag("derived").performScrollTo()
        check(has("moist-ideal-ae96-isa-v1") && has("it is not a height")) { "method not named" }
        rule.runOnUiThread { listOf("local_pressure", "temperature", "relative_humidity").forEach { c.clearOverride(it) } }
        rule.waitOrDump { tag("derived_unavailable") }
    }
}
