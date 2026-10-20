package dev.digitalgrease.squib

import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.state.ToggleableState
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
 * Plan phase 3.3, live: terrain elevation from epqs.nationalmap.gov through the real
 * Android HTTP path, with weather lookup off. The service can take 15 to 30 s.
 * Requires network; asserts provenance, not the elevation value.
 */
@RunWith(AndroidJUnit4::class)
class TerrainLiveTest {
    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    private fun has(text: String) = rule.onAllNodes(hasText(text, substring = true)).fetchSemanticsNodes().isNotEmpty()
    private fun tag(t: String) = rule.onAllNodes(hasTestTag(t)).fetchSemanticsNodes().isNotEmpty()

    private fun switchOn(t: String) = rule.onNodeWithTag(t).fetchSemanticsNode().config
        .getOrElseNullable(SemanticsProperties.ToggleableState) { null } == ToggleableState.On

    private fun set(t: String, on: Boolean) {
        if (switchOn(t) != on) rule.onNodeWithTag(t).performScrollTo().performClick()
        rule.waitOrDump { switchOn(t) == on }
    }

    @Test
    fun terrainOnlyLookupShowsGroundHeightWithItsDatum() {
        rule.onNodeWithTag("tab_conditions").performClick()
        rule.waitOrDump { tag("terrain_switch") }
        set("weather_switch", false)
        set("terrain_switch", true)
        rule.onNodeWithTag("lat").performScrollTo().performTextReplacement("40.015")
        rule.onNodeWithTag("lon").performScrollTo().performTextReplacement("-105.2705")
        rule.onNodeWithTag("set_place").performScrollTo().performClick()
        rule.waitOrDump { has("40.02, -105.27") && !tag("terrain_needs_place") }
        rule.waitOrDump { !rule.onNodeWithTag("refresh").fetchSemanticsNode().config.contains(SemanticsProperties.Disabled) }
        rule.onNodeWithTag("refresh").performScrollTo().performClick()
        rule.waitOrDump(100_000) { tag("field_elevation_terrain") }
        rule.onNodeWithTag("field_elevation_terrain").performScrollTo().performClick()
        rule.waitOrDump { has("NAVD88") && has("Dataset: 3DEP raster") && has("Terrain data") }
        rule.waitOrDump { has("U.S. Geological Survey") }

        // Off: the value and the 10 m point are gone.
        set("terrain_switch", false)
        rule.waitOrDump { !tag("field_elevation_terrain") }
    }
}
