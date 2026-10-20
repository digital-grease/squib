package dev.digitalgrease.squib

import android.app.Instrumentation
import android.content.Intent
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performScrollToNode
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.digitalgrease.squib.ui.DONATE_URL
import dev.digitalgrease.squib.ui.SOURCE_URL
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Plan 5.5 on an emulator: About shows the build's version, the full AGPL text offline,
 * third-party notices with their licence texts, and opens the source and donation links
 * only when tapped. Link intents are caught by a blocking monitor, so no browser opens.
 */
@RunWith(AndroidJUnit4::class)
class AboutFlowTest {
    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    private fun tag(t: String) = rule.onAllNodes(hasTestTag(t)).fetchSemanticsNodes().isNotEmpty()
    private fun has(t: String) = rule.onAllNodes(hasText(t, substring = true)).fetchSemanticsNodes().isNotEmpty()

    /** Tap, and catch the browser intent instead of starting a browser. */
    private fun tapAndCatchLink(tag: String): String? {
        var seen: String? = null
        val monitor = object : Instrumentation.ActivityMonitor() {
            override fun onStartActivity(intent: Intent): Instrumentation.ActivityResult? {
                if (intent.action != Intent.ACTION_VIEW) return null
                seen = intent.dataString
                return Instrumentation.ActivityResult(0, null)
            }
        }
        val inst = InstrumentationRegistry.getInstrumentation()
        inst.addMonitor(monitor)
        try {
            rule.onNodeWithTag(tag).performScrollTo().performClick()
            rule.waitUntil(5_000) { seen != null }
            return seen
        } finally {
            inst.removeMonitor(monitor)
        }
    }

    @Test
    fun aboutShowsVersionLicenceNoticesAndLinks() {
        rule.onNodeWithTag("tab_history").performClick()
        rule.waitOrDump { tag("history") }
        rule.onNodeWithTag("history").performScrollToNode(hasTestTag("open_data"))
        rule.onNodeWithTag("open_data").performClick()
        rule.waitOrDump { tag("open_about") }
        rule.onNodeWithTag("open_about").performScrollTo().performClick()
        rule.waitOrDump { tag("about") }
        check(has("Version ${BuildConfig.VERSION_NAME} (${BuildConfig.VERSION_CODE})")) { "version from the build" }
        check(has("core ") && has("schema ")) { "core versions" }
        check(has("WITHOUT ANY WARRANTY")) { "warranty notice" }

        // Links leave the app only on a tap, and go where they say.
        check(tapAndCatchLink("about_source") == SOURCE_URL) { "source link" }
        check(tapAndCatchLink("about_donate") == DONATE_URL) { "donation link" }

        // The full licence is bundled, so it reads offline.
        rule.onNodeWithTag("about_licence").performScrollTo().performClick()
        rule.waitOrDump { has("GNU AFFERO GENERAL PUBLIC LICENSE") }
        rule.runOnUiThread { rule.activity.onBackPressedDispatcher.onBackPressed() }
        rule.waitOrDump { tag("about") }

        // Notices list the real dependencies; a component opens to its licence text.
        rule.onNodeWithTag("about_notices").performScrollTo().performClick()
        rule.waitOrDump { has("Rust crates") }
        rule.onNodeWithTag("notices_list").performScrollToNode(hasText("rusqlite", substring = true))
        rule.onNodeWithText("rusqlite", substring = true).performClick()
        rule.waitOrDump { has("The rusqlite developers") }
        rule.onNodeWithTag("notices_list").performScrollToNode(hasText("net.java.dev.jna:jna", substring = true))
        check(has("LGPL-2.1-or-later OR Apache-2.0")) { "JNA's dual licence shown" }

        // Back steps out one page at a time.
        rule.runOnUiThread { rule.activity.onBackPressedDispatcher.onBackPressed() }
        rule.waitOrDump { tag("about") }
        rule.runOnUiThread { rule.activity.onBackPressedDispatcher.onBackPressed() }
        rule.waitOrDump { tag("open_about") }
    }
}
