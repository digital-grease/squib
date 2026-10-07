package net.digitalgrease.squib.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import net.digitalgrease.squib.core.DelayKind
import net.digitalgrease.squib.core.EngineView
import net.digitalgrease.squib.core.Mode
import net.digitalgrease.squib.core.RouteLevel
import net.digitalgrease.squib.core.SquibController
import net.digitalgrease.squib.core.parsMs
import net.digitalgrease.squib.core.validate

@Composable
fun TimerScreen(
    c: SquibController,
    onRequestMic: (then: () -> Unit) -> Unit,
    onReview: (String) -> Unit,
    onSetup: () -> Unit,
) {
    val v by c.view.collectAsState()
    val s by c.settings.collectAsState()
    val pf by c.preflight.collectAsState()
    val editable = !v.active && v.phase !in setOf("saved", "save_pending", "interrupted", "completing")

    Column(Modifier.verticalScroll(rememberScrollState()).padding(16.dp)) {
        Text("Timer", style = MaterialTheme.typography.headlineMedium)
        Spacer(Modifier.height(8.dp))

        if (editable) {
            Text("Mode", style = MaterialTheme.typography.titleMedium)
            FlowRow {
                FilterChip(
                    selected = s.mode == Mode.PAR_ONLY,
                    onClick = { c.updateSettings { it.copy(mode = Mode.PAR_ONLY) } },
                    label = { Text("Par-only (no microphone)") },
                    modifier = Modifier.padding(end = 8.dp).heightIn(min = 48.dp).testTag("mode_par"),
                )
                FilterChip(
                    selected = s.mode == Mode.PHONE_LIVE,
                    onClick = { c.updateSettings { it.copy(mode = Mode.PHONE_LIVE) } },
                    label = { Text("Live shots (experimental)") },
                    modifier = Modifier.heightIn(min = 48.dp).testTag("mode_live"),
                )
            }
            Spacer(Modifier.height(8.dp))
            Text("Start delay (max 30 s)", style = MaterialTheme.typography.titleMedium)
            FlowRow {
                for (k in DelayKind.entries) {
                    FilterChip(
                        selected = s.delayKind == k,
                        onClick = { c.updateSettings { it.copy(delayKind = k) } },
                        label = { Text(k.name.lowercase().replaceFirstChar { ch -> ch.uppercase() }) },
                        modifier = Modifier.padding(end = 8.dp).heightIn(min = 48.dp).testTag("delay_${k.name.lowercase()}"),
                    )
                }
            }
            when (s.delayKind) {
                DelayKind.FIXED -> SecondsField("Delay (s)", s.fixedMs) { ms -> c.updateSettings { it.copy(fixedMs = ms) } }
                DelayKind.RANDOM -> Row {
                    Box(Modifier.weight(1f)) { SecondsField("Min (s)", s.randomMinMs) { ms -> c.updateSettings { it.copy(randomMinMs = ms) } } }
                    Spacer(Modifier.width(8.dp))
                    Box(Modifier.weight(1f)) { SecondsField("Max (s)", s.randomMaxMs) { ms -> c.updateSettings { it.copy(randomMaxMs = ms) } } }
                }
                DelayKind.INSTANT -> {}
            }
            OutlinedTextField(
                value = s.parsText,
                onValueChange = { t -> c.updateSettings { it.copy(parsText = t) } },
                label = { Text("Par times (s), e.g. 2.0, 3.5") },
                modifier = Modifier.fillMaxWidth().testTag("pars"),
                singleLine = true,
            )
            if (!s.parsMs().isNullOrEmpty()) {
                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.heightIn(min = 48.dp)) {
                    Switch(checked = s.autoStop, onCheckedChange = { b -> c.updateSettings { it.copy(autoStop = b) } })
                    Spacer(Modifier.width(8.dp))
                    Text("Auto stop 1 s after last par")
                }
            }
            if (s.mode == Mode.PHONE_LIVE) {
                OutlinedTextField(
                    value = s.expectedCountText,
                    onValueChange = { t -> c.updateSettings { it.copy(expectedCountText = t.filter(Char::isDigit).take(4)) } },
                    label = { Text("Expected shots (review hint only)") },
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true,
                )
                SensitivityRow(s.manualThresholdDb, pf?.calibrationThresholdDb, pf?.calibrationLabel) { t ->
                    c.updateSettings { it.copy(manualThresholdDb = t) }
                }
                Row {
                    OutlinedButton(onClick = onSetup, modifier = Modifier.heightIn(min = 48.dp)) { Text("Sensitivity setup & cue test") }
                }
            }
            Spacer(Modifier.height(8.dp))
            pf?.let { p ->
                Card(Modifier.fillMaxWidth()) {
                    Column(Modifier.padding(12.dp)) {
                        Text(
                            when (p.level) {
                                RouteLevel.QUALIFIED -> "Route: qualified"
                                RouteLevel.EXPERIMENTAL -> "Route: experimental (not field-qualified)"
                                RouteLevel.UNSUPPORTED -> "Route: not usable for this mode"
                            },
                            fontWeight = FontWeight.Bold,
                            color = if (p.level == RouteLevel.UNSUPPORTED) MaterialTheme.colorScheme.error else Color.Unspecified,
                        )
                        p.messages.forEach { m -> Text("• $m", style = MaterialTheme.typography.bodyMedium) }
                    }
                }
            }
            s.validate()?.let { Text(it, color = MaterialTheme.colorScheme.error, modifier = Modifier.padding(top = 8.dp)) }
        }

        Spacer(Modifier.height(12.dp))
        StatusPanel(v)
        Spacer(Modifier.height(12.dp))
        MainAction(v, s.validate() == null && (pf?.canArm != false), onArm = {
            if (s.mode == Mode.PHONE_LIVE) onRequestMic { c.arm() } else c.arm()
        }, c = c)

        if (v.phase in setOf("saved", "save_pending", "cancelled", "failed")) {
            Spacer(Modifier.height(12.dp))
            ResultActions(v, c, onReview)
        }
    }
}

