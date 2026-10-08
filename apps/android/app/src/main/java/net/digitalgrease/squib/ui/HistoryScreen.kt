package net.digitalgrease.squib.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import net.digitalgrease.squib.core.SquibController
import net.digitalgrease.squib.data.DataController
import java.util.Locale
import androidx.compose.foundation.layout.Row
import androidx.compose.material3.FilterChip
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.TextButton
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment

@Composable
private fun ShooterRow(d: DataController, locked: Boolean) {
    val shooters by d.shooters.collectAsState()
    var adding by remember { mutableStateOf(false) }
    var name by remember { mutableStateOf("") }
    Text("Shooter", fontWeight = FontWeight.Bold)
    FlowRow {
        shooters.forEach { sh ->
            FilterChip(
                selected = sh.active,
                onClick = { if (!locked) d.setActiveShooter(sh.id) },
                enabled = !locked,
                label = { Text(sh.name) },
                modifier = Modifier.padding(end = 8.dp).heightIn(min = 48.dp),
            )
        }
        TextButton(onClick = { adding = !adding }, enabled = !locked) { Text("+ Add") }
    }
    if (locked) Text("Shooter can't change while a run is in progress.", style = MaterialTheme.typography.bodySmall)
    if (adding) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            OutlinedTextField(name, { name = it.take(40) }, label = { Text("Name") }, singleLine = true, modifier = Modifier.weight(1f))
            TextButton(onClick = { if (name.isNotBlank()) { d.addShooter(name); name = ""; adding = false } }) { Text("Save") }
        }
    }
}

private val EXCLUSION_TEXT = mapOf(
    "other_shooter" to "other shooters",
    "other_drill_or_version" to "other drills or versions",
    "other_mode" to "other modes",
    "other_equipment" to "other equipment",
    "outside_dates" to "outside dates",
    "interrupted" to "interrupted",
    "not_complete" to "not complete",
    "needs_review" to "need review",
    "edited" to "edited",
    "manual" to "manual results",
    "missing_required_score" to "missing score",
)

