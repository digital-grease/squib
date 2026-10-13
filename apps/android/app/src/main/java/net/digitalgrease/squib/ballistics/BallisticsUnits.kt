package net.digitalgrease.squib.ballistics

import java.util.Locale
import kotlin.math.abs
import kotlin.math.roundToInt

/**
 * Display units for the ballistics screen (plan 6.4). The core works in SI; conversion
 * happens only here. `us` follows the app's US customary choice (yd, in, fps, gr, ft·lbf,
 * °F, inHg); otherwise metric (m, cm, m/s, g, J, °C, hPa). Angles are mil or MOA, chosen
 * separately because that follows the scope, not the country.
 */
class BallisticsUnits(val us: Boolean, val mil: Boolean) {
    val distance get() = if (us) "yd" else "m"
    val small get() = if (us) "in" else "cm"
    val speed get() = if (us) "fps" else "m/s"
    val weight get() = if (us) "gr" else "g"
    val energy get() = if (us) "ft·lbf" else "J"
    val temperature get() = if (us) "°F" else "°C"
    val pressure get() = if (us) "inHg" else "hPa"
    val windSpeed get() = if (us) "mph" else "m/s"
    val angle get() = if (mil) "mil" else "MOA"

    fun distanceToSi(v: Double) = if (us) v * YD else v
    fun distanceFromSi(m: Double) = if (us) m / YD else m
    fun smallToSi(v: Double) = if (us) v * IN else v / 100
    fun smallFromSi(m: Double) = if (us) m / IN else m * 100
    fun speedToSi(v: Double) = if (us) v * FT else v
    fun speedFromSi(mps: Double) = if (us) mps / FT else mps
    fun weightToSi(v: Double) = if (us) v * GRAIN else v / 1000
    fun weightFromSi(kg: Double) = if (us) kg / GRAIN else kg * 1000
    fun energyFromSi(j: Double) = if (us) j / FTLBF else j
    fun temperatureToK(v: Double) = if (us) (v - 32) * 5 / 9 + 273.15 else v + 273.15
    fun temperatureFromK(k: Double) = if (us) (k - 273.15) * 9 / 5 + 32 else k - 273.15
    fun pressureToPa(v: Double) = if (us) v * INHG else v * 100
    fun windToSi(v: Double) = if (us) v * MPH else v
    fun windFromSi(mps: Double) = if (us) mps / MPH else mps

    /** Twist is entered as the length of one turn, in inches or centimetres. */
    fun twistToSi(v: Double) = smallToSi(v)
    fun twistFromSi(m: Double) = smallFromSi(m)

    fun fmt(v: Double, decimals: Int): String = String.format(Locale.US, "%.${decimals}f", v)

    /** A correction with its direction: "2.3 mil up", "0.4 MOA left", or "0" when none. */
    fun hold(milValue: Double, moaValue: Double, positive: String, negative: String): String {
        val v = if (mil) milValue else moaValue
        val shown = fmt(abs(v), if (mil) 1 else 1)
        if (shown.toDouble() == 0.0) return "0 $angle"
        return "$shown $angle ${if (v > 0) positive else negative}"
    }

    /** Distance from the line of sight in small units: "12.4 in low", "3.1 cm right". */
    fun offset(m: Double, positive: String, negative: String): String {
        val v = smallFromSi(m)
        val shown = fmt(abs(v), 1)
        if (shown.toDouble() == 0.0) return "0 $small"
        return "$shown $small ${if (v > 0) positive else negative}"
    }

    companion object {
        const val YD = 0.9144
        const val FT = 0.3048
        const val IN = 0.0254
        const val GRAIN = 0.000_064_798_91
        const val FTLBF = 1.355_817_948
        const val INHG = 3_386.389
        const val MPH = 0.447_04

        /** Wind clock position (12 = from the target, 3 = from the right) to degrees. */
        fun clockToDeg(hour: Int): Double = ((hour % 12) * 30).toDouble()
        fun degToClock(deg: Double): Int = (((deg % 360 + 360) % 360) / 30).roundToInt().let { if (it % 12 == 0) 12 else it % 12 }

        /**
         * Distances from `from` to `to` every `step` (display units, inclusive), as SI, or
         * null when they do not make a list of 1 to 60 distances.
         */
        fun ranges(from: Double?, to: Double?, step: Double?, toSi: (Double) -> Double): List<Double>? {
            if (from == null || to == null || step == null || step <= 0 || from < 0 || to < from) return null
            val n = ((to - from) / step + 1e-9).toInt() + 1
            if (n > 60) return null
            return (0 until n).map { toSi(from + it * step) }
        }
    }
}
