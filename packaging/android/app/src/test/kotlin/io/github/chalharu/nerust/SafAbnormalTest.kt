package io.github.chalharu.nerust

import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.FilterOutputStream
import java.io.IOException
import java.io.InputStream
import java.io.OutputStream
import java.util.concurrent.Executor
import org.junit.Assert.*
import org.junit.Test

/**
 * Abnormal-path pins for the SAF bridge that run without a device.
 *
 * No `Context`/provider exists on the host, so these JVM tests pin the pure
 * decision, bound, and session logic the emulator path shares with the
 * provider-backed code: NER-321 (transient vs confirmed-absent), NER-312
 * (bounded sidecar transfer), NER-319 (a failed copy never commits; the
 * original is untouched), and NER-322 (a truncated generation is detected,
 * so the Rust scan skips that slot while healthy ones list).
 *
 * Full provider integration (real DocumentsProvider, process-death
 * generations) stays in `androidTest` and runs on the emulator CI job;
 * there is no headless provider here.
 */
class SafAbnormalTest {
    @Test fun transientSurveyFailureIsErrorNeverAbsence() {
        // NER-321: a name with no generation and no physical file is
        // confirmed-absent ONLY after a complete successful survey. An
        // incomplete survey (an unreadable commit marker) must surface as
        // an open error so the Rust side keeps its previous state and
        // never runs the delete/tombstone/GC path over live data.
        assertFalse(SafRepository.confirmedAbsent(surveyComplete = false, fileFound = false))
        assertTrue(SafRepository.confirmedAbsent(surveyComplete = true, fileFound = false))
        // Found files are present regardless of the survey state.
        assertFalse(SafRepository.confirmedAbsent(surveyComplete = true, fileFound = true))
        assertFalse(SafRepository.confirmedAbsent(surveyComplete = false, fileFound = true))
    }

    @Test fun oversizedSidecarRejectedWithoutFullTransfer() {
        // NER-312: the batch sidecar path caps the transfer; the caller
        // drops anything over the cap, so a huge summary never crosses.
        val huge = ByteArray(SAF_SUMMARY_BYTES * 4) { 0x41 }
        var bulkReads = 0
        val counting = object : InputStream() {
            private val inner = ByteArrayInputStream(huge)
            override fun read(): Int = inner.read()
            override fun read(bytes: ByteArray, offset: Int, length: Int): Int {
                val count = inner.read(bytes, offset, length)
                if (count > 0) bulkReads++
                return count
            }
        }
        val bounded = SafRepository.readCappedBytes(counting, SAF_SUMMARY_BYTES + 1)
        // The loop stops at the cap (+1 probe byte): the provider stream is
        // never drained, and the caller rejects the over-cap result.
        assertEquals(SAF_SUMMARY_BYTES + 1, bounded.size)
        assertTrue(bulkReads * 8192 < huge.size + 8192)
        assertTrue(bounded.size > SAF_SUMMARY_BYTES)
    }

    @Test fun exactFitSidecarPassesTheCap() {
        val fitting = ByteArray(SAF_SUMMARY_BYTES) { 0x42 }
        val bounded = SafRepository.readCappedBytes(ByteArrayInputStream(fitting), SAF_SUMMARY_BYTES + 1)
        assertArrayEquals(fitting, bounded)
    }

    @Test fun failedCopyNeverCommitsAndAborts() {
        // NER-319 staging half: a copy that fails mid-stream must never
        // commit a partial document; the abort runs and the previous
        // generation (the "original") stays served.
        var committed = false
        var aborted = false
        val staged = ByteArrayOutputStream()
        val transaction = object : SafWriteSessions.Transaction {
            override val stream: OutputStream = object : FilterOutputStream(staged) {
                override fun write(bytes: ByteArray, offset: Int, length: Int) {
                    throw IOException("stuck provider")
                }
            }
            override fun commit(): Boolean {
                committed = true
                return true
            }
            override fun abort() {
                aborted = true
            }
        }
        val sessions = SafWriteSessions(Executor { it.run() })
        val id = sessions.open(transaction)
        assertTrue(id > 0)
        assertFalse(sessions.write(id, byteArrayOf(1, 2, 3)))
        assertFalse(sessions.finish(id, commit = true))
        assertFalse(committed)
        assertTrue(aborted)
    }

    @Test fun truncatedGenerationDetectedInsteadOfServingPartial() {
        // NER-322 detection half: a short generation throws at EOF instead
        // of serving partial bytes, so the Rust scan skips that slot while
        // healthy ones list.
        val short = SafVerifiedInputStream(ByteArrayInputStream(byteArrayOf(1, 2)), 10L)
        assertEquals(2, short.read(ByteArray(8), 0, 8))
        try {
            short.read()
            fail("short generation must throw at EOF")
        } catch (_: IOException) {
        }
    }

    @Test fun oversizedGenerationDetectedInsteadOfServingExtra() {
        val long = SafVerifiedInputStream(ByteArrayInputStream(byteArrayOf(1, 2, 3)), 1L)
        try {
            long.read(ByteArray(8), 0, 8)
            fail("oversized generation must throw")
        } catch (_: IOException) {
        }
    }
}
