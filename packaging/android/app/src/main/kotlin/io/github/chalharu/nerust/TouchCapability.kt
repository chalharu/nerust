package io.github.chalharu.nerust

import android.content.pm.PackageManager
import android.os.Build
import android.view.InputDevice
import android.view.KeyEvent

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
 * A counted device must also hold a controller number (zero means the
 * platform itself does not consider it a game controller) and carry a
 * gamepad button: without buttons it is not usable alternative input,
 * and hiding touch controls for one strands the user with no way back. */
internal fun connectedGamepadNames(): List<String> {
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
    val names = mutableListOf<String>()
    for (id in InputDevice.getDeviceIds()) {
        val device = InputDevice.getDevice(id) ?: continue
        if (device.id < 0) continue
        if (Build.VERSION.SDK_INT >= 29 && device.isVirtual) continue
        // Canonical "is a game controller" verdict: fingerprint sensors
        // and other button clusters never hold a controller number.
        if (device.controllerNumber == 0) continue
        val sources = device.sources
        val stick =
            sources and InputDevice.SOURCE_GAMEPAD == InputDevice.SOURCE_GAMEPAD ||
                sources and InputDevice.SOURCE_JOYSTICK == InputDevice.SOURCE_JOYSTICK
        if (!stick) continue
        if (!device.hasKeys(*buttonCodes).any { it }) continue
        names += device.name
    }
    return names.distinct()
}
