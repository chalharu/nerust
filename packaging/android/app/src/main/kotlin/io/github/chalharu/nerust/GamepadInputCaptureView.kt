package io.github.chalharu.nerust

import android.content.Context
import android.view.KeyEvent
import android.view.MotionEvent
import android.widget.FrameLayout

/** Invisible gamepad-input capture window for gameplay.
 *
 * NativeActivity attaches its input queue at the window level, so gamepad
 * keys and joystick motion never reach the activity's Java dispatch during
 * gameplay (they flow to the native queue and winit, which drops joystick
 * motion). This focusable popup is a separate window with its own Java
 * dispatch: keys and motion land here, and every event reuses the
 * activity's existing handlers (same diversion tables, same logging, same
 * bridge producers), so Kotlin stays the single producer.
 *
 * The activity window goes `FLAG_NOT_FOCUSABLE` while this window shows,
 * so screen touches (which pass through to the game underneath) never
 * steal key focus back: input stays deterministic instead of flapping
 * between the Kotlin bridge and the winit fallback. Losing window focus
 * (drawer opens, another activity covers us) releases every held
 * controller input, so a focus steal can never strand a stuck button. */
internal class GamepadInputCaptureView(
    context: Context,
    private val activity: MainActivity,
) : FrameLayout(context) {
    init {
        isFocusable = true
        isFocusableInTouchMode = true
        isClickable = false
    }

    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        // Back never reaches the activity dispatcher from this window
        // (each window has its own), so run the same stack here. UP keeps
        // the standard click shape; DOWN falls through to the gate below
        // and dies unhandled like any other unconsumed key.
        if (event.keyCode == KeyEvent.KEYCODE_BACK && event.action == KeyEvent.ACTION_UP) {
            activity.handleBackNavigation()
            return true
        }
        return activity.dispatchKeyEvent(event)
    }

    override fun dispatchGenericMotionEvent(event: MotionEvent): Boolean =
        activity.onGenericMotionEvent(event)

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (!hasFocus) {
            activity.releaseAllControllers()
        }
    }
}
