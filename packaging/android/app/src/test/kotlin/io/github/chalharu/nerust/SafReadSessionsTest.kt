package io.github.chalharu.nerust

import java.io.ByteArrayInputStream
import java.io.IOException
import java.io.InputStream
import org.junit.Assert.*
import org.junit.Test

class SafReadSessionsTest {
    @Test fun dataEofAndInvalidSessionAreDistinct() {
        val sessions = SafReadSessions()
        val id = sessions.open(ByteArrayInputStream(byteArrayOf(1, 2, 3)))
        assertTrue(id > 0)
        assertArrayEquals(byteArrayOf(1, 2), sessions.read(id, 2))
        assertArrayEquals(byteArrayOf(3), sessions.read(id, 2))
        assertArrayEquals(byteArrayOf(), sessions.read(id, 2))
        sessions.close(id)
        assertNull(sessions.read(id, 2))
        sessions.close(id) // Idempotent cleanup.
    }

    @Test fun readFailureAfterDataIsNotEofAndClosesStream() {
        var closed = false
        var reads = 0
        val stream = object : InputStream() {
            override fun read(): Int = throw IOException("provider failed")
            override fun read(bytes: ByteArray, offset: Int, length: Int): Int {
                if (reads++ > 0) throw IOException("provider failed mid-stream")
                bytes[offset] = 42
                return 1
            }
            override fun close() { closed = true }
        }
        val sessions = SafReadSessions()
        val id = sessions.open(stream)
        assertArrayEquals(byteArrayOf(42), sessions.read(id, 32))
        assertNull(sessions.read(id, 32))
        assertTrue(closed)
        assertNull(sessions.read(id, 32))
    }

    @Test fun zeroReadDoesNotBecomeFalseEofOrSpin() {
        val sessions = SafReadSessions()
        val id = sessions.open(object : InputStream() {
            override fun read(bytes: ByteArray, offset: Int, length: Int): Int = 0
            override fun read(): Int = 7
        })
        assertArrayEquals(byteArrayOf(7), sessions.read(id, 32))
        sessions.close()
    }

    @Test fun shutdownClosesAllStreamsAndRejectsLateOpen() {
        var closed = 0
        fun stream() = object : ByteArrayInputStream(byteArrayOf()) {
            override fun close() { closed++ }
        }
        val sessions = SafReadSessions()
        val first = sessions.open(stream())
        sessions.open(stream())
        sessions.close()
        assertEquals(2, closed)
        assertNull(sessions.read(first, 1))
        assertEquals(0L, sessions.open(stream()))
        assertEquals(3, closed)
        sessions.close()
        assertEquals(3, closed)
    }

    @Test fun invalidChunkSizeIsAnErrorNotEof() {
        val sessions = SafReadSessions()
        val id = sessions.open(ByteArrayInputStream(byteArrayOf(1)))
        assertNull(sessions.read(id, 0))
        assertNull(sessions.read(id, SAF_CHUNK_BYTES + 1))
        assertArrayEquals(byteArrayOf(1), sessions.read(id, 1))
        sessions.close()
    }
}