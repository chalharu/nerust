package io.github.chalharu.nerust

import android.view.InputDevice
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Pins the controller-source truth table: gamepad/joystick divert to the
 * gamepad path, everything else (keyboard, bare dpad, touch, mouse) keeps
 * platform behavior. Bare SOURCE_DPAD is deliberately not a controller
 * signal: real gamepad direction input carries the joystick class. */
class TouchCapabilityTest {
    @Test
    fun gamepadAndJoystickAreControllerSources() {
        assertTrue(isControllerSource(InputDevice.SOURCE_GAMEPAD))
        assertTrue(isControllerSource(InputDevice.SOURCE_JOYSTICK))
        assertTrue(isControllerSource(InputDevice.SOURCE_GAMEPAD or InputDevice.SOURCE_JOYSTICK))
    }

    @Test
    fun keyboardDpadTouchMouseAreNotControllerSources() {
        assertFalse(isControllerSource(InputDevice.SOURCE_KEYBOARD))
        assertFalse(isControllerSource(InputDevice.SOURCE_DPAD))
        assertFalse(isControllerSource(InputDevice.SOURCE_TOUCHSCREEN))
        assertFalse(isControllerSource(InputDevice.SOURCE_MOUSE))
        assertFalse(isControllerSource(0))
    }
}
