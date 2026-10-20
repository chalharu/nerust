package io.github.chalharu.nerust

import org.json.JSONObject

/** Single Kotlin source for the Rust bridge contract.
 * Rust pins the same numbers; check-contract.sh asserts they match. */
internal const val BRIDGE_EVENT_VERSION = 1
internal const val SETTINGS_SCHEMA_VERSION = 1

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
        .put("action", "controller")
        .put("device_id", deviceId)
        .put("key", key)
        .put("pressed", pressed)
        .toString()
