package io.github.chalharu.nerust

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.Typeface
import android.util.Log
import android.view.MotionEvent
import android.view.View
import kotlin.math.max
import kotlin.math.min

/**
 * Touch-overlay chrome extracted from MainActivity. Geometry (hit zones)
 * is owned by Rust (`touch_layout.rs` TOUCH_ZONES, canonical pin
 * `gui/frontends/android/touch_geometry.json`): the pushed zones travel
 * via `configureControlsOverlay(..., zones)` and this view only falls back
 * to its local `controlsLayout()` copy when the payload is absent or
 * malformed. Keep the fractions in sync.
 */
internal data class OverlayZoneSpec(
    val x: Float,
    val y: Float,
    val width: Float,
    val height: Float,
    val label: String,
)

internal data class FloatingJoystickSpec(
    val centerX: Float,
    val centerY: Float,
    val baseRadius: Float,
    val knobRadius: Float,
    val activationBounds: RectF,
)

@SuppressLint("ViewConstructor")
/**
 * One touch zone pushed from Rust, in relative (0.0..1.0) surface
 * coordinates. Geometry is owned by Rust (`touch_layout.rs` TOUCH_ZONES);
 * Kotlin only draws the pushed payload.
 */
internal data class PushedOverlayZone(
    val x: Float,
    val y: Float,
    val width: Float,
    val height: Float,
    val label: String,
)

internal class ControlsOverlayView(
    context: Context,
    opacityPercent: Int,
    private val scalePercent: Int,
    private val verticalOffsetPercent: Int,
    private val shouldersVisible: Boolean,
    private var pushedZones: List<PushedOverlayZone> = emptyList(),
) : View(context) {
    private val opacity = opacityPercent.coerceIn(0, 100) / 100f
    private val fillPaint =
        Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = Color.argb((48 * opacity).toInt(), 255, 255, 255)
            style = Paint.Style.FILL
        }
    private val strokePaint =
        Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = Color.argb((160 * opacity).toInt(), 255, 255, 255)
            strokeWidth = 2f
            style = Paint.Style.STROKE
        }
    private val textPaint =
        Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = Color.argb((220 * opacity).toInt(), 255, 255, 255)
            textAlign = Paint.Align.CENTER
            typeface = Typeface.create(Typeface.DEFAULT, Typeface.BOLD)
        }
    private val knobPaint =
        Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = Color.argb((220 * opacity).toInt(), 255, 255, 255)
            style = Paint.Style.FILL
        }
    private var floatingDpadActive = false
    private var floatingDpadCenterX = 0f
    private var floatingDpadCenterY = 0f
    private var floatingDpadKnobX = 0f
    private var floatingDpadKnobY = 0f

    fun updateFloatingDpad(
        active: Boolean,
        centerX: Float,
        centerY: Float,
        knobX: Float,
        knobY: Float,
    ) {
        floatingDpadActive = active
        floatingDpadCenterX = centerX
        floatingDpadCenterY = centerY
        floatingDpadKnobX = knobX
        floatingDpadKnobY = knobY
        invalidate()
    }

    fun floatingDpadStateForTest(): FloatArray =
        floatArrayOf(
            if (floatingDpadActive) 1f else 0f,
            floatingDpadCenterX,
            floatingDpadCenterY,
            floatingDpadKnobX,
            floatingDpadKnobY,
        )

    fun setPushedZones(zones: List<PushedOverlayZone>) {
        pushedZones = zones
        invalidate()
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        val viewWidth = width.toFloat()
        val viewHeight = height.toFloat()
        if (viewWidth <= 0f || viewHeight <= 0f) {
            return
        }

        val joystick = floatingJoystickLayout(viewWidth, viewHeight, scalePercent, verticalOffsetPercent)
        drawFloatingJoystick(canvas, joystick)
        // Geometry is pushed from Rust via configureControlsOverlay: prefer
        // the pushed zones so both sides render from one source of truth.
        // The local controlsLayout() copy is the fallback when no payload
        // arrived (or it failed to parse); keep the fractions in sync.
        val zones =
            pushedZones.takeIf(List<PushedOverlayZone>::isNotEmpty)?.map { zone ->
                OverlayZoneSpec(
                    x = zone.x * viewWidth,
                    y = zone.y * viewHeight,
                    width = zone.width * viewWidth,
                    height = zone.height * viewHeight,
                    label = zone.label,
                )
            }
                ?: controlsLayout(viewWidth, viewHeight, scalePercent, verticalOffsetPercent, shouldersVisible)
        zones.forEach { zone ->
            drawZone(canvas, zone.x, zone.y, zone.width, zone.height, zone.label)
        }
    }

    private fun drawFloatingJoystick(
        canvas: Canvas,
        spec: FloatingJoystickSpec,
    ) {
        val centerX = if (floatingDpadActive) floatingDpadCenterX else spec.centerX
        val centerY = if (floatingDpadActive) floatingDpadCenterY else spec.centerY
        val knobX = if (floatingDpadActive) floatingDpadKnobX else centerX
        val knobY = if (floatingDpadActive) floatingDpadKnobY else centerY
        canvas.drawCircle(centerX, centerY, spec.baseRadius, fillPaint)
        canvas.drawCircle(centerX, centerY, spec.baseRadius, strokePaint)
        canvas.drawCircle(knobX, knobY, spec.knobRadius, knobPaint)
        canvas.drawCircle(knobX, knobY, spec.knobRadius, strokePaint)
    }

    @SuppressLint("ClickableViewAccessibility")
    override fun onTouchEvent(event: MotionEvent): Boolean = false

    private fun drawZone(
        canvas: Canvas,
        x: Float,
        y: Float,
        width: Float,
        height: Float,
        label: String,
    ) {
        val rect = RectF(x, y, x + width, y + height)
        val radius = min(width, height) * 0.20f
        canvas.drawRoundRect(rect, radius, radius, fillPaint)
        canvas.drawRoundRect(rect, radius, radius, strokePaint)

        textPaint.textSize = max(12f, min(height * 0.42f, width * 0.28f))
        val centerY = rect.centerY() - (textPaint.descent() + textPaint.ascent()) / 2f
        canvas.drawText(label, rect.centerX(), centerY, textPaint)
    }
}

