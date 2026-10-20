package dev.digitalgrease.squib.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Test

class CueChannelSettingsTest {
    private val par = TimerSettings(mode = Mode.PAR_ONLY, parsText = "1.0, 1.4")

    @Test
    fun screenCuesKeepTheFlashRateLowAndOtherChannelsDoNot() {
        assertNull(par.copy(cueChannel = CueChannel.SCREEN).validate())
        assertNotNull("0.3 s apart", par.copy(cueChannel = CueChannel.SCREEN, parsText = "1.0, 1.3").validate())
        assertNotNull("first par 0.3 s after the start", par.copy(cueChannel = CueChannel.SCREEN, parsText = "0.3").validate())
        assertNull(par.copy(cueChannel = CueChannel.VIBRATION, parsText = "1.0, 1.3").validate())
        assertNull(par.copy(cueChannel = CueChannel.SOUND, parsText = "1.0, 1.3").validate())
    }

    @Test
    fun liveModeIgnoresTheParOnlyCueChoice() {
        val live = par.copy(mode = Mode.PHONE_LIVE, cueChannel = CueChannel.SCREEN, parsText = "1.0, 1.3")
        assertEquals(null, live.validate())
    }
}
