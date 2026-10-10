package net.digitalgrease.squib.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import java.util.Locale
import net.digitalgrease.squib.core.BinView
import net.digitalgrease.squib.core.HistoryPointView

private fun median(v: List<Double>): Double {
    val s = v.sorted()
    return if (s.size % 2 == 1) s[s.size / 2] else (s[s.size / 2 - 1] + s[s.size / 2]) / 2
}

private fun fmt(v: Double, unit: String) = if (unit == "s") String.format(Locale.US, "%.2f s", v) else String.format(Locale.US, "%.3f", v)

/**
 * Values in time order with the median as a dashed line. Descriptive only: no fitted
 * trend line or claim, and "too few for a trend" below five runs is stated by the caller.
 */
@Composable
fun TrendChart(title: String, values: List<Double>, unit: String) {
    if (values.isEmpty()) return
    val lo = values.min()
    val hi = values.max()
    val med = median(values)
    val desc = "$title for ${values.size} runs in time order: median ${fmt(med, unit)}, from ${fmt(lo, unit)} to ${fmt(hi, unit)}."
    val line = MaterialTheme.colorScheme.primary
    val guide = MaterialTheme.colorScheme.outline
    Text(title, fontWeight = FontWeight.Bold, modifier = Modifier.padding(top = 8.dp))
    Canvas(Modifier.fillMaxWidth().height(120.dp).semantics { contentDescription = desc }.testTag("trend_$unit")) {
        val pad = 8f
        val span = (hi - lo).takeIf { it > 0 } ?: 1.0
        fun x(i: Int) = pad + (size.width - 2 * pad) * (if (values.size == 1) 0.5f else i.toFloat() / (values.size - 1))
        fun y(v: Double) = pad + (size.height - 2 * pad) * (1f - ((v - lo) / span).toFloat())
        drawLine(guide, Offset(pad, y(med)), Offset(size.width - pad, y(med)), strokeWidth = 2f,
            pathEffect = PathEffect.dashPathEffect(floatArrayOf(12f, 8f)))
        for (i in 1 until values.size) drawLine(line, Offset(x(i - 1), y(values[i - 1])), Offset(x(i), y(values[i])), strokeWidth = 3f)
        values.forEachIndexed { i, v -> drawCircle(line, 6f, Offset(x(i), y(v))) }
    }
    Row {
        Text("Oldest", style = MaterialTheme.typography.bodySmall, modifier = Modifier.weight(1f))
        Text("dashed: median ${fmt(med, unit)}", style = MaterialTheme.typography.bodySmall, modifier = Modifier.weight(2f))
        Text("Newest", style = MaterialTheme.typography.bodySmall)
    }
}

/** Equal-width bins from the core; bar heights are counts. */
@Composable
fun HistogramChart(title: String, bins: List<BinView>, unit: String) {
    if (bins.isEmpty()) return
    val maxCount = bins.maxOf { it.count.toInt() }.coerceAtLeast(1)
    val desc = "$title: " + bins.joinToString("; ") { "${fmt(it.lo, unit)} to ${fmt(it.hi, unit)}: ${it.count}" }
    val bar = MaterialTheme.colorScheme.secondary
    Text(title, fontWeight = FontWeight.Bold, modifier = Modifier.padding(top = 8.dp))
    Canvas(Modifier.fillMaxWidth().height(80.dp).semantics { contentDescription = desc }.testTag("histogram_$unit")) {
        val w = size.width / bins.size
        bins.forEachIndexed { i, b ->
            val h = size.height * b.count.toFloat() / maxCount
            drawRect(bar, topLeft = Offset(i * w + 2f, size.height - h), size = androidx.compose.ui.geometry.Size(w - 4f, h))
        }
    }
    Row {
        Text(fmt(bins.first().lo, unit), style = MaterialTheme.typography.bodySmall, modifier = Modifier.weight(1f))
        Text(fmt(bins.last().hi, unit), style = MaterialTheme.typography.bodySmall)
    }
}

/** Newest first: time and score side by side, with how each was measured. */
@Composable
fun SpeedScoreList(points: List<HistoryPointView>) {
    if (points.isEmpty()) return
    Text("Runs in this summary (newest first)", fontWeight = FontWeight.Bold, modifier = Modifier.padding(top = 8.dp))
    Column(Modifier.testTag("speed_score_list")) {
        points.asReversed().take(20).forEach { p ->
            FlowRow(Modifier.padding(vertical = 2.dp)) {
                Text(fmtDate(p.createdUtcMs), style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(end = 8.dp))
                Text(p.finalS?.let { String.format(Locale.US, "%.2f s", it) } ?: "time n/a", modifier = Modifier.padding(end = 8.dp))
                Text(
                    p.hitFactor?.let { String.format(Locale.US, "HF %.3f", it) } ?: if (p.scoreComplete) "score complete" else "no score",
                    modifier = Modifier.padding(end = 8.dp),
                )
                if (p.edited) Badge("Edited")
                if (p.manual) Badge("Manual")
            }
        }
        if (points.size > 20) Text("…and ${points.size - 20} earlier", style = MaterialTheme.typography.bodySmall)
    }
}
