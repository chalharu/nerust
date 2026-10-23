package io.github.chalharu.nerust

import android.content.ContentResolver
import android.content.Context
import android.content.ContextWrapper
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.rule.provider.ProviderTestRule
import java.io.ByteArrayOutputStream
import org.json.JSONArray
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Emulator abnormal-path tests against a real DocumentsContract provider
 * (NER-321/322/312). The fault provider speaks the same interface as any
 * production DocumentsProvider, so no physical device is needed: outage,
 * corrupt, and oversized modes are driven through static volatiles.
 * Runs in CI inside the android-e2e matrix via e2e.sh (FAULT_CLASS).
 */
@RunWith(AndroidJUnit4::class)
class SafAbnormalProviderTest {
    private val treeUri = FaultDocumentsProvider.TREE_URI.toString()

    /**
     * In-process isolated provider: no manifest entry, so the platform
     * never installs it as a system DocumentsProvider (which would demand
     * MANAGE_DOCUMENTS protection) and no cross-APK grant is needed. The
     * repository under test talks to the real DocumentsContract interface
     * through the rule's resolver.
     */
    @get:Rule
    val providerRule: ProviderTestRule = ProviderTestRule.Builder(
        FaultDocumentsProvider::class.java,
        FaultDocumentsProvider.AUTHORITY,
    ).build()

    @Before
    fun clearFaults() {
        FaultDocumentsProvider.outageFailures.set(0)
    }

    @After
    fun resetFaults() {
        FaultDocumentsProvider.outageFailures.set(0)
    }

    private fun repository(): SafRepository {
        val appContext = ApplicationProvider.getApplicationContext<Context>()
        val isolated = object : ContextWrapper(appContext) {
            override fun getContentResolver(): ContentResolver = providerRule.resolver
        }
        return SafRepository(isolated)
    }

    private fun readFully(repository: SafRepository, sessionId: Long): ByteArray {
        val sink = ByteArrayOutputStream()
        while (true) {
            val chunk = repository.readSafReadSession(sessionId, 8192)
                ?: throw AssertionError("read error on session $sessionId")
            if (chunk.isEmpty()) break
            sink.write(chunk)
        }
        return sink.toByteArray()
    }

    @Test(timeout = 60_000)
    fun transientOutageIsScanErrorNeverMissing() {
        repository().use { repository ->
            // Provider-wide outage: every query/open burns a failure.
            FaultDocumentsProvider.outageFailures.set(100)
            assertEquals(
                "transient outage must be an error (0), never missing (-1)",
                0L,
                repository.openSafReadSession(treeUri, "good.state"),
            )
            try {
                repository.listSafFilesWithSizes(treeUri, "")
                fail("transient outage must surface as a scan error, never an empty listing")
            } catch (error: IllegalStateException) {
                assertTrue(error.message.orEmpty().contains("unreadable SAF listing"))
            }
            // Recovery: the same documents serve once the outage clears.
            FaultDocumentsProvider.outageFailures.set(0)
            val session = repository.openSafReadSession(treeUri, "good.state")
            assertTrue("recovered session must be valid, got $session", session > 0)
            try {
                assertArrayEquals(FaultDocumentsProvider.getGoodBytes(), readFully(repository, session))
            } finally {
                repository.closeSafReadSession(session)
            }
        }
    }

    @Test(timeout = 60_000)
    fun corruptDocumentReadsBackExactlyForQuarantine() {
        repository().use { repository ->
            val session = repository.openSafReadSession(treeUri, "corrupt.state")
            assertTrue("corrupt session must open, got $session", session > 0)
            try {
                // Byte fidelity: the Rust side quarantines exactly these bytes
                // instead of deleting them (NER-319/321).
                assertArrayEquals(
                    FaultDocumentsProvider.getCorruptBytes(),
                    readFully(repository, session),
                )
            } finally {
                repository.closeSafReadSession(session)
            }
        }
    }

    @Test(timeout = 60_000)
    fun oversizedSidecarSummaryIsCappedOutOfListing() {
        repository().use { repository ->
            val entries = JSONArray(repository.listSafFilesWithSizes(treeUri, ""))
            var sawSmall = false
            var sawHuge = false
            for (index in 0 until entries.length()) {
                val entry = entries.getJSONObject(index)
                when (entry.getString("name")) {
                    "slot1.state" -> {
                        sawSmall = true
                        assertTrue(
                            "small sidecar must ride the listing",
                            entry.has("summary"),
                        )
                    }
                    "slot2.state" -> {
                        sawHuge = true
                        assertFalse(
                            "200 KiB sidecar must be capped out, not transferred",
                            entry.has("summary"),
                        )
                    }
                }
            }
            assertTrue("slot1.state must be listed", sawSmall)
            assertTrue("slot2.state must be listed", sawHuge)
        }
    }
}
