package dev.digitalgrease.squib

import android.graphics.Bitmap
import android.net.Uri
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.click
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.test.performTouchInput
import androidx.lifecycle.ViewModelProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import dev.digitalgrease.squib.data.DataController
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Plan phase 4.2 on an emulator: tap marks onto a target photo, save, edit, save again. */
@RunWith(AndroidJUnit4::class)
class PhotoAnnotationFlowTest {
    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    private fun tag(t: String) = rule.onAllNodes(hasTestTag(t)).fetchSemanticsNodes().isNotEmpty()
    private fun has(t: String) = rule.onAllNodes(hasText(t, substring = true)).fetchSemanticsNodes().isNotEmpty()

    @Test
    fun marksAreTappedOnSavedAsVersionsAndListedForScreenReaders() {
        val app = rule.activity.application as SquibApp
        val d = ViewModelProvider(rule.activity)[DataController::class.java]
        val runId = app.engine.addManualRun("Club timer", 10u, "1.5", null, null, System.currentTimeMillis(), 0, "test")
        val src = File(app.cacheDir, "target-test.jpg")
        Bitmap.createBitmap(400, 400, Bitmap.Config.ARGB_8888).apply { eraseColor(android.graphics.Color.LTGRAY) }
            .let { bmp -> src.outputStream().use { bmp.compress(Bitmap.CompressFormat.JPEG, 90, it) } }
        val done = CountDownLatch(1)
        rule.runOnUiThread { d.addPhoto(runId, Uri.fromFile(src)) { done.countDown() } }
        check(done.await(20, TimeUnit.SECONDS)) { "photo import" }

        rule.onNodeWithTag("tab_history").performClick()
        rule.waitOrDump { tag("history") }
        rule.onNodeWithTag("history").performScrollToNode(hasTestTag("run_$runId"))
        rule.onNodeWithTag("run_$runId").performClick()
        rule.waitOrDump(10_000) { tag("review_screen") }
        rule.waitOrDump { tag("photo_0") }
        rule.onNodeWithTag("photo_0").performScrollTo().performClick()
        rule.waitOrDump { tag("annotate_photo") }

        // A hit near the centre, then a miss near the top left.
        rule.onNodeWithTag("annotate_photo").performTouchInput { click(center) }
        rule.waitOrDump { has("1 hits, 0 misses") }
        rule.onNodeWithTag("mark_kind_miss").performScrollTo().performClick()
        rule.onNodeWithTag("annotate_photo").performScrollTo().performTouchInput { click(Offset(width * 0.15f, height * 0.15f)) }
        rule.waitOrDump { has("1 hits, 1 misses") && has("Miss at 15% across") }
        rule.onNodeWithTag("annotate_save").performScrollTo().performClick()
        rule.waitOrDump { has("Version 1, saved with review revision 1") }

        // Remove the miss from the text list (the screen-reader path) and save again.
        rule.onNodeWithTag("annotate").performScrollToNode(androidx.compose.ui.test.hasContentDescription("Remove Miss", substring = true))
        rule.onNode(androidx.compose.ui.test.hasContentDescription("Remove Miss", substring = true)).performClick()
        rule.waitOrDump { has("1 hits, 0 misses") }
        rule.onNodeWithTag("annotate_save").performScrollTo().performClick()
        rule.waitOrDump { has("Version 2, saved") }
        check(!tag("annotate_changed")) { "nothing changed in the review" }
    }
}
