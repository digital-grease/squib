package dev.digitalgrease.squib.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Card
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import dev.digitalgrease.squib.core.SquibException
import dev.digitalgrease.squib.core.convertAngle
import dev.digitalgrease.squib.core.primerById
import dev.digitalgrease.squib.core.primers
import java.util.Locale

/**
 * In-app primers (plan phase 8, ADR-066). The text comes from the core, where every figure
 * is computed with the solver's own functions. A primer opens over the current screen, so
 * whatever was entered there (a computed range card, for example) is still there after.
 */

/** An angle from the screen a primer was opened from, to start the converter with. */
data class AngleContext(val value: Double, val unit: String, val distanceM: Double)

/** The app's US customary (yd, in) or metric (m, cm) choice, for the converter. */
val LocalUsUnits = compositionLocalOf { true }

/** A link beside a term, such as "What are mil and MOA?": opens that primer. */
@Composable
fun PrimerLink(id: String, label: String, modifier: Modifier = Modifier, context: AngleContext? = null, tag: String = "primer_$id") {
    var open by rememberSaveable { mutableStateOf(false) }
    TextButton(onClick = { open = true }, modifier = modifier.heightIn(min = 48.dp).testTag(tag)) { Text(label) }
    if (open) PrimerDialog(id, context) { open = false }
}

/**
 * A primer, or the list of all primers when [startId] is null. Related primers and the
 * list open inside the same dialog; Back returns through them, then closes.
 */
@Composable
fun PrimerDialog(startId: String?, context: AngleContext? = null, onClose: () -> Unit) {
    val us = LocalUsUnits.current
    // Each entry is a primer id, or "" for the list.
    val stack = remember { mutableStateListOf(startId ?: "") }
    val back: () -> Unit = { if (stack.size > 1) stack.removeAt(stack.lastIndex) else onClose() }
    Dialog(onDismissRequest = back, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize()) {
            Column(Modifier.verticalScroll(rememberScrollState()).padding(16.dp).testTag("primer_dialog")) {
                BackButton(if (stack.size > 1) "Back" else "Close", if (stack.size > 1) "Back to the previous primer" else "Close primer", back)
                val id = stack.last()
                if (id.isEmpty()) PrimerList { stack.add(it) }
                else PrimerPage(id, us, context.takeIf { id == startId }, onOpen = { stack.add(it) })
            }
        }
    }
}

@Composable
private fun PrimerList(onOpen: (String) -> Unit) {
    Text("What the terms mean", style = MaterialTheme.typography.headlineSmall, modifier = Modifier.semantics { heading() })
    Text("Short explanations of the units and terms Squib uses. They work offline.", style = MaterialTheme.typography.bodySmall)
    remember { primers() }.forEach { p ->
        Card(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
            TextButton(onClick = { onOpen(p.id) }, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("primer_open_${p.id}")) {
                Column(Modifier.fillMaxWidth()) {
                    Text(p.title, fontWeight = FontWeight.Bold)
                    Text(p.summary, style = MaterialTheme.typography.bodySmall)
                }
            }
        }
    }
}

@Composable
private fun PrimerPage(id: String, us: Boolean, context: AngleContext?, onOpen: (String) -> Unit) {
    val p = remember(id) { primerById(id) } ?: return
    Text(p.title, style = MaterialTheme.typography.headlineSmall, modifier = Modifier.semantics { heading() }.testTag("primer_title"))
    p.paragraphs.forEach { Text(it, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.padding(top = 8.dp)) }
    if (p.converter) AngleConverter(us, context)
    Text("What Squib does with this", style = MaterialTheme.typography.titleMedium,
        modifier = Modifier.padding(top = 16.dp).semantics { heading() })
    Text(p.inSquib, style = MaterialTheme.typography.bodyLarge)
    if (p.related.isNotEmpty()) {
        Text("Related", style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(top = 16.dp).semantics { heading() })
        p.related.forEach { r ->
            val title = remember(r) { primerById(r)?.title ?: r }
            TextButton(onClick = { onOpen(r) }, modifier = Modifier.heightIn(min = 48.dp).testTag("primer_related_$r")) { Text(title) }
        }
    }
    TextButton(onClick = { onOpen("") }, modifier = Modifier.heightIn(min = 48.dp).testTag("primer_all")) { Text("All primers") }
    if (p.sources.isNotEmpty()) {
        Text("Sources", style = MaterialTheme.typography.titleSmall, modifier = Modifier.padding(top = 8.dp))
        p.sources.forEach { Text(it, style = MaterialTheme.typography.bodySmall) }
    }
}

/** Angle converter: an angle in mil, MOA or IPHY at a distance, worked out in the core. */
@Composable
private fun AngleConverter(us: Boolean, context: AngleContext?) {
    val yd = 0.9144
    fun show(v: Double) = String.format(Locale.US, "%.2f", v).trimEnd('0').trimEnd('.').ifEmpty { "0" }
    var value by rememberSaveable { mutableStateOf(context?.let { show(it.value) } ?: "1") }
    var unit by rememberSaveable { mutableStateOf(context?.unit ?: "mil") }
    var distance by rememberSaveable { mutableStateOf(context?.let { String.format(Locale.US, "%.1f", if (us) it.distanceM / yd else it.distanceM).removeSuffix(".0") } ?: "100") }
    val distanceUnit = if (us) "yd" else "m"
    Card(Modifier.fillMaxWidth().padding(top = 12.dp).testTag("angle_converter")) {
        Column(Modifier.padding(12.dp)) {
            Text("Converter", fontWeight = FontWeight.Bold, modifier = Modifier.semantics { heading() })
            if (context != null) Text("Started with your range card's farthest elevation correction.", style = MaterialTheme.typography.bodySmall)
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(value, { value = it.take(8) }, label = { Text("Angle") }, singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal),
                    modifier = Modifier.weight(1f).testTag("converter_value"))
                OutlinedTextField(distance, { distance = it.take(6) }, label = { Text("Distance ($distanceUnit)") }, singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal),
                    modifier = Modifier.weight(1f).testTag("converter_distance"))
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                listOf("mil" to "mil", "moa" to "MOA", "iphy" to "IPHY").forEach { (k, l) ->
                    FilterChip(selected = unit == k, onClick = { unit = k }, label = { Text(l) },
                        modifier = Modifier.heightIn(min = 48.dp).testTag("converter_unit_$k"))
                }
            }
            val v = value.trim().replace(',', '.').toDoubleOrNull()
            val d = distance.trim().replace(',', '.').toDoubleOrNull()?.let { if (us) it * yd else it }
            val result = if (v == null || d == null) "Enter an angle and a distance." else try {
                convertAngle(v, unit, d, us).sentence
            } catch (e: SquibException) {
                e.message ?: "Could not convert."
            }
            Text(result, style = MaterialTheme.typography.bodyLarge,
                modifier = Modifier.padding(top = 8.dp).testTag("converter_result").semantics { liveRegion = LiveRegionMode.Polite })
        }
    }
}