@Composable
private fun SecondsField(label: String, ms: Int, onChange: (Int) -> Unit) {
    // Local text keeps partial input such as "2." editable; valid values propagate.
    var text by remember { mutableStateOf(if (ms % 1000 == 0) (ms / 1000).toString() else (ms / 1000.0).toString()) }
    OutlinedTextField(
        value = text,
        onValueChange = { t ->
            text = t.filter { it.isDigit() || it == '.' }.take(6)
            text.toDoubleOrNull()?.let { onChange((it * 1000).toInt()) }
        },
        label = { Text(label) },
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal),
        modifier = Modifier.fillMaxWidth(),
        singleLine = true,
    )
}

@Composable
private fun SensitivityRow(manual: Float?, calibrated: Float?, label: String?, onChange: (Float?) -> Unit) {
    Column(Modifier.padding(vertical = 8.dp)) {
        val effective = manual ?: calibrated ?: 24f
        Text(
            "Sensitivity threshold: ${effective.toInt()} dB above noise " +
                when {
                    manual != null -> "(manual)"
                    calibrated != null -> "(setup${label?.let { ": $it" } ?: ""})"
                    else -> "(default)"
                },
        )
        Text("Lower = more sensitive (more false events). Advanced.", style = MaterialTheme.typography.bodySmall)
        Slider(value = effective, onValueChange = { onChange(it) }, valueRange = 12f..42f, steps = 29)
        if (manual != null) {
            OutlinedButton(onClick = { onChange(null) }) { Text("Use setup/default") }
        }
    }
}

