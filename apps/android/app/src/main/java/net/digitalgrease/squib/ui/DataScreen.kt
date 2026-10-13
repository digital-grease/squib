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
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import java.util.Locale
import net.digitalgrease.squib.core.backupPassphraseProblem
import net.digitalgrease.squib.data.DataController

/** Backup, restore, CSV, deletion, round cost, and problem reports. */
@Composable
fun DataScreen(
    d: DataController,
    onBack: () -> Unit,
    onExportBackup: (includePhotos: Boolean, includeAudio: Boolean, passphrase: String?) -> Unit,
    onImport: () -> Unit,
    onExportCsv: () -> Unit,
    onReportProblem: () -> Unit,
    onOpenCompatibility: (String) -> Unit,
    onCopyText: (String) -> Unit,
    onAbout: () -> Unit,
) {
    val rounds by d.rounds.collectAsState()
    val pending by d.pendingImport.collectAsState()
    val prompt by d.passphrasePrompt.collectAsState()
    var photos by remember { mutableStateOf(true) }
    var audio by remember { mutableStateOf(false) }
    // Passphrases live in plain `remember`, never in saved instance state.
    var encrypt by remember { mutableStateOf(false) }
    var pass1 by remember { mutableStateOf("") }
    var pass2 by remember { mutableStateOf("") }
    var confirmDelete by remember { mutableStateOf(0) }
    var cost by remember { mutableStateOf("") }
    var currency by remember { mutableStateOf("USD") }
    Column(Modifier.verticalScroll(rememberScrollState()).padding(16.dp)) {
        BackButton("Back", "Back to History", onBack)
        Text("Data and backup", style = MaterialTheme.typography.headlineMedium)
        OutlinedButton(onClick = onAbout, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).padding(vertical = 6.dp).testTag("open_about")) {
            Text("About, licences and support")
        }

        Card(Modifier.fillMaxWidth().padding(vertical = 6.dp)) {
            Column(Modifier.padding(12.dp)) {
                Text("Private backup", fontWeight = FontWeight.Bold)
                Text(
                    "A full copy of your history to restore on this or another phone. It is private: it can include saved places, notes, photos, and videos. " +
                        "Unless you encrypt it with a passphrase, anyone who gets the file can read it. Store it somewhere you trust.",
                    style = MaterialTheme.typography.bodySmall,
                )
                LabeledToggle("Include photos and videos", photos, { photos = it }, controlAtEnd = true)
                LabeledToggle("Include diagnostic recordings (audio)", audio, { audio = it }, controlAtEnd = true, tag = "backup_audio")
                LabeledToggle("Encrypt with a passphrase", encrypt, { encrypt = it; if (!it) { pass1 = ""; pass2 = "" } }, controlAtEnd = true, tag = "backup_encrypt")
                val problem = if (encrypt) passphraseProblem(pass1, pass2) else null
                if (encrypt) {
                    Text(
                        "The backup is encrypted with age, a standard file encryption format, using this passphrase. Without the passphrase nobody, " +
                            "including you, can restore it: there is no recovery. Several unrelated words make a strong passphrase you can remember.",
                        style = MaterialTheme.typography.bodySmall,
                    )
                    PassphraseField(pass1, { pass1 = it }, "Passphrase", "backup_pass1")
                    PassphraseField(pass2, { pass2 = it }, "Passphrase again", "backup_pass2")
                    if (problem != null && (pass1.isNotEmpty() || pass2.isNotEmpty())) {
                        Text(problem, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall, modifier = Modifier.testTag("backup_pass_problem"))
                    }
                }
                Button(
                    onClick = { onExportBackup(photos, audio, if (encrypt) pass1 else null) },
                    enabled = problem == null,
                    modifier = Modifier.fillMaxWidth().heightIn(min = 56.dp).testTag("backup_save"),
                ) { Text(if (encrypt) "Save encrypted backup" else "Save backup") }
                OutlinedButton(onClick = onImport, modifier = Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(top = 6.dp).testTag("backup_restore")) { Text("Restore from backup") }
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
        CompatReportCard(d, onOpenCompatibility, onCopyText)
        Card(Modifier.fillMaxWidth().padding(vertical = 6.dp)) {
            Column(Modifier.padding(12.dp)) {
                Text("Delete all history", fontWeight = FontWeight.Bold, color = MaterialTheme.colorScheme.error)
                Text("Removes every run, drill, saved place, photo, video, ballistics load, range card, and cached weather on this phone. Backups you saved elsewhere are not affected.", style = MaterialTheme.typography.bodySmall)
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
    prompt?.let { p ->
        var entered by remember(p.file) { mutableStateOf("") }
        AlertDialog(
            onDismissRequest = d::cancelImport,
            confirmButton = {
                TextButton(
                    onClick = { d.unlockImport(entered) },
                    enabled = entered.isNotEmpty() && !p.checking,
                    modifier = Modifier.testTag("import_unlock"),
                ) { Text(if (p.checking) "Checking…" else "Unlock") }
            },
            dismissButton = { TextButton(onClick = d::cancelImport) { Text("Cancel") } },
            title = { Text("Encrypted backup") },
            text = {
                Column {
                    Text("This backup is encrypted. Enter the passphrase it was saved with.")
                    PassphraseField(entered, { entered = it }, "Passphrase", "import_passphrase")
                    p.error?.let {
                        Text(
                            it.replaceFirstChar { c -> c.uppercase() } + ".",
                            color = MaterialTheme.colorScheme.error,
                            modifier = Modifier.testTag("import_pass_error"),
                        )
                    }
                }
            },
        )
    }
    pending?.let { (_, v) ->
        AlertDialog(
            onDismissRequest = d::cancelImport,
            confirmButton = {
                if (v.conflicts.isEmpty()) {
                    TextButton(onClick = d::confirmImport, modifier = Modifier.testTag("import_restore")) { Text("Restore") }
                } else {
                    TextButton(onClick = d::confirmImportCopy, modifier = Modifier.testTag("import_copy")) { Text("Import as a copy") }
                }
            },
            dismissButton = { TextButton(onClick = d::cancelImport) { Text("Cancel") } },
            text = {
                Text(
                    if (v.conflicts.isNotEmpty()) {
                        "This backup conflicts with ${v.conflicts.size} records on this phone: the same items were changed differently " +
                            "(for example the same run reviewed on two phones), so it cannot be restored as-is. Nothing was changed. " +
                            "You can import it as a copy instead: everything in it is added alongside what is here, with new ids, and nothing on this phone is overwritten. " +
                            "Runs that are in both will then appear twice."
                    } else {
                        "Backup checked. It adds ${v.runsNew} runs (${v.runsAlreadyPresent} already here)" +
                            (if (v.containsLocation) " and contains saved places or location detail." else ".") + " Restore it?"
                    },
                )
            },
        )
    }
}

/** Why the two typed passphrases cannot be used yet, or null when they can. */
private fun passphraseProblem(p1: String, p2: String): String? =
    backupPassphraseProblem(p1)?.let { "Passphrase: $it." } ?: if (p1 != p2) "The passphrases do not match." else null

@Composable
private fun PassphraseField(value: String, onChange: (String) -> Unit, label: String, tag: String) {
    var visible by remember { mutableStateOf(false) }
    OutlinedTextField(
        value, onChange,
        label = { Text(label) },
        singleLine = true,
        visualTransformation = if (visible) VisualTransformation.None else PasswordVisualTransformation(),
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, autoCorrectEnabled = false),
        trailingIcon = {
            TextButton(onClick = { visible = !visible }, modifier = Modifier.heightIn(min = 48.dp)) { Text(if (visible) "Hide" else "Show") }
        },
        modifier = Modifier.fillMaxWidth().padding(top = 4.dp).testTag(tag),
    )
}
