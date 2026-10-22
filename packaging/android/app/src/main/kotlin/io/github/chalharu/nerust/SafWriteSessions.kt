package io.github.chalharu.nerust

import java.io.OutputStream
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executor

/** Registry locks protect ownership only. Provider I/O is serialized per session off the UI thread. */
internal class SafWriteSessions(private val cleanup: Executor = SafCleanup.executor) : AutoCloseable {
    interface Transaction {
        val stream: OutputStream
        fun commit(): Boolean
        fun abort()
    }

    private class Session(val transaction: Transaction) {
        var failed = false
        var closed = false
    }
    // Lock-free reads: per-session monitors never nest inside the registry monitor.
    // Registry monitor guards only id minting and teardown; lookups use the map directly.
    private val sessions = ConcurrentHashMap<Long, Session>()
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
        val session = sessions[id] ?: return false
        return synchronized(session) {
            if (sessions[id] !== session || session.failed) return false
            if (bytes.size > SAF_CHUNK_BYTES) { session.failed = true; return false }
            try { session.transaction.stream.write(bytes); true }
            catch (_: Exception) { session.failed = true; false }
        }
    }

    fun finish(id: Long, commit: Boolean): Boolean {
        val session = sessions.remove(id) ?: return false
        return synchronized(session) {
            val transaction = session.transaction
            var committed = false
            try {
                if (commit && !session.failed) transaction.stream.flush()
                // Close carries the durable sync: its failure aborts the marker publish.
                if (!closeOnce(session)) session.failed = true
                if (commit && !session.failed) committed = transaction.commit()
            } catch (_: Exception) {
                session.failed = true
                closeOnce(session)
            } finally {
                if (!committed) runCatching { transaction.abort() }
            }
            committed
        }
    }

    /** Idempotent stream close: reports success; the exception path reuses it without double-closing. */
    private fun closeOnce(session: Session): Boolean {
        if (!session.closed) {
            session.closed = true
            if (runCatching { session.transaction.stream.close() }.isFailure) return false
        }
        return !session.failed
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
}