@Composable
private fun StatusPanel(v: EngineView) {
    val (title, detail) = when (v.phase) {
        "ready" -> "Ready" to "Arm when in position."
        "preparing" -> "Preparing" to if (v.mode == Mode.PHONE_LIVE) "Opening microphone and measuring ambient sound…" else "Saving run setup…"
        "armed" -> "Armed: wait for the start cue" to "The start comes after the delay. No countdown is shown."
        "awaiting_cue" -> "Start cue" to "Listening for the cue…"
        "running" -> "GO" to if (v.mode == Mode.PHONE_LIVE) "Shots detected: ${v.acceptedCount}" +
            (if (v.uncertainCount > 0u) " (+${v.uncertainCount} uncertain)" else "") else "Par timer running"
        "completing" -> "Stopping" to "Finishing detection and saving…"
        "interrupted" -> "Interrupted" to (v.interruptReason ?: "Capture was interrupted.")
        "saved" -> "Saved · ${outcomeLabel(v.outcome)}" to (v.interruptReason ?: "")
        "save_pending" -> "Not saved" to "Saving failed: ${v.error ?: "unknown error"}. Retry below."
        "cancelled" -> "Cancelled" to "No timing was recorded."
        "failed" -> "Could not start" to (v.error ?: "")
        "calibrating" -> "Setup in progress" to ""
        else -> v.phase to ""
    }
    val running = v.phase == "running"
    Box(
        Modifier.fillMaxWidth()
            .background(if (running) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surface)
            .padding(16.dp)
            .semantics { liveRegion = LiveRegionMode.Polite }
            .testTag("status"),
    ) {
        Column {
            Text(
                title,
                fontSize = if (running) 48.sp else 28.sp,
                fontWeight = FontWeight.Black,
                color = if (running) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurface,
            )
            if (detail.isNotEmpty()) {
                Text(detail, color = if (running) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurface)
            }
            if (v.mode == Mode.PHONE_LIVE && (running || v.phase == "saved")) {
                Text("First: ${fmtSec(v.firstNs)}   Last: ${fmtSec(v.lastNs)}",
                    color = if (running) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurface)
            }
            if (v.parsTotal > 0u && (running || v.phase == "saved")) {
                Text("Par cues played: ${v.parsPlayed}/${v.parsTotal}",
                    color = if (running) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurface)
            }
            if (v.qualityLabels.isNotEmpty()) {
                FlowRow(Modifier.padding(top = 4.dp)) { v.qualityLabels.distinct().forEach { Badge(it, warning = true) } }
            }
            v.persist?.let {
                Text(
                    when (it) {
                        "saved" -> "Saved on this device"
                        "pending" -> "Saving…"
                        else -> "Not saved"
                    },
                    style = MaterialTheme.typography.bodySmall,
                    color = if (running) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurface,
                )
            }
        }
    }
}

@Composable
private fun MainAction(v: EngineView, canArm: Boolean, onArm: () -> Unit, c: SquibController) {
    val big = Modifier.fillMaxWidth().height(96.dp)
    when (v.phase) {
        "preparing", "armed", "awaiting_cue" -> Button(
            onClick = { c.cancel() },
            modifier = big.testTag("cancel"),
            colors = ButtonDefaults.buttonColors(containerColor = MaterialTheme.colorScheme.error),
        ) { Text("CANCEL", fontSize = 32.sp, fontWeight = FontWeight.Black) }
        "running" -> Button(
            onClick = { c.stop() },
            modifier = big.testTag("stop"),
            colors = ButtonDefaults.buttonColors(containerColor = MaterialTheme.colorScheme.error),
        ) { Text("STOP", fontSize = 40.sp, fontWeight = FontWeight.Black) }
        "completing", "interrupted" -> Button(onClick = {}, enabled = false, modifier = big) { Text("Saving…", fontSize = 28.sp) }
        "saved", "save_pending", "cancelled", "failed" -> {}
        else -> Button(onClick = onArm, enabled = canArm, modifier = big.testTag("arm")) {
            Text("ARM", fontSize = 40.sp, fontWeight = FontWeight.Black)
        }
    }
}

@Composable
private fun ResultActions(v: EngineView, c: SquibController, onReview: (String) -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        if (v.phase == "save_pending") {
            Button(onClick = { c.retrySave() }, modifier = Modifier.fillMaxWidth().height(64.dp).testTag("retry_save")) {
                Text("Retry save", fontSize = 22.sp)
            }
        }
        if (v.phase == "saved" && v.runId != null && v.mode == Mode.PHONE_LIVE) {
            OutlinedButton(onClick = { onReview(v.runId!!) }, modifier = Modifier.fillMaxWidth().height(56.dp).testTag("review")) {
                Text("Review / correct")
            }
        }
        if (v.phase != "save_pending") {
            Button(onClick = { c.repeat() }, modifier = Modifier.fillMaxWidth().height(72.dp).testTag("repeat")) {
                Text("REPEAT", fontSize = 28.sp, fontWeight = FontWeight.Black)
            }
            OutlinedButton(onClick = { c.dismissResult() }, modifier = Modifier.fillMaxWidth().height(56.dp).testTag("new_setup")) {
                Text("Change setup")
            }
        }
    }
}
