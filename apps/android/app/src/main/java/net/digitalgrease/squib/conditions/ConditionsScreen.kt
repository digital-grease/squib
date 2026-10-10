package net.digitalgrease.squib.conditions

import androidx.compose.foundation.clickable
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
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import java.util.Locale
import net.digitalgrease.squib.core.ConditionsView
import net.digitalgrease.squib.core.FieldView
import net.digitalgrease.squib.ui.Badge
import net.digitalgrease.squib.ui.fmtDate

/** Fields shown by default; the rest appear only when they have a value. */
private val PRIMARY = listOf("temperature", "relative_humidity", "wind_speed", "wind_gust", "wind_direction", "local_pressure", "altimeter_setting")

@Composable
fun ConditionsScreen(c: ConditionsController, onRequestLocation: (then: () -> Unit) -> Unit) {
    val v by c.view.collectAsState()
    val places by c.places.collectAsState()
    val busy by c.busy.collectAsState()
    val us by c.usUnits.collectAsState()
    val view = v ?: return
    Column(Modifier.verticalScroll(rememberScrollState()).padding(16.dp).testTag("conditions")) {
        Text("Conditions", style = MaterialTheme.typography.headlineMedium)
        Spacer(Modifier.height(8.dp))
        // Values first once a place exists; setup below. First use shows setup first.
        if (view.place != null) {
            ValuesSection(view, c, busy, us)
            LookupCard(view, c)
            PlaceCard(view, places, c, onRequestLocation)
        } else {
            LookupCard(view, c)
            PlaceCard(view, places, c, onRequestLocation)
            ValuesSection(view, c, busy, us)
        }
        PrivacyCard(view, c)
        view.attribution.forEach { Text(it, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(top = 8.dp)) }
        Text(
            "Values are estimates with their source shown. Weather can change faster than stations report.",
            style = MaterialTheme.typography.bodySmall,
            modifier = Modifier.padding(top = 4.dp),
        )
    }
}

@Composable
private fun ValuesSection(view: ConditionsView, c: ConditionsController, busy: String?, us: Boolean) {
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.padding(vertical = 8.dp)) {
        Button(
            onClick = { c.refresh() },
            enabled = view.weatherEnabled && view.place != null && busy == null,
            modifier = Modifier.heightIn(min = 56.dp).testTag("refresh"),
        ) { Text("Update weather") }
        if (c.hasBarometer) {
            OutlinedButton(onClick = { c.readPhoneSensors() }, enabled = busy == null, modifier = Modifier.heightIn(min = 56.dp)) {
                Text("Read phone barometer")
            }
        }
    }
    busy?.let { Text(it, fontWeight = FontWeight.Bold) }
    StatusLine(view)
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.heightIn(min = 48.dp)) {
        Text("US units (°F, mph, inHg)", modifier = Modifier.weight(1f))
        Switch(checked = us, onCheckedChange = c::setUsUnits)
    }
    HorizontalDivider()
    FieldList(view, us, c)
    if (view.elevationNotes.isNotEmpty()) {
        Spacer(Modifier.height(8.dp))
        view.elevationNotes.forEach { Text("• $it", style = MaterialTheme.typography.bodySmall) }
    }
    if (view.fields.any { it.reasons.contains("Station data is too old") } || view.allowOlder) {
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.heightIn(min = 48.dp)) {
            Text("Use older station data (shown with its age)", modifier = Modifier.weight(1f))
            Switch(checked = view.allowOlder, onCheckedChange = c::setAllowOlder)
        }
    }
}

@Composable
private fun LookupCard(view: ConditionsView, c: ConditionsController) {
    Card(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
        Column(Modifier.padding(12.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Weather lookup", fontWeight = FontWeight.Bold, modifier = Modifier.weight(1f))
                Switch(checked = view.weatherEnabled, onCheckedChange = c::setWeatherEnabled, modifier = Modifier.testTag("weather_switch"))
            }
            Text(
                "Sends your place, rounded to about 1 km, to the US National Weather Service, or for places it does not cover, to the " +
                    "NOAA Aviation Weather Center for nearby airport reports (METAR). Off by default; manual values work without it.",
                style = MaterialTheme.typography.bodySmall,
            )
            Text("Weather source", modifier = Modifier.padding(top = 8.dp))
            FlowRow {
                androidx.compose.material3.FilterChip(
                    selected = view.weatherProvider != "metar", onClick = { c.setWeatherProvider("auto") },
                    label = { Text("Automatic") }, modifier = Modifier.padding(end = 8.dp).heightIn(min = 48.dp).testTag("provider_auto"),
                )
                androidx.compose.material3.FilterChip(
                    selected = view.weatherProvider == "metar", onClick = { c.setWeatherProvider("metar") },
                    label = { Text("Airport reports everywhere") }, modifier = Modifier.heightIn(min = 48.dp).testTag("provider_metar"),
                )
            }
            Text(
                "Automatic uses the National Weather Service in the US and airport reports elsewhere. Airports can be tens of " +
                    "kilometres away and at a different elevation; each value shows its station and age.",
                style = MaterialTheme.typography.bodySmall,
            )
        }
    }
}

