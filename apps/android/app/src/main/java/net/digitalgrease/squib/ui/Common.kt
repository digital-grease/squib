package net.digitalgrease.squib.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.platform.testTag
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.selection.toggleable
import androidx.compose.ui.unit.dp
import java.text.DateFormat
import java.util.Date
import java.util.Locale

/** High-contrast palettes for outdoor legibility (A21). */
internal val Dark = darkColorScheme(
    primary = Color(0xFFFFD54F),
    onPrimary = Color.Black,
    secondary = Color(0xFF80DEEA),
    error = Color(0xFFFF8A80),
    onError = Color.Black,
    background = Color.Black,
    surface = Color(0xFF121212),
    onSurface = Color.White,
    onBackground = Color.White,
    // Lighter than the Material default so field borders stay clear on cards.
    outline = Color(0xFFABA6B3),
)
internal val Light = lightColorScheme(
    primary = Color(0xFF3B2F00),
    onPrimary = Color.White,
    error = Color(0xFFB00020),
    background = Color.White,
    onBackground = Color.Black,
    surface = Color.White,
    onSurface = Color.Black,
    // Darker than the Material default so field borders stay clear on cards outdoors.
    outline = Color(0xFF56525C),
)

@Composable
fun SquibTheme(content: @Composable () -> Unit) {
    MaterialTheme(colorScheme = if (isSystemInDarkTheme()) Dark else Light, content = content)
}

/** Seconds with two decimals. Display precision is not measured accuracy. */
fun fmtSec(ns: Long?): String = ns?.let { String.format(Locale.US, "%.2f s", it / 1e9) } ?: "n/a"

fun fmtDate(utcMs: Long): String = DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT).format(Date(utcMs))

fun outcomeLabel(o: String?): String = when (o) {
    "complete" -> "Complete"
    "interrupted" -> "Interrupted"
    "cancelled" -> "Cancelled"
    "failed_to_start" -> "Did not start"
    null -> "Unfinished"
    else -> o
}

fun reviewLabel(r: String?, edited: Boolean): String? = when {
    r == "needs_review" -> "Needs review"
    edited -> "Reviewed · Edited"
    r == "reviewed" -> "Reviewed"
    else -> null
}

fun startMethodLabel(m: String?): String = when (m) {
    "acoustic_cue" -> "Start: cue heard by microphone"
    "scheduled_render" -> "Start: cue playback time (not microphone-verified)"
    "requested_only" -> "Start: requested time only (degraded)"
    "vibration_request" -> "Start: vibration requested (not sound; motor delay not measured)"
    "screen_frame" -> "Start: screen cue frame (not sound; display delay not measured)"
    "unresolved", null -> "Start: unavailable"
    else -> m
}

fun timestampQualityLabel(q: String?): String = when (q) {
    "verified_sample_clock" -> "Sample clock verified by platform anchors"
    "provisional_sample_clock" -> "Sample clock (single anchor)"
    "approximate" -> "Approximate clock (delivery-based)"
    "unavailable" -> "Clock mapping unavailable"
    null -> "n/a"
    else -> q
}

/** A text badge: meaning is carried by words, not color alone. */
@Composable
fun Badge(text: String, warning: Boolean = false) {
    Surface(
        shape = RoundedCornerShape(6.dp),
        color = if (warning) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.secondary,
        contentColor = if (warning) MaterialTheme.colorScheme.onError else MaterialTheme.colorScheme.onSecondary,
        modifier = Modifier.padding(end = 6.dp, bottom = 4.dp).semantics { contentDescription = text },
    ) {
        Text(text, modifier = Modifier.padding(horizontal = 8.dp, vertical = 2.dp), fontWeight = FontWeight.SemiBold)
    }
}

@Composable
fun LabeledValue(label: String, value: String) {
    Row(
        Modifier.heightIn(min = 32.dp).semantics(mergeDescendants = true) {},
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text("$label: ", style = MaterialTheme.typography.bodyLarge)
        Text(value, style = MaterialTheme.typography.bodyLarge, fontWeight = FontWeight.Bold)
    }
}

/**
 * Back or cancel link at the top of a screen. The arrow is visual only: screen readers
 * hear the spoken label (for example "Back to History"), and the target is at least 48 dp.
 */
@Composable
fun BackButton(label: String, spoken: String, onClick: () -> Unit) {
    androidx.compose.material3.TextButton(
        onClick = onClick,
        modifier = Modifier.heightIn(min = 48.dp).semantics { contentDescription = spoken },
    ) { Text("‹ $label") }
}

/**
 * A switch or checkbox and its label as one control: a screen reader reads the label with
 * the state, and the whole row (at least 48 dp tall) is the touch target. `tag` goes on the
 * row, which carries the toggle state.
 */
@Composable
fun LabeledToggle(
    label: String,
    checked: Boolean,
    onChange: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
    checkbox: Boolean = false,
    controlAtEnd: Boolean = false,
    bold: Boolean = false,
    small: Boolean = false,
    enabled: Boolean = true,
    tag: String? = null,
) {
    Row(
        verticalAlignment = if (small) Alignment.Top else Alignment.CenterVertically,
        modifier = modifier.fillMaxWidth().heightIn(min = 48.dp)
            .toggleable(
                value = checked,
                enabled = enabled,
                role = if (checkbox) androidx.compose.ui.semantics.Role.Checkbox else androidx.compose.ui.semantics.Role.Switch,
                onValueChange = onChange,
            )
            .let { if (tag != null) it.testTag(tag) else it },
    ) {
        val control = @Composable {
            if (checkbox) androidx.compose.material3.Checkbox(checked = checked, onCheckedChange = null, enabled = enabled)
            else androidx.compose.material3.Switch(checked = checked, onCheckedChange = null, enabled = enabled)
        }
        if (!controlAtEnd) {
            control()
            androidx.compose.foundation.layout.Spacer(Modifier.width(8.dp))
        }
        Text(
            label,
            modifier = Modifier.weight(1f).padding(vertical = 4.dp),
            fontWeight = if (bold) FontWeight.Bold else null,
            style = if (small) MaterialTheme.typography.bodySmall else androidx.compose.ui.text.TextStyle.Default,
        )
        if (controlAtEnd) {
            androidx.compose.foundation.layout.Spacer(Modifier.width(8.dp))
            control()
        }
    }
}

/** What a screen reader says for a control whose visible text needs context, such as
 * "Start Repeated par" for a Start button in a list of drills. */
fun Modifier.spoken(description: String): Modifier = semantics { contentDescription = description }

/**
 * Marks a chip as one option of a single-choice group, so screen readers announce it as a
 * radio button ("selected, 1 of 3") rather than a checkbox. Put [selectableGroup] on the row.
 */
fun Modifier.oneOf(): Modifier = semantics { role = androidx.compose.ui.semantics.Role.RadioButton }