@Composable
private fun AnalyticsCard(d: DataController) {
    val a by d.analytics.collectAsState()
    val f by d.filter.collectAsState()
    val drills by d.drills.collectAsState()
    Card(Modifier.fillMaxWidth().padding(vertical = 8.dp)) {
        Column(Modifier.padding(12.dp)) {
            Text("Summary", fontWeight = FontWeight.Bold)
            FlowRow {
                FilterChip(selected = f.drillId == null, onClick = { d.setFilter(f.copy(drillId = null, drillVersion = null)) },
                    label = { Text("All runs") }, modifier = Modifier.padding(end = 6.dp))
                drills.forEach { dr ->
                    FilterChip(selected = f.drillId == dr.drillId,
                        onClick = { d.setFilter(f.copy(drillId = dr.drillId, drillVersion = null)) },
                        label = { Text(dr.input.title) }, modifier = Modifier.padding(end = 6.dp))
                }
            }
            FlowRow {
                FilterChip(selected = f.includeEdited, onClick = { d.setFilter(f.copy(includeEdited = !f.includeEdited)) }, label = { Text("Include edited") }, modifier = Modifier.padding(end = 6.dp))
                FilterChip(selected = f.includeManual, onClick = { d.setFilter(f.copy(includeManual = !f.includeManual)) }, label = { Text("Include manual") }, modifier = Modifier.padding(end = 6.dp))
                FilterChip(selected = f.includeNeedsReview, onClick = { d.setFilter(f.copy(includeNeedsReview = !f.includeNeedsReview)) }, label = { Text("Include needs review") })
            }
            val v = a
            if (v == null) {
                Text("No data yet.")
            } else {
                Text("${v.included} of ${v.considered} runs included" + if (v.insufficient) " · too few for a trend (fewer than 5)" else "",
                    modifier = Modifier.testTag("analytics_count"))
                if (v.excluded.isNotEmpty()) {
                    Text("Excluded: " + v.excluded.joinToString { "${it.count} ${EXCLUSION_TEXT[it.reason] ?: it.reason}" }, style = MaterialTheme.typography.bodySmall)
                }
                v.firstShotS?.let { Text(String.format(Locale.US, "First shot: median %.2f s (middle half %.2f-%.2f, n=%d)", it.median, it.q1, it.q3, it.n.toInt())) }
                v.splitsS?.let { Text(String.format(Locale.US, "Splits: median %.2f s (middle half %.2f-%.2f, n=%d)", it.median, it.q1, it.q3, it.n.toInt())) }
                v.finalTimeS?.let { Text(String.format(Locale.US, "Final time: median %.2f s (middle half %.2f-%.2f, n=%d)", it.median, it.q1, it.q3, it.n.toInt())) }
                v.hitFactor?.let { Text(String.format(Locale.US, "Hit factor: median %.3f (n=%d)", it.median, it.n.toInt())) }
                if (v.mixedTimingMethods.isNotEmpty()) {
                    Text("Mixed timing sources: ${v.mixedTimingMethods.joinToString()}; precision differs.", color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
                }
            }
        }
    }
}

@Composable
fun HistoryScreen(c: SquibController, d: DataController, onOpen: (String) -> Unit, onData: () -> Unit) {
    val runs by c.history.collectAsState()
    val view by c.view.collectAsState()
    LaunchedEffect(Unit) {
        c.refreshHistory()
        d.reload()
    }
    LazyColumn(Modifier.padding(horizontal = 16.dp).testTag("history")) {
        item {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("History", style = MaterialTheme.typography.headlineMedium, modifier = Modifier.padding(vertical = 16.dp).weight(1f))
                OutlinedButton(onClick = onData, modifier = Modifier.heightIn(min = 48.dp).testTag("open_data")) { Text("Data and backup") }
            }
            ShooterRow(d, locked = view.active)
            AnalyticsCard(d)
            if (c.recovered.isNotEmpty()) {
                Text(
                    "${c.recovered.size} run(s) were open when the app closed and are saved as Interrupted. " +
                        "Only events saved before closing are kept.",
                    color = MaterialTheme.colorScheme.error,
                    modifier = Modifier.padding(bottom = 8.dp),
                )
            }
            if (runs.isEmpty()) Text("No runs yet.")
        }
        items(runs, key = { it.runId }) { r ->
            Card(
                Modifier.fillMaxWidth().padding(vertical = 4.dp).heightIn(min = 72.dp)
                    .clickable(role = Role.Button, onClickLabel = "Open review") { onOpen(r.runId) }
                    .testTag("run_${r.runId}"),
            ) {
                Column(Modifier.padding(12.dp)) {
                    Text(fmtDate(r.createdUtcMs), fontWeight = FontWeight.Bold)
                    FlowRow {
                        Badge(if (r.mode == "par_only") "Par-only" else "Live (experimental)")
                        Badge(outcomeLabel(r.outcome), warning = r.outcome != "complete")
                        if (r.persist != "saved") Badge("Not saved", warning = true)
                        reviewLabel(r.reviewState, r.edited)?.let { Badge(it, warning = r.reviewState == "needs_review") }
                        if (r.qualityWarnings > 0u) Badge("${r.qualityWarnings} quality notes", warning = true)
                    }
                    if (r.mode != "par_only") {
                        Text(
                            "Shots: ${r.count ?: 0u}" + (r.expectedCount?.let { " (expected $it)" } ?: "") +
                                "   First: ${fmtSec(r.firstNs)}   Last: ${fmtSec(r.lastNs)}",
                        )
                    }
                }
            }
        }
        item {
            Text(
                "History is stored only on this device. Export and backup arrive in a later version.",
                style = MaterialTheme.typography.bodySmall,
                modifier = Modifier.padding(vertical = 16.dp),
            )
        }
    }
}
