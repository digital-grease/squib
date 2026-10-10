package net.digitalgrease.squib

import androidx.compose.ui.semantics.SemanticsProperties
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
 * Live integration checks against api.weather.gov and aviationweather.gov through the
 * real Android HTTP path.
 * Requires network; not part of CI. Asserts provenance behaviour, not weather values.
 */
@RunWith(AndroidJUnit4::class)
class ConditionsLiveTest {
    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    private fun has(text: String) = rule.onAllNodes(hasText(text, substring = true)).fetchSemanticsNodes().isNotEmpty()

    @Test
    fun liveNwsRefreshShowsNearbyObservationAndNoLocalPressure() {
        rule.onNodeWithTag("tab_conditions").performClick()
        rule.waitUntil(5_000) { rule.onAllNodes(hasTestTag("weather_switch")).fetchSemanticsNodes().isNotEmpty() }
        // Tests share the app's state: always set this test's own place and source.
        rule.onNodeWithTag("provider_auto").performScrollTo().performClick()
        rule.onNodeWithTag("lat").performScrollTo().performTextReplacement("40.015")
        rule.onNodeWithTag("lon").performScrollTo().performTextReplacement("-105.2705")
        rule.onNodeWithTag("set_place").performScrollTo().performClick()
        rule.waitUntil(5_000) { has("40.02, -105.27") }
        // Enable lookup if it is off (read the switch itself), then refresh.
        val lookupOn = {
            rule.onNodeWithTag("weather_switch").fetchSemanticsNode().config
                .getOrElseNullable(SemanticsProperties.ToggleableState) { null } == androidx.compose.ui.state.ToggleableState.On
        }
        if (!lookupOn()) rule.onNodeWithTag("weather_switch").performScrollTo().performClick()
        rule.waitUntil(5_000) { lookupOn() }
        rule.waitUntil(5_000) {
            !rule.onNodeWithTag("refresh").fetchSemanticsNode().config.contains(SemanticsProperties.Disabled)
        }
        rule.onNodeWithTag("refresh").performScrollTo().performClick()
        rule.waitUntil(30_000) { has("Nearby observation") }
        // Pressure here must stay unavailable: NWS has no local actual pressure.
        rule.waitUntil(2_000) { has("Needs a phone barometer") || has("Unavailable") }
        rule.waitUntil(2_000) { has("National Weather Service") }
        rule.waitUntil(2_000) { has("min old") || has("just now") || has("h old") }
    }

    @Test
    fun liveMetarOutsideUsFallsBackToAirportReports() {
        rule.onNodeWithTag("tab_conditions").performClick()
        rule.waitUntil(5_000) { rule.onAllNodes(hasTestTag("weather_switch")).fetchSemanticsNodes().isNotEmpty() }
        rule.onNodeWithTag("provider_auto").performScrollTo().performClick()
        rule.onNodeWithTag("lat").performScrollTo().performTextReplacement("51.5007")
        rule.onNodeWithTag("lon").performScrollTo().performTextReplacement("-0.1246")
        rule.onNodeWithTag("set_place").performScrollTo().performClick()
        rule.waitUntil(5_000) { has("51.50, -0.12") }
        val lookupOn = {
            rule.onNodeWithTag("weather_switch").fetchSemanticsNode().config
                .getOrElseNullable(SemanticsProperties.ToggleableState) { null } == androidx.compose.ui.state.ToggleableState.On
        }
        if (!lookupOn()) rule.onNodeWithTag("weather_switch").performScrollTo().performClick()
        rule.waitUntil(5_000) { lookupOn() }
        rule.waitUntil(5_000) {
            !rule.onNodeWithTag("refresh").fetchSemanticsNode().config.contains(SemanticsProperties.Disabled)
        }
        rule.onNodeWithTag("refresh").performScrollTo().performClick()
        rule.waitUntil(30_000) { has("Nearby observation") }
        rule.waitUntil(2_000) { has("aviationweather.gov") }
        // NWS declined this place, but that is not shown as a problem: METAR filled in.
        check(!has("no data for this place")) { "fallback issue should not be shown" }
    }
}
