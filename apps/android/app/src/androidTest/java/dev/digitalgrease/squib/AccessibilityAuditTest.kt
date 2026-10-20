package dev.digitalgrease.squib

import androidx.compose.ui.test.junit4.accessibility.disableAccessibilityChecks
import androidx.compose.ui.test.junit4.accessibility.enableAccessibilityChecks
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.swipeUp
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.isRoot
import com.google.android.apps.common.testing.accessibility.framework.AccessibilityCheckPreset
import com.google.android.apps.common.testing.accessibility.framework.AccessibilityCheckResult
import com.google.android.apps.common.testing.accessibility.framework.integrations.espresso.AccessibilityValidator
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.test.tryPerformAccessibilityChecks
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.AfterClass
import org.junit.BeforeClass
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * A21 on an emulator: every main screen at 2x font scale passes the Accessibility Test
 * Framework checks (labels, touch target size, contrast, duplicate descriptions), and the
 * run controls stay reachable. Findings from every screen are collected before failing.
 */
/** Latest ATF checks from the root view; warnings fail too, not only errors. */
fun strictValidator(): AccessibilityValidator = AccessibilityValidator()
    .setCheckPreset(AccessibilityCheckPreset.LATEST)
    .setRunChecksFromRootView(true)
    .setThrowExceptionFor(AccessibilityCheckResult.AccessibilityCheckResultType.WARNING)

/** Device settings for an audit pass: 2x font, and light or dark theme; restored after. */
object AuditDevice {
    private var previousScale = "1.0"

    fun shell(cmd: String): String {
        val ui = InstrumentationRegistry.getInstrumentation().uiAutomation
        val fd = ui.executeShellCommand(cmd)
        return android.os.ParcelFileDescriptor.AutoCloseInputStream(fd).bufferedReader().readText().trim()
    }

    fun apply(dark: Boolean) {
        previousScale = shell("settings get system font_scale").takeIf { it.isNotEmpty() && it != "null" } ?: "1.0"
        shell("settings put system font_scale 2.0")
        shell("cmd uimode night ${if (dark) "yes" else "no"}")
    }

    fun restore() {
        shell("settings put system font_scale $previousScale")
        shell("cmd uimode night no")
    }
}

/** The same audit in the dark theme. */
@RunWith(AndroidJUnit4::class)
class AccessibilityAuditDarkTest : AccessibilityAuditTest() {
    override val expectDark = true

    // Same names as the parent's: JUnit 4 then runs only these, not both.
    companion object {
        @BeforeClass @JvmStatic
        fun setUp() = AuditDevice.apply(dark = true)

        @AfterClass @JvmStatic
        fun tearDown() = AuditDevice.restore()
    }
}

@RunWith(AndroidJUnit4::class)
open class AccessibilityAuditTest {
    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    protected open val expectDark = false

    companion object {
        @BeforeClass @JvmStatic
        fun setUp() = AuditDevice.apply(dark = false)

        @AfterClass @JvmStatic
        fun tearDown() = AuditDevice.restore()
    }

    private fun tag(t: String) = rule.onAllNodes(hasTestTag(t)).fetchSemanticsNodes().isNotEmpty()
    private fun has(t: String) = rule.onAllNodes(hasText(t, substring = true)).fetchSemanticsNodes().isNotEmpty()
    private val findings = mutableListOf<String>()

    private fun audit(screen: String) {
        rule.waitForIdle()
        // Checks run only here, so every screen's findings are collected in one pass.
        rule.enableAccessibilityChecks(strictValidator())
        try {
            rule.onRoot().tryPerformAccessibilityChecks()
        } catch (e: Throwable) {
            findings += "[$screen] ${e.message}"
        } finally {
            rule.disableAccessibilityChecks()
        }
    }

    /** Audit what is on screen, then scroll down a screenful at a time and audit again. */
    private fun auditScrolling(screen: String, pages: Int = 4) {
        audit(screen)
        repeat(pages) { i ->
            rule.onAllNodes(isRoot())[0].performTouchInput { swipeUp() }
            audit("$screen, scrolled ${i + 1}")
        }
    }

    private fun tagStartingWith(prefix: String) =
        SemanticsMatcher("tag starts with $prefix") { (it.config.getOrNull(SemanticsProperties.TestTag) ?: "").startsWith(prefix) }

