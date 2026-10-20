package dev.digitalgrease.squib.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class WalkthroughClockTest {
    private var now = 1_000_000L
    private val clock = WalkthroughClock { now }

    @Test
    fun countsDownFromTheClockAndEndsExactlyOnce() {
        val w = clock.start("p1", "Club day", 5, beep = true)
        assertEquals("5:00", w.label)
        now += 61_500
        assertFalse(clock.tick())
        assertEquals("3:59", clock.current!!.label)
        // A long gap (screen off, other app) is measured from the clock, not from ticks.
        now += 10 * 60_000
        assertTrue("ends on this update", clock.tick())
        assertTrue(clock.current!!.finished)
        assertEquals("0:00", clock.current!!.label)
        assertFalse("ends only once", clock.tick())
        clock.stop()
        assertNull(clock.current)
    }

    @Test
    fun minutesAreBoundedAndANewStartReplacesTheOld() {
        assertEquals(60 * 60_000L, clock.start("p1", "A", 999, beep = false).leftMs)
        assertEquals(60_000L, clock.start("p2", "B", 0, beep = false).leftMs)
        assertEquals("p2", clock.current!!.planId)
        now += 500
        clock.tick()
        assertEquals("rounded up", "1:00", clock.current!!.label)
    }
}
