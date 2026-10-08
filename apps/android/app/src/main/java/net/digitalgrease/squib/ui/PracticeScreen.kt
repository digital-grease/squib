package net.digitalgrease.squib.ui

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
import androidx.compose.material3.FilterChip
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
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import java.util.Locale
import kotlinx.coroutines.launch
import net.digitalgrease.squib.core.DrillInput
import net.digitalgrease.squib.core.DrillView
import net.digitalgrease.squib.core.Mode
import net.digitalgrease.squib.core.StartDelay
import net.digitalgrease.squib.data.DataController

private fun delayText(d: StartDelay) = when (d) {
    is StartDelay.Instant -> "instant start"
    is StartDelay.Fixed -> String.format(Locale.US, "%.1f s delay", d.ms.toInt() / 1000.0)
    is StartDelay.Random -> String.format(Locale.US, "%.1f-%.1f s random delay", d.minMs.toInt() / 1000.0, d.maxMs.toInt() / 1000.0)
}

@Composable
fun PracticeScreen(
    d: DataController,
    onStartDrill: (DrillView) -> Unit,
    onShareDrill: (DrillView, ByteArray) -> Unit,
    onImportDrill: () -> Unit,
    onOpenRun: (String) -> Unit,
) {
    val drills by d.drills.collectAsState()
    var editing by remember { mutableStateOf<DrillView?>(null) }
    var creating by remember { mutableStateOf(false) }
    var manualFor by remember { mutableStateOf<DrillView?>(null) }
    var manualOpen by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    if (creating || editing != null) {
        DrillEditor(d, editing, onDone = { creating = false; editing = null })
        return
    }
    if (manualOpen) {
        ManualEntry(d, manualFor, onDone = { id -> manualOpen = false; manualFor = null; id?.let(onOpenRun) })
        return
    }
    Column(Modifier.verticalScroll(rememberScrollState()).padding(16.dp).testTag("practice")) {
        Text("Practice", style = MaterialTheme.typography.headlineMedium)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.padding(vertical = 8.dp)) {
            Button(onClick = { creating = true }, modifier = Modifier.heightIn(min = 48.dp)) { Text("New drill") }
            OutlinedButton(onClick = { manualFor = null; manualOpen = true }, modifier = Modifier.heightIn(min = 48.dp).testTag("manual_entry")) {
                Text("Enter a result from another timer")
            }
            OutlinedButton(onClick = onImportDrill, modifier = Modifier.heightIn(min = 48.dp)) { Text("Import drill file") }
        }
        drills.forEach { dr ->
            Card(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
                Column(Modifier.padding(12.dp)) {
                    Text(dr.input.title, fontWeight = FontWeight.Bold)
                    if (dr.input.description.isNotBlank()) Text(dr.input.description, style = MaterialTheme.typography.bodySmall)
                    val parts = mutableListOf(
                        if (dr.input.manual) "manual result" else if (dr.input.mode == Mode.PHONE_LIVE) "live shots (experimental)" else "par-only",
                        delayText(dr.input.delay),
                    )
                    if (dr.input.parsMs.isNotEmpty()) parts += "pars " + dr.input.parsMs.joinToString { String.format(Locale.US, "%.2f", it.toInt() / 1000.0) } + " s"
                    if (dr.input.repeats > 1u) parts += "${dr.input.repeats} strings, ${dr.input.restS} s rest"
                    parts += dr.scoringTitle
                    Text(parts.joinToString(" · "), style = MaterialTheme.typography.bodyMedium)
                    Text("Version ${dr.version}", style = MaterialTheme.typography.bodySmall)
                    FlowRow {
                        if (dr.input.manual) {
                            Button(onClick = { manualFor = dr; manualOpen = true }, modifier = Modifier.padding(end = 8.dp).heightIn(min = 48.dp)) { Text("Enter result") }
                        } else {
                            Button(onClick = { onStartDrill(dr) }, modifier = Modifier.padding(end = 8.dp).heightIn(min = 48.dp).testTag("start_${dr.drillId}")) { Text("Start") }
                        }
                        OutlinedButton(onClick = { editing = dr }, modifier = Modifier.padding(end = 8.dp).heightIn(min = 48.dp)) { Text("Edit") }
                        TextButton(onClick = { scope.launch { onShareDrill(dr, d.drillFile(dr)) } }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Share") }
                        TextButton(onClick = { d.archiveDrill(dr.drillId) }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Archive") }
                    }
                }
            }
        }
        Text(
            "Drills are data: editing creates a new version, and earlier runs keep the version they used. Scoring here is generic practice, not an official rule set.",
            style = MaterialTheme.typography.bodySmall,
            modifier = Modifier.padding(top = 8.dp),
        )
    }
}

@Composable
private fun DrillEditor(d: DataController, existing: DrillView?, onDone: () -> Unit) {
    val e = existing?.input
    var title by remember { mutableStateOf(e?.title ?: "") }
    var desc by remember { mutableStateOf(e?.description ?: "") }
    var mode by remember { mutableStateOf(when { e?.manual == true -> "manual"; e?.mode == Mode.PHONE_LIVE -> "live"; else -> "par" }) }
    var minS by remember { mutableStateOf(((e?.delay as? StartDelay.Random)?.minMs?.toInt() ?: 2000).let { (it / 1000.0).toString() }) }
    var maxS by remember { mutableStateOf(((e?.delay as? StartDelay.Random)?.maxMs?.toInt() ?: 4000).let { (it / 1000.0).toString() }) }
    var pars by remember { mutableStateOf(e?.parsMs?.joinToString(", ") { (it.toInt() / 1000.0).toString() } ?: "") }
    var repeats by remember { mutableStateOf((e?.repeats ?: 1u).toString()) }
    var rest by remember { mutableStateOf((e?.restS ?: 0u).toString()) }
    var profile by remember { mutableStateOf(e?.scoringProfileId ?: "generic-time") }
    var tags by remember { mutableStateOf(e?.equipmentTags?.joinToString(", ") ?: "") }
    var error by remember { mutableStateOf<String?>(null) }
    Column(Modifier.verticalScroll(rememberScrollState()).padding(16.dp)) {
        TextButton(onClick = onDone) { Text("‹ Cancel") }
        Text(if (existing == null) "New drill" else "Edit drill (creates version ${existing.version + 1u})", style = MaterialTheme.typography.headlineSmall)
        OutlinedTextField(title, { title = it.take(80) }, label = { Text("Title") }, singleLine = true, modifier = Modifier.fillMaxWidth())
        OutlinedTextField(desc, { desc = it.take(1000) }, label = { Text("Description") }, modifier = Modifier.fillMaxWidth())
        FlowRow {
            listOf("par" to "Par-only", "live" to "Live shots", "manual" to "Manual result").forEach { (k, l) ->
                FilterChip(selected = mode == k, onClick = { mode = k }, label = { Text(l) }, modifier = Modifier.padding(end = 8.dp))
            }
        }
        if (mode != "manual") {
            Row {
                OutlinedTextField(minS, { minS = it }, label = { Text("Min delay (s)") }, singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal), modifier = Modifier.weight(1f))
                Spacer(Modifier.padding(4.dp))
                OutlinedTextField(maxS, { maxS = it }, label = { Text("Max delay (s)") }, singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal), modifier = Modifier.weight(1f))
            }
            OutlinedTextField(pars, { pars = it }, label = { Text("Par times (s), comma separated") }, singleLine = true, modifier = Modifier.fillMaxWidth())
            Row {
                OutlinedTextField(repeats, { repeats = it.filter(Char::isDigit).take(2) }, label = { Text("Strings") }, singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number), modifier = Modifier.weight(1f))
                Spacer(Modifier.padding(4.dp))
                OutlinedTextField(rest, { rest = it.filter(Char::isDigit).take(3) }, label = { Text("Rest between (s)") }, singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number), modifier = Modifier.weight(1f))
            }
        }
        Text("Scoring (generic practice)", fontWeight = FontWeight.Bold, modifier = Modifier.padding(top = 8.dp))
        FlowRow {
            d.profiles.forEach { p ->
                FilterChip(selected = profile == p.id, onClick = { profile = p.id }, label = { Text(p.title) }, modifier = Modifier.padding(end = 8.dp))
            }
        }
        OutlinedTextField(tags, { tags = it.take(330) }, label = { Text("Equipment tags (optional, comma separated)") }, singleLine = true, modifier = Modifier.fillMaxWidth())
        error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        Button(onClick = {
            val parsMs = if (pars.isBlank()) emptyList() else pars.split(',').mapNotNull { it.trim().toDoubleOrNull()?.let { s -> (s * 1000).toInt().toUInt() } }
            val minMs = ((minS.toDoubleOrNull() ?: -1.0) * 1000).toInt()
            val maxMs = ((maxS.toDoubleOrNull() ?: -1.0) * 1000).toInt()
            if (mode != "manual" && (minMs < 0 || maxMs < minMs)) {
                error = "Delays must be non-negative with min ≤ max."
                return@Button
            }
            val input = DrillInput(
                title = title, description = desc,
                mode = if (mode == "live") Mode.PHONE_LIVE else Mode.PAR_ONLY,
                manual = mode == "manual",
                delay = if (mode == "manual") StartDelay.Instant else StartDelay.Random(minMs.toUInt(), maxMs.toUInt()),
                expectedCount = null, parsMs = if (mode == "manual") emptyList() else parsMs,
                repeats = (repeats.toIntOrNull() ?: 1).coerceAtLeast(0).toUInt(), restS = (rest.toIntOrNull() ?: 0).toUInt(),
                scoringProfileId = profile, notes = "", equipmentTags = tags.split(',').map { it.trim() }.filter { it.isNotEmpty() },
            )
            if (existing == null) d.createDrill(input) else d.editDrill(existing.drillId, input)
            onDone()
        }, modifier = Modifier.fillMaxWidth().height(56.dp).padding(top = 8.dp)) { Text("Save") }
    }
}

