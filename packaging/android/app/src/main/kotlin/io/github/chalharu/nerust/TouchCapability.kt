package io.github.chalharu.nerust

import android.content.pm.PackageManager
import android.os.Build
import android.view.InputDevice
import android.view.KeyEvent
import android.view.MotionEvent

/** Key-event sources that divert to the gamepad path. Shared by gameplay
 * (MainActivity) and capture (SettingsActivity) so the two can never
 * disagree about what counts as a controller. Bare SOURCE_DPAD is not a
 * controller signal (TV remotes, keyboard clusters); real gamepad
 * direction input always carries the joystick class. */
internal fun isControllerSource(source: Int): Boolean =
    source and InputDevice.SOURCE_GAMEPAD == InputDevice.SOURCE_GAMEPAD ||
        source and InputDevice.SOURCE_JOYSTICK == InputDevice.SOURCE_JOYSTICK

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
    if (GAMEPAD_BUTTON_CODES.none { device.hasKeys(it)[0] }) return false
    // Game axes a real controller drives; button clusters expose none.
    return GAME_AXES.any { device.getMotionRange(it) != null }
}

private val GAMEPAD_BUTTON_CODES =
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

private val GAME_AXES =
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

/** One enumerated keyboard row: stable port-set id, OS display name,
 * built-in flag. Plain data; JSON stays at the command boundary. */
internal data class KeyboardRow(
    val member: String,
    val name: String,
    val builtin: Boolean,
)

/** One passive scan feeding every device-truth consumer: keyboards,
 * counted gamepads, and touch capability from a single `getDeviceIds()`
 * walk. Hotplug/resume paths share this so settings and runtime can
 * never disagree about what is connected. */
internal data class DeviceTruth(
    val keyboards: List<KeyboardRow>,
    val gamepadNames: List<String>,
    val touchCapable: Boolean,
)

/** One keyboard row for a device, or null when it is not one.
 * The built-in row (device id 0 with letter keys) leads; externals are
 * per descriptor, BT/USB agnostic; virtual devices never reach here.
 * Button clusters (power keys, headset jacks, touch panels, fingerprint
 * sensors) also expose SOURCE_KEYBOARD, so every row must carry letter
 * keys — built-in and external share one rule. */
private fun keyboardRowFor(device: InputDevice): KeyboardRow? {
    if (device.sources and InputDevice.SOURCE_KEYBOARD != InputDevice.SOURCE_KEYBOARD) return null
    if (!device.hasKeys(KeyEvent.KEYCODE_A)[0]) return null
    return if (device.id == 0) {
        KeyboardRow("kbd:builtin", device.name, true)
    } else {
        KeyboardRow("kbd:${device.descriptor}", device.name, false)
    }
}

/** One device's contribution to the shared scan, or null when the
 * device carries nothing (negative id, virtual, or no truth at all).
 * Returns the merged verdict directly: at most one row per device. */
private fun scanOneDevice(device: InputDevice): DeviceTruth? {
    if (device.id < 0) return null
    if (Build.VERSION.SDK_INT >= 29 && device.isVirtual) return null
    val touch = device.supportsSource(InputDevice.SOURCE_TOUCHSCREEN)
    val keyboard = keyboardRowFor(device)
    val gamepad = if (isCountedGamepad(device)) device.name else null
    if (!touch && keyboard == null && gamepad == null) return null
    return DeviceTruth(
        keyboards = listOfNotNull(keyboard),
        gamepadNames = listOfNotNull(gamepad),
        touchCapable = touch,
    )
}

/** Single-pass device scan: touch fallback, keyboard rows, and counted
 * gamepads from one enumeration. Hotplug/resume paths share this so
 * settings and runtime can never disagree about what is connected.
 * Keyboards stay out of the gamepad gate on purpose: shareable auxiliary
 * input must not evict the touch controls. */
internal fun scanDeviceTruth(packageManager: PackageManager): DeviceTruth {
    var touch = packageManager.hasSystemFeature(PackageManager.FEATURE_TOUCHSCREEN)
    var builtin: KeyboardRow? = null
    val keyboards = mutableListOf<KeyboardRow>()
    val seen = mutableSetOf<String>()
    val gamepads = mutableListOf<String>()
    for (id in InputDevice.getDeviceIds()) {
        val device = InputDevice.getDevice(id) ?: continue
        val hit = scanOneDevice(device) ?: continue
        if (hit.touchCapable) touch = true
        hit.keyboards.firstOrNull()?.let { row ->
            if (row.builtin) builtin = row else if (seen.add(row.member)) keyboards += row
        }
        gamepads += hit.gamepadNames
    }
    val rows = listOfNotNull(builtin) + keyboards
    return DeviceTruth(rows, gamepads.distinct(), touch)
}
