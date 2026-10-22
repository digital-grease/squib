package dev.digitalgrease.squib

import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.digitalgrease.squib.core.primers
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Plan phase 8 on an emulator: the primers open from About, every primer shows its own
 * text, related primers and the list open inside the same dialog and Back returns through
 * them, and the converter works the plan's example (4.1 mil at 600 yd is 41 clicks).
 */
@RunWith(AndroidJUnit4::class)
class PrimerFlowTest {
    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    private fun tag(t: String) = rule.onAllNodes(hasTestTag(t)).fetchSemanticsNodes().isNotEmpty()
    private fun has(t: String) = rule.onAllNodes(hasText(t, substring = true)).fetchSemanticsNodes().isNotEmpty()

    @Test
    fun primersOpenFromAboutWithTheConverter() {
        rule.onNodeWithTag("tab_history").performClick()
        rule.waitOrDump { tag("history") }
        rule.onNodeWithTag("history").performScrollToNode(hasTestTag("open_data"))
        rule.onNodeWithTag("open_data").performClick()
        rule.waitOrDump { tag("open_about") }
        rule.onNodeWithTag("open_about").performScrollTo().performClick()
        rule.waitOrDump { tag("about") }
        rule.onNodeWithTag("about_primers").performScrollTo().performClick()
        rule.waitOrDump { tag("primer_dialog") && has("What the terms mean") }

        // Every primer opens with its own title and text, and Back returns to the list.
        val all = primers()
        check(all.size == 11) { "the confirmed scope has 11 primers" }
        for (p in all) {
            rule.onNodeWithTag("primer_open_${p.id}").performScrollTo().performClick()
            rule.waitOrDump { tag("primer_title") && has(p.paragraphs.first().take(40)) }
            check(has("What Squib does with this")) { "${p.id}: what Squib does" }
            systemBack()
            rule.waitOrDump { tag("primer_open_${p.id}") }
        }

        // The converter, in the app's units (US customary by default).
        rule.onNodeWithTag("primer_open_angle-units").performScrollTo().performClick()
        rule.waitOrDump { tag("angle_converter") }
        rule.onNodeWithTag("converter_value").performScrollTo().performTextReplacement("4.1")
        rule.onNodeWithTag("converter_distance").performTextReplacement("600")
        rule.onNodeWithTag("converter_unit_mil").performClick()
        rule.waitOrDump { has("4.1 mil at 600 yd covers 88.6 in.") && has("41 clicks of 0.1 mil, or 56 clicks of 1/4 MOA") }
        rule.onNodeWithTag("converter_value").performTextReplacement("-1")
        rule.waitOrDump { has("Enter an angle of 0 or more.") }

        // A related primer opens inside the dialog; Back returns to the one before, then the list, then closes.
        rule.onNodeWithTag("primer_related_shooters-moa").performScrollTo().performClick()
        rule.waitOrDump { has("Shooter's MOA (IPHY)") && has("difference of 14 in") }
        systemBack()
        rule.waitOrDump { tag("angle_converter") }
        systemBack()
        rule.waitOrDump { tag("primer_open_angle-units") }
        systemBack()
        rule.waitOrDump { !tag("primer_dialog") && tag("about") }
        rule.onNodeWithTag("tab_timer").performClick()
    }
}
