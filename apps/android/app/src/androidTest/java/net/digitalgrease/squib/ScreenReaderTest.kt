package net.digitalgrease.squib

import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isRoot
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipeUp
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * A21 without TalkBack (the emulator image has none): walks the merged semantics tree that
 * screen readers read, in order, and checks what they would announce. The transcript is
 * logged under SquibReader.
 */
@RunWith(AndroidJUnit4::class)
class ScreenReaderTest {
    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    private fun tag(t: String) = rule.onAllNodes(hasTestTag(t)).fetchSemanticsNodes().isNotEmpty()
    private fun has(t: String) = rule.onAllNodes(hasText(t, substring = true)).fetchSemanticsNodes().isNotEmpty()
    private val problems = mutableListOf<String>()

    private fun spoken(n: SemanticsNode): String {
        val c = n.config
        val label = c.getOrNull(SemanticsProperties.ContentDescription)?.joinToString(" ")
            ?: c.getOrNull(SemanticsProperties.Text)?.joinToString(" ") { it.text }
            ?: c.getOrNull(SemanticsProperties.EditableText)?.text
            ?: ""
        return label.trim()
    }

    private fun walk(n: SemanticsNode, out: MutableList<SemanticsNode>) {
        out += n
        n.children.forEach { walk(it, out) }
    }

    /** Announcements for the screen as a screen reader would move through them. */
    private fun read(screen: String) {
        rule.waitForIdle()
        val nodes = mutableListOf<SemanticsNode>()
        rule.onAllNodes(isRoot()).fetchSemanticsNodes().forEach { walk(it, nodes) }
        val lines = mutableListOf<String>()
        val clickable = mutableListOf<String>()
        for (n in nodes) {
            val c = n.config
            val click = c.getOrNull(SemanticsActions.OnClick) != null
            val toggle = c.getOrNull(SemanticsProperties.ToggleableState)
            val role = c.getOrNull(SemanticsProperties.Role)
            val text = spoken(n)
            if (text.isEmpty() && !click && toggle == null) continue
            if (c.getOrNull(SemanticsProperties.InvisibleToUser) != null) continue
            val disabled = c.getOrNull(SemanticsProperties.Disabled) != null
            val selected = c.getOrNull(SemanticsProperties.Selected)?.let { if (it) "selected" else "not selected" }
            lines += listOfNotNull(text.ifEmpty { "(no label)" }, role?.toString(), toggle?.toString(), selected, "disabled".takeIf { disabled })
                .joinToString(", ")
            if ((click || toggle != null) && text.isEmpty()) problems += "[$screen] ${role ?: "control"} with no label"
            if (Regex("[‹›⏱◎☁☰]").containsMatchIn(text)) problems += "[$screen] symbol read aloud: \"$text\""
            if (click && !disabled) clickable += text
        }
        clickable.groupingBy { it }.eachCount().filter { it.value > 1 && it.key.isNotEmpty() }.forEach { (label, count) ->
            problems += "[$screen] $count controls announce \"$label\""
        }
        android.util.Log.i("SquibReader", "== $screen\n" + lines.joinToString("\n"))
    }

    private fun readScrolling(screen: String, pages: Int) {
        read(screen)
        repeat(pages) { i ->
            rule.onAllNodes(isRoot())[0].performTouchInput { swipeUp() }
            read("$screen, scrolled ${i + 1}")
        }
    }

    @Test
    fun everyControlHasASpokenLabelAndNoneRepeats() {
        rule.resetTimer()
        rule.onNodeWithTag("mode_par").performClick()
        readScrolling("Timer, par-only", 2)
        rule.onNodeWithTag("mode_live").performScrollTo().performClick()
        readScrolling("Timer, live", 3)
        rule.onNodeWithTag("mode_par").performScrollTo().performClick()

        rule.onNodeWithTag("tab_practice").performClick()
        rule.waitOrDump { has("Repeated par") }
        readScrolling("Practice", 4)
        rule.onNodeWithTag("tab_conditions").performClick()
        rule.waitOrDump { tag("conditions") }
        readScrolling("Conditions", 4)
        rule.onNodeWithTag("tab_history").performClick()
        rule.waitOrDump { tag("history") }
        readScrolling("History", 2)

        problems.forEach { android.util.Log.e("SquibReader", it) }
        check(problems.isEmpty()) { problems.distinct().joinToString("\n") }
    }
}
