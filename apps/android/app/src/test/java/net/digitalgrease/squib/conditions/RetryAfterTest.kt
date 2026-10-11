package net.digitalgrease.squib.conditions

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class RetryAfterTest {
    // 2026-10-10T12:00:00Z
    private val now = 1_791_633_600_000L

    @Test
    fun secondsAndHttpDatesAreBothUnderstood() {
        assertEquals(120u, WeatherTransport.retryAfterSeconds("120", now))
        assertEquals(30u, WeatherTransport.retryAfterSeconds("Sat, 10 Oct 2026 12:00:30 GMT", now))
        assertEquals("a date in the past means now", 0u, WeatherTransport.retryAfterSeconds("Sat, 10 Oct 2026 11:00:00 GMT", now))
        assertNull(WeatherTransport.retryAfterSeconds(null, now))
        assertNull(WeatherTransport.retryAfterSeconds("", now))
        assertNull(WeatherTransport.retryAfterSeconds("soon", now))
        assertNull(WeatherTransport.retryAfterSeconds("-5", now))
    }
}
