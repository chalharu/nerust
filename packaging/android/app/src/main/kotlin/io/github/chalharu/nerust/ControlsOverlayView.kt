package io.github.chalharu.nerust

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.Typeface
import android.view.MotionEvent
import android.view.View
import kotlin.math.max
import kotlin.math.min

internal class ControlsOverlayView(
    context: Context,
    opacityPercent: Int,
    private var pushedZones: List<PushedOverlayZone> = emptyList(),
    private var pushedJoystick: PushedJoystick? = null,
) : View(context) {
    init {
        // Touches belong to winit via NativeActivity; this view never consumes them.
        isClickable = false
        isFocusable = false
    }

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

    fun setPushedJoystick(joystick: PushedJoystick?) {
        pushedJoystick = joystick
        invalidate()
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        val viewWidth = width.toFloat()
        val viewHeight = height.toFloat()
        if (viewWidth <= 0f || viewHeight <= 0f) {
            return
        }
        // Rust owns all geometry; absent pushes draw nothing, never a local copy.
        val joystick = pushedJoystick
        if (joystick != null) {
            val base = min(viewWidth, viewHeight)
            drawFloatingJoystick(
                canvas,
                centerX = if (floatingDpadActive) floatingDpadCenterX else joystick.x * viewWidth,
                centerY = if (floatingDpadActive) floatingDpadCenterY else joystick.y * viewHeight,
                knobX = if (floatingDpadActive) floatingDpadKnobX else joystick.x * viewWidth,
                knobY = if (floatingDpadActive) floatingDpadKnobY else joystick.y * viewHeight,
                baseRadius = joystick.baseRadius * base,
                knobRadius = joystick.knobRadius * base,
            )
        }
        pushedZones.forEach { zone ->
            drawZone(
                canvas,
                zone.x * viewWidth,
                zone.y * viewHeight,
                zone.width * viewWidth,
                zone.height * viewHeight,
                zone.label,
            )
        }
    }

    private fun drawFloatingJoystick(
        canvas: Canvas,
        centerX: Float,
        centerY: Float,
        knobX: Float,
        knobY: Float,
        baseRadius: Float,
        knobRadius: Float,
    ) {
        canvas.drawCircle(centerX, centerY, baseRadius, fillPaint)
        canvas.drawCircle(centerX, centerY, baseRadius, strokePaint)
        canvas.drawCircle(knobX, knobY, knobRadius, knobPaint)
        canvas.drawCircle(knobX, knobY, knobRadius, strokePaint)
    }

    @SuppressLint("ClickableViewAccessibility")
    override fun onTouchEvent(event: MotionEvent): Boolean = false

    // Reused scratch: drawZone runs per zone per frame, never allocates.
    private val scratchRect = RectF()

    private fun drawZone(
        canvas: Canvas,
        x: Float,
        y: Float,
        width: Float,
        height: Float,
        label: String,
    ) {
        val rect = scratchRect.apply { set(x, y, x + width, y + height) }
        val radius = min(width, height) * 0.20f
        canvas.drawRoundRect(rect, radius, radius, fillPaint)
        canvas.drawRoundRect(rect, radius, radius, strokePaint)

        textPaint.textSize = max(12f, min(height * 0.42f, width * 0.28f))
        val centerY = rect.centerY() - (textPaint.descent() + textPaint.ascent()) / 2f
        canvas.drawText(label, rect.centerX(), centerY, textPaint)
    }
}
