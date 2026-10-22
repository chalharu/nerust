package io.github.chalharu.nerust

/** One touch zone pushed from Rust, in relative (0.0..1.0) surface coordinates. */
internal data class PushedOverlayZone(
    val x: Float,
    val y: Float,
    val width: Float,
    val height: Float,
    val label: String,
)

/** Joystick home pushed from Rust: center over surface, radii over minor axis. */
internal data class PushedJoystick(
    val x: Float,
    val y: Float,
    val baseRadius: Float,
    val knobRadius: Float,
)

/** Parse the Rust-pushed zone payload (`LABEL:x,y,w,h;...`, relative coordinates).
 * Pure: no Android framework calls, so plain JVM tests cover it.
 * Malformed entries are dropped silently; the e2e parse tests pin the shape. */
internal fun parsePushedZones(payload: String): List<PushedOverlayZone> {
    if (payload.isBlank()) return emptyList()
    val zones = mutableListOf<PushedOverlayZone>()
    payload.split(';').forEach { entry ->
        if (entry.isBlank()) return@forEach
        val parts = entry.split(':')
        if (parts.size != 2 || parts[0].isBlank()) return@forEach
        val coords = parts[1].split(',').mapNotNull { it.toFloatOrNull() }
        if (coords.size != 4) return@forEach
        zones +=
            PushedOverlayZone(
                x = coords[0],
                y = coords[1],
                width = coords[2],
                height = coords[3],
                label = parts[0],
            )
    }
    return zones
}

/** Parse the Rust-pushed joystick payload (`cx,cy,baseR,knobR`); null when malformed. */
internal fun parsePushedJoystick(payload: String): PushedJoystick? {
    val coords = payload.split(',').mapNotNull { it.toFloatOrNull() }
    if (coords.size != 4) return null
    return PushedJoystick(
        x = coords[0],
        y = coords[1],
        baseRadius = coords[2],
        knobRadius = coords[3],
    )
}
