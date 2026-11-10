package io.github.chalharu.nerust

import android.content.pm.PackageManager
import android.os.Build
import android.view.InputDevice

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
