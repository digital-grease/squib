package net.digitalgrease.squib.ui

import android.media.MediaPlayer
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.Checkbox
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import java.io.File
import java.util.Locale
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import net.digitalgrease.squib.core.DiagnosticPreview
import net.digitalgrease.squib.core.DiagnosticView
import net.digitalgrease.squib.data.DataController

/**
 * The run's opt-in diagnostic recording: listen with markers, delete it, or export it
 * for detector research after a preview and consent. Nothing is sent anywhere.
 */
@Composable
fun DiagnosticSection(d: DataController, runId: String, onExport: () -> Unit) {
    var rec by remember { mutableStateOf<DiagnosticView?>(null) }
    var version by remember { mutableIntStateOf(0) }
    var preview by remember { mutableStateOf<DiagnosticPreview?>(null) }
    var confirmDelete by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    LaunchedEffect(runId, version) {
        // The recording is attached just after the run is saved; look again briefly.
        repeat(20) {
            rec = d.diagnostic(runId)
            if (rec != null) return@LaunchedEffect
            delay(250)
        }
    }
    val v = rec ?: return
    var player by remember { mutableStateOf<MediaPlayer?>(null) }
    var pos by remember { mutableLongStateOf(0L) }
    DisposableEffect(v.relativePath) {
        val p = runCatching { MediaPlayer().apply { setDataSource(File(d.attachmentRoot, v.relativePath).path); prepare() } }.getOrNull()
        player = p
        onDispose {
            p?.release()
            player = null
        }
    }
    LaunchedEffect(player) {
        while (player != null) {
            pos = runCatching { player?.currentPosition?.toLong() ?: 0L }.getOrDefault(0L)
            delay(100)
        }
    }
    val current = v.markers.lastOrNull { pos >= it.fileMs && pos - it.fileMs < 1200 }
    Card(Modifier.fillMaxWidth().padding(vertical = 8.dp).testTag("diagnostic_section")) {
        Column(Modifier.padding(12.dp)) {
            Text("Diagnostic recording", fontWeight = FontWeight.Bold)
            Text(
                String.format(Locale.US, "%.1f s, %.1f MB. Kept only on this phone until you delete it or export it.", v.durationMs / 1000.0, v.bytes / 1e6),
                style = MaterialTheme.typography.bodySmall,
            )
            if (v.gaps > 0u || v.truncated) {
                Text(
                    listOfNotNull(
                        "${v.gaps} gap(s) where audio could not be kept are silent".takeIf { v.gaps > 0u },
                        "stopped at the 60 s limit".takeIf { v.truncated },
                    ).joinToString("; ") + ".",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error,
                )
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                OutlinedButton(onClick = { player?.let { if (it.isPlaying) it.pause() else it.start() } },
                    modifier = Modifier.heightIn(min = 48.dp).padding(end = 8.dp).testTag("diag_play")) { Text("Play / pause") }
                Text(String.format(Locale.US, "%.2f s", pos / 1000.0) + (current?.let { "  ·  ${it.label}" } ?: ""),
                    modifier = Modifier.testTag("diag_position"))
            }
            FlowRow {
                v.markers.forEach { m ->
                    FilterChip(
                        selected = current == m,
                        onClick = { player?.let { it.seekTo((m.fileMs - 300).coerceAtLeast(0).toInt()); it.start() } },
                        label = { Text(m.label + if (m.edited) " (edited)" else "") },
                        modifier = Modifier.padding(end = 6.dp).heightIn(min = 48.dp).testTag("diag_marker_${m.kind}"),
                    )
                }
            }
            FlowRow {
                Button(onClick = { scope.launch { preview = d.diagnosticPreview(runId) } },
                    modifier = Modifier.padding(end = 8.dp).heightIn(min = 48.dp).testTag("diag_export")) { Text("Export for research…") }
                TextButton(onClick = { confirmDelete = true }, modifier = Modifier.heightIn(min = 48.dp).testTag("diag_delete")) {
                    Text("Delete recording")
                }
            }
        }
    }
    preview?.let { p -> ExportPreview(p, onCancel = { preview = null }, onConfirm = { preview = null; onExport() }) }
    if (confirmDelete) {
        AlertDialog(
            onDismissRequest = { confirmDelete = false },
            confirmButton = {
                TextButton(onClick = {
                    confirmDelete = false
                    player?.release()
                    player = null
                    d.deleteDiagnostic(runId) { rec = null; version++ }
                }, modifier = Modifier.testTag("diag_delete_confirm")) { Text("Delete") }
            },
            dismissButton = { TextButton(onClick = { confirmDelete = false }) { Text("Keep") } },
            text = { Text("Delete this recording from the phone? The run and its times stay. Copies you exported are not affected.") },
        )
    }
}

@Composable
private fun ExportPreview(p: DiagnosticPreview, onCancel: () -> Unit, onConfirm: () -> Unit) {
    var consent by remember { mutableStateOf(false) }
    AlertDialog(
        onDismissRequest = onCancel,
        title = { Text("Export diagnostic recording") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                Text(String.format(Locale.US, "%.1f s of audio, %d labelled shots, %d detector events.",
                    p.durationMs / 1000.0, p.acceptedEvents.toInt(), p.detectorCandidates.toInt()))
                if (!p.reviewedByPerson) {
                    Text("You have not reviewed this run: the labels are the detector's suggestions.", color = MaterialTheme.colorScheme.error,
                        style = MaterialTheme.typography.bodySmall)
                }
                Text("Included", fontWeight = FontWeight.Bold, modifier = Modifier.padding(top = 8.dp))
                p.includes.forEach { Text("• $it", style = MaterialTheme.typography.bodySmall) }
                Text("Not included", fontWeight = FontWeight.Bold, modifier = Modifier.padding(top = 8.dp))
                p.excludes.forEach { Text("• $it", style = MaterialTheme.typography.bodySmall) }
                Text("The file is saved where you choose. Squib does not send it anywhere.", style = MaterialTheme.typography.bodySmall,
                    modifier = Modifier.padding(top = 8.dp))
                Row(verticalAlignment = Alignment.Top, modifier = Modifier.padding(top = 8.dp)) {
                    Checkbox(checked = consent, onCheckedChange = { consent = it }, modifier = Modifier.testTag("diag_consent"))
                    Text(p.consentStatement, style = MaterialTheme.typography.bodySmall)
                }
            }
        },
        confirmButton = {
            TextButton(onClick = onConfirm, enabled = consent, modifier = Modifier.testTag("diag_save")) { Text("Choose where to save") }
        },
        dismissButton = { TextButton(onClick = onCancel) { Text("Cancel") } },
    )
}
