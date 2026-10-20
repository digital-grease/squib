package dev.digitalgrease.squib

import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Plan phase 2.6 on an emulator: the walkthrough countdown keeps running off its screen. */
@RunWith(AndroidJUnit4::class)
class WalkthroughFlowTest {
    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    private fun tag(t: String) = rule.onAllNodes(hasTestTag(t)).fetchSemanticsNodes().isNotEmpty()

    private fun bannerText(): String = rule.onNodeWithTag("walk_banner", useUnmergedTree = true).fetchSemanticsNode().let { n ->
        fun texts(x: androidx.compose.ui.semantics.SemanticsNode): List<String> =
            (x.config.getOrNull(SemanticsProperties.Text)?.map { it.text } ?: emptyList()) + x.children.flatMap { texts(it) }
        texts(n).joinToString(" ")
    }

    @Test
    fun countdownSurvivesLeavingThePlanAndCanBeStoppedFromAnywhere() {
        rule.onNodeWithTag("tab_practice").performClick()
        rule.waitOrDump { tag("new_plan") }
        rule.onNodeWithTag("practice").performScrollToNode(hasTestTag("new_plan"))
        rule.onNodeWithTag("new_plan").performClick()
        rule.onNodeWithTag("plan_title").performTextInput("Walk test")
        rule.onNodeWithTag("plan_create").performSemanticsAction(SemanticsActions.OnClick)
        rule.waitOrDump { tag("plan_screen") }
        rule.onNodeWithTag("walk_minutes").performScrollTo().performTextReplacement("1")
        rule.onNodeWithTag("walk_start").performScrollTo().performClick()
        rule.waitOrDump { tag("walk_left") }
        check(!tag("walk_banner")) { "no banner on the countdown's own plan" }

        // Leave the plan: the banner counts on.
        rule.onNodeWithTag("tab_timer").performClick()
        rule.waitOrDump { tag("walk_banner") }
        val first = bannerText()
        rule.waitOrDump(5_000) { bannerText() != first }
        check(bannerText().contains("left · Walk test")) { bannerText() }

        // Back to the plan through the banner: still running, not reset to the start form.
        rule.onNode(hasContentDescription("Open Walk test")).performClick()
        rule.waitOrDump { tag("walk_left") && !tag("walk_banner") }
        check(!tag("walk_minutes")) { "countdown was reset" }

        // Stop from another screen.
        rule.onNodeWithTag("tab_history").performClick()
        rule.waitOrDump { tag("walk_banner") }
        rule.onNodeWithTag("walk_banner_stop").performClick()
        rule.waitOrDump { !tag("walk_banner") }
        // The Practice tab returns to the open plan; after Stop it offers a new countdown.
        rule.onNodeWithTag("tab_practice").performClick()
        rule.waitOrDump { tag("plan_screen") }
        rule.waitOrDump { tag("walk_minutes") }
    }
}
