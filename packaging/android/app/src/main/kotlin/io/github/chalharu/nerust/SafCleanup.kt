package io.github.chalharu.nerust

import java.util.ArrayDeque
import java.util.concurrent.Executor
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.RejectedExecutionHandler
import java.util.concurrent.ThreadFactory
import java.util.concurrent.ThreadPoolExecutor
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger

/** Single bounded cleanup lane; queue, threads, and dock stay capped. */
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
            // Spill to an overflow thread instead of growing the queue
            // or blocking the submitter on provider I/O.
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
                // Fully saturated: park instead of running inline; the next
                // completion resubmits the oldest parked task.
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
        // Single critical section keeps every observable state quiescent;
        // the guard is reentrant so the pump below simply re-acquires it.
        synchronized(parkedGuard) {
            parked.addLast(task)
            if (parked.size > MAX_PARKED) {
                // Bounded dock overflow: withdraw the entry and run it on a
                // short-lived daemon thread, exactly once either way.
                if (parked.peekLast() === task) {
                    parked.removeLast()
                    Thread({
                        task.run()
                        pumpParked()
                    }, "nerust-saf-cleanup-spill").apply { isDaemon = true }.start()
                }
                return
            }
            // Lost-wakeup guard: a park on an idle lane resubmits one entry
            // now since no later completion will drain it.
            pumpParked()
        }
    }

    internal fun pumpParked() {
        val pool = executor as ThreadPoolExecutor
        synchronized(parkedGuard) {
            while (true) {
                if (parked.isEmpty()) return
                val next = parked.removeFirst()
                // Offer without the reject handler: a full queue means the
                // worker chain will drain the dock later.
                if (!pool.queue.offer(next)) {
                    parked.addFirst(next)
                    return
                }
                pool.prestartCoreThread()
            }
        }
    }

    /** Cleanups parked beyond the lane, visible for tests and metrics. */
    internal fun parkedCount(): Int = synchronized(parkedGuard) { parked.size }
}
