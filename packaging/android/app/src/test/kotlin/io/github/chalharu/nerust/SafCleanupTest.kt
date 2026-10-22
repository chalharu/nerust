package io.github.chalharu.nerust

import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import org.junit.Assert.*
import org.junit.Test

class SafCleanupTest {
    @Test fun saturatedLaneSpillsWithoutRunningOnSubmitter() {
        val workerBlock = CountDownLatch(1)
        val workerRelease = CountDownLatch(1)
        val workerReleased = AtomicBoolean(false)
        // Occupy the single worker so the queue fills deterministically.
        SafCleanup.executor.execute {
            workerBlock.countDown()
            workerReleased.set(workerRelease.await(10, TimeUnit.SECONDS))
        }
        assertTrue(workerBlock.await(10, TimeUnit.SECONDS))
        val queued = (0 until SafCleanup.MAX_PENDING).map {
            CountDownLatch(1).also { done ->
                SafCleanup.executor.execute { done.countDown() }
            }
        }
        // The queue is full: this task must spill to an overflow thread,
        // never run on the submitting test thread.
        val spilledThread = AtomicReference<Thread>()
        val spilledDone = CountDownLatch(1)
        SafCleanup.executor.execute {
            spilledThread.set(Thread.currentThread())
            spilledDone.countDown()
        }
        assertTrue(spilledDone.await(10, TimeUnit.SECONDS))
        assertNotEquals(Thread.currentThread(), spilledThread.get())
        assertTrue(spilledThread.get().name.contains("overflow"))
        workerRelease.countDown()
        assertTrue(workerRelease.await(10, TimeUnit.SECONDS))
        // The worker must observe the release rather than time out: poll briefly.
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(10)
        while (!workerReleased.get() && System.nanoTime() < deadline) Thread.sleep(10)
        assertTrue(workerReleased.get())
        queued.forEach { assertTrue(it.await(10, TimeUnit.SECONDS)) }
    }
}
