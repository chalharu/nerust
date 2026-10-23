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
 * fully saturated lane parks in a bounded dock that every completion and
 * every park drains. Submitters (including Activity.onDestroy) never run
 * provider I/O and ownership is never dropped. No Android dependencies:
 * JVM-testable.
 *
 * Bounds (NER-308): lane queue [MAX_PENDING], overflow threads
 * [MAX_OVERFLOW_THREADS], dock [MAX_PARKED]. A dock overflow cannot drop
 * (FD leak) or stall the submitter (ANR risk), so the excess task runs on
 * a short-lived daemon thread as a documented last resort; the submitter
 * still never runs provider I/O, and [parkedCount] exposes the pressure.
 */
internal object SafCleanup {
    const val MAX_PENDING = 64
    private const val MAX_OVERFLOW_THREADS = 4
    private const val MAX_PARKED = MAX_PENDING
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

    internal fun park(task: Runnable) {
        synchronized(parkedGuard) { parked.addLast(task) }
        if (synchronized(parkedGuard) { parked.size } > MAX_PARKED) {
            // Bounded dock overflow: drop the just-added entry back out and
            // run it on a short-lived daemon thread. A concurrent pump may
            // have taken it first; then there is nothing to withdraw and the
            // lane copy runs instead, exactly once either way.
            val ours = synchronized(parkedGuard) {
                if (parked.peekLast() === task) {
                    parked.removeLast()
                    true
                } else {
                    false
                }
            }
            if (ours) {
                Thread({
                    task.run()
                    pumpParked()
                }, "nerust-saf-cleanup-spill").apply { isDaemon = true }.start()
            }
            return
        }
        // Lost-wakeup guard: completions only chain while work is in
        // flight. A park that lands on an idle lane would otherwise wait for
        // a completion that never comes, so resubmit one entry now (NER-308).
        pumpParked()
    }

    internal fun pumpParked() {
        val pool = executor as ThreadPoolExecutor
        while (true) {
            val next = synchronized(parkedGuard) {
                if (parked.isEmpty()) return
                parked.removeFirst()
            }
            // Offer without the reject handler: a full queue means the worker
            // is busy, so its completion chain will drain the dock. Removing
            // from the dock before offering keeps every entry submitted at
            // most once across concurrent pumps.
            if (!pool.queue.offer(next)) {
                synchronized(parkedGuard) { parked.addFirst(next) }
                return
            }
            pool.prestartCoreThread()
        }
    }

    /** Cleanups parked beyond the lane, visible for tests and metrics. */
    internal fun parkedCount(): Int = synchronized(parkedGuard) { parked.size }
}
