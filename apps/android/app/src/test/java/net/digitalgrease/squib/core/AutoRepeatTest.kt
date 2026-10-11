package net.digitalgrease.squib.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class AutoRepeatTest {
    private val set = TimerSettings(drillId = "d", drillRepeats = 3, drillRestS = 20, stringIndex = 1, autoRepeat = true)

    @Test
    fun onlyACompleteSavedStringBeforeTheLastArmsTheNext() {
        assertEquals(20, set.autoRepeatDelayS("saved", "complete"))
        assertEquals(20, set.copy(stringIndex = 2).autoRepeatDelayS("saved", "complete"))
        assertNull("last string ends the set", set.copy(stringIndex = 3).autoRepeatDelayS("saved", "complete"))
        assertNull("opt-in", set.copy(autoRepeat = false).autoRepeatDelayS("saved", "complete"))
        assertNull("drill sets only", set.copy(drillId = null).autoRepeatDelayS("saved", "complete"))
        assertNull(set.autoRepeatDelayS("saved", "interrupted"))
        assertNull(set.autoRepeatDelayS("cancelled", "cancelled"))
        assertNull(set.autoRepeatDelayS("save_pending", "complete"))
    }

    @Test
    fun theCountdownIsNeverShorterThanTheMinimum() {
        assertEquals(AUTO_REPEAT_MIN_S, set.copy(drillRestS = 0).autoRepeatDelayS("saved", "complete"))
        assertEquals(AUTO_REPEAT_MIN_S, set.copy(drillRestS = 2).autoRepeatDelayS("saved", "complete"))
    }
}
