package dev.digitalgrease.squib.conditions

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class RefreshPolicyTest {
    private val now = 1_791_633_600_000L
    private val min = 60_000L

    private fun due(last: Long?, auto: Long? = null, enabled: Boolean = true, place: Boolean = true, busy: Boolean = false, run: Boolean = false) =
        RefreshPolicy.due(now, last, auto, enabled, place, busy, run)

    @Test
    fun refreshesWhenFifteenMinutesHavePassedSinceTheLastAttempt() {
        assertTrue("first use", due(null))
        assertFalse(due(now - 14 * min))
        assertTrue(due(now - 15 * min))
        assertFalse("a failed automatic attempt counts too", due(now - 30 * min, auto = now - 5 * min))
        assertTrue("clock set backwards", due(now + 60 * min))
    }

    @Test
    fun neverWithoutOptInAPlaceOrDuringARun() {
        assertFalse(due(null, enabled = false))
        assertFalse(due(null, place = false))
        assertFalse(due(null, busy = true))
        assertFalse("no network work during a run", due(null, run = true))
    }
}
