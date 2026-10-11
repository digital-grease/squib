package net.digitalgrease.squib.ui

import androidx.compose.material3.ColorScheme
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * A21 outdoor legibility: every text colour on its background in both themes meets WCAG AA
 * for normal text (4.5:1), and control outlines meet 3:1 against the surface.
 */
class ContrastTest {
    private fun ratio(a: Color, b: Color): Double {
        val (hi, lo) = listOf(a.luminance().toDouble(), b.luminance().toDouble()).sortedDescending()
        return (hi + 0.05) / (lo + 0.05)
    }

    private fun pairs(s: ColorScheme) = mapOf(
        "primary" to (s.onPrimary to s.primary),
        "primaryContainer" to (s.onPrimaryContainer to s.primaryContainer),
        "secondary" to (s.onSecondary to s.secondary),
        "secondaryContainer" to (s.onSecondaryContainer to s.secondaryContainer),
        "tertiary" to (s.onTertiary to s.tertiary),
        "tertiaryContainer" to (s.onTertiaryContainer to s.tertiaryContainer),
        "error" to (s.onError to s.error),
        "errorContainer" to (s.onErrorContainer to s.errorContainer),
        "background" to (s.onBackground to s.background),
        "surface" to (s.onSurface to s.surface),
        "surfaceVariant" to (s.onSurfaceVariant to s.surfaceVariant),
        "onSurfaceVariant on surface" to (s.onSurfaceVariant to s.surface),
        "surfaceContainerHighest" to (s.onSurface to s.surfaceContainerHighest),
        "surfaceContainer" to (s.onSurface to s.surfaceContainer),
        "inverseSurface" to (s.inverseOnSurface to s.inverseSurface),
        "primary text on surface" to (s.primary to s.surface),
        "error text on surface" to (s.error to s.surface),
        "error text on background" to (s.error to s.background),
    )

    @Test
    fun textAndControlsMeetWcagAaInBothThemes() {
        val failures = mutableListOf<String>()
        for ((theme, s) in listOf("light" to Light, "dark" to Dark)) {
            pairs(s).forEach { (name, p) ->
                val r = ratio(p.first, p.second)
                println("%-6s %-30s %5.2f".format(theme, name, r))
                if (r < 4.5) failures += "$theme $name %.2f".format(r)
            }
            // Field borders sit next to their labels on surfaces and cards; keep them at text
            // contrast so neither reads as faint.
            for ((name, bg) in listOf("surface" to s.surface, "card" to s.surfaceContainerHighest)) {
                val r = ratio(s.outline, bg)
                println("%-6s %-30s %5.2f".format(theme, "outline on $name", r))
                if (r < 4.5) failures += "$theme outline on $name %.2f".format(r)
            }
        }
        // Fixed colours drawn by screens: screen cues (black text), video overlay (white on scrim).
        listOf(
            "screen cue GO" to ratio(Color.Black, Color(0xFF00C853)),
            "screen cue PAR" to ratio(Color.Black, Color(0xFFFFAB00)),
        ).forEach { (name, r) ->
            println("fixed  %-30s %5.2f".format(name, r))
            if (r < 4.5) failures += "$name %.2f".format(r)
        }
        assertTrue("below WCAG AA: $failures", failures.isEmpty())
    }
}
