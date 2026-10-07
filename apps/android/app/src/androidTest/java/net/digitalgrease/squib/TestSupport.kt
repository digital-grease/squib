package net.digitalgrease.squib

import androidx.compose.ui.test.hasTestTag
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