    @Test
    fun mainScreensPassAccessibilityChecksAtTwiceTheFontSize() {
        val config = rule.activity.resources.configuration
        check(config.fontScale >= 2f) { "font scale not applied" }
        val dark = (config.uiMode and android.content.res.Configuration.UI_MODE_NIGHT_MASK) == android.content.res.Configuration.UI_MODE_NIGHT_YES
        android.util.Log.i("SquibA11y", "audit: font scale ${config.fontScale}, dark $dark")
        check(dark == expectDark) { "theme not applied (dark $dark)" }
        rule.resetTimer()
        rule.onNodeWithTag("mode_live").performClick()
        auditScrolling("Timer, live")
        rule.onNodeWithTag("mode_par").performScrollTo().performClick()
        auditScrolling("Timer, par-only")
        // Sensitivity setup is not audited here: opening it asks for the microphone, and
        // the system permission dialog replaces the app (other tests need it not granted).

        // A finished run: status, result actions.
        rule.onNodeWithTag("delay_instant").performScrollTo().performClick()
        rule.onNodeWithTag("arm").performScrollTo().performClick()
        rule.waitOrDump(10_000) { tag("stop") }
        rule.onNodeWithTag("stop").performScrollTo().performClick()
        rule.waitOrDump(10_000) { has("Saved · Complete") }
        auditScrolling("Timer, saved run", pages = 2)
        rule.onNodeWithTag("new_setup").performScrollTo().performClick()

        // Practice, a drill on the Timer (set controls, automatic repeat), the manual result
        // form, and Review with its score card.
        rule.onNodeWithTag("tab_practice").performClick()
        rule.waitOrDump { has("Repeated par") }
        auditScrolling("Practice", pages = 6)
        rule.onNodeWithTag("practice").performScrollToNode(hasTestTag("start_starter-2"))
        rule.onNodeWithTag("start_starter-2").performClick()
        rule.waitOrDump { tag("drill_banner") }
        rule.onNodeWithTag("auto_repeat_switch").performScrollTo().performClick()
        auditScrolling("Timer with a drill")
        rule.onNodeWithTag("auto_repeat_switch").performScrollTo().performClick()
        rule.onNodeWithText("Clear").performScrollTo().performClick()

        rule.onNodeWithTag("tab_practice").performClick()
        rule.waitOrDump { tag("manual_entry") }
        rule.onNodeWithTag("practice").performScrollToNode(hasTestTag("manual_entry"))
        rule.onNodeWithTag("manual_entry").performClick()
        rule.waitOrDump { tag("manual_label") }
        audit("Manual result form")
        rule.onNodeWithTag("manual_label").performTextInput("Club timer")
        rule.onNodeWithTag("manual_times").performTextInput("1.20 1.65 2.10")
        // At 2x the keyboard covers Save; click it through its semantics action.
        rule.onNodeWithTag("manual_save").performSemanticsAction(SemanticsActions.OnClick)
        rule.waitOrDump(10_000) { tag("review_screen") }
        auditScrolling("Review", pages = 6)
        rule.runOnUiThread { rule.activity.onBackPressedDispatcher.onBackPressed() }

        // Day plan.
        rule.onNodeWithTag("tab_practice").performClick()
        rule.waitOrDump { tag("new_plan") }
        rule.onNodeWithTag("practice").performScrollToNode(hasTestTag("new_plan"))
        rule.onNodeWithTag("new_plan").performClick()
        rule.onNodeWithTag("plan_title").performTextInput(if (expectDark) "Audit plan dark" else "Audit plan light")
        audit("New plan dialog")
        rule.onNodeWithTag("plan_create").performSemanticsAction(SemanticsActions.OnClick)
        rule.waitOrDump { tag("plan_screen") }
        auditScrolling("Plan", pages = 2)
        rule.onNodeWithTag("add_item").performScrollTo().performClick()
        rule.waitOrDump { tag("item_save") }
        audit("Plan item dialog")
        rule.runOnUiThread { rule.activity.onBackPressedDispatcher.onBackPressed() }
        rule.waitForIdle()

        rule.onNodeWithTag("tab_conditions").performClick()
        rule.waitOrDump { tag("conditions") }
        auditScrolling("Conditions", pages = 6)
        rule.onNodeWithTag("open_ballistics").performScrollTo().performClick()
        rule.waitOrDump { tag("ballistics") }
        auditScrolling("Ballistics", pages = 4)
        rule.onNodeWithTag("load_add").performScrollTo().performClick()
        rule.waitOrDump { tag("load_editor") }
        auditScrolling("Load editor", pages = 3)
        rule.runOnUiThread { rule.activity.onBackPressedDispatcher.onBackPressed() }
        rule.waitOrDump { tag("ballistics") }
        rule.runOnUiThread { rule.activity.onBackPressedDispatcher.onBackPressed() }
        rule.waitOrDump { tag("conditions") }

        rule.onNodeWithTag("tab_history").performClick()
        rule.waitOrDump { tag("history") }
        auditScrolling("History", pages = 3)
        rule.onNodeWithTag("history").performScrollToNode(hasTestTag("toggle_charts"))
        rule.onNodeWithTag("toggle_charts").performClick()
        auditScrolling("History, charts", pages = 3)
        rule.onNodeWithTag("history").performScrollToNode(tagStartingWith("session_"))
        rule.onAllNodes(tagStartingWith("session_"))[0].performClick()
        rule.waitOrDump { tag("session_screen") }
        auditScrolling("Visit", pages = 2)
        rule.runOnUiThread { rule.activity.onBackPressedDispatcher.onBackPressed() }
        rule.waitOrDump { tag("history") }

        rule.onNodeWithTag("history").performScrollToNode(hasTestTag("open_data"))
        rule.onNodeWithTag("open_data").performClick()
        rule.waitOrDump { !tag("history") }
        auditScrolling("Data", pages = 6)

        rule.onNodeWithTag("open_about").performScrollTo().performClick()
        rule.waitOrDump { tag("about") }
        auditScrolling("About", pages = 4)
        rule.onNodeWithTag("about_notices").performScrollTo().performClick()
        rule.waitOrDump { has("rusqlite") || has("Squib is built on") }
        auditScrolling("Third-party licences", pages = 2)

        android.util.Log.i("SquibA11y", "findings: ${findings.size}")
        findings.forEach { android.util.Log.e("SquibA11y", it) }
        check(findings.isEmpty()) { findings.joinToString("\n\n") }
    }
}
