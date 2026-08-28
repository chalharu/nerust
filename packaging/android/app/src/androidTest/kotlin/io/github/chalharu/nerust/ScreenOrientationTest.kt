package io.github.chalharu.nerust

import android.content.pm.ActivityInfo
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
}
