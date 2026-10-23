package io.github.chalharu.nerust

import androidx.documentfile.provider.DocumentFile
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Test

/** Minimal stand-in: the resolution rule under test only moves map entries
 * around and never touches the provider, so every method stays unimplemented. */
private class FakeDoc(private val fakeName: String) : DocumentFile(null) {
    override fun createFile(mimeType: String, displayName: String): DocumentFile? =
        throw UnsupportedOperationException()
    override fun createDirectory(displayName: String): DocumentFile? =
        throw UnsupportedOperationException()
    override fun getName(): String = fakeName
    override fun getType(): String? = throw UnsupportedOperationException()
    override fun isDirectory(): Boolean = throw UnsupportedOperationException()
    override fun isFile(): Boolean = throw UnsupportedOperationException()
    override fun isVirtual(): Boolean = throw UnsupportedOperationException()
    override fun lastModified(): Long = throw UnsupportedOperationException()
    override fun length(): Long = throw UnsupportedOperationException()
    override fun listFiles(): Array<DocumentFile> = throw UnsupportedOperationException()
    override fun getParentFile(): DocumentFile? = throw UnsupportedOperationException()
    override fun renameTo(displayName: String): Boolean = throw UnsupportedOperationException()
    override fun getUri(): android.net.Uri = throw UnsupportedOperationException()
    override fun canRead(): Boolean = throw UnsupportedOperationException()
    override fun canWrite(): Boolean = throw UnsupportedOperationException()
    override fun delete(): Boolean = throw UnsupportedOperationException()
    override fun exists(): Boolean = throw UnsupportedOperationException()
}

class SafBatchSidecarTest {
    private val committed = FakeDoc("committed-sidecar")
    private val legacy = FakeDoc("legacy-sidecar")

    @Test fun committedGenerationWinsOverPhysicalName() {
        // NER-310: a generation-tracked sidecar has no stable physical name,
        // so the batch must resolve it through the committed map. The raw
        // listing alone would always miss it and force a full rescan.
        assertSame(
            committed,
            SafRepository.batchSidecarFile(
                "7",
                { name -> if (name == "7.summary") committed else null },
                mapOf("7.summary" to legacy),
            ),
        )
    }

    @Test fun tombstonedSidecarResolvesToNothing() {
        // A tombstone (committed without data) is a deliberate deletion, not
        // a miss: the scan falls back to the archive instead of adopting
        // the stale physical file.
        assertNull(
            SafRepository.batchSidecarFile(
                "7",
                { _ -> null },
                emptyMap(),
            ),
        )
    }

    @Test fun legacyPhysicalSidecarStillServes() {
        assertSame(
            legacy,
            SafRepository.batchSidecarFile("7", { _ -> null }, mapOf("7.summary" to legacy)),
        )
    }

    @Test fun absentSidecarResolvesToNothing() {
        assertNull(SafRepository.batchSidecarFile("7", { _ -> null }, emptyMap()))
    }
}
