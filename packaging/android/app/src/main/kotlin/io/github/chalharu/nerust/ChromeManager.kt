package io.github.chalharu.nerust

import android.graphics.Color
import android.graphics.drawable.ColorDrawable
import android.os.SystemClock
import android.util.Log
import android.view.Gravity
import android.view.HapticFeedbackConstants
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.widget.FrameLayout
import android.widget.PopupWindow
import android.widget.TextView
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.clickable
import androidx.compose.material3.DrawerValue
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalDrawerSheet
import androidx.compose.material3.ModalNavigationDrawer
import androidx.compose.material3.NavigationDrawerItem
import androidx.compose.material3.NavigationDrawerItemDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.rememberDrawerState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.ComposeView
import androidx.compose.ui.platform.ViewCompositionStrategy
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch

internal const val CONTROLS_OVERLAY_TAG = "nerust-controls-overlay"
internal const val DRAWER_COMPOSE_TAG = "nerust-drawer-compose"
internal const val DRAWER_EDGE_HANDLE_TAG = "nerust-drawer-edge-handle"
internal const val DRAWER_OVERLAY_TAG = "nerust-drawer-overlay"
internal const val FPS_OVERLAY_TAG = "nerust-fps-overlay"
internal const val DRAWER_TITLE = "Nerust"

private data class DrawerAction(
    val label: String,
    val action: String,
)

private const val MENU_ACTION_EXIT = "exit"
private const val MENU_ACTION_LOAD_STATE = "load_state"
private const val MENU_ACTION_OPEN_ROM = "open_rom"
private const val MENU_ACTION_OPEN_SETTINGS = "open_settings"
private const val MENU_ACTION_RESET = "reset"
private const val MENU_ACTION_SAVE_STATE = "save_state"
private const val MENU_ACTION_TOGGLE_PAUSE = "toggle_pause"
private const val MENU_ACTION_UNLOAD = "unload"

private val DRAWER_ACTIONS =
    listOf(
        DrawerAction("Open ROM", MENU_ACTION_OPEN_ROM),
        DrawerAction("Settings", MENU_ACTION_OPEN_SETTINGS),
        DrawerAction("Pause / Resume", MENU_ACTION_TOGGLE_PAUSE),
        DrawerAction("Save State", MENU_ACTION_SAVE_STATE),
        DrawerAction("Load State", MENU_ACTION_LOAD_STATE),
        DrawerAction("Reset", MENU_ACTION_RESET),
        DrawerAction("Unload ROM", MENU_ACTION_UNLOAD),
        DrawerAction("Exit", MENU_ACTION_EXIT),
    )

/**
 * Non-dialog chrome: touch overlay, drawer, FPS label, and attach retries.
 *
 * Extracted from MainActivity. Owns all `PopupWindow` chrome plus the
 * touch-overlay configuration pushed from Rust. The activity forwards
 * lifecycle calls and keeps the test probes; JNI entry points on the
 * activity delegate here.
 */
class ChromeManager(private val activity: MainActivity) {
    private val ensureChromeAttachedRunnable = Runnable { ensureChromeAttached() }
    private val restoreControllerOverlayRunnable =
        Runnable {
            controllerOverlayHiddenUntil = 0L
            scheduleChromeAttach()
        }
    private var chromeAttachAttempts = 0
    var chromeAttachEnabled = false
    private var controlsOverlayPopup: PopupWindow? = null
    private var controlsOverlayView: View? = null
    private var controllerOverlayHiddenUntil = 0L
    private var controlsVisibility = "auto"
    private var controlsOpacityPercent = 65
    private var controlsScalePercent = 100
    private var controlsVerticalOffsetPercent = 0
    private var controlsHaptics = true
    private var controlsShouldersVisible = true
    private var controlsPushedZones: List<PushedOverlayZone> = emptyList()
    private var drawerChromePopup: PopupWindow? = null
    private var drawerEdgeHandleView: View? = null
    private var drawerShowing = false
    private var drawerFullScreenPopup: PopupWindow? = null
    private var drawerOverlayView: View? = null
    private var drawerComposeView: View? = null
    private var fpsOverlayView: TextView? = null
    private var fpsOverlayPopup: PopupWindow? = null
    private var fpsValue: Float = 0f
    private var fpsVisible: Boolean = false
    var lastDrawerStateForTest = "not requested"
        private set

