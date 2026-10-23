package io.github.chalharu.nerust

import androidx.documentfile.provider.DocumentFile
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Test

/** Real file-backed documents: `fromFile` is public and JVM-safe, so the
 * resolution rule under test runs against genuine `getName` values without
 * touching an Android provider. */
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