internal fun controlsLayout(
    width: Float,
    height: Float,
    scalePercent: Int = 100,
    verticalOffsetPercent: Int = 0,
    shouldersVisible: Boolean = true,
): List<OverlayZoneSpec> {
    // Local fallback copy: geometry is owned and pushed by Rust
    // (touch_layout.rs TOUCH_ZONES via configureControlsOverlay) and the
    // overlay view prefers the pushed zones. This computation only draws
    // when no payload arrived; keep the fractions in sync.
    val portrait = height >= width
    val base = min(width, height)
    val scale = scalePercent.coerceIn(50, 150) / 100f
    val verticalOffset = height * verticalOffsetPercent.coerceIn(-30, 30) / 100f
    val controlTop = if (portrait) height * 0.54f else 0f
    val controlHeight = height - controlTop
    val joystick = floatingJoystickLayout(width, height, scalePercent, verticalOffsetPercent)
    val actionSize = base * 0.14f * scale
    val actionGap = base * 0.04f
    val actionLeft = if (portrait) width * 0.64f else width - base * 0.08f - actionSize * 2f - actionGap
    val actionTop = joystick.centerY - actionSize * 0.5f
    val centerButtonWidth = base * 0.10f * scale
    val centerButtonHeight = base * 0.068f * scale
    val centerGap = base * 0.03f
    val centerRowWidth = centerButtonWidth * 2f + centerGap
    val centerStartX = (width - centerRowWidth) * 0.5f
    val centerTop =
        (if (portrait) controlTop + controlHeight * 0.16f else height * 0.82f) + verticalOffset
    // Shoulder strip above the D-Pad activation area and the center row.
    // Mirrors ProfileTouchOverlay in android.rs; keep the fractions in sync.
    val shoulderWidth = base * 0.20f * scale
    val shoulderHeight = base * 0.08f * scale
    val shoulderMarginX = width * 0.02f
    val shoulderTop = controlTop + base * 0.015f + verticalOffset

    return buildList {
        add(
            OverlayZoneSpec(
                x = actionLeft,
                y = actionTop,
                width = actionSize,
                height = actionSize,
                label = "B",
            ),
        )
        add(
            OverlayZoneSpec(
                x = actionLeft + actionSize + actionGap,
                y = actionTop,
                width = actionSize,
                height = actionSize,
                label = "A",
            ),
        )
        if (shouldersVisible) {
            add(
                OverlayZoneSpec(
                    x = shoulderMarginX,
                    y = shoulderTop,
                    width = shoulderWidth,
                    height = shoulderHeight,
                    label = "L",
                ),
            )
            add(
                OverlayZoneSpec(
                    x = width - shoulderMarginX - shoulderWidth,
                    y = shoulderTop,
                    width = shoulderWidth,
                    height = shoulderHeight,
                    label = "R",
                ),
            )
        }
        add(
            OverlayZoneSpec(
                x = centerStartX,
                y = centerTop,
                width = centerButtonWidth,
                height = centerButtonHeight,
                label = "SELECT",
            ),
        )
        add(
            OverlayZoneSpec(
                x = centerStartX + centerButtonWidth + centerGap,
                y = centerTop,
                width = centerButtonWidth,
                height = centerButtonHeight,
                label = "START",
            ),
        )
    }
}

