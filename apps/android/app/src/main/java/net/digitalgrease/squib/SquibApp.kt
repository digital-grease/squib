package net.digitalgrease.squib

import android.app.Application
import net.digitalgrease.squib.core.SquibEngine

class SquibApp : Application() {
    /** Opened once per process; migrations and interrupted-run recovery run here. */
    val engine: SquibEngine by lazy {
        SquibEngine(getDatabasePath("journal.db").also { it.parentFile?.mkdirs() }.absolutePath, System.currentTimeMillis())
    }
}
