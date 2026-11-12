package io.github.chalharu.nerust

import android.os.Handler
import android.os.Looper
import java.util.concurrent.Executors
import java.util.concurrent.ScheduledExecutorService
import java.util.concurrent.TimeUnit

/** Single background lane for every JNI/query/scan: the wire format stays
 * identical, results post back to the main looper. One thread so Rust vault
 * commands stay ordered and hotplug bursts cannot overlap. */
internal object BridgeIo {
    private val io: ScheduledExecutorService =
        Executors.newSingleThreadScheduledExecutor { runnable ->
            Thread(runnable, "nerust-bridge-io").apply { isDaemon = true }
        }

    private val mainHandler: Handler? by lazy {
        try {
            val looper = Looper.getMainLooper() ?: return@lazy null
            Handler(looper)
        } catch (_: Exception) {
            null
        }
    }

    fun runIo(block: () -> Unit) {
        io.execute { block() }
    }

    fun scheduleIo(delayMs: Long, block: () -> Unit) {
        io.schedule({ block() }, delayMs.coerceAtLeast(0L), TimeUnit.MILLISECONDS)
    }

    fun postMain(block: () -> Unit) {
        val handler = mainHandler
        if (handler == null) {
            block()
            return
        }
        if (Looper.myLooper() === handler.looper) {
            block()
        } else {
            handler.post { block() }
        }
    }
}
