package io.github.chalharu.nerust

import org.json.JSONObject

// Versions, MenuAction and ControllerKey are generated in BridgeWire.kt.

/** v1 menu event: {"v":1,"type":"menu","action":...,"pressed":...}.
 * `pressed` rides every event (ignored except by controller/touch_gate)
 * so the gate needs no third builder: the checker pins exactly two. */
internal fun menuEventJson(
    action: String,
    respond: Boolean,
): String =
    JSONObject()
        .put("v", BRIDGE_EVENT_VERSION)
        .put("type", "menu")
        .put("action", action)
        .put("pressed", respond)
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
