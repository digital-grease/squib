package dev.digitalgrease.squib

import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.lifecycle.ViewModelProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.digitalgrease.squib.conditions.ConditionsController
import dev.digitalgrease.squib.conditions.RefreshPolicy
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Plan phase 3.2 on an emulator (uses the network like ConditionsLiveTest): with weather on
 * and a place set, opening the Timer refreshes conditions by itself when a refresh is due,
 * and does not when the last one is recent. No tap and no dialog.
 */
@RunWith(AndroidJUnit4::class)
class ConditionsAutoRefreshTest {
    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    private fun tag(t: String) = rule.onAllNodes(hasTestTag(t)).fetchSemanticsNodes().isNotEmpty()

    @Test
    fun theTimerRefreshesConditionsWhenDueAndOnlyThen() {
        val c = ViewModelProvider(rule.activity)[ConditionsController::class.java]
        rule.onNodeWithTag("tab_practice").performClick()
        rule.runOnUiThread { c.setWeatherEnabled(true); c.setManualPlace(40.015, -105.2705, "Auto refresh test") }
        rule.waitOrDump(10_000) { c.view.value?.weatherEnabled == true && c.view.value?.place != null }
        val last = c.view.value!!.lastRefreshUtcMs
        val autoBefore = c.lastAutoAttemptUtcMs
        val recent = last != null && System.currentTimeMillis() - last < RefreshPolicy.INTERVAL_MS
        rule.onNodeWithTag("tab_timer").performClick()
        rule.waitOrDump { tag("mode_par") }
        if (recent) {
            Thread.sleep(3_000)
            rule.waitForIdle()
            check(c.lastAutoAttemptUtcMs == autoBefore) { "refreshed although the last refresh was recent" }
        } else {
            rule.waitOrDump(5_000) { c.lastAutoAttemptUtcMs != autoBefore }
            rule.waitOrDump(30_000) { c.view.value?.lastRefreshUtcMs?.let { it != last } == true }
            // Going away and back right after does not refresh again.
            val auto = c.lastAutoAttemptUtcMs
            rule.onNodeWithTag("tab_history").performClick()
            rule.onNodeWithTag("tab_timer").performClick()
            rule.waitOrDump { tag("mode_par") }
            Thread.sleep(2_000)
            rule.waitForIdle()
            check(c.lastAutoAttemptUtcMs == auto) { "refreshed again within 15 minutes" }
        }
        check(c.message.value == null) { "automatic refresh showed a dialog: ${c.message.value}" }
        android.util.Log.i("SquibTest", "auto refresh: recent=$recent")
        rule.runOnUiThread { c.setWeatherEnabled(false) }
    }
}
