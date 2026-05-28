package io.github.chalharu.nerust

import android.app.AlertDialog
import android.app.NativeActivity
import android.content.Context
import android.content.Intent
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.Typeface
import android.graphics.drawable.ColorDrawable
import android.os.Bundle
import android.util.Log
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.FrameLayout
import android.widget.ListView
import android.widget.PopupWindow
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.statusBarsPadding
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
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.ComposeView
import androidx.compose.ui.platform.ViewCompositionStrategy
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.LifecycleRegistry
import androidx.lifecycle.ViewModelStore
import androidx.lifecycle.ViewModelStoreOwner
import androidx.lifecycle.setViewTreeLifecycleOwner
import androidx.lifecycle.setViewTreeViewModelStoreOwner
import androidx.savedstate.SavedStateRegistry
import androidx.savedstate.SavedStateRegistryController
import androidx.savedstate.SavedStateRegistryOwner
import androidx.savedstate.setViewTreeSavedStateRegistryOwner
import kotlin.math.max
import kotlin.math.min
import kotlinx.coroutines.launch

private const val CONTROLS_OVERLAY_TAG = "nerust-controls-overlay"
private const val DRAWER_COMPOSE_TAG = "nerust-drawer-compose"
private const val DRAWER_OVERLAY_TAG = "nerust-drawer-overlay"
private const val MENU_ACTION_LOAD_STATE = "load_state"
private const val MENU_ACTION_OPEN_LIBRARY = "open_library"
private const val MENU_ACTION_OPEN_SETTINGS = "open_settings"
private const val MENU_ACTION_RESET = "reset"
private const val MENU_ACTION_SAVE_STATE = "save_state"
private const val MENU_ACTION_TOGGLE_PAUSE = "toggle_pause"
private const val MENU_BUTTON_TAG = "nerust-menu-button"
private const val DRAWER_TITLE = "Nerust"

private data class DrawerAction(val label: String, val action: String)

private val DRAWER_ACTIONS = listOf(
    DrawerAction("ROM Library", MENU_ACTION_OPEN_LIBRARY),
    DrawerAction("Settings", MENU_ACTION_OPEN_SETTINGS),
    DrawerAction("Pause / Resume", MENU_ACTION_TOGGLE_PAUSE),
    DrawerAction("Save State", MENU_ACTION_SAVE_STATE),
    DrawerAction("Load State", MENU_ACTION_LOAD_STATE),
    DrawerAction("Reset", MENU_ACTION_RESET),
)

private fun createRomPickerIntent(): Intent =
    Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
        addCategory(Intent.CATEGORY_OPENABLE)
        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION)
        type = "*/*"
    }

class MainActivity : NativeActivity(), LifecycleOwner, SavedStateRegistryOwner, ViewModelStoreOwner {
    private val lifecycleRegistry = LifecycleRegistry(this)
    private val registryController = SavedStateRegistryController.create(this)
    private val store = ViewModelStore()
    private val ensureMenuChromeAttachedRunnable = Runnable { ensureMenuChromeAttached() }
    private var menuChromeAttachAttempts = 0
    private var chromeAttachEnabled = false
    private var controlsOverlayPopup: PopupWindow? = null
    private var controlsOverlayView: View? = null
    private var menuButtonPopup: PopupWindow? = null
    private var menuChromeContainer: FrameLayout? = null
    private var menuButtonView: View? = null
    private var drawerShowing = false
    private var drawerOverlayView: View? = null
    private var drawerComposeView: View? = null
    private var lastDrawerStateForTest = "not requested"

    override val lifecycle: Lifecycle
        get() = lifecycleRegistry

    override val savedStateRegistry: SavedStateRegistry
        get() = registryController.savedStateRegistry

    override val viewModelStore: ViewModelStore
        get() = store

