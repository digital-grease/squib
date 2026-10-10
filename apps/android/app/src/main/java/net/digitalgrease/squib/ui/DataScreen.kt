package net.digitalgrease.squib.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import java.util.Locale
import net.digitalgrease.squib.data.DataController

/** Backup, restore, CSV, deletion, round cost, and problem reports. */
@Composable
fun DataScreen(
    d: DataController,
    onBack: () -> Unit,
    onExportBackup: (includePhotos: Boolean) -> Unit,
    onImport: () -> Unit,
    onExportCsv: () -> Unit,
    onReportProblem: () -> Unit,
) {
    val rounds by d.rounds.collectAsState()
    val pending by d.pendingImport.collectAsState()
    var photos by remember { mutableStateOf(true) }
    var confirmDelete by remember { mutableStateOf(0) }
    var cost by remember { mutableStateOf("") }
    var currency by remember { mutableStateOf("USD") }
    Column(Modifier.verticalScroll(rememberScrollState()).padding(16.dp)) {
        TextButton(onClick = onBack, modifier = Modifier.heightIn(min = 48.dp)) { Text("‹ Back") }
        Text("Data and backup", style = MaterialTheme.typography.headlineMedium)

        Card(Modifier.fillMaxWidth().padding(vertical = 6.dp)) {
            Column(Modifier.padding(12.dp)) {
                Text("Private backup", fontWeight = FontWeight.Bold)
                Text(
                    "A full copy of your history to restore on this or another phone. It is not encrypted and is private: it can include saved places, notes, photos, and videos. Store it somewhere you trust.",
                    style = MaterialTheme.typography.bodySmall,
                )
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text("Include photos and videos", modifier = Modifier.weight(1f))
                    Switch(checked = photos, onCheckedChange = { photos = it })
                }
                Button(onClick = { onExportBackup(photos) }, modifier = Modifier.fillMaxWidth().heightIn(min = 56.dp)) { Text("Save backup") }
                OutlinedButton(onClick = onImport, modifier = Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(top = 6.dp)) { Text("Restore from backup") }
            }
        }
        Card(Modifier.fillMaxWidth().padding(vertical = 6.dp)) {
            Column(Modifier.padding(12.dp)) {
                Text("Spreadsheet export (CSV)", fontWeight = FontWeight.Bold)
                Text("One row per run with units in the column names. For analysis only; it cannot restore history.", style = MaterialTheme.typography.bodySmall)
                OutlinedButton(onClick = onExportCsv, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) { Text("Export CSV") }
            }
        }
        Card(Modifier.fillMaxWidth().padding(vertical = 6.dp)) {
            Column(Modifier.padding(12.dp)) {
                Text("Rounds and cost", fontWeight = FontWeight.Bold)
                rounds?.let { r ->
                    Text("Confirmed rounds: ${r.confirmedRounds}" + if (r.unconfirmedRuns > 0u) " (${r.unconfirmedRuns} runs not yet confirmed)" else "")
                    if (r.costMinor != null) Text(String.format(Locale.US, "Cost: %.2f %s", r.costMinor!! / 100.0, r.currency ?: ""))
                }
                Row {
                    OutlinedTextField(cost, { cost = it }, label = { Text("Cost per round") }, singleLine = true,
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal), modifier = Modifier.weight(1f))
                    OutlinedTextField(currency, { currency = it.take(3).uppercase() }, label = { Text("Currency") }, singleLine = true, modifier = Modifier.weight(0.6f))
                }
                OutlinedButton(onClick = {
                    val c = cost.toDoubleOrNull()
                    d.setCost(c?.let { Math.round(it * 100) }, if (c == null) null else currency)
                }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Save cost") }
            }
        }
        Card(Modifier.fillMaxWidth().padding(vertical = 6.dp)) {
            Column(Modifier.padding(12.dp)) {
                Text("Report a problem", fontWeight = FontWeight.Bold)
                Text(
                    "Opens a GitHub issue form in your browser, pre-filled with app versions, device model, timing diagnostics, and recent run outcomes. You review it there before submitting. It never includes audio, location, station names, notes, or run identifiers.",
                    style = MaterialTheme.typography.bodySmall,
                )
                OutlinedButton(onClick = onReportProblem, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) { Text("Report a problem on GitHub") }
            }
        }
        Card(Modifier.fillMaxWidth().padding(vertical = 6.dp)) {
            Column(Modifier.padding(12.dp)) {
                Text("Delete all history", fontWeight = FontWeight.Bold, color = MaterialTheme.colorScheme.error)
                Text("Removes every run, drill, saved place, photo, video, and cached weather on this phone. Backups you saved elsewhere are not affected.", style = MaterialTheme.typography.bodySmall)
                Button(onClick = { confirmDelete = 1 }, colors = ButtonDefaults.buttonColors(containerColor = MaterialTheme.colorScheme.error),
                    modifier = Modifier.heightIn(min = 48.dp)) { Text("Delete all history") }
            }
        }
    }
    if (confirmDelete > 0) {
        AlertDialog(
            onDismissRequest = { confirmDelete = 0 },
            confirmButton = {
                TextButton(onClick = {
                    if (confirmDelete == 1) confirmDelete = 2 else { confirmDelete = 0; d.deleteAll() }
                }) { Text(if (confirmDelete == 1) "Continue" else "Delete everything") }
            },
            dismissButton = { TextButton(onClick = { confirmDelete = 0 }) { Text("Cancel") } },
            text = { Text(if (confirmDelete == 1) "Delete all history on this phone?" else "This cannot be undone. Delete everything now?") },
        )
    }
    pending?.let { (_, v) ->
        AlertDialog(
            onDismissRequest = d::cancelImport,
            confirmButton = { TextButton(onClick = d::confirmImport, enabled = v.conflicts.isEmpty()) { Text("Restore") } },
            dismissButton = { TextButton(onClick = d::cancelImport) { Text("Cancel") } },
            text = {
                Text(
                    if (v.conflicts.isNotEmpty()) "This backup conflicts with ${v.conflicts.size} different records already on this phone, so it cannot be restored as-is. Nothing was changed."
                    else "Backup checked. It adds ${v.runsNew} runs (${v.runsAlreadyPresent} already here)" +
                        (if (v.containsLocation) " and contains saved places or location detail." else ".") + " Restore it?",
                )
            },
        )
    }
}
