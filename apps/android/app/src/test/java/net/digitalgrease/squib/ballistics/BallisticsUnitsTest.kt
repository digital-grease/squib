package net.digitalgrease.squib.ballistics

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class BallisticsUnitsTest {
    private val us = BallisticsUnits(us = true, mil = false)
    private val metric = BallisticsUnits(us = false, mil = true)

    @Test
    fun conversionsRoundTrip() {
        assertEquals(914.4, us.distanceToSi(1000.0), 1e-9)
        assertEquals(1000.0, us.distanceFromSi(us.distanceToSi(1000.0)), 1e-9)
        assertEquals(0.0381, us.smallToSi(1.5), 1e-12)
        assertEquals(0.015, metric.smallToSi(1.5), 1e-12)
        assertEquals(822.96, us.speedToSi(2700.0), 1e-9)
        assertEquals(168.0, us.weightFromSi(us.weightToSi(168.0)), 1e-9)
        assertEquals(10.89, metric.weightFromSi(0.01089), 1e-9)
        assertEquals(288.15, us.temperatureToK(59.0), 1e-9)
        assertEquals(15.0, metric.temperatureFromK(288.15), 1e-9)
        assertEquals(101_325.0, metric.pressureToPa(1013.25), 1e-6)
        assertEquals(100_000.0, us.pressureToPa(29.53), 50.0)
        assertEquals(1.0, us.energyFromSi(1.355_817_948), 1e-9)
        assertEquals(10.0, us.windFromSi(us.windToSi(10.0)), 1e-9)
        // Bullet size: inches, or millimetres.
        assertEquals(0.007_823_2, us.bulletToSi(0.308), 1e-12)
        assertEquals(0.0315, metric.bulletToSi(31.5), 1e-12)
        assertEquals(7.82, metric.bulletFromSi(0.007_82), 1e-9)
    }

    @Test
    fun windClock() {
        assertEquals(0.0, BallisticsUnits.clockToDeg(12), 0.0)
        assertEquals(90.0, BallisticsUnits.clockToDeg(3), 0.0)
        assertEquals(270.0, BallisticsUnits.clockToDeg(9), 0.0)
        for (h in 1..12) assertEquals(h, BallisticsUnits.degToClock(BallisticsUnits.clockToDeg(h)))
        assertEquals(12, BallisticsUnits.degToClock(-5.0))
    }

    @Test
    fun holdsSayWhichWay() {
        assertEquals("2.3 mil up", metric.hold(2.31, 7.9, "up", "down"))
        assertEquals("1.4 MOA left", us.hold(-0.4, -1.37, "right", "left"))
        assertEquals("0 MOA", us.hold(0.01, 0.02, "up", "down"))
        assertEquals("12.4 in low", us.offset(-12.4 * 0.0254, "high", "low"))
        assertEquals("3.0 cm right", metric.offset(0.03, "right", "left"))
    }

    @Test
    fun rangeLists() {
        val r = BallisticsUnits.ranges(100.0, 1000.0, 100.0) { it * 0.9144 }!!
        assertEquals(10, r.size)
        assertEquals(914.4, r.last(), 1e-9)
        assertEquals(listOf(50.0), BallisticsUnits.ranges(50.0, 50.0, 25.0) { it })
        assertNull(BallisticsUnits.ranges(100.0, 50.0, 10.0) { it })
        assertNull(BallisticsUnits.ranges(0.0, 1000.0, 10.0) { it })
        assertNull(BallisticsUnits.ranges(0.0, 100.0, 0.0) { it })
        assertNull(BallisticsUnits.ranges(null, 100.0, 10.0) { it })
    }
}