@Composable
private fun PlaceCard(
    view: ConditionsView,
    places: List<net.digitalgrease.squib.core.SavedPlaceView>,
    c: ConditionsController,
    onRequestLocation: (then: () -> Unit) -> Unit,
) {
    var lat by rememberSaveable { mutableStateOf("") }
    var lon by rememberSaveable { mutableStateOf("") }
    var label by rememberSaveable { mutableStateOf("") }
    Card(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
        Column(Modifier.padding(12.dp)) {
            Text("Place", fontWeight = FontWeight.Bold)
            val p = view.place
            Text(
                if (p == null) "No place chosen."
                else (p.label?.let { "$it · " } ?: "") + String.format(Locale.US, "%.2f, %.2f", p.lat, p.lon) +
                    when (p.source) {
                        "gps" -> " (from location)"
                        "saved_place" -> " (saved place)"
                        else -> " (entered)"
                    },
                modifier = Modifier.testTag("place_text"),
            )
            FlowRow {
                OutlinedButton(onClick = { onRequestLocation { c.useMyLocation() } }, modifier = Modifier.padding(end = 8.dp).heightIn(min = 48.dp)) {
                    Text("Use my location")
                }
                places.forEach { sp ->
                    OutlinedButton(onClick = { c.useSavedPlace(sp) }, modifier = Modifier.padding(end = 8.dp).heightIn(min = 48.dp)) { Text(sp.label) }
                }
            }
            Row {
                OutlinedTextField(lat, { lat = it }, label = { Text("Latitude") }, singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal), modifier = Modifier.weight(1f).testTag("lat"))
                Spacer(Modifier.width(8.dp))
                OutlinedTextField(lon, { lon = it }, label = { Text("Longitude") }, singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal), modifier = Modifier.weight(1f).testTag("lon"))
            }
            OutlinedTextField(label, { label = it.take(80) }, label = { Text("Label (optional)") }, singleLine = true, modifier = Modifier.fillMaxWidth())
            FlowRow {
                OutlinedButton(
                    onClick = {
                        val a = lat.toDoubleOrNull()
                        val b = lon.toDoubleOrNull()
                        if (a != null && b != null) c.setManualPlace(a, b, label)
                    },
                    modifier = Modifier.padding(end = 8.dp).heightIn(min = 48.dp).testTag("set_place"),
                ) { Text("Use these coordinates") }
                if (view.place != null) {
                    OutlinedButton(
                        onClick = { c.savePlace(label.ifBlank { view.place?.label ?: "Range" }) },
                        modifier = Modifier.heightIn(min = 48.dp),
                    ) { Text("Save place") }
                }
            }
        }
    }
}

@Composable
private fun StatusLine(view: ConditionsView) {
    val parts = mutableListOf<String>()
    view.lastRefreshUtcMs?.let { parts += "Last update attempt ${fmtDate(it)}" }
    if (view.usedCache) parts += "showing saved data"
    if (parts.isNotEmpty()) Text(parts.joinToString(" · "), style = MaterialTheme.typography.bodySmall)
    view.issues.forEach { Text("• $it", color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium) }
}

@Composable
fun FieldList(view: ConditionsView, us: Boolean, c: ConditionsController?) {
    val shown = view.fields.filter { it.field in PRIMARY || it.valueState != "missing" }
    shown.forEach { f -> FieldRow(f, us, c) }
}

