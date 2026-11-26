package io.github.chalharu.nerust

import org.junit.Assert.assertEquals
import org.junit.Test

/** Pins the cartridge quantize gate: 0.01g steps with the schema range
 * as the clamp. JVM-runnable: pure float math, no framework calls. */
class QuantizeAccelerationTest {
    @Test
    fun quantumStepsRound() {
        assertEquals(0.5f, quantizeAcceleration(0.5f), 0.0f)
        assertEquals(0.57f, quantizeAcceleration(0.567f), 1e-6f)
        assertEquals(-0.57f, quantizeAcceleration(-0.567f), 1e-6f)
        assertEquals(-0.25f, quantizeAcceleration(-0.25f), 0.0f)
        assertEquals(0.0f, quantizeAcceleration(0.004f), 0.0f)
    }

    @Test
    fun overRangeSamplesClampToSchemaRange() {
        assertEquals(8.0f, quantizeAcceleration(9.5f), 0.0f)
        assertEquals(-8.0f, quantizeAcceleration(-9.5f), 0.0f)
        assertEquals(8.0f, quantizeAcceleration(8.0f), 0.0f)
    }
}
