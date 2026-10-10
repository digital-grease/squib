package net.digitalgrease.squib

import android.Manifest
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onFirst
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * M5 experiment 3 on an emulator: an opt-in diagnostic recording of one live run is
 * kept, listed in Review, export is gated on consent, and deletion keeps the run.
 *
 * Grants the microphone, which `ParOnlyFlowTest` must not have: the runner orders test
 * classes by name, so this class is named to run after the par-only tests.
 */
@RunWith(AndroidJUnit4::class)
class RecordingDiagnosticFlowTest {
    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    private fun tag(t: String) = rule.onAllNodes(hasTestTag(t)).fetchSemanticsNodes().isNotEmpty()

    @Before
    fun grantMic() {
        val inst = InstrumentationRegistry.getInstrumentation()
        inst.uiAutomation.grantRuntimePermission(inst.targetContext.packageName, Manifest.permission.RECORD_AUDIO)
    }

    @Test
    fun oneLiveRunIsRecordedThenReviewedGatedAndDeleted() {
        rule.resetTimer()
        rule.onNodeWithTag("mode_live").performClick()
        rule.waitUntil(5_000) { tag("diagnostic_toggle") }
        rule.onNodeWithTag("diagnostic_toggle").performScrollTo().performClick()
        rule.onNodeWithTag("delay_instant").performScrollTo().performClick()
        rule.waitUntil(10_000) {
            rule.onAllNodes(hasTestTag("arm")).fetchSemanticsNodes().firstOrNull()
                ?.config?.contains(SemanticsProperties.Disabled) == false
        }
        rule.onNodeWithTag("arm").performScrollTo().performClick()
        // The emulator cannot hear its own start beep: the run ends Interrupted, or we stop it.
        rule.waitUntil(30_000) { tag("new_setup") || tag("stop") }
        if (tag("stop")) {
            Thread.sleep(1_500)
            rule.onNodeWithTag("stop").performScrollTo().performClick()
        }
        rule.waitUntil(30_000) { tag("new_setup") }
        rule.onNodeWithTag("new_setup").performScrollTo().performClick()
        // The switch turned itself off after one run.
        rule.waitUntil(5_000) { tag("diagnostic_toggle") }
        val on = rule.onNodeWithTag("diagnostic_toggle").fetchSemanticsNode().config
            .getOrElseNullable(SemanticsProperties.ToggleableState) { null } == androidx.compose.ui.state.ToggleableState.On
        check(!on) { "diagnostic recording must reset after one run" }

        rule.onNodeWithTag("tab_history").performClick()
        rule.waitUntil(5_000) { rule.onAllNodes(hasText("Live (experimental)")).fetchSemanticsNodes().isNotEmpty() }
        rule.onAllNodes(hasText("Live (experimental)")).onFirst().performClick()
        rule.waitUntil(15_000) { tag("diagnostic_section") }
        rule.onNodeWithTag("diag_play", useUnmergedTree = true).assertExists()

        // Export: the save button stays disabled until the consent statement is ticked.
        rule.onNodeWithTag("diag_export").performScrollTo().performClick()
        rule.waitUntil(5_000) { tag("diag_save") }
        check(rule.onNodeWithTag("diag_save").fetchSemanticsNode().config.contains(SemanticsProperties.Disabled))
        rule.onNodeWithTag("diag_consent").performClick()
        rule.waitUntil(2_000) { !rule.onNodeWithTag("diag_save").fetchSemanticsNode().config.contains(SemanticsProperties.Disabled) }
        rule.onAllNodes(hasText("Cancel")).onFirst().performClick()

        // Deleting the recording keeps the run in Review.
        rule.onNodeWithTag("diag_delete").performScrollTo().performClick()
        rule.onNodeWithTag("diag_delete_confirm").performClick()
        rule.waitUntil(5_000) { !tag("diagnostic_section") }
        rule.onNodeWithTag("review_screen").assertExists()
        rule.onNodeWithTag("tab_timer").performClick()
        rule.resetTimer()
        rule.onNodeWithTag("mode_par").performClick()
    }
}