@Composable
private fun ManualEntry(d: DataController, drill: DrillView?, onDone: (String?) -> Unit) {
    var label by remember { mutableStateOf("") }
    var precision by remember { mutableStateOf(10) }
    var times by remember { mutableStateOf("") }
    Column(Modifier.verticalScroll(rememberScrollState()).padding(16.dp)) {
        TextButton(onClick = { onDone(null) }) { Text("‹ Cancel") }
        Text("Enter a result from another timer", style = MaterialTheme.typography.headlineSmall)
        drill?.let { Text("Drill: ${it.input.title} (v${it.version})") }
        Text("Times are kept at the timer's own precision and marked as manual; no detection is involved.", style = MaterialTheme.typography.bodySmall)
        OutlinedTextField(label, { label = it.take(80) }, label = { Text("Timer name, e.g. Club timer") }, singleLine = true, modifier = Modifier.fillMaxWidth().testTag("manual_label"))
        Text("Timer precision", modifier = Modifier.padding(top = 8.dp))
        FlowRow {
            listOf(1 to "0.001 s", 10 to "0.01 s", 100 to "0.1 s").forEach { (ms, l) ->
                FilterChip(selected = precision == ms, onClick = { precision = ms }, label = { Text(l) }, modifier = Modifier.padding(end = 8.dp))
            }
        }
        OutlinedTextField(
            times, { times = it }, label = { Text("Shot times from the timer's start (s), e.g. 1.42 1.83 2.25") },
            modifier = Modifier.fillMaxWidth().testTag("manual_times"),
        )
        Button(onClick = { d.addManual(label, precision, times, drill) { id -> onDone(id) } },
            modifier = Modifier.fillMaxWidth().height(56.dp).padding(top = 8.dp).testTag("manual_save")) { Text("Save result") }
    }
}
