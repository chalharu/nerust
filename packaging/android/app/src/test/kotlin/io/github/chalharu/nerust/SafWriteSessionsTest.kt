package io.github.chalharu.nerust

import java.io.ByteArrayOutputStream
import java.io.IOException
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import org.junit.Assert.*
import org.junit.Test

class SafWriteSessionsTest {
    private class Transaction(private val fail: String? = null) : SafWriteSessions.Transaction {
        val calls = mutableListOf<String>()
        override val stream = object : ByteArrayOutputStream() {
            override fun write(bytes: ByteArray, offset: Int, length: Int) {
                if (fail == "write") throw IOException("write")
                super.write(bytes, offset, length)
            }
            override fun flush() { calls += "flush"; if (fail == "flush") throw IOException("flush") }
            override fun close() { calls += "close"; if (fail == "close") throw IOException("close") }
        }
        override fun commit(): Boolean { calls += "commit"; return fail != "commit" }
        override fun abort() { calls += "abort" }
    }

    @Test fun chunksUseOneStreamAndPublishOnlyAfterClose() {
        val sessions = SafWriteSessions()
        val tx = Transaction()
        val id = sessions.open(tx)
        assertTrue(sessions.write(id, byteArrayOf(1, 2)))
        assertTrue(sessions.write(id, byteArrayOf(3)))
        assertTrue(sessions.finish(id, true))
        assertArrayEquals(byteArrayOf(1, 2, 3), (tx.stream as ByteArrayOutputStream).toByteArray())
        assertEquals(listOf("flush", "close", "commit"), tx.calls)
        assertFalse(sessions.finish(id, true))
    }

    @Test fun everyFailureAbortsWithoutPublishingPartialData() {
        for (failure in listOf("write", "flush", "close", "commit")) {
            val sessions = SafWriteSessions()
            val tx = Transaction(failure)
            val id = sessions.open(tx)
            sessions.write(id, byteArrayOf(1))
            assertFalse(failure, sessions.finish(id, true))
            assertEquals("abort", tx.calls.last())
            if (failure != "commit") assertFalse(tx.calls.contains("commit"))
        }
    }

    @Test fun cancellationAndOversizedChunkNeverCommit() {
        val sessions = SafWriteSessions()
        val tx = Transaction()
        val id = sessions.open(tx)
        assertFalse(sessions.write(id, ByteArray(SAF_CHUNK_BYTES + 1)))
        assertFalse(sessions.finish(id, true))
        assertFalse(tx.calls.contains("commit"))
        val cancelled = Transaction()
        assertFalse(sessions.finish(sessions.open(cancelled), false))
        assertEquals(listOf("close", "abort"), cancelled.calls)
    }

    @Test fun destroyDetachesWithoutWaitingForBlockedProvider() {
        val entered = CountDownLatch(1)
        val release = CountDownLatch(1)
        val aborted = CountDownLatch(1)
        val sessions = SafWriteSessions()
        val tx = object : SafWriteSessions.Transaction {
            override val stream = object : java.io.OutputStream() {
                override fun write(value: Int) { entered.countDown(); check(release.await(5, TimeUnit.SECONDS)) }
            }
            override fun commit(): Boolean = error("must not commit")
            override fun abort() { aborted.countDown() }
        }
        val id = sessions.open(tx)
        val writer = Thread { sessions.write(id, byteArrayOf(1)) }.apply { start() }
        try {
            assertTrue(entered.await(2, TimeUnit.SECONDS))
            val detached = CountDownLatch(1)
            Thread { sessions.close(); detached.countDown() }.start()
            assertTrue(detached.await(2, TimeUnit.SECONDS))
            assertFalse(sessions.finish(id, true))
        } finally { release.countDown(); writer.join(2000) }
        assertTrue(aborted.await(2, TimeUnit.SECONDS))
    }

    @Test fun committedLengthIsVerifiedEvenWhenProviderSizeIsUnknown() {
        val short = SafVerifiedInputStream(java.io.ByteArrayInputStream(byteArrayOf(1)), 2)
        assertEquals(1, short.read())
        assertThrows(IOException::class.java) { short.read() }
        val long = SafVerifiedInputStream(java.io.ByteArrayInputStream(byteArrayOf(1, 2)), 1)
        assertThrows(IOException::class.java) { long.read(ByteArray(2)) }
    }
}