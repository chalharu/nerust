package io.github.chalharu.nerust

import java.io.InputStream
import java.util.concurrent.Executor
import java.util.concurrent.Executors

/** Reads/explicit close serialize per session. Activity destruction only detaches ownership.
 * No Android dependencies: the actual transport semantics are JVM-testable. */
internal class SafReadSessions(
    private val closeExecutor: Executor = cleanupExecutor,
) : AutoCloseable {
    private class Session(val stream: InputStream)
    private val streams = mutableMapOf<Long, Session>()
    private var nextId = 1L
    private var closed = false

    fun open(stream: InputStream): Long {
        val id = synchronized(this) {
            if (closed || nextId <= 0) 0L else nextId++.also { streams[it] = Session(stream) }
        }
        if (id == 0L) closeExecutor.execute { runCatching { stream.close() } }
        return id
    }

    /** null is an error, never EOF. Short reads remain data. */
    fun read(id: Long, maxBytes: Int): ByteArray? {
        if (maxBytes !in 1..SAF_CHUNK_BYTES) return null
        val session = synchronized(this) { streams[id] } ?: return null
        return synchronized(session) {
            if (synchronized(this) { streams[id] !== session }) return null
            val stream = session.stream
            try {
                val buffer = ByteArray(maxBytes)
                when (val count = stream.read(buffer)) {
                    -1 -> ByteArray(0)
                    0 -> {
                        // Some providers return zero without EOF; force progress.
                        val byte = stream.read()
                        if (byte < 0) ByteArray(0) else byteArrayOf(byte.toByte())
                    }
                    else -> buffer.copyOf(count)
                }
            } catch (_: Exception) {
                close(id)
                null
            }
        }
    }

    fun close(id: Long) {
        val session = synchronized(this) { streams.remove(id) } ?: return
        synchronized(session) { runCatching { session.stream.close() } }
    }

    override fun close() {
        val detached = synchronized(this) {
            closed = true
            streams.values.toList().also { streams.clear() }
        }
        detached.forEach { session ->
            closeExecutor.execute {
                synchronized(session) { runCatching { session.stream.close() } }
            }
        }
    }

    companion object {
        // Provider reads/close may block indefinitely: never join from Activity.onDestroy.
        private val cleanupExecutor = Executors.newCachedThreadPool { runnable ->
            Thread(runnable, "nerust-saf-close").apply { isDaemon = true }
        }
    }
}