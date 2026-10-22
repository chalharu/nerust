package io.github.chalharu.nerust

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SafMarkerSweepTest {
    @Test fun olderSameNameMarkersSweep() {
        assertTrue(SafRepository.isSupersededMarker("7.state", "0000000000000000001-x", "7.state", "0000000000000000002-y"))
    }

    @Test fun newerOrSameOrderMarkersStay() {
        assertFalse(SafRepository.isSupersededMarker("7.state", "0000000000000000002-y", "7.state", "0000000000000000002-y"))
        assertFalse(SafRepository.isSupersededMarker("7.state", "0000000000000000003-z", "7.state", "0000000000000000002-y"))
    }

    @Test fun otherNamesStay() {
        assertFalse(SafRepository.isSupersededMarker("8.state", "0000000000000000001-x", "7.state", "0000000000000000002-y"))
    }
}