@Composable
private fun FieldRow(f: FieldView, us: Boolean, c: ConditionsController?) {
    var open by remember { mutableStateOf(false) }
    val value = Units.format(f.field, f.valueSi, f.valueState, us)
    val origin = Units.originLabel(f.origin)
    val age = if (f.origin == "unavailable") "" else Units.age(f.ageMs)
    Column(
        Modifier.fillMaxWidth().clickable(role = Role.Button, onClickLabel = "Details") { open = !open }.padding(vertical = 8.dp)
            .semantics(mergeDescendants = true) { contentDescription = "${f.label}: $value, $origin${if (age.isNotEmpty()) ", $age" else ""}" }
            .testTag("field_${f.field}"),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(f.label, modifier = Modifier.weight(1f))
            Text(if (f.origin == "unavailable") "Unavailable" else value, fontWeight = FontWeight.Bold)
        }
        FlowRow {
            Badge(origin, warning = f.origin == "unavailable")
            if (age.isNotEmpty()) Badge(age, warning = f.freshness == "stale" || f.freshness == "expired")
            if (f.freshness == "stale") Badge("Stale", warning = true)
            if (f.freshness == "expired") Badge("Old data (allowed)", warning = true)
            if (f.disagreement) Badge("Sources disagree", warning = true)
        }
        if (open) Details(f, us, c)
    }
    HorizontalDivider()
}

@Composable
private fun Details(f: FieldView, us: Boolean, c: ConditionsController?) {
    Column(Modifier.padding(start = 8.dp, top = 4.dp)) {
        f.stationId?.let {
            Text("Station: $it ${f.stationName ?: ""}")
            Text("Distance ${Units.distance(f.stationDistanceM, us)}" +
                (f.stationElevationM?.let { e -> " · station elevation ${Units.format("elevation_msl", e, "si", us)}" } ?: ""))
        }
        f.observedUtcMs?.let { Text("Observed ${fmtDate(it)}") } ?: if (f.valueSi != null) Text("Observation time unknown") else Unit
        f.fetchedUtcMs?.let { Text("Fetched ${fmtDate(it)}") }
        f.qc?.let { Text("Provider quality code: $it") }
        if (f.originalValue != null && f.originalUnit != null) Text("As received: ${f.originalValue} ${f.originalUnit}")
        f.datum?.let { Text("Height reference: $it") }
        f.accuracy?.let { Text(String.format(Locale.US, "Reported accuracy: %.1f", it)) }
        f.reasons.forEach { Text("Why: $it") }
        if (f.alternatives.isNotEmpty()) {
            Text("Other sources:", fontWeight = FontWeight.SemiBold)
            f.alternatives.forEach { a ->
                Text("• ${Units.format(f.field, a.valueSi, a.valueState, us)} · ${a.sourceLabel} · ${Units.age(a.ageMs)}")
            }
        }
        if (c != null) OverrideEditor(f, us, c)
    }
}

@Composable
private fun OverrideEditor(f: FieldView, us: Boolean, c: ConditionsController) {
    val unit = Units.unitFor(f.field, us) ?: return
    var text by remember { mutableStateOf("") }
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(top = 4.dp)) {
        OutlinedTextField(
            text, { text = it }, singleLine = true,
            label = { Text("Your value (${unit.label})") },
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal),
            modifier = Modifier.weight(1f).testTag("override_${f.field}"),
        )
        Spacer(Modifier.width(8.dp))
        OutlinedButton(onClick = {
            text.toDoubleOrNull()?.let { c.setOverride(f.field, unit.toSi(it), it, unit.label) }
        }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Set") }
    }
    FlowRow {
        if (f.field == "wind_direction") {
            TextButton(onClick = { c.setCalmOverride(true) }) { Text("Calm") }
            TextButton(onClick = { c.setCalmOverride(false) }) { Text("Variable") }
        }
        if (f.overridden) TextButton(onClick = { c.clearOverride(f.field) }) { Text("Clear my value") }
    }
    if (f.field == "local_pressure") {
        Text("Enter actual pressure at this place (not sea-level or altimeter setting).", style = MaterialTheme.typography.bodySmall)
    }
}

@Composable
private fun PrivacyCard(view: ConditionsView, c: ConditionsController) {
    Card(Modifier.fillMaxWidth().padding(vertical = 8.dp)) {
        Column(Modifier.padding(12.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Save precise location with runs", fontWeight = FontWeight.Bold, modifier = Modifier.weight(1f))
                Switch(checked = view.preciseRetention, onCheckedChange = c::setPreciseRetention)
            }
            Text(
                "Off: runs keep weather values, sources, and ages, but not coordinates, station names, or elevation. On: full detail is kept on this phone.",
                style = MaterialTheme.typography.bodySmall,
            )
            TextButton(onClick = { c.clearCache() }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Clear saved weather data") }
        }
    }
}
