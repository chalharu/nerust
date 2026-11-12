package io.github.chalharu.nerust

import org.json.JSONObject

// Versions, MenuAction and ControllerKey are generated in BridgeWire.kt.

/** v1 menu event: {"v":1,"type":"menu","action":...}. The single producer:
 * plain actions pass no extras; gated actions (touch_gate, controller)
 * carry their required fields in `extras`, pinned at the call sites. */
internal fun menuEvent(action: String, extras: JSONObject.() -> Unit = {}): String =
    JSONObject()
        .put("v", BRIDGE_EVENT_VERSION)
        .put("type", "menu")
        .put("action", action)
        .apply(extras)
        .toString()
