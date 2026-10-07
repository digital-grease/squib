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
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import java.text.DateFormat
import java.util.Date
import java.util.Locale

/** High-contrast palettes for outdoor legibility (A21). */
private val Dark = darkColorScheme(
    primary = Color(0xFFFFD54F),
    onPrimary = Color.Black,
    secondary = Color(0xFF80DEEA),
    error = Color(0xFFFF8A80),
    onError = Color.Black,
    background = Color.Black,
    surface = Color(0xFF121212),
    onSurface = Color.White,
    onBackground = Color.White,
)
private val Light = lightColorScheme(
    primary = Color(0xFF3B2F00),
    onPrimary = Color.White,
    error = Color(0xFFB00020),
    background = Color.White,
    onBackground = Color.Black,
    surface = Color.White,
    onSurface = Color.Black,
)

@Composable
fun SquibTheme(content: @Composable () -> Unit) {
    MaterialTheme(colorScheme = if (isSystemInDarkTheme()) Dark else Light, content = content)
}

/** Seconds with two decimals. Display precision is not measured accuracy. */
fun fmtSec(ns: Long?): String = ns?.let { String.format(Locale.US, "%.2f s", it / 1e9) } ?: "—"

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
    "unresolved", null -> "Start: unavailable"
    else -> m
}

fun timestampQualityLabel(q: String?): String = when (q) {
    "verified_sample_clock" -> "Sample clock verified by platform anchors"
    "provisional_sample_clock" -> "Sample clock (single anchor)"
    "approximate" -> "Approximate clock (delivery-based)"
    "unavailable" -> "Clock mapping unavailable"
    null -> "—"
    else -> q
}

/** A text badge: meaning is carried by words, not color alone. */
@Composable
fun Badge(text: String, warning: Boolean = false) {
    Surface(
        shape = RoundedCornerShape(6.dp),
        color = if (warning) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.secondary,
        contentColor = Color.Black,
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
