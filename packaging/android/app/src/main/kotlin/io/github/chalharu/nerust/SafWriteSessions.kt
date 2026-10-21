package io.github.chalharu.nerust

import java.io.OutputStream
import java.util.concurrent.Executor
import java.util.concurrent.Executors

/** Registry locks protect ownership only. Provider I/O is serialized per session off the UI thread. */
internal class SafWriteSessions(private val cleanup: Executor = cleanupExecutor) : AutoCloseable {
    interface Transaction {
        val stream: OutputStream
        fun commit(): Boolean
        fun abort()
    }

    private class Session(val transaction: Transaction) { var failed = false }
    private val sessions = mutableMapOf<Long, Session>()
    private var nextId = 1L
    private var closed = false

    fun open(transaction: Transaction): Long {
        val id = synchronized(this) {
            if (closed || nextId <= 0) 0L else nextId++.also { sessions[it] = Session(transaction) }
        }
        if (id == 0L) cleanup.execute { discard(transaction) }
        return id
    }

    fun write(id: Long, bytes: ByteArray): Boolean {
        val session = synchronized(this) { sessions[id] } ?: return false
        return synchronized(session) {
            if (synchronized(this) { sessions[id] !== session } || session.failed) return false
            if (bytes.size > SAF_CHUNK_BYTES) { session.failed = true; return false }
            try { session.transaction.stream.write(bytes); true }
            catch (_: Exception) { session.failed = true; false }
        }
    }

    fun finish(id: Long, commit: Boolean): Boolean {
        val session = synchronized(this) { sessions.remove(id) } ?: return false
        return synchronized(session) {
            val transaction = session.transaction
            var committed = false
            try {
                if (commit && !session.failed) transaction.stream.flush()
                transaction.stream.close() // Includes durable sync before publishing any marker.
                if (commit && !session.failed) committed = transaction.commit()
            } catch (_: Exception) {
                runCatching { transaction.stream.close() }
            } finally {
                if (!committed) runCatching { transaction.abort() }
            }
            committed
        }
    }

    override fun close() {
        val detached = synchronized(this) {
            closed = true
            sessions.values.toList().also { sessions.clear() }
        }
        detached.forEach { session ->
            cleanup.execute { synchronized(session) { discard(session.transaction) } }
        }
    }

    private fun discard(transaction: Transaction) {
        runCatching { transaction.stream.close() }
        runCatching { transaction.abort() }
    }

    companion object {
        private val cleanupExecutor = Executors.newCachedThreadPool { runnable ->
            Thread(runnable, "nerust-saf-abort").apply { isDaemon = true }
        }
    }
}