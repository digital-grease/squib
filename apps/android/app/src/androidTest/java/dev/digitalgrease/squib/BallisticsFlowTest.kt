package dev.digitalgrease.squib

import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.test.performTextClearance
import androidx.compose.ui.test.performTextInput
import androidx.lifecycle.ViewModelProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.digitalgrease.squib.conditions.ConditionsController
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Plan 6.4 on an emulator: add a load, enter the air, calculate a range card, check it
 * against the validated reference (py-ballisticcalc case g7_308_175_standard: 10.97 m
 * below the line of sight at 1000 yd, 12.0 mil), save, reopen, edit the load into a new
 * version, and remove everything again. Plan 6.5: the optional effects are off until
 * chosen, spin drift needs the bullet's size, and both show on the card (reference
 * py-ballisticcalc case spin_308_175_rh10: 0.332 m of spin drift at 1000 yd, stability 2.40).
 */
@RunWith(AndroidJUnit4::class)
class BallisticsFlowTest {
    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    private fun tag(t: String) = rule.onAllNodes(hasTestTag(t)).fetchSemanticsNodes().isNotEmpty()
    private fun has(t: String) = rule.onAllNodes(hasText(t, substring = true)).fetchSemanticsNodes().isNotEmpty()

    private fun spokenRow(prefix: String) = SemanticsMatcher("spoken row $prefix") { n ->
        n.config.getOrNull(SemanticsProperties.ContentDescription)?.any { it.startsWith(prefix) } == true
    }

    private fun type(tag: String, text: String) {
        rule.onNodeWithTag(tag).performScrollTo().performTextClearance()
        rule.onNodeWithTag(tag).performTextInput(text)
    }

    private fun scrollTo(screen: String, t: String) {
        rule.onNodeWithTag(screen).performScrollToNode(hasTestTag(t))
    }

    @Test
    fun loadCardSaveAndVersions() {
        rule.runOnUiThread { ViewModelProvider(rule.activity)[ConditionsController::class.java].setUsUnits(true) }
        val name = "Flow 175 SMK ${System.currentTimeMillis() % 100000}"
        rule.onNodeWithTag("tab_conditions").performClick()
        rule.waitOrDump { tag("conditions") }
        rule.onNodeWithTag("open_ballistics").performScrollTo().performClick()
        rule.waitOrDump { tag("ballistics") }
        check(has("Experimental") && has("confirm every distance on paper")) { "labelled Experimental with its limits" }

        // A load in US units.
        rule.onNodeWithTag("load_add").performScrollTo().performClick()
        rule.waitOrDump { tag("load_editor") }
        type("load_name", name)
        rule.onNodeWithTag("model_g7").performScrollTo().performClick()
        type("load_bc", "0.243")
        type("load_weight", "175")
        type("load_mv", "2600")
        type("load_sight", "1.5")
        type("load_zero", "100")
        type("load_mv", "26000")
        rule.onNodeWithTag("load_save").performScrollTo().performClick()
        rule.waitOrDump { has("muzzle velocity must be") }
        type("load_mv", "2600")
        rule.onNodeWithTag("load_save").performScrollTo().performClick()
        rule.waitOrDump { tag("ballistics") && has(name) }
        check(has("(version 1)"))

        // Standard air entered by hand, no wind, level, 100 to 1000 yd.
        rule.onNodeWithTag("air_manual").performScrollTo().performClick()
        type("air_pressure", "29.92")
        type("air_temperature", "59")
        type("air_humidity", "0")
        type("wind_speed", "0")
        type("shot_angle", "0")
        type("range_from", "100")
        type("range_to", "1000")
        type("range_step", "100")
        if (!has("Corrections in mil")) error("unit toggle missing")
        rule.onNodeWithTag("ballistics_solve").performScrollTo().performClick()
        rule.waitOrDump(20_000) { tag("range_card") }
        scrollTo("ballistics", "range_card")
        rule.waitOrDump { rule.onAllNodes(spokenRow("1000 yd: elevation 12.0 mil up")).fetchSemanticsNodes().isNotEmpty() }
        check(rule.onAllNodes(spokenRow("100 yd: elevation 0 mil")).fetchSemanticsNodes().isNotEmpty()) { "zeroed at 100 yd" }
        check(rule.onAllNodes(spokenRow("1000 yd")).fetchSemanticsNodes().single().config[SemanticsProperties.ContentDescription].single()
            .contains("less reliable")) { "the transonic 1000 yd row is flagged" }
        check(has("Method: squib-pointmass-v1")) { "method shown" }

        // Save, then reopen from the saved list.
        type("card_title", "Flow card")
        rule.onNodeWithTag("card_save").performScrollTo().performClick()
        rule.waitOrDump { has("Range card saved") }
        check(has("Range card: Flow card"))
        rule.onNodeWithTag("ballistics").performScrollToNode(hasText("Flow card", substring = true))

        // Editing the load makes version 2; the saved card keeps version 1.
        rule.onNodeWithTag("load_edit").performScrollTo().performClick()
        rule.waitOrDump { tag("load_editor") && has("Saving makes version 2") }
        type("load_mv", "2650")
        rule.onNodeWithTag("load_save").performScrollTo().performClick()
        rule.waitOrDump { tag("ballistics") && has("(version 2)") }
        rule.onNodeWithTag("ballistics").performScrollToNode(hasTestTag("card_open"))
        rule.onAllNodes(hasTestTag("card_open")).fetchSemanticsNodes().size.let { check(it >= 1) }
        rule.onAllNodes(hasText("Flow card", substring = true) and hasTestTag("card_open").let { it })[0].performClick()
        rule.waitOrDump { has("$name (version 1)") }

        // Clean up: delete the card, remove the load.
        rule.onNodeWithTag("ballistics").performScrollToNode(hasText("Delete"))
        rule.onAllNodes(SemanticsMatcher("delete Flow card") { n ->
            n.config.getOrNull(SemanticsProperties.ContentDescription)?.any { it == "Delete range card Flow card" } == true
        })[0].performClick()
        rule.waitOrDump { !has("Range card: Flow card") }
        rule.onNodeWithTag("load_edit").performScrollTo().performClick()
        rule.waitOrDump { tag("load_archive") || tag("load_editor") }
        rule.onNodeWithTag("load_archive").performScrollTo().performClick()
        rule.waitOrDump { tag("ballistics") && !has(name) }
        rule.runOnUiThread { rule.activity.onBackPressedDispatcher.onBackPressed() }
        rule.waitOrDump { tag("conditions") }
    }

