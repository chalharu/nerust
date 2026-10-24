package io.github.chalharu.nerust

import java.io.IOException
import java.io.InputStream
import java.io.OutputStream
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executor
import java.util.concurrent.TimeUnit
import org.junit.Assert.*
import org.junit.Test

/** Stuck-provider injection: a transport that blocks until released. */
private class StalledInputStream : InputStream() {
    val entered = CountDownLatch(1)
    private val release = CountDownLatch(1)
    var closes = 0
        private set
    override fun read(): Int {
        entered.countDown()
        release.await()
        return -1
    }
    override fun read(bytes: ByteArray, offset: Int, length: Int): Int {
        entered.countDown()
        release.await()
        return -1
    }
    fun unblock() = release.countDown()
    @Synchronized override fun close() { closes++ }
}

private class StalledOutputStream : OutputStream() {
    val entered = CountDownLatch(1)
    private val release = CountDownLatch(1)
    var closes = 0
        private set
    override fun write(value: Int) {
        entered.countDown()
        release.await()
    }
    fun unblock() = release.countDown()
    @Synchronized override fun close() { closes++ }
}

private fun background(): Executor =
    Executor { Thread(it, "stall-test-lane").apply { isDaemon = true }.start() }

private fun await(latch: CountDownLatch, what: String) {
    assertTrue("$what finished in time", latch.await(10, TimeUnit.SECONDS))
}

class SafStallInjectionTest {
    /** Teardown during a stalled read returns promptly; the lane reclaims later. */
    @Test fun teardownDuringStalledReadReturnsPromptly() {
        val sessions = SafReadSessions(background())
        val stream = StalledInputStream()
        val id = sessions.open(stream)
        assertTrue(id > 0)
        val readerDone = CountDownLatch(1)
        var readerOutcome: ByteArray? = byteArrayOf(9)
        var readerError: Throwable? = null
        Thread {
            try {
                readerOutcome = sessions.read(id, 16)
            } catch (error: Throwable) {
                readerError = error
            } finally {
                readerDone.countDown()
            }
        }.apply { isDaemon = true; start() }
        await(stream.entered, "stalled read starts")
        val tornDown = CountDownLatch(1)
        Thread { sessions.close(); tornDown.countDown() }.apply { isDaemon = true; start() }
        await(tornDown, "teardown during a stalled read")
        stream.unblock()
        await(readerDone, "parked read settles")
        assertNull(readerError)
        // The read had already passed admission, so the late provider
        // answer is delivered as EOF; the session itself is gone.
        assertNotNull(readerOutcome)
        assertEquals(0, readerOutcome!!.size)
        assertNull("detached session serves nothing after teardown", sessions.read(id, 1))
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(10)
        while (stream.closes == 0 && System.nanoTime() < deadline) Thread.sleep(10)
        assertEquals("parked lane closes the stream once", 1, stream.closes)
        sessions.close()
    }

    /** Teardown never waits for a stalled write; the lane discards later. */
    @Test fun teardownDuringStalledWriteReturnsPromptly() {
        val sessions = SafWriteSessions(background())
        val stream = StalledOutputStream()
        var aborted = false
        val id = sessions.open(object : SafWriteSessions.Transaction {
            override val stream: OutputStream = stream
            override fun commit(): Boolean = throw AssertionError("stalled write must not commit")
            override fun abort() { aborted = true }
        })
        assertTrue(id > 0)
        val writerDone = CountDownLatch(1)
        var writerOutcome = true
        var writerError: Throwable? = null
        Thread {
            try {
                writerOutcome = sessions.write(id, byteArrayOf(1, 2))
            } catch (error: Throwable) {
                writerError = error
            } finally {
                writerDone.countDown()
            }
        }.apply { isDaemon = true; start() }
        await(stream.entered, "stalled write starts")
        val tornDown = CountDownLatch(1)
        Thread { sessions.close(); tornDown.countDown() }.apply { isDaemon = true; start() }
        await(tornDown, "teardown during a stalled write")
        stream.unblock()
        await(writerDone, "parked write settles")
        assertNull(writerError)
        // The write had already passed admission, so the provider keeps
        // the bytes; detachment only guarantees no commit follows.
        assertTrue(writerOutcome)
        assertFalse("detached session takes no more writes", sessions.write(id, byteArrayOf(3)))
        assertFalse("detached session never commits", sessions.finish(id, true))
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(10)
        while (!aborted && System.nanoTime() < deadline) Thread.sleep(10)
        assertTrue("parked lane aborts the detached write", aborted)
        assertEquals("parked lane closes the stream once", 1, stream.closes)
        sessions.close()
    }

    /** A throwing write marks the session failed, so finish aborts. */
    @Test fun failedWriteAbortsInsteadOfCommitting() {
        val sessions = SafWriteSessions(Executor { it.run() })
        var committed = false
        var aborted = false
        val id = sessions.open(object : SafWriteSessions.Transaction {
            override val stream: OutputStream = object : OutputStream() {
                override fun write(value: Int) = throw IOException("provider failed")
            }
            override fun commit(): Boolean {
                committed = true
                return true
            }
            override fun abort() { aborted = true }
        })
        assertTrue(id > 0)
        assertFalse(sessions.write(id, byteArrayOf(1, 2)))
        assertFalse("failed session must not commit", sessions.finish(id, true))
        assertFalse(committed)
        assertTrue("failed session is aborted", aborted)
        sessions.close()
    }
}
