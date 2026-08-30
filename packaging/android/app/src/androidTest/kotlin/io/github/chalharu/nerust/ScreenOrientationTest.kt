package io.github.chalharu.nerust

import android.content.pm.ActivityInfo
import android.hardware.SensorManager
import android.view.Surface
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ScreenOrientationTest {
    @Test
    fun settingsMapToAndroidOrientationRequests() {
        assertEquals(ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED, screenOrientationRequest(0))
        assertEquals(ActivityInfo.SCREEN_ORIENTATION_PORTRAIT, screenOrientationRequest(1))
        assertEquals(ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE, screenOrientationRequest(2))
        assertNull(screenOrientationRequest(-1))
        assertNull(screenOrientationRequest(3))
    }

    @Test
    fun accelerometerAxesStayRelativeToTheScreen() {
        val gravity = SensorManager.GRAVITY_EARTH
        val leftDownSamples =
            listOf(
                screenRelativeAcceleration(gravity, 0f, Surface.ROTATION_0),
                screenRelativeAcceleration(0f, -gravity, Surface.ROTATION_90),
                screenRelativeAcceleration(-gravity, 0f, Surface.ROTATION_180),
                screenRelativeAcceleration(0f, gravity, Surface.ROTATION_270),
            )
        val topDownSamples =
            listOf(
                screenRelativeAcceleration(0f, -gravity, Surface.ROTATION_0),
                screenRelativeAcceleration(-gravity, 0f, Surface.ROTATION_90),
                screenRelativeAcceleration(0f, gravity, Surface.ROTATION_180),
                screenRelativeAcceleration(gravity, 0f, Surface.ROTATION_270),
            )

        leftDownSamples.forEach { sample ->
            assertEquals(1f, sample[0], 0.0001f)
            assertEquals(0f, sample[1], 0.0001f)
        }
        topDownSamples.forEach { sample ->
            assertEquals(0f, sample[0], 0.0001f)
            assertEquals(1f, sample[1], 0.0001f)
        }
    }
}