internal fun floatingJoystickLayout(
    width: Float,
    height: Float,
    scalePercent: Int = 100,
    verticalOffsetPercent: Int = 0,
): FloatingJoystickSpec {
    val portrait = height >= width
    val base = min(width, height)
    val scale = scalePercent.coerceIn(50, 150) / 100f
    val verticalOffset = height * verticalOffsetPercent.coerceIn(-30, 30) / 100f
    val controlTop = if (portrait) height * 0.54f else 0f
    val controlHeight = height - controlTop
    val dpadSize = base * 0.28f * scale
    val baseRadius = dpadSize * 0.42f
    val centerX = base * 0.08f + dpadSize * 0.5f
    val centerY =
        (if (portrait) controlTop + controlHeight * 0.58f else height * 0.65f) + verticalOffset
    val activationRight = if (portrait) width * 0.52f else width * 0.45f
    val activationTop = if (portrait) controlTop else height * 0.25f
    return FloatingJoystickSpec(
        centerX = centerX,
        centerY = centerY,
        baseRadius = baseRadius,
        knobRadius = baseRadius * 0.48f,
        activationBounds =
            RectF(
                baseRadius,
                activationTop + baseRadius,
                activationRight - baseRadius,
                height - baseRadius,
            ),
    )
}

/**
 * Parse the Rust-pushed zone payload (`LABEL:x,y,w,h;...`, relative
 * coordinates). Malformed entries are dropped with a warning log (never a
 * silent `mapNotNull`): a bad entry means the Rust/Kotlin geometry
 * contract drifted and must be visible in logcat.
 */
internal fun parsePushedZones(payload: String): List<PushedOverlayZone> {
    if (payload.isBlank()) return emptyList()
    val zones = mutableListOf<PushedOverlayZone>()
    payload.split(';').forEachIndexed { index, entry ->
        if (entry.isBlank()) return@forEachIndexed
        val parts = entry.split(':')
        if (parts.size != 2 || parts[0].isBlank()) {
            Log.w(TAG, "parsePushedZones: dropping malformed entry #$index: '$entry'")
            return@forEachIndexed
        }
        val coords = parts[1].split(',').mapNotNull { it.toFloatOrNull() }
        if (coords.size != 4) {
            Log.w(TAG, "parsePushedZones: dropping entry #$index with bad coords: '$entry'")
            return@forEachIndexed
        }
        zones +=
            PushedOverlayZone(
                x = coords[0],
                y = coords[1],
                width = coords[2],
                height = coords[3],
                label = parts[0],
            )
    }
    if (zones.isEmpty()) {
        Log.w(TAG, "parsePushedZones: payload yielded no zones: '$payload'")
    }
    return zones
}

private const val TAG = "Nerust"