    override fun onCreate(savedInstanceState: Bundle?) {
        registryController.performAttach()
        registryController.performRestore(savedInstanceState)
        super.onCreate(savedInstanceState)
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_CREATE)
        scheduleMenuChromeAttach()
    }

    override fun onStart() {
        super.onStart()
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_START)
    }

    override fun onResume() {
        super.onResume()
        chromeAttachEnabled = true
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_RESUME)
        scheduleMenuChromeAttach()
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus) {
            scheduleMenuChromeAttach()
        }
    }

    override fun onPause() {
        chromeAttachEnabled = false
        removePendingChromeAttachCallbacks()
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_PAUSE)
        super.onPause()
    }

    override fun onStop() {
        removePendingChromeAttachCallbacks()
        dismissChromePopups()
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_STOP)
        super.onStop()
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        registryController.performSave(outState)
    }

    override fun onDestroy() {
        chromeAttachEnabled = false
        removePendingChromeAttachCallbacks()
        dismissChromePopups()
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_DESTROY)
        store.clear()
        super.onDestroy()
    }

    @Suppress("DEPRECATION")
    override fun onBackPressed() {
        if (removeDrawerOverlay()) {
            return
        }
        super.onBackPressed()
    }

    @Suppress("DEPRECATION")
    fun startRomPicker() {
        startActivityForResult(createRomPickerIntent(), ROM_PICKER_REQUEST_CODE)
    }

    @Suppress("DEPRECATION")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != ROM_PICKER_REQUEST_CODE) {
            return
        }

        val uri = if (resultCode == RESULT_OK) data?.data else null
        if (uri != null) {
            val takeFlags = data?.flags
                ?.and(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                ?.takeIf { it != 0 }
                ?: Intent.FLAG_GRANT_READ_URI_PERMISSION
            try {
                contentResolver.takePersistableUriPermission(uri, takeFlags)
            } catch (error: SecurityException) {
                Log.w(TAG, "Failed to keep Android ROM URI permission", error)
            }
        }

        onFilePickerResult(uri?.toString())
    }

    fun isChromeViewShowingForTest(tag: String): Boolean =
        when (tag) {
            CONTROLS_OVERLAY_TAG -> controlsOverlayPopup?.isShowing == true &&
                controlsOverlayView.isShownInWindowForTest()
            DRAWER_COMPOSE_TAG -> drawerShowing && menuButtonPopup?.isShowing == true &&
                drawerComposeView.isShownInWindowForTest()
            DRAWER_OVERLAY_TAG -> drawerShowing && menuButtonPopup?.isShowing == true &&
                drawerOverlayView.isShownInWindowForTest()
            MENU_BUTTON_TAG -> menuButtonPopup?.isShowing == true &&
                menuButtonView.isShownInWindowForTest()
            else -> false
        }

    fun findChromeViewForTest(tag: String): View? =
        when (tag) {
            CONTROLS_OVERLAY_TAG -> controlsOverlayView
            DRAWER_COMPOSE_TAG -> drawerComposeView
            DRAWER_OVERLAY_TAG -> drawerOverlayView
            MENU_BUTTON_TAG -> menuButtonView
            else -> window.decorView.findViewWithTag(tag)
        }

    fun chromeDebugStateForTest(tag: String): String =
        "tag=$tag, destroyed=$isDestroyed, finishing=$isFinishing, chromeAttachEnabled=$chromeAttachEnabled, " +
            "attachAttempts=$menuChromeAttachAttempts, decor=${window.decorView.debugViewState()}, " +
            "controlsPopup=${controlsOverlayPopup.debugPopupState()}, controlsView=${controlsOverlayView.debugViewState()}, " +
            "menuPopup=${menuButtonPopup.debugPopupState()}, menuView=${menuButtonView.debugViewState()}, " +
            "drawerShowing=$drawerShowing, drawerOverlay=${drawerOverlayView.debugViewState()}, " +
            "drawerCompose=${drawerComposeView.debugViewState()}, lastDrawer=$lastDrawerStateForTest"

    /**
     * Show a modal ROM library dialog.
     *
     * The first item is always "Import new ROM…"; the remaining items are the
     * provided library entries in order.  When the user makes a selection this
     * method calls [onRomLibrarySelected] with the appropriate id and then
     * returns control to Rust.  On cancel/dismiss it calls
     * [onRomLibrarySelected] with `null`.
     *
     * Called from the Rust JNI bridge on the Java main thread.
     */
    fun showRomLibraryDialog(entryNames: Array<String>, entryIds: Array<String>) {
        val items = ArrayList<String>(entryNames.size + 1)
        items.add("Import new ROM\u2026")
        items.addAll(entryNames)
        var resultSent = false

        AlertDialog.Builder(this)
            .setTitle("ROM Library")
            .setItems(items.toTypedArray()) { _, which ->
                resultSent = true
                if (which == 0) {
                    // User wants to import – tell Rust first, which will
                    // trigger the SAF picker on its own next event-loop turn.
                    onRomLibrarySelected(IMPORT_ACTION_ID)
                } else {
                    onRomLibrarySelected(entryIds[which - 1])
                }
            }
            .setOnDismissListener {
                if (!resultSent) {
                    onRomLibrarySelected(null)
                }
            }
            .show()
    }

    /**
     * Show a modal Android settings dialog.
     *
     * Presents an Android-relevant subset of settings. Each setting is backed
     * by a tab-separated list of choices; the current selection is identified
     * by index.  Tapping a row in the list opens a single-choice sub-dialog.
     * Tapping "Save" calls [onSettingsDialogResult] with a comma-separated
     * string of the final choice indices.  Cancel/dismiss calls it with `null`.
     *
     * @param keys           Stable setting identifiers (kept for documentation
     *                       clarity and potential future use).
     * @param labels         Human-readable label for each setting.
     * @param choiceStrings  Tab-separated choice labels for each setting.
     * @param currentIndices Current choice index as a string for each setting.
     *
     * Called from the Rust JNI bridge on the Java main thread.
     */
    fun showSettingsDialog(
        keys: Array<String>,
        labels: Array<String>,
        choiceStrings: Array<String>,
        currentIndices: Array<String>,
    ) {
        val selectedIndices = IntArray(labels.size) { i ->
            currentIndices.getOrNull(i)?.toIntOrNull() ?: 0
        }
        val choiceArrays: Array<Array<String>> = Array(choiceStrings.size) { i ->
            choiceStrings[i].split('\t').toTypedArray()
        }

        fun itemText(i: Int): String =
            "${labels[i]}: ${choiceArrays[i].getOrElse(selectedIndices[i]) { "?" }}"

        val displayItems = Array(labels.size) { i -> itemText(i) }
        val listView = ListView(this)
        val adapter = ArrayAdapter(this, android.R.layout.simple_list_item_1, displayItems)
        listView.adapter = adapter

        var resultSent = false

        val parentDialog = AlertDialog.Builder(this)
            .setTitle("Settings")
            .setView(listView)
            .setPositiveButton("Save") { _, _ ->
                resultSent = true
                onSettingsDialogResult(selectedIndices.joinToString(","))
            }
            .setNegativeButton("Cancel") { _, _ ->
                resultSent = true
                onSettingsDialogResult(null)
            }
            .setOnDismissListener {
                if (!resultSent) {
                    onSettingsDialogResult(null)
                }
            }
            .create()

        listView.setOnItemClickListener { _, _, which, _ ->
            AlertDialog.Builder(this)
                .setTitle(labels[which])
                .setSingleChoiceItems(choiceArrays[which], selectedIndices[which]) { subDialog, choice ->
                    selectedIndices[which] = choice
                    displayItems[which] = itemText(which)
                    adapter.notifyDataSetChanged()
                    subDialog.dismiss()
                }
                .setNegativeButton("Cancel", null)
                .show()
        }

        parentDialog.show()
    }

    private fun scheduleMenuChromeAttach() {
        if (!chromeAttachEnabled || isFinishing || isDestroyed) {
            return
        }
        menuChromeAttachAttempts = 0
        ensureMenuChromeAttached()
        window.decorView.post(ensureMenuChromeAttachedRunnable)
    }

    private fun ensureMenuChromeAttached() {
        if (!chromeAttachEnabled || isFinishing || isDestroyed) {
            return
        }
        val anchor = popupAnchor() ?: run {
            retryMenuChromeAttach()
            return
        }
        installComposeOwners(anchor)
        val controlsAttached = ensureControlsOverlayPopup(anchor)
        val buttonAttached = ensureMenuButtonPopup(anchor)
        if (!controlsAttached || !buttonAttached) {
            retryMenuChromeAttach()
        }
    }

    private fun retryMenuChromeAttach() {
        if (menuChromeAttachAttempts >= MENU_CHROME_MAX_ATTACH_ATTEMPTS) {
            Log.w(TAG, "Menu chrome attach skipped because Android window token was unavailable")
            return
        }
        menuChromeAttachAttempts += 1
        window.decorView.postDelayed(ensureMenuChromeAttachedRunnable, MENU_CHROME_ATTACH_RETRY_DELAY_MS)
    }

    private fun removePendingChromeAttachCallbacks() {
        window.decorView.removeCallbacks(ensureMenuChromeAttachedRunnable)
    }

    private fun ensureControlsOverlayPopup(anchor: View): Boolean {
        val existing = controlsOverlayPopup
        if (existing?.isShowing == true && controlsOverlayView != null) {
            return true
        }

        controlsOverlayPopup?.dismiss()
        val view = createControlsOverlay()
        val popup = PopupWindow(
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
        if (showPopupAtLocation(popup, anchor, Gravity.TOP or Gravity.START, 0, 0)) {
            return true
        }
        controlsOverlayPopup = null
        controlsOverlayView = null
        return false
    }

    private fun ensureMenuButtonPopup(anchor: View): Boolean {
        val existing = menuButtonPopup
        if (existing?.isShowing == true && (menuButtonView != null || drawerShowing)) {
            return true
        }

        menuButtonPopup?.dismiss()
        val container = FrameLayout(this)
        val view = createMenuButtonOverlay()
        container.addView(view)
        val popup = PopupWindow(
            container,
            ViewGroup.LayoutParams.WRAP_CONTENT,
            ViewGroup.LayoutParams.WRAP_CONTENT,
            false,
        ).apply {
            isClippingEnabled = false
            inputMethodMode = PopupWindow.INPUT_METHOD_NOT_NEEDED
            elevation = dp(8).toFloat()
            setBackgroundDrawable(ColorDrawable(Color.TRANSPARENT))
        }

        menuChromeContainer = container
        menuButtonView = view
        menuButtonPopup = popup
        val margin = dp(16)
        if (showPopupAtLocation(popup, anchor, Gravity.TOP or Gravity.START, margin, statusBarHeight() + margin)) {
            return true
        }
        menuButtonPopup = null
        menuChromeContainer = null
        menuButtonView = null
        return false
    }

    private fun installComposeOwners(root: View) {
        listOf(window.decorView, root).forEach { view ->
            view.setViewTreeLifecycleOwner(this)
            view.setViewTreeSavedStateRegistryOwner(this)
            view.setViewTreeViewModelStoreOwner(this)
        }
    }

    private fun createControlsOverlay(): View =
        ControlsOverlayView(this).apply {
            tag = CONTROLS_OVERLAY_TAG
            layoutParams = FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            )
            isClickable = false
            isFocusable = false
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        }

    private fun createMenuButtonOverlay(): Button =
        Button(this).apply {
            tag = MENU_BUTTON_TAG
            text = "Menu"
            contentDescription = "Menu"
            setAllCaps(false)
            setOnClickListener { showDrawerOverlay() }
            layoutParams = FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
                Gravity.TOP or Gravity.START,
            )
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_YES
        }

    private fun showDrawerOverlay() {
        if (popupAnchor() == null) {
            lastDrawerStateForTest = "anchor unavailable: decor=${window.decorView.debugViewState()}"
            return
        }
        val popup = menuButtonPopup ?: run {
            lastDrawerStateForTest = "menu popup unavailable"
            return
        }
        val container = menuChromeContainer ?: run {
            lastDrawerStateForTest = "menu container unavailable"
            return
        }
        if (drawerShowing) {
            lastDrawerStateForTest = "already showing"
            return
        }
        lastDrawerStateForTest = "creating"

        val overlay = ComposeOwnerFrameLayout(this).apply {
            tag = DRAWER_OVERLAY_TAG
            setTag(R.id.nerust_drawer_content_probe, drawerContentDescription())
            layoutParams = FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            )
        }
        val drawerContent = ComposeView(this).apply {
            tag = DRAWER_COMPOSE_TAG
            layoutParams = FrameLayout.LayoutParams(
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
        installComposeOwners(overlay)
        installComposeOwners(drawerContent)
        overlay.addView(drawerContent)

        container.removeAllViews()
        container.addView(overlay)
        drawerOverlayView = overlay
        drawerComposeView = drawerContent
        drawerShowing = true
        menuButtonView = null
        val shown = updateMenuChromePopupForDrawer(popup)
        lastDrawerStateForTest =
            "showInMenuPopup=$shown, popup=${popup.debugPopupState()}, overlay=${overlay.debugViewState()}"
        if (!shown) {
            clearDrawerWindowReferences()
            restoreMenuButtonOverlay()
        }
    }

    private fun removeDrawerOverlay(): Boolean {
        if (!drawerShowing) {
            return false
        }
        clearDrawerWindowReferences()
        return restoreMenuButtonOverlay()
    }

    private fun popupAnchor(): View? =
        window.decorView.takeIf { it.isAttachedToWindow && it.windowToken != null }

    private fun showPopupAtLocation(
        popup: PopupWindow,
        anchor: View,
        gravity: Int,
        x: Int,
        y: Int,
    ): Boolean =
        try {
            popup.showAtLocation(anchor, gravity, x, y)
            true
        } catch (_: WindowManager.BadTokenException) {
            false
        } catch (_: IllegalStateException) {
            false
        }

    private fun updateMenuChromePopupForDrawer(popup: PopupWindow): Boolean =
        updatePopupWindow(popup, 0, 0, ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT)

    private fun restoreMenuButtonOverlay(): Boolean {
        val popup = menuButtonPopup ?: return false
        val container = menuChromeContainer ?: return false
        container.removeAllViews()
        val button = createMenuButtonOverlay()
        container.addView(button)
        menuButtonView = button
        val margin = dp(16)
        return updatePopupWindow(
            popup,
            margin,
            statusBarHeight() + margin,
            ViewGroup.LayoutParams.WRAP_CONTENT,
            ViewGroup.LayoutParams.WRAP_CONTENT,
        )
    }

    private fun updatePopupWindow(popup: PopupWindow, x: Int, y: Int, width: Int, height: Int): Boolean =
        try {
            popup.width = width
            popup.height = height
            popup.update(x, y, width, height)
            true
        } catch (_: WindowManager.BadTokenException) {
            false
        } catch (_: IllegalStateException) {
            false
        }

    private fun clearDrawerWindowReferences() {
        drawerShowing = false
        drawerOverlayView = null
        drawerComposeView = null
    }

    private fun View?.isShownInWindowForTest(): Boolean =
        this != null && visibility == View.VISIBLE && isAttachedToWindow && windowToken != null

    private fun View?.debugViewState(): String =
        if (this == null) {
            "null"
        } else {
            "visibility=$visibility, attached=$isAttachedToWindow, token=${windowToken != null}, shown=$isShown"
        }

    private fun PopupWindow?.debugPopupState(): String =
        if (this == null) "null" else "showing=$isShowing"

    private fun dismissChromePopups() {
        clearDrawerWindowReferences()
        menuButtonPopup?.dismiss()
        menuButtonPopup = null
        menuChromeContainer = null
        menuButtonView = null
        controlsOverlayPopup?.dismiss()
        controlsOverlayPopup = null
        controlsOverlayView = null
    }

    private fun dp(value: Int): Int =
        (value * resources.displayMetrics.density).toInt()

    private fun statusBarHeight(): Int {
        val resourceId = resources.getIdentifier("status_bar_height", "dimen", "android")
        return if (resourceId > 0) resources.getDimensionPixelSize(resourceId) else 0
    }

    private fun drawerContentDescription(): String =
        (listOf(DRAWER_TITLE) + DRAWER_ACTIONS.map { it.label }).joinToString("\n")

    private fun dispatchMenuAction(action: String) {
        removeDrawerOverlay()
        onMenuAction(action)
    }

    private inner class ComposeOwnerFrameLayout(context: Context) : FrameLayout(context) {
        override fun onAttachedToWindow() {
            installComposeOwners(this)
            installComposeOwners(rootView)
            super.onAttachedToWindow()
        }
    }

    private external fun onFilePickerResult(uri: String?)

    private external fun onMenuAction(action: String)

    private external fun onRomLibrarySelected(id: String?)

    private external fun onSettingsDialogResult(result: String?)

    companion object {
        private const val TAG = "Nerust"
        private const val ROM_PICKER_REQUEST_CODE = 0x4E45
        private const val MENU_CHROME_ATTACH_RETRY_DELAY_MS = 100L
        private const val MENU_CHROME_MAX_ATTACH_ATTEMPTS = 100
        // Must match `android/library.rs::IMPORT_ACTION_ID`.
        private const val IMPORT_ACTION_ID = "__import__"

        fun createRomPickerIntentForTest(): Intent = createRomPickerIntent()
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
                Text(
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
            modifier = Modifier
                .fillMaxSize()
                .padding(WindowInsets.safeDrawing.asPaddingValues()),
        )
    }
}

