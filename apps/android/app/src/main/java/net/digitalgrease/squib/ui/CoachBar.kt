package net.digitalgrease.squib.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
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
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import net.digitalgrease.squib.data.DataController

/**
 * Who is shooting, always visible on the Timer. A coach can switch shooters between
 * runs and keep a rotation; switching is disabled while a run is in progress.
 */
@Composable
fun CoachBar(d: DataController, locked: Boolean, afterRun: Boolean) {
    val shooters by d.shooters.collectAsState()
    val coach by d.coach.collectAsState()
    var open by remember { mutableStateOf(false) }
    val active = shooters.firstOrNull { it.active }
    Card(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
        Column(Modifier.padding(horizontal = 12.dp, vertical = 4.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Shooter: ${active?.name ?: "Me"}", fontWeight = FontWeight.Bold, modifier = Modifier.weight(1f).testTag("shooter_banner"))
                if (shooters.size > 1 || coach?.enabled == true) {
                    TextButton(onClick = { open = !open }, enabled = !locked, modifier = Modifier.heightIn(min = 48.dp).testTag("switch_shooter")) {
                        Text(if (open) "Done" else "Switch")
                    }
                }
            }
            val next = coach?.next
            if (afterRun && !locked && next != null) {
                Button(onClick = { d.advanceShooter() }, modifier = Modifier.fillMaxWidth().heightIn(min = 56.dp).testTag("next_shooter")) {
                    Text("Next up: ${next.name}")
                }
            }
            if (open && !locked) {
                FlowRow {
                    shooters.forEach { sh ->
                        FilterChip(selected = sh.active, onClick = { d.setActiveShooter(sh.id) }, label = { Text(sh.name) },
                            modifier = Modifier.padding(end = 8.dp).heightIn(min = 48.dp).testTag("pick_shooter_${sh.name}"))
                    }
                }
                val c = coach
                if (c != null && shooters.size > 1) {
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.heightIn(min = 48.dp)) {
                        Switch(checked = c.enabled, onCheckedChange = { on ->
                            d.setCoach(on, if (c.squad.isEmpty()) shooters.map { it.id } else c.squad.map { it.id })
                        }, modifier = Modifier.testTag("coach_mode"))
                        Spacer(Modifier.width(8.dp))
                        Text("Coach rotation: offer the next shooter after each run")
                    }
                    if (c.enabled) {
                        Text("Order: " + c.squad.joinToString(" → ") { it.name }, style = MaterialTheme.typography.bodySmall)
                        Text("Tap a name to add it or move it to the end; tap the last name to remove it.", style = MaterialTheme.typography.bodySmall)
                        FlowRow {
                            shooters.forEach { sh ->
                                val inSquad = c.squad.any { it.id == sh.id }
                                FilterChip(selected = inSquad, onClick = {
                                    val ids = c.squad.map { it.id }.filter { it != sh.id }
                                    d.setCoach(true, if (inSquad && ids.isNotEmpty() && c.squad.last().id == sh.id) ids else ids + sh.id)
                                }, label = { Text(sh.name) }, modifier = Modifier.padding(end = 8.dp).heightIn(min = 48.dp))
                            }
                        }
                    }
                }
                Text("Add shooters from History.", style = MaterialTheme.typography.bodySmall)
            }
        }
    }
}
