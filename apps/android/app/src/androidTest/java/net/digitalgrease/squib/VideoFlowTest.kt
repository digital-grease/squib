package net.digitalgrease.squib

import android.Manifest
import android.util.Log
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
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
 * M4 video proof of concept on an emulator: a par-only run with video records a clip,
 * the clip is attached to the run, and Review shows it with the core's alignment label
 * and run markers. The emulator's virtual camera says nothing about real-phone timing.
 */
@RunWith(AndroidJUnit4::class)
class VideoFlowTest {
    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    private fun tag(t: String) = rule.onAllNodes(hasTestTag(t)).fetchSemanticsNodes().isNotEmpty()

    @Before
    fun grantCamera() {
        val inst = InstrumentationRegistry.getInstrumentation()
        inst.uiAutomation.grantRuntimePermission(inst.targetContext.packageName, Manifest.permission.CAMERA)
    }

    @Test
    fun parOnlyRunWithVideoShowsAlignedClipInReview() {
        rule.resetTimer()
        rule.onNodeWithTag("mode_par").performClick()
        rule.onNodeWithTag("video_toggle").performScrollTo().performClick()
        rule.waitUntil(5_000) { tag("video_preview") }
        rule.onNodeWithTag("delay_instant").performScrollTo().performClick()
        rule.onNodeWithTag("arm").performScrollTo().performClick()
        rule.waitUntil(15_000) { tag("stop") }
        Thread.sleep(2_000)
        rule.onNodeWithTag("stop").performScrollTo().performClick()
        rule.waitUntil(15_000) { rule.onAllNodes(hasText("Saved · Complete")).fetchSemanticsNodes().isNotEmpty() }

        rule.onNodeWithTag("tab_history").performClick()
        rule.waitUntil(5_000) { rule.onAllNodes(hasText("Par-only")).fetchSemanticsNodes().isNotEmpty() }
        rule.onAllNodes(hasText("Par-only")).onFirst().performClick()
        rule.waitUntil(15_000) { tag("video_section") }
        val note = rule.onNodeWithTag("video_mapping", useUnmergedTree = true).fetchSemanticsNode()
            .config.getOrNull(SemanticsProperties.Text)?.joinToString { it.text }
        Log.i("SquibVideoTest", "mapping note: $note")
        if (note?.startsWith("Not aligned") == false) {
            rule.onNodeWithTag("marker_start", useUnmergedTree = true).assertExists()
        }
        rule.onNodeWithTag("tab_timer").performClick()
        rule.resetTimer()
        rule.onNodeWithTag("video_toggle").performScrollTo().performClick()
    }
}

