package net.digitalgrease.squib.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import net.digitalgrease.squib.data.DataController

private val PLACEMENTS = listOf("On a bench or table", "On a tripod or stand", "Held or worn", "Other")
private val ENVIRONMENTS = listOf("Outdoor open range", "Covered bay", "Indoor range", "Other")
private val VERDICTS = listOf("Works well", "Works with corrections", "Unreliable", "Does not work")

@Composable
private fun Choice(title: String, options: List<String>, value: String?, tag: String, onPick: (String) -> Unit) {
    Text(title, style = MaterialTheme.typography.titleSmall, modifier = Modifier.padding(top = 8.dp))
    FlowRow(Modifier.selectableGroup()) {
        options.forEachIndexed { i, o ->
            FilterChip(selected = value == o, onClick = { onPick(o) }, label = { Text(o) },
                modifier = Modifier.oneOf().padding(end = 8.dp).heightIn(min = 48.dp).testTag("${tag}_$i"))
        }
    }
}

/**
 * Shareable compatibility report (plan phase 4.3): the user picks placement, environment and
 * their verdict, sees the exact text, then opens the pre-filled GitHub form or copies it.
 * Nothing is sent by the app.
 */
@Composable
fun CompatReportCard(d: DataController, onOpenForm: (String) -> Unit, onCopy: (String) -> Unit) {
    var placement by remember { mutableStateOf<String?>(null) }
    var environment by remember { mutableStateOf<String?>(null) }
    var verdict by remember { mutableStateOf<String?>(null) }
    var comment by remember { mutableStateOf("") }
    var preview by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    Card(Modifier.fillMaxWidth().padding(vertical = 6.dp).testTag("compat")) {
        Column(Modifier.padding(12.dp)) {
            Text("Share a compatibility report", fontWeight = FontWeight.Bold)
            Text(
                "Helps build the list of phones and audio routes that time shots well. It summarises your recent live runs by " +
                    "audio route (counts, clock quality, sensitivity setup) with the device model, versions, and today's date. " +
                    "No audio, location, notes, run identifiers, or times. You see the full text first.",
                style = MaterialTheme.typography.bodySmall,
            )
            Choice("Where was the phone?", PLACEMENTS, placement, "compat_place") { placement = it; preview = null }
            Choice("Where were you shooting?", ENVIRONMENTS, environment, "compat_env") { environment = it; preview = null }
            Choice("How well did shot timing work?", VERDICTS, verdict, "compat_verdict") { verdict = it; preview = null }
            OutlinedTextField(comment, { comment = it.take(500); preview = null }, label = { Text("Comment (optional, no personal details)") },
                modifier = Modifier.fillMaxWidth().testTag("compat_comment"))
            val ready = placement != null && environment != null && verdict != null
            OutlinedButton(onClick = {
                scope.launch { preview = d.compatibilityReport(placement!!, environment!!, verdict!!, comment) }
            }, enabled = ready, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).padding(top = 8.dp).testTag("compat_preview")) {
                Text("Preview report")
            }
            preview?.let { text ->
                Text(text, fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.bodySmall,
                    modifier = Modifier.padding(vertical = 8.dp).testTag("compat_text"))
                Button(onClick = { onOpenForm(text) }, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("compat_open")) {
                    Text("Open the GitHub form")
                }
                OutlinedButton(onClick = { onCopy(text) }, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("compat_copy")) {
                    Text("Copy the text")
                }
            }
        }
    }
}
