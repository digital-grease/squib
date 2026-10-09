package net.digitalgrease.squib.ui

import android.media.AudioManager
import android.media.ToneGenerator
import android.os.SystemClock
import androidx.compose.foundation.layout.Arrangement
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
import androidx.compose.material3.Card
import androidx.compose.material3.Checkbox
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import java.util.Locale
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import net.digitalgrease.squib.core.ChecklistView
import net.digitalgrease.squib.core.DrillView
import net.digitalgrease.squib.core.PlanItemInput
import net.digitalgrease.squib.core.PlanItemView
import net.digitalgrease.squib.core.PlanView
import net.digitalgrease.squib.data.DataController

/** Day plans list shown on the Practice tab. */
@Composable
fun PlansSection(d: DataController, onOpenPlan: (String) -> Unit, onTemplate: () -> Unit) {
    val plans by d.plans.collectAsState()
    var creating by remember { mutableStateOf(false) }
    var title by remember { mutableStateOf("") }
    var date by remember { mutableStateOf(d.today()) }
    var kind by remember { mutableStateOf("practice") }
    Text("Day plans", style = MaterialTheme.typography.titleLarge, modifier = Modifier.padding(top = 8.dp))
    Text("An agenda for a practice or match day: drills to run, stage notes, times, and a checklist.", style = MaterialTheme.typography.bodySmall)
    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Button(onClick = { creating = !creating }, modifier = Modifier.heightIn(min = 48.dp).testTag("new_plan")) { Text("New day plan") }
        OutlinedButton(onClick = onTemplate, modifier = Modifier.heightIn(min = 48.dp).testTag("checklist_template")) { Text("Checklist template") }
    }
    if (creating) {
        Card(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
            Column(Modifier.padding(12.dp)) {
                OutlinedTextField(title, { title = it.take(200) }, label = { Text("Title, e.g. Club match") }, singleLine = true,
                    modifier = Modifier.fillMaxWidth().testTag("plan_title"))
                OutlinedTextField(date, { date = it.take(10) }, label = { Text("Date (YYYY-MM-DD)") }, singleLine = true, modifier = Modifier.fillMaxWidth())
                FlowRow {
                    listOf("practice" to "Practice", "match" to "Match").forEach { (k, l) ->
                        FilterChip(selected = kind == k, onClick = { kind = k }, label = { Text(l) }, modifier = Modifier.padding(end = 8.dp))
                    }
                }
                Button(onClick = {
                    d.createPlan(title, date, kind) { id -> creating = false; title = ""; onOpenPlan(id) }
                }, modifier = Modifier.heightIn(min = 48.dp).testTag("plan_create")) { Text("Create") }
            }
        }
    }
    plans.forEach { p ->
        Card(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
            Row(Modifier.padding(12.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(p.title, fontWeight = FontWeight.Bold)
                    Text("${p.dateLocal} · ${if (p.kind == "match") "Match" else "Practice"} · ${p.doneItems} of ${p.items} drills done",
                        style = MaterialTheme.typography.bodySmall)
                }
                Button(onClick = { onOpenPlan(p.id) }, modifier = Modifier.heightIn(min = 48.dp).testTag("open_plan_${p.title}")) { Text("Open") }
            }
        }
    }
}

private fun progressText(i: PlanItemView): String {
    val target = i.targetStrings?.toInt() ?: 1
    val parts = mutableListOf("${i.completed} of $target strings done")
    if (i.aborted > 0u) parts += "${i.aborted} not completed"
    return parts.joinToString(" · ")
}

