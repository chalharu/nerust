package io.github.chalharu.nerust

import androidx.documentfile.provider.DocumentFile
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Test

/** Real file-backed documents: resolution runs against genuine names. */
private fun fileDoc(dir: java.io.File, name: String): DocumentFile {
    java.io.File(dir, name).createNewFile()
    return DocumentFile.fromFile(java.io.File(dir, name))
}

class SafBatchSidecarTest {
    private val dir =
        java.nio.file.Files.createTempDirectory("batch-sidecar").toFile().also { it.deleteOnExit() }
    private val committed = fileDoc(dir, "committed-sidecar")
    private val legacy = fileDoc(dir, "legacy-sidecar")

    @Test fun committedGenerationWinsOverPhysicalName() {
        // The batch resolves generation-tracked sidecars through the
        // committed map, not the raw listing.
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
        // A tombstone is a deliberate deletion: the scan falls back
        // to the archive instead of adopting the stale file.
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