    fun onResume() {
        chromeAttachEnabled = true
        scheduleChromeAttach()
    }

    fun onPause() {
        chromeAttachEnabled = false
        removePendingChromeAttachCallbacks()
        activity.window.decorView.removeCallbacks(restoreControllerOverlayRunnable)
    }

    fun onStop() {
        removePendingChromeAttachCallbacks()
        dismissChromePopups()
    }

    /**
     * Destroy path: like [onStop] but also disables re-attachment so no
     * posted runnable can resurrect chrome after the window is gone.
     * Callers (and tests via [hasLiveChromeForTest]) assert nothing
     * survives this.
     */
    fun onDestroy() {
        chromeAttachEnabled = false
        activity.window.decorView.removeCallbacks(ensureChromeAttachedRunnable)
        activity.window.decorView.removeCallbacks(restoreControllerOverlayRunnable)
        removePendingChromeAttachCallbacks()
        dismissChromePopups()
    }

    /** Test probe: true while any chrome popup is still showing. */
    fun hasLiveChromeForTest(): Boolean =
        controlsOverlayPopup?.isShowing == true ||
            drawerChromePopup?.isShowing == true ||
            drawerFullScreenPopup?.isShowing == true ||
            fpsOverlayPopup?.isShowing == true

    fun onConfigurationChanged() {
        controlsOverlayPopup?.dismiss()
        controlsOverlayPopup = null
        controlsOverlayView = null
        drawerChromePopup?.dismiss()
        drawerChromePopup = null
        drawerEdgeHandleView = null
        drawerFullScreenPopup?.setOnDismissListener(null)
        drawerFullScreenPopup?.dismiss()
        drawerFullScreenPopup = null
        clearDrawerWindowReferences()
        fpsOverlayPopup?.dismiss()
        fpsOverlayPopup = null
        fpsOverlayView = null
        removePendingChromeAttachCallbacks()
        scheduleChromeAttach()
    }

    fun configureControlsOverlay(
        visibility: String,
        opacityPercent: Int,
        scalePercent: Int,
        verticalOffsetPercent: Int,
        haptics: Boolean,
        shouldersVisible: Boolean,
        zones: String,
    ) {
        controlsVisibility = visibility
        controlsOpacityPercent = opacityPercent.coerceIn(0, 100)
        controlsScalePercent = scalePercent.coerceIn(50, 150)
        controlsVerticalOffsetPercent = verticalOffsetPercent.coerceIn(-30, 30)
        controlsHaptics = haptics
        controlsShouldersVisible = shouldersVisible
        controlsPushedZones = parsePushedZones(zones)
        controlsOverlayPopup?.dismiss()
        controlsOverlayPopup = null
        controlsOverlayView = null
        scheduleChromeAttach()
    }

    fun performControlHaptic() {
        if (controlsHaptics) {
            activity.window.decorView.performHapticFeedback(HapticFeedbackConstants.KEYBOARD_TAP)
        }
    }

    fun updateFloatingDpad(
        active: Boolean,
        centerX: Float,
        centerY: Float,
        knobX: Float,
        knobY: Float,
    ) {
        (controlsOverlayView as? ControlsOverlayView)?.updateFloatingDpad(
            active,
            centerX,
            centerY,
            knobX,
            knobY,
        )
    }

    fun floatingDpadStateForTest(): FloatArray? =
        (controlsOverlayView as? ControlsOverlayView)?.floatingDpadStateForTest()

