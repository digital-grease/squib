package net.digitalgrease.squib

import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.test.junit4.accessibility.enableAccessibilityChecks
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.tryPerformAccessibilityChecks
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Control for [AccessibilityAuditTest]: the same checks do catch a known-bad control. */
@RunWith(AndroidJUnit4::class)
class AccessibilityCheckControlTest {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    @Test
    fun aTinyUnlabelledTapTargetIsReported() {
        rule.setContent { Box(Modifier.size(12.dp).background(Color.Gray).clickable {}) }
        rule.enableAccessibilityChecks(strictValidator())
        val caught = runCatching { rule.onRoot().tryPerformAccessibilityChecks() }.exceptionOrNull()
        android.util.Log.i("SquibA11y", "control: ${caught?.message}")
        check(caught != null) { "accessibility checks did not report a 12 dp unlabelled button" }
    }
}
