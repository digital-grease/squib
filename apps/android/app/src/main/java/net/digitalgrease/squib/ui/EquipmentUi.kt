package net.digitalgrease.squib.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
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
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import net.digitalgrease.squib.core.EquipmentView
import net.digitalgrease.squib.data.DataController

fun categoryLabel(c: String) = when (c) {
    "pcc" -> "PCC"
    else -> c.replaceFirstChar { it.uppercase() }
}

/** Equipment list on the Practice tab: optional labels, versioned when edited. */
@Composable
fun EquipmentSection(d: DataController) {
    val items by d.equipment.collectAsState()
    var editing by remember { mutableStateOf<EquipmentView?>(null) }
    var adding by remember { mutableStateOf(false) }
    Text("Equipment", style = MaterialTheme.typography.titleLarge, modifier = Modifier.padding(top = 16.dp))
    Text("Optional labels for what you shoot with. Editing keeps earlier runs on the version they used. No serial numbers.",
        style = MaterialTheme.typography.bodySmall)
    items.forEach { e ->
        Card(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
            Column(Modifier.padding(12.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text("${e.name} · ${categoryLabel(e.category)}", fontWeight = FontWeight.Bold, modifier = Modifier.weight(1f))
                    if (e.active) Badge("In use")
                }
                if (e.notes.isNotBlank()) Text(e.notes, style = MaterialTheme.typography.bodySmall)
                Text("Version ${e.version}", style = MaterialTheme.typography.bodySmall)
                FlowRow {
                    TextButton(onClick = { editing = e }, modifier = Modifier.heightIn(min = 48.dp).spoken("Edit ${e.name}")) { Text("Edit") }
                    TextButton(onClick = { d.archiveEquipment(e.id) }, modifier = Modifier.heightIn(min = 48.dp).testTag("equipment_archive_${e.name}").spoken("Archive ${e.name}")) { Text("Archive") }
                }
            }
        }
    }
    if (adding || editing != null) {
        EquipmentEditor(d, editing, onDone = { adding = false; editing = null })
    } else {
        TextButton(onClick = { adding = true }, modifier = Modifier.heightIn(min = 48.dp).testTag("equipment_add")) { Text("+ Add equipment") }
    }
}

@Composable
private fun EquipmentEditor(d: DataController, existing: EquipmentView?, onDone: () -> Unit) {
    var name by remember { mutableStateOf(existing?.name ?: "") }
    var category by remember { mutableStateOf(existing?.category ?: "pistol") }
    var notes by remember { mutableStateOf(existing?.notes ?: "") }
    Card(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
        Column(Modifier.padding(12.dp)) {
            Text(if (existing == null) "New equipment" else "Edit (creates version ${existing.version + 1u})", fontWeight = FontWeight.Bold)
            OutlinedTextField(name, { name = it.take(80) }, label = { Text("Name, e.g. Carry pistol") }, singleLine = true,
                modifier = Modifier.fillMaxWidth().testTag("equipment_name"))
            FlowRow(Modifier.selectableGroup()) {
                d.equipmentCategories.forEach { c ->
                    FilterChip(selected = category == c, onClick = { category = c }, label = { Text(categoryLabel(c)) },
                        modifier = Modifier.oneOf().padding(end = 6.dp).heightIn(min = 48.dp).testTag("equipment_cat_$c"))
                }
            }
            OutlinedTextField(notes, { notes = it.take(1000) }, label = { Text("Notes (optic, load, setup)") }, modifier = Modifier.fillMaxWidth())
            Row {
                Button(onClick = {
                    if (existing == null) d.addEquipment(name, category, notes) else d.editEquipment(existing.id, name, category, notes)
                    onDone()
                }, enabled = name.isNotBlank(), modifier = Modifier.heightIn(min = 48.dp).testTag("equipment_save")) { Text("Save") }
                TextButton(onClick = onDone, modifier = Modifier.heightIn(min = 48.dp)) { Text("Cancel") }
            }
        }
    }
}

/** Equipment for new runs, shown on the Timer; changing it is disabled during a run. */
@Composable
fun EquipmentBar(d: DataController, locked: Boolean) {
    val items by d.equipment.collectAsState()
    if (items.isEmpty()) return
    var open by remember { mutableStateOf(false) }
    val active = items.firstOrNull { it.active }
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(horizontal = 4.dp)) {
        Text("Equipment: " + (active?.let { "${it.name} (${categoryLabel(it.category)})" } ?: "none"),
            modifier = Modifier.weight(1f).testTag("equipment_banner"))
        TextButton(onClick = { open = !open }, enabled = !locked, modifier = Modifier.heightIn(min = 48.dp).testTag("equipment_switch")) {
            Text(if (open) "Done" else "Change")
        }
    }
    if (open && !locked) {
        FlowRow(Modifier.selectableGroup()) {
            FilterChip(selected = active == null, onClick = { d.setActiveEquipment(null) }, label = { Text("None") },
                modifier = Modifier.oneOf().padding(end = 6.dp).heightIn(min = 48.dp))
            items.forEach { e ->
                FilterChip(selected = e.active, onClick = { d.setActiveEquipment(e.id) }, label = { Text(e.name) },
                    modifier = Modifier.oneOf().padding(end = 6.dp).heightIn(min = 48.dp).testTag("pick_equipment_${e.name}"))
            }
        }
    }
}