    @Test
    fun optionalEffects() {
        rule.runOnUiThread { ViewModelProvider(rule.activity)[ConditionsController::class.java].setUsUnits(true) }
        val name = "Flow spin ${System.currentTimeMillis() % 100000}"
        rule.onNodeWithTag("tab_conditions").performClick()
        rule.waitOrDump { tag("conditions") }
        rule.onNodeWithTag("open_ballistics").performScrollTo().performClick()
        rule.waitOrDump { tag("ballistics") }

        rule.onNodeWithTag("load_add").performScrollTo().performClick()
        rule.waitOrDump { tag("load_editor") }
        type("load_name", name)
        rule.onNodeWithTag("model_g7").performScrollTo().performClick()
        type("load_bc", "0.243")
        type("load_weight", "175")
        type("load_mv", "2600")
        type("load_sight", "1.5")
        type("load_zero", "100")
        type("load_twist", "10")
        rule.onNodeWithTag("load_save").performScrollTo().performClick()
        rule.waitOrDump { tag("ballistics") && has(name) }

        // Without the bullet's size spin drift cannot be chosen, and says why.
        scrollTo("ballistics", "spin_drift")
        rule.onNodeWithTag("spin_drift").assertIsNotEnabled()
        check(has("Needs the load's twist, bullet diameter and bullet length")) { "spin drift says what it needs" }
        rule.onNodeWithTag("load_edit").performScrollTo().performClick()
        rule.waitOrDump { tag("load_editor") }
        type("load_diameter", "0.308")
        type("load_length", "1.240")
        rule.onNodeWithTag("load_save").performScrollTo().performClick()
        rule.waitOrDump { tag("ballistics") && has("(version 2)") }

        rule.onNodeWithTag("air_manual").performScrollTo().performClick()
        type("air_pressure", "29.92")
        type("air_temperature", "59")
        type("air_humidity", "0")
        type("wind_speed", "0")
        type("shot_angle", "0")
        type("range_from", "100")
        type("range_to", "1000")
        type("range_step", "100")
        scrollTo("ballistics", "spin_drift")
        rule.onNodeWithTag("spin_drift").assertIsEnabled().performClick()
        rule.onNodeWithTag("coriolis").performScrollTo().performClick()
        rule.waitOrDump { tag("coriolis_latitude") }
        type("coriolis_latitude", "45")
        rule.onNodeWithTag("ballistics_solve").performScrollTo().performClick()
        rule.waitOrDump { has("Enter the shot direction") }
        type("coriolis_bearing", "90")
        rule.onNodeWithTag("ballistics_solve").performScrollTo().performClick()
        rule.waitOrDump(20_000) { tag("range_card") }
        scrollTo("ballistics", "range_card")
        rule.waitOrDump { has("Spin drift included (stability 2.40)") && has("latitude 45.0°, shot direction 90°") }
        val row = rule.onAllNodes(spokenRow("1000 yd")).fetchSemanticsNodes().single().config[SemanticsProperties.ContentDescription].single()
        check(row.contains("of which spin drift 13.1 in right")) { row }
        check(row.contains("left")) { "spin drift and Earth's rotation to the right: hold left. $row" }
        check(has("Litz") && has("Coriolis")) { "method names both terms" }

        rule.onNodeWithTag("load_edit").performScrollTo().performClick()
        rule.waitOrDump { tag("load_archive") }
        rule.onNodeWithTag("load_archive").performScrollTo().performClick()
        rule.waitOrDump { tag("ballistics") && !has(name) }
    }
}
