package net.digitalgrease.squib

import android.Manifest
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToNode
import androidx.lifecycle.ViewModelProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import net.digitalgrease.squib.data.DataController
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Plan phase 4.1 on an emulator: a drill's QR code is shown; the scanner opens the camera
 * and can be cancelled; a scanned code's text previews without saving and imports as a new
 * drill. The emulator camera cannot be pointed at a code, so decoding from camera frames is
 * covered by the core test (qr_flows), not here.
 */
@RunWith(AndroidJUnit4::class)
class DrillQrFlowTest {
    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    private fun tag(t: String) = rule.onAllNodes(hasTestTag(t)).fetchSemanticsNodes().isNotEmpty()
    private fun has(t: String) = rule.onAllNodes(hasText(t, substring = true)).fetchSemanticsNodes().isNotEmpty()

    @Test
    fun showScanAndImportADrillQr() {
        val d = ViewModelProvider(rule.activity)[DataController::class.java]
        rule.onNodeWithTag("tab_practice").performClick()
        rule.waitOrDump { has("Repeated par") }
        rule.onNodeWithTag("practice").performScrollToNode(hasTestTag("qr_starter-2"))
        rule.onNodeWithTag("qr_starter-2").performClick()
        rule.waitOrDump { tag("drill_qr") && has("not a link") }
        rule.onNodeWithTag("qr_close").performClick()
        rule.waitOrDump { !tag("drill_qr") }

        // The scanner asks for the camera at the point of use; grant it as a user would.
        val inst = InstrumentationRegistry.getInstrumentation()
        inst.uiAutomation.grantRuntimePermission(inst.targetContext.packageName, Manifest.permission.CAMERA)
        rule.onNodeWithTag("practice").performScrollToNode(hasTestTag("scan_drill_qr"))
        rule.onNodeWithTag("scan_drill_qr").performClick()
        rule.waitOrDump { tag("qr_scan") && has("Point the camera at a drill QR code") }
        // System Back closes the scanner and stays in Practice.
        rule.runOnUiThread { rule.activity.onBackPressedDispatcher.onBackPressed() }
        rule.waitOrDump { tag("practice") && !tag("qr_scan") }

        // What a scan hands over: preview saves nothing, import adds a new drill.
        val drill = d.drills.value.first { it.drillId == "starter-2" }
        val text = runBlocking { d.drillQr(drill) }.getOrThrow().text
        val before = d.drills.value.size
        val p = runBlocking { d.previewDrillQr(text) }.getOrThrow()
        check(p.input.title == "Repeated par")
        check(d.drills.value.size == before) { "preview saved something" }
        var imported: String? = null
        rule.runOnUiThread { d.importDrillQr(text) { imported = it?.drillId } }
        rule.waitOrDump(10_000) { imported != null && d.drills.value.size == before + 1 }
        // Tidy the shared list.
        rule.runOnUiThread { d.archiveDrill(imported!!) }
        rule.waitOrDump { d.drills.value.size == before }
        check(!runBlocking { d.previewDrillQr("https://example.com/drill") }.isSuccess) { "a link is not a drill" }
    }
}
