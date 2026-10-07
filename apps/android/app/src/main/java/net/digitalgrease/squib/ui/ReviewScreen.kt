package net.digitalgrease.squib.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import net.digitalgrease.squib.core.EventRefFfi
import net.digitalgrease.squib.core.ReviewAction
import net.digitalgrease.squib.core.ReviewEvent
import net.digitalgrease.squib.core.ReviewView
import net.digitalgrease.squib.core.SquibController

/**
 * Review: original detections are never edited in place. Pending actions are applied
 * together as one new revision; the previous revision remains in history.
 */
@Composable
fun ReviewScreen(c: SquibController, runId: String, onBack: () -> Unit, onRepeat: () -> Unit) {
    var review by remember { mutableStateOf<ReviewView?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    val pending = remember { mutableStateListOf<ReviewAction>() }
    var manualText by remember { mutableStateOf("") }
    val scope = rememberCoroutineScope()

    LaunchedEffect(runId) {
        c.loadReview(runId).onSuccess { review = it }.onFailure { error = it.message }
    }

    Column(Modifier.verticalScroll(rememberScrollState()).padding(16.dp).testTag("review_screen")) {
        TextButton(onClick = onBack, modifier = Modifier.heightIn(min = 48.dp)) { Text("‹ Back") }
        error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        val r = review ?: return@Column
        Text("Review", style = MaterialTheme.typography.headlineMedium)
        Text(fmtDate(r.createdUtcMs))
        FlowRow(Modifier.padding(vertical = 4.dp)) {
            Badge(if (r.mode == "par_only") "Par-only" else "Live (experimental)")
            Badge(outcomeLabel(r.outcome), warning = r.outcome != "complete")
            reviewLabel(r.reviewState, r.edited)?.let { Badge(it, warning = r.reviewState == "needs_review") }
            if (r.revisionNumber > 0u) Badge("Revision ${r.revisionNumber}")
            if (r.uncommittedTailPossible) Badge("Events after last save may be missing", warning = true)
        }
        Text(startMethodLabel(r.startMethod), style = MaterialTheme.typography.bodyMedium)
        Text("Clock: ${timestampQualityLabel(r.timestampQuality)}", style = MaterialTheme.typography.bodyMedium)
        Text(
            "Times are displayed to 0.01 s. Display precision is not measured accuracy; timing on this phone is experimental.",
            style = MaterialTheme.typography.bodySmall,
        )

        Card(Modifier.fillMaxWidth().padding(vertical = 8.dp)) {
            Column(Modifier.padding(12.dp)) {
                LabeledValue("Shots", r.count.toString() + (r.expectedCount?.let { " (expected $it — hint only)" } ?: ""))
                LabeledValue("First shot", if (r.originResolved) fmtSec(r.firstNs) else "Unavailable (no start reference)")
                LabeledValue("Last shot", if (r.originResolved) fmtSec(r.lastNs) else "Unavailable")
                LabeledValue("Splits", if (r.splitsNs.isEmpty()) "—" else r.splitsNs.joinToString("  ") { fmtSec(it).removeSuffix(" s") } + " s")
                if (r.zeroSplits > 0u) Text("${r.zeroSplits} identical timestamps: insufficient resolution", color = MaterialTheme.colorScheme.error)
            }
        }

        Timeline(r)

        if (r.quality.isNotEmpty()) {
            Text("Quality", style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(top = 8.dp))
            r.quality.forEach { q ->
                Text("• ${q.label}${if (q.severity == "integrity") " (interrupts run)" else ""}" +
                    (q.timelineNs?.let { " at ${fmtSec(it)}" } ?: "") + (if (q.detail.isNotBlank()) " — ${q.detail}" else ""))
            }
        }

        if (r.revisionNumber > 0u) {
            Text("Events", style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(top = 12.dp))
            r.events.forEach { e -> EventRow(e, pending) }
            HorizontalDivider(Modifier.padding(vertical = 8.dp))
            Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                OutlinedTextField(
                    value = manualText,
                    onValueChange = { manualText = it },
                    label = { Text("Add missed shot at (s)") },
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal),
                    singleLine = true,
                    modifier = Modifier.weight(1f).testTag("manual_time"),
                )
                Spacer(Modifier.padding(4.dp))
                OutlinedButton(
                    onClick = {
                        manualText.toDoubleOrNull()?.takeIf { it >= 0 }?.let {
                            pending += ReviewAction.AddManual((it * 1e9).toLong())
                            manualText = ""
                        }
                    },
                    modifier = Modifier.heightIn(min = 48.dp).testTag("add_manual"),
                ) { Text("Add") }
            }
            if (pending.isNotEmpty()) {
                Text("${pending.size} pending correction(s)", fontWeight = FontWeight.Bold, modifier = Modifier.padding(top = 8.dp))
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Button(
                        onClick = {
                            val actions = pending.toList()
                            scope.launch {
                                c.applyReview(r.runId, r.revisionNumber, actions, null)
                                    .onSuccess { review = it; pending.clear(); error = null }
                                    .onFailure { error = it.message }
                            }
                        },
                        modifier = Modifier.heightIn(min = 56.dp).testTag("save_corrections"),
                    ) { Text("Save corrections") }
                    OutlinedButton(onClick = { pending.clear() }, modifier = Modifier.heightIn(min = 56.dp)) { Text("Discard") }
                }
            }
        } else {
            Text("No detected events: this run has no microphone timeline.", modifier = Modifier.padding(top = 8.dp))
        }

        Spacer(Modifier.height(16.dp))
        Button(onClick = onRepeat, modifier = Modifier.fillMaxWidth().height(64.dp)) { Text("REPEAT this setup", fontWeight = FontWeight.Black) }
    }
}