@Composable
private fun DrawerActionItem(label: String, onClick: () -> Unit) {
    NavigationDrawerItem(
        label = { Text(label) },
        selected = false,
        onClick = onClick,
        modifier = Modifier
            .semantics { contentDescription = label }
            .padding(NavigationDrawerItemDefaults.ItemPadding),
    )
}

private class ControlsOverlayView(context: Context) : View(context) {
    private val fillPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = Color.argb(48, 255, 255, 255)
        style = Paint.Style.FILL
    }
    private val strokePaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = Color.argb(160, 255, 255, 255)
        strokeWidth = 2f
        style = Paint.Style.STROKE
    }
    private val textPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = Color.argb(220, 255, 255, 255)
        textAlign = Paint.Align.CENTER
        typeface = Typeface.create(Typeface.DEFAULT, Typeface.BOLD)
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        val viewWidth = width.toFloat()
        val viewHeight = height.toFloat()
        if (viewWidth <= 0f || viewHeight <= 0f) {
            return
        }

        val controlTop = viewHeight * 0.52f
        val controlHeight = viewHeight - controlTop
        val dpadLeft = viewWidth * 0.06f
        val dpadSize = viewWidth * 0.30f
        val actionSize = viewWidth * 0.16f
        val actionRight = viewWidth * 0.76f
        val faceTop = controlTop + controlHeight * 0.10f
        val centerTop = controlTop + controlHeight * 0.38f

        drawZone(
            canvas,
            dpadLeft + dpadSize * 0.25f,
            controlTop + controlHeight * 0.05f,
            dpadSize * 0.50f,
            dpadSize * 0.28f,
            "UP",
        )
        drawZone(
            canvas,
            dpadLeft + dpadSize * 0.25f,
            controlTop + controlHeight * 0.47f,
            dpadSize * 0.50f,
            dpadSize * 0.28f,
            "DOWN",
        )
        drawZone(
            canvas,
            dpadLeft,
            controlTop + controlHeight * 0.26f,
            dpadSize * 0.28f,
            dpadSize * 0.36f,
            "LEFT",
        )
        drawZone(
            canvas,
            dpadLeft + dpadSize * 0.47f,
            controlTop + controlHeight * 0.26f,
            dpadSize * 0.28f,
            dpadSize * 0.36f,
            "RIGHT",
        )
        drawZone(canvas, actionRight - actionSize * 1.1f, faceTop + actionSize * 0.55f, actionSize, actionSize, "B")
        drawZone(canvas, actionRight, faceTop, actionSize, actionSize, "A")
        drawZone(canvas, viewWidth * 0.36f, centerTop, viewWidth * 0.12f, viewHeight * 0.05f, "SELECT")
        drawZone(canvas, viewWidth * 0.52f, centerTop, viewWidth * 0.12f, viewHeight * 0.05f, "START")
    }

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
