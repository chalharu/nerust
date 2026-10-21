package io.github.chalharu.nerust

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SafMarkerSweepTest {
    @Test fun olderSameNameMarkersSweep() {
        assertTrue(isSupersededMarker("7.state", "0000000000000000001-x", "7.state", "0000000000000000002-y"))
    }

    @Test fun newerOrSameOrderMarkersStay() {
        assertFalse(isSupersededMarker("7.state", "0000000000000000002-y", "7.state", "0000000000000000002-y"))
        assertFalse(isSupersededMarker("7.state", "0000000000000000003-z", "7.state", "0000000000000000002-y"))
    }

    @Test fun otherNamesStay() {
        assertFalse(isSupersededMarker("8.state", "0000000000000000001-x", "7.state", "0000000000000000002-y"))
    }
}
