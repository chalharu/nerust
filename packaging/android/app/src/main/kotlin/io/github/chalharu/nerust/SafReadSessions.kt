package io.github.chalharu.nerust

import java.io.InputStream

/** Synchronous stream ownership. Read and close serialize, including Activity destruction.
 * No Android dependencies: the actual transport semantics are JVM-testable. */
internal class SafReadSessions : AutoCloseable {
    private val streams = mutableMapOf<Long, InputStream>()
    private var nextId = 1L
    private var closed = false

    @Synchronized
    fun open(stream: InputStream): Long {
        if (closed || nextId <= 0) {
            stream.close()
            return 0L
        }
        val id = nextId++
        streams[id] = stream
        return id
    }

    /** null is an error, never EOF. Short reads remain data. */
    @Synchronized
    fun read(id: Long, maxBytes: Int): ByteArray? {
        val stream = streams[id] ?: return null
        if (maxBytes !in 1..SAF_CHUNK_BYTES) return null
        return try {
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

    @Synchronized
    fun close(id: Long) {
        val stream = streams.remove(id) ?: return
        try { stream.close() } catch (_: Exception) { /* Already failed; ownership released. */ }
    }

    @Synchronized
    override fun close() {
        closed = true
        streams.keys.toList().forEach(::close)
    }
}