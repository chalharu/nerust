package io.github.chalharu.nerust

import java.util.ArrayDeque
import java.util.concurrent.Executor
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.RejectedExecutionHandler
import java.util.concurrent.ThreadFactory
import java.util.concurrent.ThreadPoolExecutor
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger

/**
 * Single bounded cleanup lane shared by SAF read and write sessions.
 * Queue memory and worker threads stay capped when a provider stalls; a
 * saturated lane spills to a bounded number of overflow threads, and a
 * fully saturated lane parks tasks in a dock that every completion drains.
 * Submitters (including Activity.onDestroy) never run provider I/O and
 * ownership is never dropped. No Android dependencies: JVM-testable.
 */
internal object SafCleanup {
    const val MAX_PENDING = 64
    private const val MAX_OVERFLOW_THREADS = 4
    private val overflowThreads = AtomicInteger(0)
    private val parkedGuard = Any()
    private val parked = ArrayDeque<Runnable>()

    val executor: Executor = object : ThreadPoolExecutor(
        1,
        1,
        60L,
        TimeUnit.SECONDS,
        LinkedBlockingQueue(MAX_PENDING),
        ThreadFactory { runnable ->
            Thread(runnable, "nerust-saf-cleanup").apply { isDaemon = true }
        },
        RejectedExecutionHandler { task, _ ->
            // Only reachable with a stuck provider: spill instead of growing
            // the queue or blocking the submitter on provider I/O.
            if (overflowThreads.incrementAndGet() <= MAX_OVERFLOW_THREADS) {
                Thread({
                    try {
                        task.run()
                    } finally {
                        overflowThreads.decrementAndGet()
                        pumpParked()
                    }
                }, "nerust-saf-cleanup-overflow").apply { isDaemon = true }.start()
            } else {
                overflowThreads.decrementAndGet()
                // Fully saturated: park instead of running inline. The next
                // completion resubmits the oldest parked task (chain), so the
                // submitter never runs provider I/O and nothing is dropped.
                park(task)
            }
        },
    ) {
        override fun afterExecute(task: Runnable, error: Throwable?) {
            super.afterExecute(task, error)
            pumpParked()
        }
    }

    private fun park(task: Runnable) {
        synchronized(parkedGuard) { parked.addLast(task) }
    }

    private fun pumpParked() {
        val next = synchronized(parkedGuard) {
            if (parked.isEmpty()) null else parked.removeFirst()
        } ?: return
        // Still saturated: the handler parks it again. No loss, no inline run.
        executor.execute(next)
    }

    /** Cleanups parked beyond the lane, visible for tests and metrics. */
    internal fun parkedCount(): Int = synchronized(parkedGuard) { parked.size }
}
