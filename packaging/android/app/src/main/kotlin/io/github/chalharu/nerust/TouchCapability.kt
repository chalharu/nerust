package io.github.chalharu.nerust

import android.content.pm.PackageManager
import android.os.Build
import android.view.InputDevice
import android.view.KeyEvent
import android.view.MotionEvent

/** Touchscreen truth shared by the runtime overlay gate (MainActivity) and
 * the settings capability push (SettingsActivity): the feature flag, or a
 * touchscreen input device when the flag is unset on prototype builds.
 * One function so settings and runtime can never disagree. */
internal fun hasTouchscreen(packageManager: PackageManager): Boolean {
    if (packageManager.hasSystemFeature(PackageManager.FEATURE_TOUCHSCREEN)) return true
    for (id in InputDevice.getDeviceIds()) {
        val device = InputDevice.getDevice(id) ?: continue
        if (device.id < 0) continue
        if (Build.VERSION.SDK_INT >= 29 && device.isVirtual) continue
        if (device.supportsSource(InputDevice.SOURCE_TOUCHSCREEN)) return true
    }
    return false
}

/** Names of devices the Auto overlay gate counts, shared by the runtime
 * gate (MainActivity) and the settings status push (SettingsActivity).
 * One function so the two can never disagree about what "connected" means.
 * Sources alone prove nothing: button clusters (power keys, headset jacks,
 * fingerprint sensors, touch panels) can expose gamepad or joystick bits.
 * See [isCountedGamepad] for the per-device verdict; every layer errs
 * toward showing the overlay, never stranding the user with no input. */
internal fun connectedGamepadNames(): List<String> {
    val names = mutableListOf<String>()
    for (id in InputDevice.getDeviceIds()) {
        val device = InputDevice.getDevice(id) ?: continue
        if (isCountedGamepad(device)) names += device.name
    }
    return names.distinct()
}

/** One-device verdict for the Auto gate. A counted device must expose the
 * joystick class with a gamepad source or a non-alphabetic keyboard
 * (proven uinput-fpc filter, libgdx #5007), hold a controller number
 * (zero means the platform itself does not consider it a game controller),
 * carry a gamepad button, and drive a game axis: without axes it is not
 * usable alternative input, and hiding touch controls for one strands the
 * user with no way back.
 * The fingerprint sensor still passes every signal above on some terminals:
 * it pretends to be SOURCE_GAMEPAD and SOURCE_JOYSTICK (sources mask
 * 16778513 on POCO X4 Pro, Godot #47656) so the OS itself lies and no
 * signal filter can catch it. The only working guard is the name prefix
 * the engine settled on: every such ghost starts with "uinput-"
 * (fpc/goodix/synaptics/elan/vfs/atrus/fortsense). Real BT/USB controllers
 * never carry it. */
private fun isCountedGamepad(device: InputDevice): Boolean {
    // OS-bug guard first (Godot #47656): ghost fingerprint sensors lie
    // about sources, buttons, and axes, so signals cannot catch them.
    if (device.name.startsWith("uinput-")) return false
    if (device.id < 0) return false
    if (Build.VERSION.SDK_INT >= 29 && device.isVirtual) return false
    // Canonical "is a game controller" verdict: fingerprint sensors
    // and other button clusters never hold a controller number.
    if (device.controllerNumber == 0) return false
    val sources = device.sources
    val joystickClass =
        sources and InputDevice.SOURCE_CLASS_JOYSTICK == InputDevice.SOURCE_CLASS_JOYSTICK
    val gamepadSource =
        sources and InputDevice.SOURCE_GAMEPAD == InputDevice.SOURCE_GAMEPAD
    val nonAlphaKeyboard =
        device.keyboardType != InputDevice.KEYBOARD_TYPE_ALPHABETIC
    if (!joystickClass || !(gamepadSource || nonAlphaKeyboard)) return false
    // Canonical gamepad buttons; any one proves a real controller.
    // All exist since API 1, so no version guard is needed.
    val buttonCodes =
        intArrayOf(
            KeyEvent.KEYCODE_BUTTON_A,
            KeyEvent.KEYCODE_BUTTON_B,
            KeyEvent.KEYCODE_BUTTON_X,
            KeyEvent.KEYCODE_BUTTON_Y,
            KeyEvent.KEYCODE_BUTTON_L1,
            KeyEvent.KEYCODE_BUTTON_R1,
            KeyEvent.KEYCODE_BUTTON_L2,
            KeyEvent.KEYCODE_BUTTON_R2,
            KeyEvent.KEYCODE_BUTTON_START,
            KeyEvent.KEYCODE_BUTTON_SELECT,
        )
    if (!device.hasKeys(*buttonCodes).any { it }) return false
    // Game axes a real controller drives; button clusters expose none.
    val gameAxes =
        intArrayOf(
            MotionEvent.AXIS_X,
            MotionEvent.AXIS_Y,
            MotionEvent.AXIS_Z,
            MotionEvent.AXIS_RX,
            MotionEvent.AXIS_RY,
            MotionEvent.AXIS_RZ,
            MotionEvent.AXIS_HAT_X,
            MotionEvent.AXIS_HAT_Y,
            MotionEvent.AXIS_LTRIGGER,
            MotionEvent.AXIS_RTRIGGER,
        )
    return gameAxes.any { device.getMotionRange(it) != null }
}
