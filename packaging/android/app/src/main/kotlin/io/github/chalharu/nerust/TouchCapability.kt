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
 * One function so the two can never disagree about what "connected" means:
 * `SOURCE_GAMEPAD` as-is; `SOURCE_JOYSTICK`-only with a gamepad button
 * (button clusters without one can't replace touch). Empty means Auto
 * currently hides nothing. */
internal fun connectedGamepadNames(): List<String> {
    val names = mutableListOf<String>()
    for (id in InputDevice.getDeviceIds()) {
        val device = InputDevice.getDevice(id) ?: continue
        if (device.id < 0) continue
        if (Build.VERSION.SDK_INT >= 29 && device.isVirtual) continue
        val sources = device.sources
        val gamepad = sources and InputDevice.SOURCE_GAMEPAD == InputDevice.SOURCE_GAMEPAD
        val joystickWithButtons =
            sources and InputDevice.SOURCE_JOYSTICK == InputDevice.SOURCE_JOYSTICK &&
                device.hasKeys(KeyEvent.KEYCODE_BUTTON_A)[0]
        if (gamepad || joystickWithButtons) names += device.name
    }
    return names.distinct()
}
