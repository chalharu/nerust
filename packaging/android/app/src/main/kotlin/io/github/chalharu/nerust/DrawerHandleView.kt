package io.github.chalharu.nerust

import android.annotation.SuppressLint
import android.content.Context
import android.view.MotionEvent
import android.view.View
import kotlin.math.abs

/**
 * Drawer edge handle extracted from MainActivity. No lifecycle: the owning
 * ChromeManager attaches/detaches it with the drawer chrome popup.
 */
@SuppressLint("ViewConstructor")
internal class DrawerEdgeSwipeHandleView(
    context: Context,
    private val onDrawerOpen: () -> Unit,
) : View(context) {
    private val swipeThresholdPx = context.resources.displayMetrics.density * 24f
    private val verticalTolerancePx = context.resources.displayMetrics.density * 32f
    private var downX = 0f
    private var downY = 0f
    private var trackingSwipe = false

    override fun onTouchEvent(event: MotionEvent): Boolean =
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                downX = event.x
                downY = event.y
                trackingSwipe = true
                true
            }

            MotionEvent.ACTION_MOVE -> {
                if (!trackingSwipe) {
                    false
                } else {
                    val deltaX = event.x - downX
                    val deltaY = abs(event.y - downY)
                    if (deltaY > verticalTolerancePx) {
                        trackingSwipe = false
                    } else if (deltaX >= swipeThresholdPx) {
                        trackingSwipe = false
                        performClick()
                        onDrawerOpen()
                    }
                    true
                }
            }

            MotionEvent.ACTION_UP,
            MotionEvent.ACTION_CANCEL,
            -> {
                trackingSwipe = false
                true
            }

            else -> {
                super.onTouchEvent(event)
            }
        }

    override fun performClick(): Boolean = super.performClick()
}
