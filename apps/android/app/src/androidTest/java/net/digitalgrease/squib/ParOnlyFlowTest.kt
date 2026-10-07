package net.digitalgrease.squib

import android.Manifest
import android.content.pm.PackageManager
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertNotEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** A01/A02 on an emulator: par-only timing without microphone permission or network. */
@RunWith(AndroidJUnit4::class)
class ParOnlyFlowTest {
    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    @Test
    fun parOnlyRunCompletesAndIsSavedWithoutMicrophone() {
        val ctx = InstrumentationRegistry.getInstrumentation().targetContext
        assertNotEquals(
            "test requires microphone permission NOT granted",
            PackageManager.PERMISSION_GRANTED,
            ctx.checkSelfPermission(Manifest.permission.RECORD_AUDIO),
        )
        rule.onNodeWithTag("mode_par").performClick()
        rule.onNodeWithTag("delay_instant").performClick()
        rule.onNodeWithTag("arm").performScrollTo().performClick()
        rule.waitUntil(10_000) { rule.onAllNodes(hasTestTag("stop")).fetchSemanticsNodes().isNotEmpty() }
        rule.onNodeWithText("GO").assertIsDisplayed()
        Thread.sleep(700)
        rule.onNodeWithTag("stop").performScrollTo().performClick()
        rule.waitUntil(10_000) {
            rule.onAllNodes(hasText("Saved · Complete")).fetchSemanticsNodes().isNotEmpty()
        }
        rule.onNodeWithText("Saved on this device").assertIsDisplayed()
        rule.onNodeWithTag("repeat").assertIsDisplayed()
        rule.onNodeWithTag("tab_history").performClick()
        rule.waitUntil(5_000) { rule.onAllNodes(hasText("Par-only")).fetchSemanticsNodes().isNotEmpty() }
    }
}
