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

/** Abnormal-path pins for the SAF bridge that run without a device. */
class SafAbnormalTest {
    @Test fun transientSurveyFailureIsErrorNeverAbsence() {
        // Only a complete survey confirms absence; an incomplete survey
        // surfaces as an open error so prior state is kept.
        assertFalse(SafRepository.confirmedAbsent(surveyComplete = false, fileFound = false))
        assertTrue(SafRepository.confirmedAbsent(surveyComplete = true, fileFound = false))
        // Found files are present regardless of the survey state.
        assertFalse(SafRepository.confirmedAbsent(surveyComplete = true, fileFound = true))
        assertFalse(SafRepository.confirmedAbsent(surveyComplete = false, fileFound = true))
    }

    @Test fun oversizedSidecarRejectedWithoutFullTransfer() {
        // The batch sidecar path caps the transfer; over-cap summaries
        // are dropped before crossing.
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
        // A copy that fails mid-stream never commits; the abort runs instead.
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
        // A short generation throws at EOF instead of serving partial bytes.
        val short = SafVerifiedInputStream(ByteArrayInputStream(byteArrayOf(1, 2)), 10L)
        assertEquals(2, short.read(ByteArray(8), 0, 8))
        assertThrows(IOException::class.java) { short.read() }
    }

    @Test fun oversizedGenerationDetectedInsteadOfServingExtra() {
        val long = SafVerifiedInputStream(ByteArrayInputStream(byteArrayOf(1, 2, 3)), 1L)
        assertThrows(IOException::class.java) { long.read(ByteArray(8), 0, 8) }
    }
}
