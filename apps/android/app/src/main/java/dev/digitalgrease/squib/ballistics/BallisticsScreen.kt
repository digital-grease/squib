package dev.digitalgrease.squib.ballistics

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
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
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import dev.digitalgrease.squib.core.AirChoice
import dev.digitalgrease.squib.core.CardView
import dev.digitalgrease.squib.core.CoriolisInput
import dev.digitalgrease.squib.core.LoadInput
import dev.digitalgrease.squib.core.LoadView
import dev.digitalgrease.squib.ui.AngleContext
import dev.digitalgrease.squib.ui.BackButton
import dev.digitalgrease.squib.ui.Badge
import dev.digitalgrease.squib.ui.LabeledToggle
import dev.digitalgrease.squib.ui.PrimerLink
import dev.digitalgrease.squib.ui.fmtDate

/**
 * Ballistics calculator (plan 6.4, Experimental). Reached from Conditions. A range card
 * from a saved load, the air (current conditions or entered), wind and shot angle, and
 * optionally spin drift and Coriolis (plan 6.5), both off unless chosen.
 */
@Composable
fun BallisticsScreen(c: BallisticsController, us: Boolean, onBack: () -> Unit) {
    var page by rememberSaveable { mutableStateOf("main") }
    var editing by rememberSaveable { mutableStateOf<String?>(null) }
    val mil by c.mil.collectAsState()
    val u = BallisticsUnits(us, mil)
    LaunchedEffect(Unit) { c.reload() }
    BackHandler(enabled = page != "main") { page = "main" }
    when (page) {
        "edit" -> LoadEditor(c, u, editing, onDone = { page = "main" })
        else -> MainPage(c, u, onBack, onEdit = { editing = it; page = "edit" })
    }
}

@Composable
private fun Section(title: String, content: @Composable () -> Unit) {
    Card(Modifier.fillMaxWidth().padding(vertical = 6.dp)) {
        Column(Modifier.padding(12.dp)) {
            Text(title, fontWeight = FontWeight.Bold, modifier = Modifier.semantics { heading() })
            content()
        }
    }
}

@Composable
private fun Choice(label: String, selected: Boolean, tag: String, onSelect: () -> Unit) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)
            .selectable(selected = selected, role = Role.RadioButton, onClick = onSelect).testTag(tag),
    ) {
        RadioButton(selected = selected, onClick = null)
        Text(label, modifier = Modifier.padding(start = 8.dp))
    }
}

@Composable
private fun NumberField(value: String, onChange: (String) -> Unit, label: String, tag: String, modifier: Modifier = Modifier) {
    OutlinedTextField(
        value, onChange,
        label = { Text(label) },
        singleLine = true,
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal),
        modifier = modifier.padding(top = 4.dp).testTag(tag),
    )
}

private fun num(s: String): Double? = s.trim().replace(',', '.').toDoubleOrNull()