@Composable
fun PlanScreen(
    d: DataController,
    planId: String,
    onBack: () -> Unit,
    onStartDrill: (DrillView, String, String, Int) -> Unit,
    onOpenRun: (String) -> Unit,
) {
    var plan by remember { mutableStateOf<PlanView?>(null) }
    var version by remember { mutableIntStateOf(0) }
    var adding by remember { mutableStateOf(false) }
    var manualFor by remember { mutableStateOf<Pair<DrillView, String>?>(null) }
    val scope = rememberCoroutineScope()
    val reload: () -> Unit = { version++ }
    LaunchedEffect(planId, version) { plan = d.loadPlan(planId) }
    // Schedule line refresh (display only; uses the device's local clock).
    LaunchedEffect(planId) {
        while (true) {
            delay(30_000)
            plan = d.loadPlan(planId)
        }
    }
    manualFor?.let { (drill, item) ->
        ManualEntry(d, drill, planItemId = item, onDone = { id -> manualFor = null; reload(); id?.let(onOpenRun) })
        return
    }
    val p = plan
    Column(Modifier.verticalScroll(rememberScrollState()).padding(16.dp).testTag("plan_screen")) {
        TextButton(onClick = onBack) { Text("‹ Practice") }
        if (p == null) {
            Text("Loading…")
            return@Column
        }
        Text(p.title, style = MaterialTheme.typography.headlineMedium)
        Text("${p.dateLocal} · ${if (p.kind == "match") "Match" else "Practice"}", style = MaterialTheme.typography.bodyMedium)
        p.nextUp?.let { n ->
            val whenText = when {
                n.minutesUntil > 0 -> "in ${n.minutesUntil} min"
                n.minutesUntil == 0 -> "now"
                else -> "${-n.minutesUntil} min ago"
            }
            Card(Modifier.fillMaxWidth().padding(vertical = 8.dp)) {
                Text("Next: ${n.title} at ${n.timeLocal} ($whenText)", fontWeight = FontWeight.Bold,
                    modifier = Modifier.padding(12.dp).semantics { liveRegion = LiveRegionMode.Polite }.testTag("next_up"))
            }
        }

        Text("Agenda", style = MaterialTheme.typography.titleLarge, modifier = Modifier.padding(top = 8.dp))
        if (p.items.isEmpty()) Text("No items yet.")
        p.items.forEachIndexed { idx, i ->
            PlanItemCard(
                d, i, first = idx == 0, last = idx == p.items.lastIndex, reload = reload,
                onStart = {
                    val id = i.drillId
                    val v = i.drillVersion
                    if (id != null && v != null) {
                        scope.launch {
                            val dv = d.drillVersion(id, v)
                            when {
                                dv == null -> {}
                                dv.input.manual -> manualFor = dv to i.id
                                else -> onStartDrill(dv, i.id, p.title, i.targetStrings?.toInt() ?: dv.input.repeats.toInt())
                            }
                        }
                    }
                },
            )
        }
        if (adding) AddItemForm(d, p.id, onDone = { adding = false; reload() })
        else OutlinedButton(onClick = { adding = true }, modifier = Modifier.heightIn(min = 48.dp).testTag("add_item")) { Text("+ Add item") }

        WalkthroughTimer()

        Text("Checklist", style = MaterialTheme.typography.titleLarge, modifier = Modifier.padding(top = 16.dp))
        ChecklistEditor(d, p.id, p.checklist, reload)

        Text("Notes", style = MaterialTheme.typography.titleLarge, modifier = Modifier.padding(top = 16.dp))
        NotesField(p.notes, "plan_notes") { n -> d.setPlanNotes(p.id, n, reload) }
        Text("Notes stay on this device and are not included when you share results.", style = MaterialTheme.typography.bodySmall)
        TextButton(onClick = { d.archivePlan(p.id) { onBack() } }, modifier = Modifier.padding(top = 8.dp)) { Text("Archive plan (runs stay in history)") }
    }
}