    fun updateFpsOverlay(fps: Float, visible: Boolean) {
        Log.d(TAG, "updateFpsOverlay: fps=$fps visible=$visible")
        fpsValue = fps
        fpsVisible = visible
        if (!visible) {
            fpsOverlayPopup?.dismiss()
            fpsOverlayPopup = null
            return
        }
        popupAnchor()?.let { ensureFpsOverlayPopup(it) } ?: scheduleChromeAttach()
    }

    fun showDrawerOverlay() {
        val anchor =
            popupAnchor() ?: run {
                lastDrawerStateForTest =
                    "anchor unavailable: decor=${activity.window.decorView.debugViewState()}"
                Log.w(TAG, "showDrawerOverlay: anchor unavailable decor=${activity.window.decorView.debugViewState()}")
                return
            }
        if (drawerShowing) {
            lastDrawerStateForTest = "already showing"
            Log.i(TAG, "showDrawerOverlay: already showing")
            return
        }
        lastDrawerStateForTest = "creating"
        Log.i(TAG, "showDrawerOverlay: creating")

        // Hide the edge-handle popup while the drawer is open.
        drawerChromePopup?.dismiss()

        val overlay =
            ComposeOwnerFrameLayout(activity) { view ->
                activity.installComposeOwners(view)
            }.apply {
                tag = DRAWER_OVERLAY_TAG
                setTag(R.id.nerust_drawer_content_probe, drawerContentDescription())
                layoutParams =
                    FrameLayout.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT,
                        ViewGroup.LayoutParams.MATCH_PARENT,
                    )
            }
        val drawerContent =
            ComposeView(activity).apply {
                tag = DRAWER_COMPOSE_TAG
                layoutParams =
                    FrameLayout.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT,
                        ViewGroup.LayoutParams.MATCH_PARENT,
                    )
                setViewCompositionStrategy(ViewCompositionStrategy.DisposeOnDetachedFromWindow)
                setContent {
                    MaterialTheme {
                        NerustDrawerOverlay(
                            onDismissRequest = { removeDrawerOverlay() },
                            onMenuAction = ::dispatchMenuAction,
                        )
                    }
                }
            }
        activity.installComposeOwners(overlay)
        activity.installComposeOwners(drawerContent)
        overlay.addView(drawerContent)

        // Use a brand-new full-screen popup so Compose measures against the
        // correct screen dimensions from the start (no resizing needed).
        val fullScreenPopup =
            PopupWindow(
                overlay,
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
                true,
            ).apply {
                isTouchable = true
                isClippingEnabled = false
                inputMethodMode = PopupWindow.INPUT_METHOD_NOT_NEEDED
                elevation = dp(8).toFloat()
                setBackgroundDrawable(ColorDrawable(Color.TRANSPARENT))
                setOnDismissListener { removeDrawerOverlay() }
            }

        drawerOverlayView = overlay
        drawerComposeView = drawerContent
        drawerEdgeHandleView = null
        drawerShowing = true

        val shown =
            showPopupAtLocation("drawer-fullscreen", fullScreenPopup, anchor, Gravity.TOP or Gravity.START, 0, 0)
        if (shown) {
            drawerFullScreenPopup = fullScreenPopup
        } else {
            clearDrawerWindowReferences()
        }
        lastDrawerStateForTest =
            "showInDrawerPopup=$shown, popup=${fullScreenPopup.debugPopupState()}, overlay=${overlay.debugViewState()}"
        Log.i(TAG, "showDrawerOverlay: shown=$shown state=$lastDrawerStateForTest")
    }

    fun removeDrawerOverlay(): Boolean {
        if (!drawerShowing) {
            return false
        }
        Log.i(TAG, "removeDrawerOverlay")
        drawerFullScreenPopup?.setOnDismissListener(null)
        drawerFullScreenPopup?.dismiss()
        drawerFullScreenPopup = null
        clearDrawerWindowReferences()
        // Re-show the narrow edge-handle popup for future swipe detection.
        popupAnchor()?.let { ensureDrawerChromePopup(it) }
        return true
    }

    fun dispatchMenuAction(action: String) {
        removeDrawerOverlay()
        activity.sendMenuAction(action)
    }

    fun dismissChromePopups() {
        clearDrawerWindowReferences()
        drawerFullScreenPopup?.setOnDismissListener(null)
        drawerFullScreenPopup?.dismiss()
        drawerFullScreenPopup = null
        drawerChromePopup?.dismiss()
        drawerChromePopup = null
        drawerEdgeHandleView = null
        controlsOverlayPopup?.dismiss()
        controlsOverlayPopup = null
        controlsOverlayView = null
        fpsOverlayPopup?.dismiss()
        fpsOverlayPopup = null
        fpsOverlayView = null
    }

    fun isViewShowingForTest(tag: String): Boolean? =
        when (tag) {
            CONTROLS_OVERLAY_TAG -> {
                controlsOverlayPopup?.isShowing == true &&
                    controlsOverlayView.isShownInWindowForTest()
            }
            DRAWER_COMPOSE_TAG -> {
                drawerShowing && drawerFullScreenPopup?.isShowing == true &&
                    drawerComposeView.isShownInWindowForTest()
            }
            DRAWER_EDGE_HANDLE_TAG -> {
                !drawerShowing && drawerChromePopup?.isShowing == true &&
                    drawerEdgeHandleView.isShownInWindowForTest()
            }
            DRAWER_OVERLAY_TAG -> {
                drawerShowing && drawerFullScreenPopup?.isShowing == true &&
                    drawerOverlayView.isShownInWindowForTest()
            }
            FPS_OVERLAY_TAG -> {
                fpsOverlayPopup?.isShowing == true &&
                    fpsOverlayView.isShownInWindowForTest()
            }
            else -> null
        }

    fun findViewForTest(tag: String): View? =
        when (tag) {
            CONTROLS_OVERLAY_TAG -> controlsOverlayView
            DRAWER_COMPOSE_TAG -> drawerComposeView
            DRAWER_EDGE_HANDLE_TAG -> drawerEdgeHandleView
            DRAWER_OVERLAY_TAG -> drawerOverlayView
            FPS_OVERLAY_TAG -> fpsOverlayView
            else -> null
        }

    fun debugStateForTest(): String =
        "chromeAttachEnabled=$chromeAttachEnabled, attachAttempts=$chromeAttachAttempts, " +
            "controlsPopup=${controlsOverlayPopup.debugPopupState()}, controlsView=${controlsOverlayView.debugViewState()}, " +
            "drawerPopup=${drawerChromePopup.debugPopupState()}, drawerHandle=${drawerEdgeHandleView.debugViewState()}, " +
            "drawerShowing=$drawerShowing, drawerFullScreenPopup=${drawerFullScreenPopup.debugPopupState()}, " +
            "drawerOverlay=${drawerOverlayView.debugViewState()}, drawerCompose=${drawerComposeView.debugViewState()}, " +
            "lastDrawer=$lastDrawerStateForTest, fpsVisible=$fpsVisible, fpsValue=$fpsValue, " +
            "fpsPopup=${fpsOverlayPopup.debugPopupState()}, fpsView=${fpsOverlayView.debugViewState()}"

    fun scheduleChromeAttach() {
        if (!chromeAttachEnabled || activity.isFinishing || activity.isDestroyed) {
            Log.i(
                TAG,
                "scheduleChromeAttach: skipped (enabled=$chromeAttachEnabled finishing=${activity.isFinishing} destroyed=${activity.isDestroyed})",
            )
            return
        }
        chromeAttachAttempts = 0
        Log.i(TAG, "scheduleChromeAttach: decor=${activity.window.decorView.debugViewState()}")
        ensureChromeAttached()
        activity.window.decorView.post(ensureChromeAttachedRunnable)
    }

    private fun ensureChromeAttached() {
        if (!chromeAttachEnabled || activity.isFinishing || activity.isDestroyed) {
            Log.i(
                TAG,
                "ensureChromeAttached: skipped (enabled=$chromeAttachEnabled finishing=${activity.isFinishing} destroyed=${activity.isDestroyed})",
            )
            return
        }
        val anchor =
            popupAnchor() ?: run {
                Log.i(TAG, "ensureChromeAttached: anchor unavailable decor=${activity.window.decorView.debugViewState()}")
                retryChromeAttach()
                return
            }
        activity.installComposeOwners(anchor)
        val controlsAttached = ensureControlsOverlayPopup(anchor)
        val drawerAttached = ensureDrawerChromePopup(anchor)
        val fpsAttached = !fpsVisible || ensureFpsOverlayPopup(anchor)
        Log.i(
            TAG,
            "ensureChromeAttached: controlsAttached=$controlsAttached drawerAttached=$drawerAttached " +
                "fpsAttached=$fpsAttached anchor=${anchor.debugViewState()}",
        )
        if (!controlsAttached || !drawerAttached || !fpsAttached) {
            retryChromeAttach()
        }
    }

    private fun retryChromeAttach() {
        if (chromeAttachAttempts >= MENU_CHROME_MAX_ATTACH_ATTEMPTS) {
            Log.w(TAG, "Chrome attach skipped because Android window token was unavailable")
            return
        }
        chromeAttachAttempts += 1
        Log.i(
            TAG,
            "retryChromeAttach: scheduling attempt $chromeAttachAttempts/$MENU_CHROME_MAX_ATTACH_ATTEMPTS",
        )
        activity.window.decorView.postDelayed(
            ensureChromeAttachedRunnable,
            MENU_CHROME_ATTACH_RETRY_DELAY_MS,
        )
    }

    private fun removePendingChromeAttachCallbacks() {
        activity.window.decorView.removeCallbacks(ensureChromeAttachedRunnable)
    }

    private fun ensureControlsOverlayPopup(anchor: View): Boolean {
        if (
            controlsVisibility == "hidden" ||
            controlsVisibility == "auto" && SystemClock.elapsedRealtime() < controllerOverlayHiddenUntil
        ) {
            return true
        }
        val existing = controlsOverlayPopup
        if (existing?.isShowing == true && controlsOverlayView != null) {
            return true
        }

        controlsOverlayPopup?.dismiss()
        val view = createControlsOverlay()
        val popup =
            PopupWindow(
                view,
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
                false,
            ).apply {
                isTouchable = false
                isClippingEnabled = false
                inputMethodMode = PopupWindow.INPUT_METHOD_NOT_NEEDED
                setBackgroundDrawable(ColorDrawable(Color.TRANSPARENT))
            }

        controlsOverlayView = view
        controlsOverlayPopup = popup
        if (showPopupAtLocation("controls-overlay", popup, anchor, Gravity.TOP or Gravity.START, 0, 0)) {
            return true
        }
        controlsOverlayPopup = null
        controlsOverlayView = null
        return false
    }

    private fun hideControlsForControllerInput() {
        if (controlsVisibility != "auto") {
            return
        }
        controllerOverlayHiddenUntil = SystemClock.elapsedRealtime() + CONTROLLER_OVERLAY_HIDE_MS
        controlsOverlayPopup?.dismiss()
        controlsOverlayPopup = null
        controlsOverlayView = null
        activity.window.decorView.removeCallbacks(restoreControllerOverlayRunnable)
        activity.window.decorView.postDelayed(
            restoreControllerOverlayRunnable,
            CONTROLLER_OVERLAY_HIDE_MS,
        )
    }

    fun noteControllerInput() {
        hideControlsForControllerInput()
    }

    private fun ensureDrawerChromePopup(anchor: View): Boolean {
        val existing = drawerChromePopup
        if (existing?.isShowing == true && (drawerEdgeHandleView != null || drawerShowing)) {
            return true
        }

        drawerChromePopup?.dismiss()
        val container = FrameLayout(activity)
        val edgeHandle = createDrawerEdgeHandleOverlay()
        container.addView(edgeHandle)
        val popup =
            PopupWindow(
                container,
                dp(DRAWER_EDGE_HANDLE_WIDTH_DP),
                ViewGroup.LayoutParams.MATCH_PARENT,
                false,
            ).apply {
                isTouchable = true
                isClippingEnabled = false
                inputMethodMode = PopupWindow.INPUT_METHOD_NOT_NEEDED
                elevation = dp(8).toFloat()
                setBackgroundDrawable(ColorDrawable(Color.TRANSPARENT))
            }

        drawerEdgeHandleView = edgeHandle
        drawerChromePopup = popup
        if (showPopupAtLocation("drawer-edge-handle", popup, anchor, Gravity.TOP or Gravity.START, 0, 0)) {
            return true
        }
        drawerChromePopup = null
        drawerEdgeHandleView = null
        return false
    }

    private fun createControlsOverlay(): View =
        ControlsOverlayView(
            activity,
            controlsOpacityPercent,
            controlsScalePercent,
            controlsVerticalOffsetPercent,
            controlsShouldersVisible,
            controlsPushedZones,
        ).apply {
            tag = CONTROLS_OVERLAY_TAG
            layoutParams =
                FrameLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT,
                    ViewGroup.LayoutParams.MATCH_PARENT,
                )
            isClickable = false
            isFocusable = false
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        }

    private fun createDrawerEdgeHandleOverlay(): View =
        DrawerEdgeSwipeHandleView(activity) { showDrawerOverlay() }.apply {
            tag = DRAWER_EDGE_HANDLE_TAG
            contentDescription = "Open navigation drawer"
            layoutParams =
                FrameLayout.LayoutParams(
                    dp(DRAWER_EDGE_HANDLE_WIDTH_DP),
                    ViewGroup.LayoutParams.MATCH_PARENT,
                    Gravity.START,
                )
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_YES
        }

    private fun createFpsOverlayView(): TextView =
        TextView(activity).apply {
            tag = FPS_OVERLAY_TAG
            textSize = 11f
            setTextColor(Color.WHITE)
            setBackgroundColor(Color.argb(128, 0, 0, 0))
            val padding = dp(4)
            setPadding(padding, dp(2), padding, dp(2))
            isClickable = false
            isFocusable = false
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        }

    private fun refreshFpsOverlayText() {
        fpsOverlayView?.text = "%.1f fps".format(fpsValue)
    }

    private fun ensureFpsOverlayPopup(anchor: View): Boolean {
        val existing = fpsOverlayPopup
        if (existing?.isShowing == true && fpsOverlayView != null) {
            refreshFpsOverlayText()
            return true
        }
        fpsOverlayPopup?.dismiss()
        val view = fpsOverlayView ?: createFpsOverlayView().also { fpsOverlayView = it }
        refreshFpsOverlayText()
        val popup =
            PopupWindow(
                view,
                ViewGroup.LayoutParams.WRAP_CONTENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
                false,
            ).apply {
                isTouchable = false
                isFocusable = false
                isClippingEnabled = false
                inputMethodMode = PopupWindow.INPUT_METHOD_NOT_NEEDED
                setBackgroundDrawable(ColorDrawable(Color.TRANSPARENT))
            }
        fpsOverlayPopup = popup
        if (showPopupAtLocation("fps-overlay", popup, anchor, Gravity.BOTTOM or Gravity.END, dp(8), dp(8))) {
            return true
        }
        fpsOverlayPopup = null
        return false
    }

    private fun popupAnchor(): View? =
        activity.window.decorView.takeIf { it.isAttachedToWindow && it.windowToken != null }

    private fun showPopupAtLocation(
        popupName: String,
        popup: PopupWindow,
        anchor: View,
        gravity: Int,
        x: Int,
        y: Int,
    ): Boolean {
        if (anchor.windowToken == null) {
            Log.w(TAG, "showPopupAtLocation: $popupName skipped with null window token")
            return false
        }
        return try {
            popup.showAtLocation(anchor, gravity, x, y)
            Log.i(
                TAG,
                "showPopupAtLocation: $popupName shown at ($x,$y) anchor=${anchor.debugViewState()}",
            )
            true
        } catch (error: WindowManager.BadTokenException) {
            Log.w(TAG, "showPopupAtLocation: $popupName failed with bad token", error)
            false
        } catch (error: IllegalStateException) {
            Log.w(TAG, "showPopupAtLocation: $popupName failed with illegal state", error)
            false
        }
    }

    private fun clearDrawerWindowReferences() {
        drawerShowing = false
        drawerOverlayView = null
        drawerComposeView = null
    }

    private fun dp(value: Int): Int = (value * activity.resources.displayMetrics.density).toInt()

    private fun drawerContentDescription(): String =
        (listOf(DRAWER_TITLE) + DRAWER_ACTIONS.map { it.label }).joinToString("\n")

    private fun View?.isShownInWindowForTest(): Boolean =
        this != null && visibility == View.VISIBLE && isAttachedToWindow && windowToken != null

    private fun View?.debugViewState(): String =
        if (this == null) {
            "null"
        } else {
            "visibility=$visibility, attached=$isAttachedToWindow, token=${windowToken != null}, shown=$isShown"
        }

    private fun PopupWindow?.debugPopupState(): String = if (this == null) "null" else "showing=$isShowing"

    companion object {
        private const val TAG = "Nerust"
        private const val DRAWER_EDGE_HANDLE_WIDTH_DP = 24
        private const val MENU_CHROME_ATTACH_RETRY_DELAY_MS = 100L
        private const val MENU_CHROME_MAX_ATTACH_ATTEMPTS = 100
        private const val CONTROLLER_OVERLAY_HIDE_MS = 5_000L
    }
}

