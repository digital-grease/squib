package dev.digitalgrease.squib.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import java.util.Locale
import dev.digitalgrease.squib.core.DrillView
import dev.digitalgrease.squib.core.Mode
import dev.digitalgrease.squib.core.QrCodeView
import dev.digitalgrease.squib.data.DataController
import dev.digitalgrease.squib.video.QrScanner

/**
 * A drill's QR code (plan phase 4.1). Always dark on white with a quiet zone, whatever the
 * theme, so cameras read it. The code holds the recipe itself, never a link.
 */
@Composable
fun DrillQrDialog(drill: DrillView, qr: Result<QrCodeView>, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        confirmButton = { TextButton(onClick = onDismiss, modifier = Modifier.heightIn(min = 48.dp).testTag("qr_close")) { Text("Close") } },
        title = { Text("QR code: ${drill.input.title}") },
        text = {
            Column {
                qr.onSuccess { code ->
                    val n = code.size.toInt()
                    Canvas(
                        Modifier.fillMaxWidth().aspectRatio(1f).background(Color.White).testTag("drill_qr")
                            .semantics { contentDescription = "QR code for the drill ${drill.input.title}" },
                    ) {
                        val quiet = 4
                        val cell = size.width / (n + 2 * quiet)
                        for (y in 0 until n) for (x in 0 until n) {
                            if (code.modules[y * n + x].toInt() == 1) {
                                drawRect(Color.Black, Offset((x + quiet) * cell, (y + quiet) * cell), Size(cell + 0.5f, cell + 0.5f))
                            }
                        }
                    }
                    Text(
                        "Scan it with Squib on another phone (Practice, Scan drill QR). It contains the drill itself, " +
                            "not a link; nothing is downloaded.",
                        style = MaterialTheme.typography.bodySmall,
                        modifier = Modifier.padding(top = 8.dp),
                    )
                }
                qr.onFailure { e -> Text(e.message ?: "This drill cannot be shown as a QR code.", modifier = Modifier.testTag("qr_error")) }
            }
        },
    )
}

/**
 * Scan a drill QR code with the camera, then show what it holds before anything is saved.
 * A code that is not a Squib drill is refused and scanning goes on.
 */
@Composable
fun DrillQrScan(d: DataController, onClose: () -> Unit, onImported: (DrillView) -> Unit) {
    val context = LocalContext.current
    val scanner = remember { QrScanner(context.applicationContext) }
    var surface by remember { mutableStateOf<android.graphics.SurfaceTexture?>(null) }
    var found by remember { mutableStateOf<String?>(null) }
    var preview by remember { mutableStateOf<DrillView?>(null) }
    var status by remember { mutableStateOf("Point the camera at a drill QR code.") }
    DisposableEffect(Unit) { onDispose { scanner.release() } }
    LaunchedEffect(surface, found) {
        val s = surface
        if (s != null && found == null) {
            scanner.start(s, onError = { status = it }) { text -> if (found == null) found = text }
        }
    }
    LaunchedEffect(found) {
        val text = found ?: return@LaunchedEffect
        scanner.stop()
        val p = d.previewDrillQr(text)
        p.onSuccess { preview = it }
        p.onFailure { e ->
            // Not a drill, or damaged: say so and keep scanning.
            status = e.message ?: "Not a Squib drill code."
            found = null
        }
    }
    Column(Modifier.fillMaxWidth().padding(16.dp).verticalScroll(rememberScrollState()).testTag("qr_scan")) {
        BackButton("Cancel", "Cancel scanning", onClose)
        Text("Scan drill QR", style = MaterialTheme.typography.headlineSmall)
        androidx.compose.ui.viewinterop.AndroidView(
            factory = { ctx ->
                android.view.TextureView(ctx).apply {
                    surfaceTextureListener = object : android.view.TextureView.SurfaceTextureListener {
                        override fun onSurfaceTextureAvailable(t: android.graphics.SurfaceTexture, w: Int, h: Int) {
                            surface = t
                        }

                        override fun onSurfaceTextureSizeChanged(t: android.graphics.SurfaceTexture, w: Int, h: Int) {}

                        override fun onSurfaceTextureDestroyed(t: android.graphics.SurfaceTexture): Boolean {
                            scanner.stop()
                            surface = null
                            return true
                        }

                        override fun onSurfaceTextureUpdated(t: android.graphics.SurfaceTexture) {}
                    }
                }
            },
            modifier = Modifier.fillMaxWidth().height(320.dp).padding(top = 8.dp).background(Color.Black)
                .semantics { contentDescription = "Camera view for scanning" },
        )
        Text(status, modifier = Modifier.padding(top = 8.dp).testTag("qr_status"))
        Text(
            "The camera is used only while this screen is open; frames are read on the phone and not saved.",
            style = MaterialTheme.typography.bodySmall,
        )
    }
    preview?.let { p ->
        AlertDialog(
            onDismissRequest = { preview = null; found = null },
            title = { Text("Import this drill?") },
            text = {
                Column {
                    Text(p.input.title, fontWeight = FontWeight.Bold, modifier = Modifier.testTag("qr_preview_title"))
                    if (p.input.description.isNotBlank()) Text(p.input.description)
                    Text(drillSummary(p), style = MaterialTheme.typography.bodySmall)
                    Text(
                        "It is saved as a new drill on this phone. Nothing is downloaded or run.",
                        style = MaterialTheme.typography.bodySmall,
                        modifier = Modifier.padding(top = 8.dp),
                    )
                }
            },
            confirmButton = {
                Button(onClick = {
                    val text = found ?: return@Button
                    d.importDrillQr(text) { saved -> if (saved != null) onImported(saved) }
                }, modifier = Modifier.heightIn(min = 48.dp).testTag("qr_import")) { Text("Import") }
            },
            dismissButton = {
                OutlinedButton(onClick = { preview = null; found = null }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Scan again") }
            },
        )
    }
}

private fun drillSummary(d: DrillView): String {
    val i = d.input
    val pars = if (i.parsMs.isEmpty()) "no par times" else "par " + i.parsMs.joinToString(", ") { String.format(Locale.US, "%.1f s", it.toInt() / 1000.0) }
    val strings = if (i.repeats.toInt() > 1) "${i.repeats} strings, ${i.restS} s rest" else "1 string"
    val mode = if (i.mode == Mode.PHONE_LIVE) "Live shots" else "Par-only"
    return "$mode · $pars · $strings · ${d.scoringTitle}"
}