@Composable
private fun MainPage(c: BallisticsController, u: BallisticsUnits, onBack: () -> Unit, onEdit: (String?) -> Unit) {
    val loads by c.loads.collectAsState()
    val air by c.air.collectAsState()
    val result by c.result.collectAsState()
    val cards by c.cards.collectAsState()
    val busy by c.busy.collectAsState()
    val message by c.message.collectAsState()
    val active = loads.firstOrNull { it.active } ?: loads.firstOrNull()

    var useConditions by rememberSaveable { mutableStateOf(true) }
    var pressure by rememberSaveable { mutableStateOf("") }
    var temperature by rememberSaveable { mutableStateOf("") }
    var humidity by rememberSaveable { mutableStateOf("50") }
    var wind by rememberSaveable { mutableStateOf("0") }
    var clock by rememberSaveable { mutableStateOf(3) }
    var angle by rememberSaveable { mutableStateOf("0") }
    var from by rememberSaveable { mutableStateOf("100") }
    var to by rememberSaveable { mutableStateOf("1000") }
    var step by rememberSaveable { mutableStateOf("100") }
    var title by rememberSaveable { mutableStateOf("") }
    var spin by rememberSaveable { mutableStateOf(false) }
    var coriolis by rememberSaveable { mutableStateOf(false) }
    var latitude by rememberSaveable { mutableStateOf("") }
    var bearing by rememberSaveable { mutableStateOf("") }
    val canSpin = BallisticsController.canSpin(active)
    // Suggest the conditions' latitude (whole degrees) once, if the shooter has not typed one.
    LaunchedEffect(air?.latitudeDeg) { air?.latitudeDeg?.let { if (latitude.isBlank()) latitude = u.fmt(it, 0) } }

    fun request(): Pair<dev.digitalgrease.squib.core.CardRequest?, String?> {
        val load = active ?: return null to "Add a load first."
        val choice = if (useConditions) AirChoice.Conditions else {
            val p = num(pressure) ?: return null to "Enter the pressure."
            val t = num(temperature) ?: return null to "Enter the temperature."
            val h = num(humidity) ?: return null to "Enter the humidity."
            AirChoice.Manual(u.pressureToPa(p), u.temperatureToK(t), h / 100)
        }
        val w = num(wind) ?: return null to "Enter the wind speed (0 for none)."
        val a = num(angle) ?: return null to "Enter the shot angle (0 for level)."
        val r = BallisticsUnits.ranges(num(from), num(to), num(step), u::distanceToSi)
            ?: return null to "Distances: from, to and step must make 1 to 60 distances."
        val earth = if (!coriolis) null else {
            val lat = num(latitude) ?: return null to "Enter the latitude for Earth's rotation."
            val az = num(bearing) ?: return null to "Enter the shot direction for Earth's rotation."
            CoriolisInput(lat, az)
        }
        return BallisticsController.request(load.id, choice, u.windToSi(w), BallisticsUnits.clockToDeg(clock), a, r, spin && canSpin, earth) to null
    }

    Column(Modifier.verticalScroll(rememberScrollState()).padding(16.dp).testTag("ballistics")) {
        BackButton("Back", "Back to Conditions", onBack)
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text("Ballistics", style = MaterialTheme.typography.headlineMedium, modifier = Modifier.weight(1f))
            Badge("Experimental", warning = true)
        }
        Text(
            "A point-mass trajectory from your load's published ballistic coefficient and your measured muzzle velocity. " +
                "It agrees with another solver and with published tables for supersonic flight, but it is only as good as " +
                "its inputs: confirm every distance on paper before relying on it.",
            style = MaterialTheme.typography.bodySmall,
            modifier = Modifier.testTag("ballistics_notice"),
        )

        Section("Load") {
            if (loads.isEmpty()) Text("No loads yet. Add one with its bullet, ballistic coefficient and muzzle velocity.")
            loads.forEach { l ->
                Choice("${l.input.name} (${l.input.dragModel} ${u.fmt(l.input.bcLbIn2, 3)})", l.id == active?.id, "load_${l.input.name}") {
                    c.selectLoad(l.id)
                }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedButton(onClick = { onEdit(null) }, modifier = Modifier.heightIn(min = 48.dp).testTag("load_add")) { Text("Add load") }
                if (active != null) {
                    OutlinedButton(onClick = { onEdit(active.id) }, modifier = Modifier.heightIn(min = 48.dp).testTag("load_edit")) { Text("Edit") }
                }
            }
            active?.let { l ->
                Text(
                    "${u.fmt(u.weightFromSi(l.input.bulletMassKg), 1)} ${u.weight}, ${u.fmt(u.speedFromSi(l.input.muzzleVelocityMps), 0)} ${u.speed}, " +
                        "zeroed at ${u.fmt(u.distanceFromSi(l.input.zeroRangeM), 0)} ${u.distance}, sight ${u.fmt(u.smallFromSi(l.input.sightHeightM), 2)} ${u.small} " +
                        "(version ${l.version})",
                    style = MaterialTheme.typography.bodySmall,
                )
            }
        }

        Section("Air") {
            Choice("Current conditions", useConditions, "air_conditions") { useConditions = true }
            Choice("Enter the air here", !useConditions, "air_manual") { useConditions = false }
            if (useConditions) {
                val a = air
                if (a == null || !a.available) {
                    Text(a?.reason ?: "Checking conditions…", color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("air_reason"))
                } else {
                    Text(
                        "Air density ${u.fmt(a.densityKgM3!!, 3)} kg/m³, ${u.fmt(u.temperatureFromK(a.temperatureK!!), 0)} ${u.temperature}",
                        modifier = Modifier.testTag("air_summary"),
                    )
                    a.inputs.forEach { Text(it, style = MaterialTheme.typography.bodySmall) }
                    if (a.usesOlderData) Text("Some of these values are old.", color = MaterialTheme.colorScheme.error)
                }
            } else {
                NumberField(pressure, { pressure = it }, "Station pressure (${u.pressure}), as read, not sea level", "air_pressure", Modifier.fillMaxWidth())
                NumberField(temperature, { temperature = it }, "Temperature (${u.temperature})", "air_temperature", Modifier.fillMaxWidth())
                NumberField(humidity, { humidity = it }, "Humidity (%)", "air_humidity", Modifier.fillMaxWidth())
            }
        }

        Section("Wind and angle") {
            NumberField(wind, { wind = it }, "Wind speed (${u.windSpeed})", "wind_speed", Modifier.fillMaxWidth())
            Row(verticalAlignment = Alignment.CenterVertically) {
                OutlinedButton(onClick = { clock = if (clock == 1) 12 else clock - 1 }, modifier = Modifier.heightIn(min = 48.dp).semantics {
                    contentDescription = "Wind direction counter-clockwise"
                }) { Text("−") }
                Text(
                    "From ${clock} o'clock" + when (clock) { 12 -> " (head)"; 3 -> " (right)"; 6 -> " (tail)"; 9 -> " (left)"; else -> "" },
                    modifier = Modifier.weight(1f).padding(horizontal = 8.dp).testTag("wind_clock"),
                )
                OutlinedButton(onClick = { clock = if (clock == 12) 1 else clock + 1 }, modifier = Modifier.heightIn(min = 48.dp).semantics {
                    contentDescription = "Wind direction clockwise"
                }) { Text("+") }
            }
            Text("12 o'clock is wind from the target, 3 o'clock from your right.", style = MaterialTheme.typography.bodySmall)
            PrimerLink("drop-windage", "How much of the wind counts?")
            NumberField(angle, { angle = it }, "Shot angle (degrees, uphill positive)", "shot_angle", Modifier.fillMaxWidth())
        }

        Section("Distances (${u.distance})") {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                NumberField(from, { from = it }, "From", "range_from", Modifier.weight(1f))
                NumberField(to, { to = it }, "To", "range_to", Modifier.weight(1f))
                NumberField(step, { step = it }, "Step", "range_step", Modifier.weight(1f))
            }
            LabeledToggle("Corrections in mil (off: MOA)", u.mil, { c.setMil(it) }, controlAtEnd = true, tag = "units_mil")
            PrimerLink("angle-units", "What are mil and MOA?")
        }

        Section("Optional effects") {
            Text(
                "Both are off unless you turn them on. Each is checked against another solver; " +
                    "together they move the bullet a few centimetres at 500 m and tens of centimetres at 1000 m.",
                style = MaterialTheme.typography.bodySmall,
            )
            LabeledToggle("Spin drift", spin && canSpin, { spin = it }, controlAtEnd = true, enabled = canSpin, tag = "spin_drift")
            Text(
                if (canSpin) "From the load's twist and bullet size (Litz's approximation)."
                else "Needs the load's twist, bullet diameter and bullet length: add them with Edit.",
                style = MaterialTheme.typography.bodySmall,
                modifier = Modifier.testTag("spin_note"),
            )
            LabeledToggle("Earth's rotation (Coriolis)", coriolis, { coriolis = it }, controlAtEnd = true, tag = "coriolis")
            if (coriolis) {
                NumberField(latitude, { latitude = it }, "Latitude (degrees, south negative)", "coriolis_latitude", Modifier.fillMaxWidth())
                if (air?.latitudeDeg != null) Text("Suggested from the conditions' place, to the nearest degree.", style = MaterialTheme.typography.bodySmall)
                NumberField(bearing, { bearing = it }, "Shot direction (compass degrees from true north)", "coriolis_bearing", Modifier.fillMaxWidth())
            }
        }

        var problem by remember { mutableStateOf<String?>(null) }
        Button(
            onClick = { c.clearMessage(); val (req, why) = request(); problem = why; req?.let { c.solve(it) } },
            enabled = !busy,
            modifier = Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(top = 6.dp).testTag("ballistics_solve"),
        ) { Text("Calculate") }
        problem?.let { Text(it, color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("ballistics_problem")) }
        message?.let { Text(it, modifier = Modifier.padding(top = 4.dp).testTag("ballistics_message")) }

        result?.let { r ->
            RangeCard(r, u)
            if (r.id == null) {
                OutlinedTextField(title, { title = it.take(80) }, label = { Text("Title (optional)") }, singleLine = true,
                    modifier = Modifier.fillMaxWidth().testTag("card_title"))
                OutlinedButton(
                    onClick = { val (req, why) = request(); problem = why; req?.let { c.save(it, title.trim()) } },
                    enabled = !busy,
                    modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).padding(top = 4.dp).testTag("card_save"),
                ) { Text("Save range card") }
            }
        }

        if (cards.isNotEmpty()) {
            Section("Saved range cards") {
                cards.forEach { card ->
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        TextButton(onClick = { c.show(card) }, modifier = Modifier.weight(1f).heightIn(min = 48.dp).testTag("card_open")) {
                            Text("${card.title.ifEmpty { card.load.input.name }}, ${card.createdUtcMs?.let { fmtDate(it) } ?: ""}")
                        }
                        TextButton(onClick = { card.id?.let(c::deleteCard) }, modifier = Modifier.heightIn(min = 48.dp).semantics {
                            contentDescription = "Delete range card ${card.title.ifEmpty { card.load.input.name }}"
                        }) { Text("Delete") }
                    }
                }
            }
        }
    }
}

