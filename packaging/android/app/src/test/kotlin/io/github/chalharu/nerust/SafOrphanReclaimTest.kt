package io.github.chalharu.nerust

import org.junit.Assert.assertEquals
import org.junit.Test

class SafOrphanReclaimTest {
    private val now = 10_000_000L
    private val old = now - 3_600_001L
    private val fresh = now - 1_000L

    @Test fun unreferencedOldStagesReclaimInStableOrder() {
        // Extension filtering is the caller's job: this predicate sees only
        // staged-data candidates plus already-known references.
        val files = mapOf("b.data" to old, "a.data" to old)
        assertEquals(
            listOf("a.data", "b.data"),
            SafRepository.selectOrphanedStages(files, emptySet(), now),
        )
    }

    @Test fun referencedFreshAndUnknownMtimesStay() {
        val files = mapOf(
            "used.data" to old,
            "fresh.data" to fresh,
            "unknown.data" to 0L,
        )
        assertEquals(
            emptyList<String>(),
            SafRepository.selectOrphanedStages(files, setOf("used.data"), now),
        )
    }

    @Test fun reclaimIsCappedPerListing() {
        val files = (0 until 10).associate { "s$it.data" to old }
        assertEquals(4, SafRepository.selectOrphanedStages(files, emptySet(), now).size)
    }

    @Test fun incompleteSurveyReclaimsNothing() {
        // A single unreadable marker makes the reference set suspect:
        // live data must survive until the next complete survey (NER-311).
        val files = mapOf("a.data" to old)
        assertEquals(
            emptyList<String>(),
            SafRepository.selectOrphanedStages(files, emptySet(), now, false),
        )
        // Same input with a complete survey still reclaims.
        assertEquals(
            listOf("a.data"),
            SafRepository.selectOrphanedStages(files, emptySet(), now, true),
        )
    }
}
