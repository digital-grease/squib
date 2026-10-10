package net.digitalgrease.squib.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import java.util.Locale
import net.digitalgrease.squib.conditions.Units
import net.digitalgrease.squib.core.RunListItem
import net.digitalgrease.squib.data.DataController

/** One range visit: notes, totals at the price saved with it, conditions, and its runs. */
@Composable
fun SessionScreen(d: DataController, sessionId: String, runActive: Boolean, onBack: () -> Unit, onOpenRun: (String) -> Unit) {
    val sessions by d.sessions.collectAsState()
    val s = sessions.firstOrNull { it.id == sessionId }
    var runs by remember { mutableStateOf<List<RunListItem>>(emptyList()) }
    var confirmDelete by remember { mutableStateOf(false) }
    LaunchedEffect(sessionId, s?.runCount) { runs = d.sessionRuns(sessionId) }
    if (s == null) {
        Column(Modifier.padding(16.dp)) {
            TextButton(onClick = onBack) { Text("‹ History") }
            Text("This visit no longer exists.")
        }
        return
    }
    val us = Locale.getDefault().country == "US"
    var notes by remember(s.id, s.notes) { mutableStateOf(s.notes) }
    var price by remember(s.id, s.perRoundMinor) { mutableStateOf(s.perRoundMinor?.let { String.format(Locale.US, "%.2f", it / 100.0) } ?: "") }
    var currency by remember(s.id, s.currency) { mutableStateOf(s.currency ?: "") }
    LazyColumn(Modifier.padding(horizontal = 16.dp).testTag("session_screen")) {
        item {
            TextButton(onClick = onBack) { Text("‹ History") }
            Text("Visit", style = MaterialTheme.typography.headlineMedium)
            Text(fmtDate(s.startedUtcMs) + (s.endedUtcMs?.let { " to " + fmtDate(it) } ?: "") + " · " + s.shooterName)
            FlowRow {
                Badge(if (s.active) "In progress" else "Ended")
                Badge("${s.runCount} run(s), ${s.completeRuns} complete")
            }
            Card(Modifier.fillMaxWidth().padding(vertical = 8.dp)) {
                Column(Modifier.padding(12.dp)) {
                    Text("Rounds", fontWeight = FontWeight.Bold)
                    Text("${s.confirmedRounds} confirmed" + (s.costMinor?.let { " · " + money(it, s.currency) } ?: ""), modifier = Modifier.testTag("session_rounds"))
                    if (s.unconfirmedRuns > 0u) {
                        Text("${s.unconfirmedRuns} run(s) still need a confirmed round count; they are not in the total.",
                            style = MaterialTheme.typography.bodySmall)
                    }
                    Text("Price saved with this visit (later price changes do not change it)", style = MaterialTheme.typography.bodySmall)
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        OutlinedTextField(price, { price = it.filter { c -> c.isDigit() || c == '.' }.take(8) }, label = { Text("Per round") },
                            singleLine = true, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal), modifier = Modifier.weight(1f))
                        OutlinedTextField(currency, { currency = it.filter(Char::isLetter).take(3).uppercase() }, label = { Text("Currency") },
                            singleLine = true, modifier = Modifier.weight(1f).padding(start = 8.dp))
                        TextButton(onClick = {
                            val minor = price.toDoubleOrNull()?.let { (it * 100).toLong() }
                            d.setSessionCost(s.id, minor, currency.ifBlank { null }.takeIf { minor != null })
                        }) { Text("Save") }
                    }
                }
            }
            if (s.conditions.isNotEmpty()) {
                Card(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
                    Column(Modifier.padding(12.dp)) {
                        Text("Conditions (latest run)", fontWeight = FontWeight.Bold)
                        s.conditions.forEach { f ->
                            Text("${f.label}: ${Units.format(f.field, f.valueSi, f.valueState, us)} · ${Units.originLabel(f.origin)} · ${Units.age(f.ageMs)}",
                                style = MaterialTheme.typography.bodyMedium)
                        }
                        Text("Each run keeps the conditions pinned when it started.", style = MaterialTheme.typography.bodySmall)
                    }
                }
            }
            OutlinedTextField(notes, { notes = it.take(4000) }, label = { Text("Notes for this visit") },
                modifier = Modifier.fillMaxWidth().padding(top = 8.dp).testTag("session_notes"))
            if (notes != s.notes) {
                TextButton(onClick = { d.setSessionNotes(s.id, notes) }, modifier = Modifier.testTag("session_notes_save")) { Text("Save notes") }
            }
            FlowRow(Modifier.padding(vertical = 8.dp)) {
                if (s.active) {
                    OutlinedButton(onClick = { d.endSession(s.id) }, enabled = !runActive,
                        modifier = Modifier.padding(end = 8.dp).heightIn(min = 48.dp).testTag("session_end")) { Text("End visit") }
                }
                TextButton(onClick = { confirmDelete = true }, enabled = !runActive,
                    modifier = Modifier.heightIn(min = 48.dp).testTag("session_delete")) { Text("Delete visit") }
            }
            Text("Runs", style = MaterialTheme.typography.titleLarge)
        }
        items(runs, key = { it.runId }) { r -> RunCard(r, onOpenRun) }
    }
    if (confirmDelete) {
        AlertDialog(
            onDismissRequest = { confirmDelete = false },
            confirmButton = {
                TextButton(onClick = { confirmDelete = false; d.deleteSession(s.id) { onBack() } },
                    modifier = Modifier.testTag("session_delete_confirm")) { Text("Delete visit") }
            },
            dismissButton = { TextButton(onClick = { confirmDelete = false }) { Text("Keep") } },
            text = { Text("Delete this visit and its ${s.runCount} run(s), with their scores, photos, videos, and recordings? Copies you exported or OS backups are not affected.") },
        )
    }
}
