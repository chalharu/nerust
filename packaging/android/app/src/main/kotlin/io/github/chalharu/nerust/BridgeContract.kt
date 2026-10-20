package io.github.chalharu.nerust

import org.json.JSONObject

/** Single Kotlin source for the Rust bridge contract.
 * Rust pins the same numbers; check-contract.sh asserts they match. */
internal const val BRIDGE_EVENT_VERSION = 1
internal const val SETTINGS_SCHEMA_VERSION = 1

/** Canonical menu actions: every Kotlin sender resolves to these.
 * Rust `ACTION_*` pins the same set; check-contract.sh asserts it. */
internal object MenuAction {
    const val EXIT = "exit"
    const val LOAD_STATE = "load_state"
    const val OPEN_ROM = "open_rom"
    const val OPEN_SETTINGS = "open_settings"
    const val ORIENTATION_CHANGED = "orientation_changed"
    const val RESET = "reset"
    const val SAVE_STATE = "save_state"
    const val TOGGLE_PAUSE = "toggle_pause"
    const val UNLOAD = "unload"
    const val CONTROLLER = "controller"

    val ALL: Set<String> =
        setOf(
            EXIT, LOAD_STATE, OPEN_ROM, OPEN_SETTINGS, ORIENTATION_CHANGED,
            RESET, SAVE_STATE, TOGGLE_PAUSE, UNLOAD, CONTROLLER,
        )
}

/** Canonical controller keys: the decoder table, the schema enum,
 * and these constants list exactly the same keys. */
internal object ControllerKey {
    const val BUTTON1 = "button1"
    const val BUTTON2 = "button2"
    const val BUTTON5 = "button5"
    const val BUTTON6 = "button6"
    const val START = "start"
    const val SELECT = "select"
    const val UP = "up"
    const val DOWN = "down"
    const val LEFT = "left"
    const val RIGHT = "right"

    val ALL: Set<String> =
        setOf(
            BUTTON1, BUTTON2, BUTTON5, BUTTON6, START, SELECT,
            UP, DOWN, LEFT, RIGHT,
        )
}

/** Settings section fallback labels: Rust pushes the label per section.
 * Used only for test-constructed content without a pushed label. */
internal object SettingsSectionFallback {
    const val AUDIO = "Audio"
    const val VIDEO = "Video"
    const val CONTROLS = "Controls"
    const val STORAGE = "Storage"
    const val NOTIFICATIONS = "Notifications"
    const val GENERAL = "General"

    fun labelFor(id: String): String =
        when (id) {
            "audio" -> AUDIO
            "video" -> VIDEO
            "controls" -> CONTROLS
            "storage" -> STORAGE
            "notifications" -> NOTIFICATIONS
            "general" -> GENERAL
            "system.nes" -> "Nintendo Entertainment System"
            "system.gbc" -> "Game Boy Color"
            "system.gba" -> "Game Boy Advance"
            else -> id.substringAfterLast('.').replace('_', ' ').replaceFirstChar(Char::uppercase)
        }
}

/** v1 menu event: {"v":1,"type":"menu","action":...}. */
internal fun menuEventJson(action: String): String =
    JSONObject()
        .put("v", BRIDGE_EVENT_VERSION)
        .put("type", "menu")
        .put("action", action)
        .toString()

/** v1 controller event: typed device, key, and press state. */
internal fun controllerEventJson(deviceId: Int, key: String, pressed: Boolean): String =
    JSONObject()
        .put("v", BRIDGE_EVENT_VERSION)
        .put("type", "menu")
        .put("action", MenuAction.CONTROLLER)
        .put("device_id", deviceId)
        .put("key", key)
        .put("pressed", pressed)
        .toString()
