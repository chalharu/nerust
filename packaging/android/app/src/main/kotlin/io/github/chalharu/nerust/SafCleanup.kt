package io.github.chalharu.nerust

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
 * saturated lane spills to a bounded number of overflow threads so submitters
 * (including Activity.onDestroy) never run provider I/O and ownership is
 * never dropped. No Android dependencies: JVM-testable.
 */
internal object SafCleanup {
    const val MAX_PENDING = 64
    private const val MAX_OVERFLOW_THREADS = 4
    private val overflowThreads = AtomicInteger(0)

    val executor: Executor = ThreadPoolExecutor(
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
                    }
                }, "nerust-saf-cleanup-overflow").apply { isDaemon = true }.start()
            } else {
                // Deeply wedged: preserve ownership by running inline.
                overflowThreads.decrementAndGet()
                task.run()
            }
        },
    )
}