@Composable
private fun NerustDrawerOverlay(
    onDismissRequest: () -> Unit,
    onMenuAction: (String) -> Unit,
) {
    val drawerState = rememberDrawerState(initialValue = DrawerValue.Open)
    val actionPending = remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    fun closeAndRun(action: String) {
        actionPending.value = true
        scope.launch {
            drawerState.close()
            onMenuAction(action)
        }
    }

    LaunchedEffect(drawerState.currentValue, actionPending.value) {
        if (drawerState.currentValue == DrawerValue.Closed && !actionPending.value) {
            onDismissRequest()
        }
    }

    ModalNavigationDrawer(
        drawerState = drawerState,
        drawerContent = {
            ModalDrawerSheet(modifier = Modifier.statusBarsPadding()) {
                androidx.compose.material3.Text(
                    text = DRAWER_TITLE,
                    style = MaterialTheme.typography.titleLarge,
                    modifier = Modifier.padding(start = 24.dp, top = 24.dp, end = 24.dp, bottom = 8.dp),
                )
                Text(
                    text = "Open ROMs and control the current session.",
                    style = MaterialTheme.typography.bodyMedium,
                    modifier = Modifier.padding(start = 24.dp, end = 24.dp, bottom = 16.dp),
                )
                DRAWER_ACTIONS.forEachIndexed { index, action ->
                    DrawerActionItem(
                        label = action.label,
                        onClick = {
                            closeAndRun(action.action)
                        },
                    )
                    if (index == 1) {
                        HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp))
                    }
                }
            }
        },
    ) {
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(WindowInsets.safeDrawing.asPaddingValues()),
        )
    }
}

@Composable
private fun DrawerActionItem(
    label: String,
    onClick: () -> Unit,
) {
    NavigationDrawerItem(
        label = { Text(label) },
        selected = false,
        onClick = onClick,
        modifier =
            Modifier
                .semantics { contentDescription = label }
                .padding(NavigationDrawerItemDefaults.ItemPadding),
    )
}