@Composable
private fun EventRow(e: ReviewEvent, pending: MutableList<ReviewAction>) {
    val label = when (e.state) {
        "accepted" -> if (e.origin == "manual") "Manual" else if (e.origin == "moved") "Accepted (moved)" else "Accepted"
        "uncertain" -> "Uncertain"
        else -> "Rejected"
    }
    Card(Modifier.fillMaxWidth().padding(vertical = 3.dp)) {
        Column(Modifier.padding(10.dp).semantics(mergeDescendants = true) {
            contentDescription = "$label event at ${fmtSec(e.timelineNs)}"
        }) {
            Row {
                Text(fmtSec(e.timelineNs), fontWeight = FontWeight.Bold, modifier = Modifier.weight(1f))
                Badge(label, warning = e.state != "accepted")
            }
            if (e.reasons.isNotEmpty()) Text(e.reasons.joinToString(" · "), style = MaterialTheme.typography.bodySmall)
            e.detectorScore?.let { Text("Detector ranking score ${"%.2f".format(it)} (not a probability)", style = MaterialTheme.typography.bodySmall) }
            FlowRow {
                val seq = e.reference.sequence
                if (seq != null && e.state != "accepted") {
                    OutlinedButton(onClick = { pending += ReviewAction.Accept(seq) }, modifier = Modifier.padding(end = 6.dp).heightIn(min = 48.dp)) { Text("Accept") }
                }
                if (seq != null && e.state != "rejected") {
                    OutlinedButton(onClick = { pending += ReviewAction.Reject(seq) }, modifier = Modifier.padding(end = 6.dp).heightIn(min = 48.dp)) { Text("Reject") }
                }
                if (e.state == "accepted") {
                    for ((label2, d) in listOf("−10 ms" to -10_000_000L, "−1 ms" to -1_000_000L, "+1 ms" to 1_000_000L, "+10 ms" to 10_000_000L)) {
                        OutlinedButton(
                            onClick = { pending += ReviewAction.Move(EventRefFfi(e.reference.kind, e.reference.sequence, e.reference.manualId), (e.timelineNs + d).coerceAtLeast(0)) },
                            modifier = Modifier.padding(end = 4.dp).heightIn(min = 48.dp),
                        ) { Text(label2) }
                    }
                }
                val mid = e.reference.manualId
                if (mid != null) {
                    OutlinedButton(onClick = { pending += ReviewAction.RemoveManual(mid) }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Remove") }
                }
            }
        }
    }
}

/** Markers by state with distinct shapes, plus cue and quality markers and an "Energy" strip. */
@Composable
private fun Timeline(r: ReviewView) {
    val events = r.events
    val endNs = maxOf(
        events.maxOfOrNull { it.timelineNs } ?: 0L,
        r.energyStartNs + r.energyHopNs * r.energyDb.size,
        1_000_000_000L,
    )
    val startNs = minOf(0L, r.energyStartNs)
    val accent = MaterialTheme.colorScheme.primary
    val warn = MaterialTheme.colorScheme.error
    val fg = MaterialTheme.colorScheme.onSurface
    Text("Energy (coarse, not a waveform)", style = MaterialTheme.typography.labelLarge, modifier = Modifier.padding(top = 8.dp))
    Canvas(
        Modifier.fillMaxWidth().height(120.dp).semantics {
            contentDescription = "Timeline with ${events.count { it.state == "accepted" }} accepted, " +
                "${events.count { it.state == "uncertain" }} uncertain, ${events.count { it.state == "rejected" }} rejected events"
        },
    ) {
        val w = size.width
        val h = size.height
        fun x(ns: Long) = ((ns - startNs).toFloat() / (endNs - startNs).toFloat()) * w
        // Energy strip in the lower half: −80..0 dBFS.
        r.energyDb.forEachIndexed { i, db ->
            val t = r.energyStartNs + r.energyHopNs * i
            val frac = ((db + 80f) / 80f).coerceIn(0f, 1f)
            drawLine(fg.copy(alpha = 0.35f), Offset(x(t), h), Offset(x(t), h - frac * h * 0.5f), strokeWidth = 2f)
        }
        // Start reference.
        drawLine(accent, Offset(x(0), 0f), Offset(x(0), h), strokeWidth = 3f)
        r.cues.forEach { cue ->
            cue.timelineNs?.let { t ->
                drawLine(accent, Offset(x(t), 0f), Offset(x(t), h), strokeWidth = 2f,
                    pathEffect = PathEffect.dashPathEffect(floatArrayOf(8f, 8f)))
            }
        }
        r.quality.forEach { q -> q.timelineNs?.let { t -> drawRect(warn.copy(alpha = 0.3f), Offset(x(t), 0f), androidx.compose.ui.geometry.Size(6f, h)) } }
        events.forEach { e ->
            val cx = x(e.timelineNs)
            when (e.state) {
                "accepted" -> drawCircle(if (e.origin == "manual") Color(0xFF80DEEA) else accent, 10f, Offset(cx, h * 0.25f))
                "uncertain" -> drawRect(warn, Offset(cx - 8f, h * 0.25f - 8f), androidx.compose.ui.geometry.Size(16f, 16f))
                else -> drawLine(fg.copy(alpha = 0.6f), Offset(cx - 8f, h * 0.25f - 8f), Offset(cx + 8f, h * 0.25f + 8f), strokeWidth = 3f)
            }
        }
    }
    Text("● accepted   ■ uncertain   ╲ rejected   ┆ cue   ▌ quality", style = MaterialTheme.typography.bodySmall)
}
