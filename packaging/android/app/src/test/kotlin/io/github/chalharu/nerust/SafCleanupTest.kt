package io.github.chalharu.nerust

import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import org.junit.Assert.*
import org.junit.Test

class SafCleanupTest {
    @Test fun saturatedLaneSpillsWithoutRunningOnSubmitter() {
        val workerBlock = CountDownLatch(1)
        val workerRelease = CountDownLatch(1)
        // Occupy the single worker so the queue fills deterministically.
        SafCleanup.executor.execute {
            workerBlock.countDown()
            workerRelease.await(10, TimeUnit.SECONDS)
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
        queued.forEach { assertTrue(it.await(10, TimeUnit.SECONDS)) }
    }
}
