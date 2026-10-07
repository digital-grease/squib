package net.digitalgrease.squib.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import java.util.Locale
import net.digitalgrease.squib.core.SquibController

/** Guided sensitivity setup, cue test, and capture diagnostics. */
@Composable
fun SetupScreen(c: SquibController, onRequestMic: (then: () -> Unit) -> Unit, onBack: () -> Unit) {
    val probe by c.probe.collectAsState()
    val last by c.lastProbeResult.collectAsState()
    val diag by c.diagnostics.collectAsState()
    var label by remember { mutableStateOf("") }

    Column(Modifier.verticalScroll(rememberScrollState()).padding(16.dp)) {
        TextButton(onClick = onBack, modifier = Modifier.heightIn(min = 48.dp)) { Text("‹ Back") }
        Text("Sensitivity setup", style = MaterialTheme.typography.headlineMedium)
        Text(
            "Place the phone where it will sit during practice. Squib measures ambient sound for 3 seconds, then " +
                "listens for a few test impulses made during normal, supervised range operation. The result is " +
                "saved for this phone, audio route, and sample rate only. Audio is processed in memory and not recorded.",
        )
        Spacer(Modifier.height(12.dp))
        val p = probe
        if (p == null) {
            OutlinedTextField(label, { label = it.take(80) }, label = { Text("Optional label, e.g. Indoor / behind shooter") },
                singleLine = true, modifier = Modifier.fillMaxWidth())
            Button(onClick = { onRequestMic { c.startCalibration() } }, modifier = Modifier.fillMaxWidth().height(64.dp).padding(top = 8.dp)) {
                Text("Start sensitivity setup")
            }
            OutlinedButton(onClick = { onRequestMic { c.startCueTest() } }, modifier = Modifier.fillMaxWidth().height(56.dp).padding(top = 8.dp)) {
                Text("Cue test: play the start cue and listen for it")
            }
        } else {
            Card(Modifier.fillMaxWidth()) {
                Column(Modifier.padding(12.dp)) {
                    Text("Stage: ${p.stage}", fontWeight = FontWeight.Bold)
                    Text(p.message)
                    p.ambientMedianDbfs?.let { Text(String.format(Locale.US, "Ambient median %.1f dBFS, p99 %.1f dBFS", it, p.ambientP99Dbfs ?: it)) }
                    if (p.impulseRatiosDb.isNotEmpty()) {
                        Text("Test impulses: " + p.impulseRatiosDb.joinToString { String.format(Locale.US, "%.0f dB", it) })
                    }
                }
            }
            if (p.stage == "impulses" || p.stage == "failed") {
                Button(onClick = { c.finishProbe(true, label.ifBlank { null }) }, modifier = Modifier.fillMaxWidth().height(64.dp).padding(top = 8.dp)) {
                    Text("Finish and save")
                }
            }
            OutlinedButton(onClick = { c.finishProbe(false, null) }, modifier = Modifier.fillMaxWidth().height(56.dp).padding(top = 8.dp)) {
                Text(if (p.stage == "done") "Close" else "Cancel")
            }
        }
        last?.let { r ->
            Card(Modifier.fillMaxWidth().padding(top = 12.dp)) {
                Column(Modifier.padding(12.dp)) {
                    Text("Last result", fontWeight = FontWeight.Bold)
                    Text(r.message)
                    r.suggestedThresholdDb?.let { Text("Saved threshold: ${it.toInt()} dB above noise (${r.verdict})") }
                }
            }
        }
        Spacer(Modifier.height(16.dp))
        Text("Capture diagnostics", style = MaterialTheme.typography.titleMedium)
        val d = diag
        if (d == null) {
            Text("No capture in this session yet.")
        } else {
            Text("Blocks: ${d.blocks}  Block: ${d.blockDurationNs / 1000u} µs")
            Text("DSP p50 ${d.dspP50Ns / 1000u} µs, p99 ${d.dspP99Ns / 1000u} µs, max ${d.dspMaxNs / 1000u} µs (budget p99 < ${d.blockDurationNs / 2000u} µs)")
            Text("Queue: capacity ${d.queueCapacityBlocks} blocks, high water ${d.queueHighWaterBlocks}, overflows ${d.queueOverflows}")
            Text("Clock: ${timestampQualityLabel(d.timestampQuality)}; anchors ok ${d.anchorsAccepted}, rejected ${d.anchorsRejected}")
            d.driftPpm?.let { Text(String.format(Locale.US, "Clock drift vs nominal: %.1f ppm", it)) }
            Text("Delivery delay mean ${d.deliveryMeanNs?.div(1_000_000) ?: "—"} ms, max ${d.deliveryMaxNs / 1_000_000} ms (affects display only)")
        }
        Spacer(Modifier.height(8.dp))
        c.coreVersions().forEach { Text(it, style = MaterialTheme.typography.bodySmall) }
    }
}
