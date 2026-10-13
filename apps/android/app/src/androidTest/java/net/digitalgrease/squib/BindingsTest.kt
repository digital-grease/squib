package net.digitalgrease.squib

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import net.digitalgrease.squib.core.InternalException
import net.digitalgrease.squib.core.SquibEngine
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Plan 5.3 through the generated Kotlin bindings: a panic in the core arrives as an
 * exception (the process keeps running), the engine stays usable after it, and calls
 * from several JVM threads at once neither deadlock nor lose writes. Uses its own
 * journal, separate from the app's.
 */
@RunWith(AndroidJUnit4::class)
class BindingsTest {
    private fun engine(name: String): SquibEngine {
        val dir = File(InstrumentationRegistry.getInstrumentation().targetContext.cacheDir, "bindings-$name")
        dir.deleteRecursively()
        dir.mkdirs()
        return SquibEngine(File(dir, "journal.db").absolutePath, System.currentTimeMillis())
    }

    @Test
    fun panicsInTheCoreBecomeExceptionsAndTheEngineCarriesOn() {
        val e = engine("panic")
        val before = e.listDrills().size
        try {
            e.debugInjectPanic("call")
            fail("expected an exception")
        } catch (x: InternalException) {
            // UniFFI caught the panic at the boundary.
        }
        // The panic poisoned the engine lock; the next calls recover it.
        e.debugInjectPanic("store")
        val input = e.listDrills().first().input.copy(title = "After a panic")
        e.createDrill(input, System.currentTimeMillis())
        assertEquals(before + 1, e.listDrills().size)
        e.destroy()
    }

    @Test
    fun callsFromSeveralThreadsAtOnce() {
        val e = engine("threads")
        val before = e.listDrills().size
        val template = e.listDrills().first().input
        val created = AtomicInteger()
        val failures = AtomicInteger()
        val threads = 8
        val perThread = 25
        val done = CountDownLatch(threads)
        repeat(threads) { t ->
            Thread {
                try {
                    repeat(perThread) { i ->
                        when ((t + i) % 4) {
                            0 -> e.listDrills()
                            1 -> e.conditionsView(System.currentTimeMillis())
                            2 -> {
                                e.createDrill(template.copy(title = "Thread $t drill $i"), System.currentTimeMillis())
                                created.incrementAndGet()
                            }
                            else -> e.listRuns(20u)
                        }
                    }
                } catch (x: Exception) {
                    failures.incrementAndGet()
                } finally {
                    done.countDown()
                }
            }.start()
        }
        assertTrue("a thread is stuck: deadlock", done.await(60, TimeUnit.SECONDS))
        assertEquals(0, failures.get())
        assertEquals(before + created.get(), e.listDrills().size)
        e.destroy()
    }
}
