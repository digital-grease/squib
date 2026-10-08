package net.digitalgrease.squib.conditions

import java.util.Locale
import kotlin.math.roundToInt

/**
 * Display conversion at the UI boundary only. The core stores SI (K, fraction, m/s,
 * degrees true FROM, Pa, m).
 */
object Units {
    data class DisplayUnit(val label: String, val toDisplay: (Double) -> Double, val toSi: (Double) -> Double, val decimals: Int)

    fun unitFor(field: String, us: Boolean): DisplayUnit? = when (field) {
        "temperature" -> if (us) DisplayUnit("°F", { (it - 273.15) * 9 / 5 + 32 }, { (it - 32) * 5 / 9 + 273.15 }, 0)
        else DisplayUnit("°C", { it - 273.15 }, { it + 273.15 }, 1)
        "relative_humidity" -> DisplayUnit("%", { it * 100 }, { it / 100 }, 0)
        "wind_speed", "wind_gust" -> if (us) DisplayUnit("mph", { it / 0.44704 }, { it * 0.44704 }, 0)
        else DisplayUnit("km/h", { it * 3.6 }, { it / 3.6 }, 0)
        "wind_direction" -> DisplayUnit("°", { it }, { it }, 0)
        "local_pressure", "remote_station_pressure", "altimeter_setting", "sea_level_pressure" ->
            if (us) DisplayUnit("inHg", { it / 3386.389 }, { it * 3386.389 }, 2) else DisplayUnit("hPa", { it / 100 }, { it * 100 }, 1)
        "elevation_msl", "elevation_ellipsoid" -> if (us) DisplayUnit("ft", { it / 0.3048 }, { it * 0.3048 }, 0)
        else DisplayUnit("m", { it }, { it }, 0)
        else -> null
    }

    private val compass = listOf("N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW", "NW", "NNW")

    fun format(field: String, valueSi: Double?, state: String, us: Boolean): String {
        when (state) {
            "calm" -> return "Calm"
            "variable" -> return "Variable"
            "missing" -> return "n/a"
        }
        val v = valueSi ?: return "n/a"
        val u = unitFor(field, us) ?: return v.toString()
        if (field == "wind_direction") {
            val i = ((v % 360 + 360) % 360 / 22.5).roundToInt() % 16
            return "${compass[i]} (${v.roundToInt()}°)"
        }
        return String.format(Locale.US, "%.${u.decimals}f %s", u.toDisplay(v), u.label)
    }

    fun age(ms: Long?): String = when {
        ms == null -> "age unknown"
        ms < 90_000 -> "just now"
        ms < 90 * 60_000 -> "${ms / 60_000} min old"
        ms < 48 * 3_600_000 -> String.format(Locale.US, "%.1f h old", ms / 3_600_000.0)
        else -> "${ms / 86_400_000} days old"
    }

    fun originLabel(origin: String): String = when (origin) {
        "here" -> "Here"
        "nearby_observation" -> "Nearby observation"
        "model_estimate" -> "Model estimate"
        "manual" -> "Manual"
        else -> "Unavailable"
    }

    fun distance(m: Double?, us: Boolean): String = when {
        m == null -> ""
        us -> String.format(Locale.US, "%.1f mi", m / 1609.344)
        else -> String.format(Locale.US, "%.1f km", m / 1000)
    }
}
