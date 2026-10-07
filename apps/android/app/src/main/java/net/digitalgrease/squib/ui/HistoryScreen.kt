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

@Composable
fun HistoryScreen(c: SquibController, onOpen: (String) -> Unit) {
    val runs by c.history.collectAsState()
    LaunchedEffect(Unit) { c.refreshHistory() }
    LazyColumn(Modifier.padding(horizontal = 16.dp).testTag("history")) {
        item {
            Text("History", style = MaterialTheme.typography.headlineMedium, modifier = Modifier.padding(vertical = 16.dp))
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
