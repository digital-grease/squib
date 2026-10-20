package dev.digitalgrease.squib

import android.content.ClipboardManager
import android.content.Context
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performScrollToNode
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Plan phase 4.3 on an emulator: choose placement, environment and verdict, preview the
 * exact report, copy it. The GitHub form is not opened here (it would leave the app).
 */
@RunWith(AndroidJUnit4::class)
class CompatReportFlowTest {
    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    private fun tag(t: String) = rule.onAllNodes(hasTestTag(t)).fetchSemanticsNodes().isNotEmpty()
    private fun has(t: String) = rule.onAllNodes(hasText(t, substring = true)).fetchSemanticsNodes().isNotEmpty()

    @Test
    fun reportIsPreviewedThenCopiedWithoutIdentifiers() {
        rule.onNodeWithTag("tab_history").performClick()
        rule.waitOrDump { tag("history") }
        rule.onNodeWithTag("history").performScrollToNode(hasTestTag("open_data"))
        rule.onNodeWithTag("open_data").performClick()
        rule.waitOrDump { tag("compat") }
        rule.onNodeWithTag("compat_preview").performScrollTo()
        check(!tag("compat_text")) { "no report before the choices" }
        for (t in listOf("compat_place_0", "compat_env_1", "compat_verdict_1")) rule.onNodeWithTag(t).performScrollTo().performClick()
        rule.onNodeWithTag("compat_preview").performScrollTo().performClick()
        rule.waitOrDump { tag("compat_text") }
        rule.onNodeWithTag("compat_text").performScrollTo()
        check(has("Squib compatibility report") && has("Placement: On a bench or table") && has("Environment: Covered bay")) { "report fields" }
        check(has("Verdict (user): Works with corrections") && has("Date: ")) { "verdict and date" }
        check(has("Contains no audio, location, notes, run identifiers")) { "privacy line" }
        rule.onNodeWithTag("compat_copy").performScrollTo().performClick()
        var clip = ""
        rule.runOnUiThread {
            val cm = rule.activity.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
            clip = cm.primaryClip?.getItemAt(0)?.text?.toString().orEmpty()
        }
        check(clip.startsWith("Squib compatibility report") && clip.contains("Environment: Covered bay")) { "copied: $clip" }
    }
}