@Composable
private fun PlanItemCard(d: DataController, i: PlanItemView, first: Boolean, last: Boolean, reload: () -> Unit, onStart: () -> Unit) {
    Card(Modifier.fillMaxWidth().padding(vertical = 4.dp).testTag("item_${i.title}")) {
        Column(Modifier.padding(12.dp)) {
            val kindLabel = when (i.kind) { "drill" -> "Drill"; "stage" -> "Stage"; else -> "Event" }
            Text((i.timeLocal?.let { "$it · " } ?: "") + "$kindLabel · ${i.title}", fontWeight = FontWeight.Bold)
            FlowRow {
                if (i.skipped) Badge("Skipped", warning = true)
                if (i.done) Badge("Done")
            }
            if (i.kind == "drill") Text(progressText(i), modifier = Modifier.testTag("progress_${i.title}"))
            if (i.kind == "stage") NotesField(i.notes, "notes_${i.title}") { n -> d.setItemNotes(i.id, n, reload) }
            else if (i.notes.isNotBlank()) Text(i.notes, style = MaterialTheme.typography.bodyMedium)
            FlowRow {
                if (i.kind == "drill" && !i.skipped) {
                    Button(onClick = onStart, modifier = Modifier.padding(end = 8.dp).heightIn(min = 48.dp).testTag("run_item_${i.title}")) {
                        Text(if (i.done) "Run again" else "Start")
                    }
                }
                TextButton(onClick = { d.setItemSkipped(i.id, !i.skipped, reload) }, modifier = Modifier.heightIn(min = 48.dp).testTag("skip_${i.title}")) {
                    Text(if (i.skipped) "Unskip" else "Skip")
                }
                if (!first) TextButton(onClick = { d.movePlanItem(i.id, true, reload) }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Move up") }
                if (!last) TextButton(onClick = { d.movePlanItem(i.id, false, reload) }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Move down") }
                TextButton(onClick = { d.deletePlanItem(i.id, reload) }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Remove") }
            }
        }
    }
}

@Composable
private fun NotesField(initial: String, tag: String, onSave: (String) -> Unit) {
    var text by remember(initial) { mutableStateOf(initial) }
    OutlinedTextField(text, { text = it.take(4000) }, label = { Text("Notes") }, modifier = Modifier.fillMaxWidth().testTag(tag))
    if (text != initial) TextButton(onClick = { onSave(text) }, modifier = Modifier.heightIn(min = 48.dp).testTag("${tag}_save")) { Text("Save notes") }
}

@Composable
private fun AddItemForm(d: DataController, planId: String, onDone: () -> Unit) {
    val drills by d.drills.collectAsState()
    var kind by remember { mutableStateOf("drill") }
    var title by remember { mutableStateOf("") }
    var time by remember { mutableStateOf("") }
    var drill by remember { mutableStateOf<DrillView?>(null) }
    var strings by remember { mutableStateOf("") }
    var notes by remember { mutableStateOf("") }
    Card(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
        Column(Modifier.padding(12.dp)) {
            FlowRow {
                listOf("drill" to "Drill", "stage" to "Stage", "event" to "Event").forEach { (k, l) ->
                    FilterChip(selected = kind == k, onClick = { kind = k }, label = { Text(l) },
                        modifier = Modifier.padding(end = 8.dp).testTag("item_kind_$k"))
                }
            }
            if (kind == "drill") {
                Text("Drill", fontWeight = FontWeight.Bold)
                FlowRow {
                    drills.forEach { dr ->
                        FilterChip(selected = drill?.drillId == dr.drillId, onClick = { drill = dr; if (strings.isBlank()) strings = dr.input.repeats.toString() },
                            label = { Text(dr.input.title) }, modifier = Modifier.padding(end = 6.dp).testTag("pick_${dr.drillId}"))
                    }
                }
                OutlinedTextField(strings, { strings = it.filter(Char::isDigit).take(2) }, label = { Text("Strings to run") }, singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number), modifier = Modifier.fillMaxWidth().testTag("item_strings"))
            }
            OutlinedTextField(title, { title = it.take(200) }, singleLine = true, modifier = Modifier.fillMaxWidth().testTag("item_title"),
                label = { Text(if (kind == "drill") "Title (optional; defaults to the drill's)" else "Title, e.g. Stage 3 or Shooters meeting") })
            OutlinedTextField(time, { time = it.filter { c -> c.isDigit() || c == ':' }.take(5) }, singleLine = true,
                label = { Text("Time (HH:MM, optional)") }, modifier = Modifier.fillMaxWidth().testTag("item_time"))
            if (kind != "drill") {
                OutlinedTextField(notes, { notes = it.take(4000) }, label = { Text("Notes (start position, round count, plan)") },
                    modifier = Modifier.fillMaxWidth())
            }
            Row {
                Button(onClick = {
                    d.addPlanItem(planId, PlanItemInput(
                        kind = kind, title = title, timeLocal = time.ifBlank { null },
                        drillId = drill?.drillId.takeIf { kind == "drill" }, drillVersion = drill?.version.takeIf { kind == "drill" },
                        targetStrings = strings.toIntOrNull()?.toUInt().takeIf { kind == "drill" }, notes = notes,
                    ), onDone)
                }, modifier = Modifier.heightIn(min = 48.dp).testTag("item_save")) { Text("Add") }
                Spacer(Modifier.width(8.dp))
                TextButton(onClick = onDone, modifier = Modifier.heightIn(min = 48.dp)) { Text("Cancel") }
            }
        }
    }
}

@Composable
private fun ChecklistEditor(d: DataController, planId: String?, items: List<ChecklistView>, reload: () -> Unit) {
    var text by remember { mutableStateOf("") }
    items.forEach { c ->
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.heightIn(min = 48.dp)) {
            if (planId != null) Checkbox(checked = c.checked, onCheckedChange = { d.setChecked(c.id, it, reload) }, modifier = Modifier.testTag("check_${c.text}"))
            Text(c.text, modifier = Modifier.weight(1f))
            TextButton(onClick = { d.deleteChecklistItem(c.id, reload) }) { Text("Remove") }
        }
    }
    Row(verticalAlignment = Alignment.CenterVertically) {
        OutlinedTextField(text, { text = it.take(200) }, label = { Text("Add item") }, singleLine = true, modifier = Modifier.weight(1f).testTag("checklist_new"))
        TextButton(onClick = { if (text.isNotBlank()) { d.addChecklistItem(planId, text, reload); text = "" } }, modifier = Modifier.testTag("checklist_add")) { Text("Add") }
    }
}

@Composable
fun ChecklistTemplateScreen(d: DataController, onBack: () -> Unit) {
    var items by remember { mutableStateOf<List<ChecklistView>>(emptyList()) }
    var version by remember { mutableIntStateOf(0) }
    LaunchedEffect(version) { items = d.checklistTemplate() }
    Column(Modifier.verticalScroll(rememberScrollState()).padding(16.dp)) {
        TextButton(onClick = onBack) { Text("‹ Practice") }
        Text("Checklist template", style = MaterialTheme.typography.headlineSmall)
        Text("Copied into each new day plan. Changing it does not change existing plans." +
            if (items.isEmpty()) " While it is empty, new plans get a basic range-day list." else "",
            style = MaterialTheme.typography.bodySmall)
        ChecklistEditor(d, null, items) { version++ }
    }
}

/**
 * Countdown for walkthroughs and prep time. A plain wall-clock aid: it does not start
 * runs and is not part of run timing. The optional end tone is a notification beep.
 */
@Composable
private fun WalkthroughTimer() {
    var minutes by remember { mutableStateOf("5") }
    var endAt by remember { mutableLongStateOf(0L) }
    var left by remember { mutableLongStateOf(0L) }
    var tone by remember { mutableStateOf(true) }
    LaunchedEffect(endAt) {
        if (endAt == 0L) return@LaunchedEffect
        while (true) {
            left = (endAt - SystemClock.elapsedRealtime()).coerceAtLeast(0)
            if (left == 0L) break
            delay(200)
        }
        if (tone) {
            runCatching {
                val g = ToneGenerator(AudioManager.STREAM_MUSIC, 90)
                g.startTone(ToneGenerator.TONE_PROP_BEEP2, 800)
                delay(1000)
                g.release()
            }
        }
    }
    Card(Modifier.fillMaxWidth().padding(top = 16.dp)) {
        Column(Modifier.padding(12.dp)) {
            Text("Walkthrough countdown", fontWeight = FontWeight.Bold)
            if (endAt == 0L) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    OutlinedTextField(minutes, { minutes = it.filter(Char::isDigit).take(2) }, label = { Text("Minutes") }, singleLine = true,
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number), modifier = Modifier.weight(1f).testTag("walk_minutes"))
                    Spacer(Modifier.width(8.dp))
                    Button(onClick = {
                        val m = minutes.toIntOrNull()?.coerceIn(1, 60) ?: return@Button
                        endAt = SystemClock.elapsedRealtime() + m * 60_000L
                        left = m * 60_000L
                    }, modifier = Modifier.heightIn(min = 48.dp).testTag("walk_start")) { Text("Start") }
                }
                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.heightIn(min = 48.dp)) {
                    Switch(checked = tone, onCheckedChange = { tone = it })
                    Spacer(Modifier.width(8.dp))
                    Text("Beep at the end")
                }
            } else {
                val s = (left + 999) / 1000
                Text(String.format(Locale.US, "%d:%02d", s / 60, s % 60), fontSize = 40.sp, fontWeight = FontWeight.Black,
                    modifier = Modifier.testTag("walk_left"))
                if (left == 0L) Text("Time.", style = MaterialTheme.typography.titleMedium)
                OutlinedButton(onClick = { endAt = 0L }, modifier = Modifier.heightIn(min = 48.dp).testTag("walk_stop")) {
                    Text(if (left == 0L) "Done" else "Stop")
                }
            }
            Text("Uses the phone clock. Not a shot timer.", style = MaterialTheme.typography.bodySmall)
        }
    }
    Spacer(Modifier.height(4.dp))
}