@Composable
private fun RangeCard(r: CardView, u: BallisticsUnits) {
    Section(if (r.id != null) "Range card: ${r.title.ifEmpty { r.load.input.name }}" else "Range card") {
        Text(
            "${r.load.input.name} (version ${r.load.version}), air ${u.fmt(r.densityKgM3, 3)} kg/m³ at " +
                "${u.fmt(u.temperatureFromK(r.temperatureK), 0)} ${u.temperature} " + (if (r.airFromConditions) "from conditions" else "as entered") +
                ", wind ${u.fmt(u.windFromSi(r.windMps), 0)} ${u.windSpeed} from ${BallisticsUnits.degToClock(r.windFromDeg)} o'clock, " +
                "angle ${u.fmt(r.lookAngleDeg, 0)}°" + (r.createdUtcMs?.let { ", saved ${fmtDate(it)}" } ?: ""),
            style = MaterialTheme.typography.bodySmall,
        )
        val effects = listOfNotNull(
            r.stability?.let { "spin drift included (stability ${u.fmt(it, 2)})" },
            r.coriolis?.let { "Earth's rotation included (latitude ${u.fmt(it.latitudeDeg, 1)}°, shot direction ${u.fmt(it.azimuthDeg, 0)}°)" },
        )
        Text(
            if (effects.isEmpty()) "No spin drift or Earth's rotation." else effects.joinToString("; ").replaceFirstChar { it.uppercase() } + ".",
            style = MaterialTheme.typography.bodySmall,
            modifier = Modifier.testTag("card_effects"),
        )
        Column(Modifier.testTag("range_card")) {
            r.rows.forEach { row ->
                val d = "${u.fmt(u.distanceFromSi(row.rangeM), 0)} ${u.distance}"
                val elev = u.hold(row.elevationMil, row.elevationMoa, "up", "down")
                val wind = u.hold(row.windageMil, row.windageMoa, "right", "left")
                val drop = u.offset(row.dropM, "high", "low")
                val vel = "${u.fmt(u.speedFromSi(row.velocityMps), 0)} ${u.speed}"
                val time = "${u.fmt(row.timeS, 2)} s"
                val energy = "${u.fmt(u.energyFromSi(row.energyJ), 0)} ${u.energy}"
                val slow = if (row.transonic) ". Below Mach 1.2: less reliable" else ""
                val drift = if (r.spinDrift) ", of which spin drift ${u.offset(row.spinDriftM, "right", "left")}" else ""
                Column(
                    Modifier.fillMaxWidth().padding(vertical = 4.dp)
                        .clearAndSetSemantics { contentDescription = "$d: elevation $elev, wind $wind$drift, bullet $drop, $vel, $time, $energy$slow" },
                ) {
                    Text("$d   $elev   $wind", fontWeight = FontWeight.Bold)
                    Text("$drop · $vel · $time · $energy", style = MaterialTheme.typography.bodySmall)
                    if (r.spinDrift) Text("Spin drift ${u.offset(row.spinDriftM, "right", "left")}", style = MaterialTheme.typography.bodySmall)
                    if (row.transonic) Text("Below Mach 1.2: less reliable", color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
                }
            }
            if (r.rows.any { it.transonic }) PrimerLink("transonic", "Why is below Mach 1.2 less reliable?")
            if (r.unreachedM.isNotEmpty()) {
                Text("Not reached: ${r.unreachedM.joinToString { "${u.fmt(u.distanceFromSi(it), 0)} ${u.distance}" }}", color = MaterialTheme.colorScheme.error)
            }
        }
        if (r.usesOlderData) Text("Made with some older condition values.", color = MaterialTheme.colorScheme.error)
        // The converter starts from the farthest elevation correction on this card.
        val far = r.rows.lastOrNull()?.let { AngleContext(kotlin.math.abs(if (u.mil) it.elevationMil else it.elevationMoa), if (u.mil) "mil" else "moa", it.rangeM) }
        PrimerLink("angle-units", "Reading this card: mil, MOA and the converter", context = far, tag = "primer_card_units")
        PrimerLink("clicks", "Dial or hold these corrections?")
        Text("Method: ${r.method}", style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(top = 4.dp))
    }
}

@Composable
private fun LoadEditor(c: BallisticsController, u: BallisticsUnits, id: String?, onDone: () -> Unit) {
    val loads by c.loads.collectAsState()
    val equipment by c.equipment.collectAsState()
    val busy by c.busy.collectAsState()
    val existing: LoadView? = loads.firstOrNull { it.id == id }
    val e = existing?.input
    var name by rememberSaveable { mutableStateOf(e?.name ?: "") }
    var g7 by rememberSaveable { mutableStateOf(e?.dragModel != "G1") }
    var bc by rememberSaveable { mutableStateOf(e?.bcLbIn2?.toString() ?: "") }
    var weight by rememberSaveable { mutableStateOf(e?.let { u.fmt(u.weightFromSi(it.bulletMassKg), 1) } ?: "") }
    var mv by rememberSaveable { mutableStateOf(e?.let { u.fmt(u.speedFromSi(it.muzzleVelocityMps), 0) } ?: "") }
    var sight by rememberSaveable { mutableStateOf(e?.let { u.fmt(u.smallFromSi(it.sightHeightM), 2) } ?: (if (u.us) "1.5" else "4")) }
    var zero by rememberSaveable { mutableStateOf(e?.let { u.fmt(u.distanceFromSi(it.zeroRangeM), 0) } ?: "100") }
    var twist by rememberSaveable { mutableStateOf(e?.twistM?.let { u.fmt(u.twistFromSi(kotlin.math.abs(it)), 1) } ?: "") }
    var leftTwist by rememberSaveable { mutableStateOf((e?.twistM ?: 0.0) < 0) }
    var diameter by rememberSaveable { mutableStateOf(e?.bulletDiameterM?.let { u.fmt(u.bulletFromSi(it), if (u.us) 3 else 2) } ?: "") }
    var length by rememberSaveable { mutableStateOf(e?.bulletLengthM?.let { u.fmt(u.bulletFromSi(it), if (u.us) 3 else 2) } ?: "") }
    var rifle by rememberSaveable { mutableStateOf(e?.equipmentId) }
    var notes by rememberSaveable { mutableStateOf(e?.notes ?: "") }
    var problem by remember { mutableStateOf<String?>(null) }
    val message by c.message.collectAsState()

    Column(Modifier.verticalScroll(rememberScrollState()).padding(16.dp).testTag("load_editor")) {
        BackButton("Back", "Back to Ballistics without saving", onDone)
        Text(if (existing == null) "New load" else "Edit load", style = MaterialTheme.typography.headlineSmall)
        if (existing != null) Text("Saving makes version ${existing.version + 1u}; saved range cards keep the version they used.", style = MaterialTheme.typography.bodySmall)
        OutlinedTextField(name, { name = it.take(80) }, label = { Text("Name") }, singleLine = true, modifier = Modifier.fillMaxWidth().testTag("load_name"))
        Text("Ballistic coefficient", fontWeight = FontWeight.Bold, modifier = Modifier.padding(top = 8.dp))
        PrimerLink("bc", "What are BC, G1 and G7?")
        Choice("G7 (boat-tail match and long-range bullets)", g7, "model_g7") { g7 = true }
        Choice("G1 (most published hunting and flat-base bullets)", !g7, "model_g1") { g7 = false }
        NumberField(bc, { bc = it }, "BC (${if (g7) "G7" else "G1"}, as published, lb/in²)", "load_bc", Modifier.fillMaxWidth())
        NumberField(weight, { weight = it }, "Bullet weight (${u.weight})", "load_weight", Modifier.fillMaxWidth())
        NumberField(mv, { mv = it }, "Muzzle velocity (${u.speed}), measured from your rifle", "load_mv", Modifier.fillMaxWidth())
        NumberField(sight, { sight = it }, "Sight height above the bore (${u.small})", "load_sight", Modifier.fillMaxWidth())
        NumberField(zero, { zero = it }, "Zero distance (${u.distance})", "load_zero", Modifier.fillMaxWidth())
        PrimerLink("zero", "What do zero and sight height do?")
        Text("For spin drift (optional)", fontWeight = FontWeight.Bold, modifier = Modifier.padding(top = 8.dp))
        NumberField(twist, { twist = it }, "Barrel twist, one turn in (${u.small})", "load_twist", Modifier.fillMaxWidth())
        if (twist.isNotBlank()) LabeledToggle("Left-hand twist", leftTwist, { leftTwist = it }, controlAtEnd = true)
        NumberField(diameter, { diameter = it }, "Bullet diameter (${u.bullet})", "load_diameter", Modifier.fillMaxWidth())
        NumberField(length, { length = it }, "Bullet length (${u.bullet}), from the maker or measured", "load_length", Modifier.fillMaxWidth())
        if (equipment.isNotEmpty()) {
            Text("Rifle (optional)", fontWeight = FontWeight.Bold, modifier = Modifier.padding(top = 8.dp))
            Choice("None", rifle == null, "rifle_none") { rifle = null }
            equipment.forEach { eq -> Choice(eq.name, rifle == eq.id, "rifle_${eq.name}") { rifle = eq.id } }
        }
        OutlinedTextField(notes, { notes = it.take(1000) }, label = { Text("Notes") }, modifier = Modifier.fillMaxWidth().testTag("load_notes"))
        Button(
            onClick = {
                val b = num(bc); val w = num(weight); val v = num(mv); val s = num(sight); val z = num(zero)
                val t = if (twist.isBlank()) null else num(twist)
                val dia = if (diameter.isBlank()) null else num(diameter)
                val len = if (length.isBlank()) null else num(length)
                problem = when {
                    name.isBlank() -> "Enter a name."
                    b == null -> "Enter the ballistic coefficient."
                    w == null -> "Enter the bullet weight."
                    v == null -> "Enter the muzzle velocity."
                    s == null -> "Enter the sight height."
                    z == null -> "Enter the zero distance."
                    twist.isNotBlank() && t == null -> "Twist must be a number, or leave it empty."
                    diameter.isNotBlank() && dia == null -> "Bullet diameter must be a number, or leave it empty."
                    length.isNotBlank() && len == null -> "Bullet length must be a number, or leave it empty."
                    else -> null
                }
                if (problem == null) {
                    c.clearMessage()
                    val input = LoadInput(
                        name.trim(), rifle, if (g7) "G7" else "G1", b!!, u.weightToSi(w!!), u.speedToSi(v!!), u.smallToSi(s!!),
                        u.distanceToSi(z!!), dia?.let(u::bulletToSi), len?.let(u::bulletToSi),
                        t?.let { u.twistToSi(it) * if (leftTwist) -1 else 1 }, notes,
                    )
                    c.saveLoad(id, input, onDone)
                }
            },
            enabled = !busy,
            modifier = Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(top = 8.dp).testTag("load_save"),
        ) { Text("Save load") }
        (problem ?: message)?.let { Text(it, color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("load_problem")) }
        if (existing != null) {
            TextButton(onClick = { c.archiveLoad(existing.id); onDone() }, modifier = Modifier.heightIn(min = 48.dp).testTag("load_archive")) {
                Text("Remove from the list (saved cards keep it)")
            }
        }
    }
}
