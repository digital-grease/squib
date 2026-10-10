package net.digitalgrease.squib

import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.isRoot
import androidx.compose.ui.test.junit4.AndroidComposeTestRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo

/** The engine is app-wide; a previous test may leave a saved result on the Timer screen. */
fun AndroidComposeTestRule<*, *>.resetTimer() {
    waitForIdle()
    if (onAllNodes(hasTestTag("new_setup")).fetchSemanticsNodes().isNotEmpty()) {
        onNodeWithTag("new_setup").performScrollTo().performClick()
    }
    waitUntil(5_000) { onAllNodes(hasTestTag("mode_par")).fetchSemanticsNodes().isNotEmpty() }
}

/** Wait; on timeout, log every visible text so the failure shows what was on screen. */
fun AndroidComposeTestRule<*, *>.waitOrDump(ms: Long = 5_000, cond: () -> Boolean) {
    try {
        waitUntil(ms, cond)
    } catch (e: Throwable) {
        fun walk(x: SemanticsNode): List<String> =
            (x.config.getOrNull(SemanticsProperties.Text)?.map { it.text } ?: emptyList()) + x.children.flatMap(::walk)
        val texts = onAllNodes(isRoot()).fetchSemanticsNodes().flatMap(::walk)
        android.util.Log.e("SquibTest", "screen: " + texts.joinToString(" | "))
        throw e
    }
}
