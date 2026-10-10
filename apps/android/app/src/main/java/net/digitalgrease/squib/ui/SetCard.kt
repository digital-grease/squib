package net.digitalgrease.squib.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import java.util.Locale
import net.digitalgrease.squib.core.StringSetView

private fun setValue(metric: String, v: Double): String =
    if (metric == "hit_factor") String.format(Locale.US, "HF %.4f", v) else String.format(Locale.US, "%.2f s", v)

private fun noteText(n: String?): String = when (n) {
    "not_complete" -> "not completed, not counted"
    "score_incomplete" -> "score missing, not counted"
    "extra" -> "beyond the set, not counted"
    else -> ""
}

/** A drill's set result: the rule, the value or why there is none, and each string's part. */
@Composable
fun SetCard(s: StringSetView) {
    Card(Modifier.fillMaxWidth().padding(vertical = 4.dp).testTag("set_${s.drillId}")) {
        Column(Modifier.padding(12.dp)) {
            Text("${s.drillTitle} · ${s.label}", fontWeight = FontWeight.Bold)
            val v = s.value
            if (s.status == "complete" && v != null) {
                Text(setValue(s.metric, v), style = MaterialTheme.typography.titleLarge, modifier = Modifier.testTag("set_value"))
            } else {
                Text("Incomplete: ${s.reason ?: "not enough scored strings"}", color = MaterialTheme.colorScheme.error,
                    modifier = Modifier.testTag("set_incomplete"))
            }
            s.strings.forEachIndexed { i, st ->
                val value = st.value?.let { setValue(s.metric, it) } ?: "no result"
                val role = when {
                    st.counted -> "counted"
                    st.dropped -> "dropped (not among the best)"
                    else -> noteText(st.note)
                }
                Text("String ${i + 1}: $value · $role", style = MaterialTheme.typography.bodySmall)
            }
        }
    }
}
