package io.github.chalharu.nerust

import android.view.KeyEvent
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** Pins the keycode-to-bridge-key table: every mapped button, the Guide
 * shortcut button, and the deliberate gaps (BUTTON_Z falls through to
 * the platform). JVM-runnable: keycodes and bridge keys are constants. */
class ControllerKeyMappingTest {
    @Test
    fun faceButtonsMapInOrder() {
        assertEquals(ControllerKey.BUTTON1, controllerKeyFor(KeyEvent.KEYCODE_BUTTON_A))
        assertEquals(ControllerKey.BUTTON2, controllerKeyFor(KeyEvent.KEYCODE_BUTTON_B))
        assertEquals(ControllerKey.BUTTON3, controllerKeyFor(KeyEvent.KEYCODE_BUTTON_X))
        assertEquals(ControllerKey.BUTTON4, controllerKeyFor(KeyEvent.KEYCODE_BUTTON_Y))
    }

    @Test
    fun shouldersAndTriggersMap() {
        assertEquals(ControllerKey.BUTTON5, controllerKeyFor(KeyEvent.KEYCODE_BUTTON_L1))
        assertEquals(ControllerKey.BUTTON6, controllerKeyFor(KeyEvent.KEYCODE_BUTTON_R1))
        assertEquals(ControllerKey.BUTTON7, controllerKeyFor(KeyEvent.KEYCODE_BUTTON_L2))
        assertEquals(ControllerKey.BUTTON8, controllerKeyFor(KeyEvent.KEYCODE_BUTTON_R2))
        assertEquals(ControllerKey.L3, controllerKeyFor(KeyEvent.KEYCODE_BUTTON_THUMBL))
        assertEquals(ControllerKey.R3, controllerKeyFor(KeyEvent.KEYCODE_BUTTON_THUMBR))
    }

    @Test
    fun systemAndDirectionButtonsMap() {
        assertEquals(ControllerKey.GUIDE, controllerKeyFor(KeyEvent.KEYCODE_BUTTON_MODE))
        assertEquals(ControllerKey.CAPTURE, controllerKeyFor(KeyEvent.KEYCODE_BUTTON_C))
        assertEquals(ControllerKey.START, controllerKeyFor(KeyEvent.KEYCODE_BUTTON_START))
        assertEquals(ControllerKey.SELECT, controllerKeyFor(KeyEvent.KEYCODE_BUTTON_SELECT))
        assertEquals(ControllerKey.UP, controllerKeyFor(KeyEvent.KEYCODE_DPAD_UP))
        assertEquals(ControllerKey.DOWN, controllerKeyFor(KeyEvent.KEYCODE_DPAD_DOWN))
        assertEquals(ControllerKey.LEFT, controllerKeyFor(KeyEvent.KEYCODE_DPAD_LEFT))
        assertEquals(ControllerKey.RIGHT, controllerKeyFor(KeyEvent.KEYCODE_DPAD_RIGHT))
    }

    @Test
    fun unmappedKeysFallThrough() {
        assertNull(controllerKeyFor(KeyEvent.KEYCODE_BUTTON_Z))
        assertNull(controllerKeyFor(KeyEvent.KEYCODE_A))
        assertNull(controllerKeyFor(KeyEvent.KEYCODE_VOLUME_UP))
    }
}
