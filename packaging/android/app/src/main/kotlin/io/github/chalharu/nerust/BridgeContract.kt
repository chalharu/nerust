package io.github.chalharu.nerust

import org.json.JSONObject

// Versions, MenuAction and ControllerKey are generated in BridgeWire.kt.

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
