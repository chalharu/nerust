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

    @Test fun fullySaturatedLaneParksWithoutRunningOnSubmitter() {
        val workerBlock = CountDownLatch(1)
        val workerRelease = CountDownLatch(1)
        SafCleanup.executor.execute {
            workerBlock.countDown()
            workerRelease.await(10, TimeUnit.SECONDS)
        }
        assertTrue(workerBlock.await(10, TimeUnit.SECONDS))
        // Fill the queue so the next four tasks spill to overflow threads.
        val queued = (0 until SafCleanup.MAX_PENDING).map {
            CountDownLatch(1).also { done ->
                SafCleanup.executor.execute { done.countDown() }
            }
        }
        // Stall all four overflow threads.
        val overflowBlock = CountDownLatch(4)
        val overflowRelease = CountDownLatch(1)
        repeat(4) {
            SafCleanup.executor.execute {
                overflowBlock.countDown()
                overflowRelease.await(10, TimeUnit.SECONDS)
            }
        }
        assertTrue(overflowBlock.await(10, TimeUnit.SECONDS))
        // Fully saturated: this submit must park, never run on the submitter.
        val parkedThread = AtomicReference<Thread>()
        val parkedDone = CountDownLatch(1)
        SafCleanup.executor.execute {
            parkedThread.set(Thread.currentThread())
            parkedDone.countDown()
        }
        Thread.sleep(500)
        assertNull(parkedThread.get())
        assertEquals(1, SafCleanup.parkedCount())
        // Release everything: the parked task runs exactly once, off-submitter.
        overflowRelease.countDown()
        workerRelease.countDown()
        assertTrue(parkedDone.await(10, TimeUnit.SECONDS))
        assertNotEquals(Thread.currentThread(), parkedThread.get())
        assertEquals(0, SafCleanup.parkedCount())
        queued.forEach { assertTrue(it.await(10, TimeUnit.SECONDS)) }
    }
}
