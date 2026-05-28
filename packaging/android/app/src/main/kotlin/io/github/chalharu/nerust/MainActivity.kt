package io.github.chalharu.nerust

import android.app.Dialog
import android.app.NativeActivity
import android.content.Context
import android.content.Intent
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
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
import android.widget.FrameLayout
import android.widget.PopupWindow
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material3.DrawerValue
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalDrawerSheet
import androidx.compose.material3.ModalNavigationDrawer
import androidx.compose.material3.NavigationDrawerItem
import androidx.compose.material3.NavigationDrawerItemDefaults
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberDrawerState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
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
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min
import kotlinx.coroutines.launch

private const val CONTROLS_OVERLAY_TAG = "nerust-controls-overlay"
private const val DRAWER_COMPOSE_TAG = "nerust-drawer-compose"
private const val DRAWER_EDGE_HANDLE_TAG = "nerust-drawer-edge-handle"
private const val DRAWER_OVERLAY_TAG = "nerust-drawer-overlay"
private const val MENU_ACTION_LOAD_STATE = "load_state"
private const val MENU_ACTION_OPEN_LIBRARY = "open_library"
private const val MENU_ACTION_OPEN_SETTINGS = "open_settings"
private const val MENU_ACTION_RESET = "reset"
private const val MENU_ACTION_SAVE_STATE = "save_state"
private const val MENU_ACTION_TOGGLE_PAUSE = "toggle_pause"
private const val MENU_BUTTON_TAG = "nerust-menu-button"
private const val ROM_LIBRARY_DIALOG_TAG = "nerust-rom-library-dialog"
private const val SETTINGS_DIALOG_TAG = "nerust-settings-dialog"
private const val DRAWER_TITLE = "Nerust"

private data class DrawerAction(val label: String, val action: String)

private data class AndroidSetting(val key: String, val label: String, val choices: List<String>)

internal data class OverlayZoneSpec(
    val x: Float,
    val y: Float,
    val width: Float,
    val height: Float,
    val label: String,
)

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
    private val ensureChromeAttachedRunnable = Runnable { ensureChromeAttached() }
    private var chromeAttachAttempts = 0
    private var chromeAttachEnabled = false
    private var controlsOverlayPopup: PopupWindow? = null
    private var controlsOverlayView: View? = null
    private var drawerChromePopup: PopupWindow? = null
    private var drawerChromeContainer: FrameLayout? = null
    private var drawerEdgeHandleView: View? = null
    private var drawerShowing = false
    private var drawerOverlayView: View? = null
    private var drawerComposeView: View? = null
    private var composeDialog: Dialog? = null
    private var composeDialogRootView: View? = null
    private var composeDialogComposeView: View? = null
    private var composeDialogTag: String? = null
    private var composeDialogDismissCallback: (() -> Unit)? = null
    private var composeDialogOwnedByTest = false
    private var lastDrawerStateForTest = "not requested"
    private var lastDialogStateForTest = "not requested"

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
        scheduleChromeAttach()
    }

    override fun onStart() {
        super.onStart()
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_START)
    }

    override fun onResume() {
        super.onResume()
        activeActivityForTest = this
        chromeAttachEnabled = true
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_RESUME)
        scheduleChromeAttach()
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus) {
            scheduleChromeAttach()
        }
    }

    override fun onPause() {
        if (activeActivityForTest === this) {
            activeActivityForTest = null
        }
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
        if (activeActivityForTest === this) {
            activeActivityForTest = null
        }
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
            val takeFlags =
                data?.flags
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
            DRAWER_COMPOSE_TAG -> drawerShowing && drawerChromePopup?.isShowing == true &&
                drawerComposeView.isShownInWindowForTest()
            DRAWER_EDGE_HANDLE_TAG -> !drawerShowing && drawerChromePopup?.isShowing == true &&
                drawerEdgeHandleView.isShownInWindowForTest()
            DRAWER_OVERLAY_TAG -> drawerShowing && drawerChromePopup?.isShowing == true &&
                drawerOverlayView.isShownInWindowForTest()
            ROM_LIBRARY_DIALOG_TAG,
            SETTINGS_DIALOG_TAG,
            ->
                composeDialogTag == tag &&
                    composeDialog?.isShowing == true &&
                    composeDialogRootView.isShownInWindowForTest()
            else -> window.decorView.findViewWithTag<View>(tag).isShownInWindowForTest()
        }

    fun findChromeViewForTest(tag: String): View? =
        when (tag) {
            CONTROLS_OVERLAY_TAG -> controlsOverlayView
            DRAWER_COMPOSE_TAG -> drawerComposeView
            DRAWER_EDGE_HANDLE_TAG -> drawerEdgeHandleView
            DRAWER_OVERLAY_TAG -> drawerOverlayView
            ROM_LIBRARY_DIALOG_TAG,
            SETTINGS_DIALOG_TAG,
            ->
                composeDialogRootView.takeIf { composeDialogTag == tag }
            else -> window.decorView.findViewWithTag(tag)
        }

    fun chromeDebugStateForTest(tag: String): String =
        "tag=$tag, destroyed=$isDestroyed, finishing=$isFinishing, chromeAttachEnabled=$chromeAttachEnabled, " +
            "attachAttempts=$chromeAttachAttempts, decor=${window.decorView.debugViewState()}, " +
            "controlsPopup=${controlsOverlayPopup.debugPopupState()}, controlsView=${controlsOverlayView.debugViewState()}, " +
            "drawerPopup=${drawerChromePopup.debugPopupState()}, drawerHandle=${drawerEdgeHandleView.debugViewState()}, " +
            "drawerShowing=$drawerShowing, drawerOverlay=${drawerOverlayView.debugViewState()}, " +
            "drawerCompose=${drawerComposeView.debugViewState()}, dialogTag=$composeDialogTag, " +
            "dialog=${composeDialog.debugDialogState()}, dialogRoot=${composeDialogRootView.debugViewState()}, " +
            "dialogCompose=${composeDialogComposeView.debugViewState()}, lastDrawer=$lastDrawerStateForTest, " +
            "lastDialog=$lastDialogStateForTest"

    fun dispatchMenuActionForTest(action: String) {
        dispatchMenuAction(action)
    }

    fun openDrawerForTest() {
        showDrawerOverlay()
    }

    fun dismissComposeDialogForTest() {
        dismissComposeDialog(notifyDismiss = false)
    }

    fun resetChromeStateForTest() {
        dismissComposeDialog(notifyDismiss = !composeDialogOwnedByTest)
        removeDrawerOverlay()
    }

    fun showRomLibraryDialogForTest(entryNames: Array<String>, entryIds: Array<String>) {
        showRomLibraryDialogInternal(entryNames, entryIds, ownedByTest = true)
    }

    fun showSettingsDialogForTest(
        keys: Array<String>,
        labels: Array<String>,
        choiceStrings: Array<String>,
        currentIndices: Array<String>,
    ) {
        showSettingsDialogInternal(
            keys = keys,
            labels = labels,
            choiceStrings = choiceStrings,
            currentIndices = currentIndices,
            ownedByTest = true,
        )
    }

    /**
     * Show a modal ROM library dialog.
     *
     * The first item is always "Import new ROM…"; the remaining items are the
     * provided library entries in order. When the user makes a selection this
     * method calls [onRomLibrarySelected] with the appropriate id and then
     * returns control to Rust. On cancel/dismiss it calls
     * [onRomLibrarySelected] with `null`.
     *
     * Called from the Rust JNI bridge on the Java main thread.
     */
    fun showRomLibraryDialog(entryNames: Array<String>, entryIds: Array<String>) {
        showRomLibraryDialogInternal(entryNames, entryIds, ownedByTest = false)
    }

    private fun showRomLibraryDialogInternal(
        entryNames: Array<String>,
        entryIds: Array<String>,
        ownedByTest: Boolean,
    ) {
        var resultSent = false
        showComposeDialog(
            dialogTag = ROM_LIBRARY_DIALOG_TAG,
            contentDescription = romLibraryContentDescription(entryNames.asList()),
            ownedByTest = ownedByTest,
            onDismiss = {
                if (!resultSent) {
                    onRomLibrarySelected(null)
                }
            },
        ) { dismiss ->
            NerustRomLibraryDialogCard(
                entryNames = entryNames.asList(),
                onDismissRequest = dismiss,
                onImport = {
                    resultSent = true
                    onRomLibrarySelected(IMPORT_ACTION_ID)
                    dismiss()
                },
                onSelectEntry = { index ->
                    resultSent = true
                    onRomLibrarySelected(entryIds[index])
                    dismiss()
                },
            )
        }
    }

    /**
     * Show a modal Android settings dialog.
     *
     * Presents an Android-relevant subset of settings. Each setting is backed
     * by a tab-separated list of choices; the current selection is identified
     * by index. Tapping a row in the list opens a choice picker rendered in
     * Compose. Tapping "Save" calls [onSettingsDialogResult] with a
     * comma-separated string of the final choice indices. Cancel/dismiss calls
     * it with `null`.
     *
     * Called from the Rust JNI bridge on the Java main thread.
     */
    fun showSettingsDialog(
        keys: Array<String>,
        labels: Array<String>,
        choiceStrings: Array<String>,
        currentIndices: Array<String>,
    ) {
        showSettingsDialogInternal(
            keys = keys,
            labels = labels,
            choiceStrings = choiceStrings,
            currentIndices = currentIndices,
            ownedByTest = false,
        )
    }

    private fun showSettingsDialogInternal(
        keys: Array<String>,
        labels: Array<String>,
        choiceStrings: Array<String>,
        currentIndices: Array<String>,
        ownedByTest: Boolean,
    ) {
        val settings =
            labels.indices.map { index ->
                AndroidSetting(
                    key = keys.getOrNull(index) ?: "setting_$index",
                    label = labels[index],
                    choices =
                        choiceStrings
                            .getOrNull(index)
                            ?.split('\t')
                            ?.filter(String::isNotEmpty)
                            ?.ifEmpty { listOf("?") }
                            ?: listOf("?"),
                )
            }
        val initialSelections =
            settings.mapIndexed { index, setting ->
                val maxIndex = max(0, setting.choices.lastIndex)
                (currentIndices.getOrNull(index)?.toIntOrNull() ?: 0).coerceIn(0, maxIndex)
            }
        var resultSent = false

        showComposeDialog(
            dialogTag = SETTINGS_DIALOG_TAG,
            contentDescription = settingsContentDescription(settings, initialSelections),
            ownedByTest = ownedByTest,
            onDismiss = {
                if (!resultSent) {
                    onSettingsDialogResult(null)
                }
            },
        ) { dismiss ->
            NerustSettingsDialogCard(
                settings = settings,
                initialSelections = initialSelections,
                onDismissRequest = dismiss,
                onSave = { selections ->
                    resultSent = true
                    onSettingsDialogResult(selections.joinToString(","))
                    dismiss()
                },
            )
        }
    }

    private fun scheduleChromeAttach() {
        if (!chromeAttachEnabled || isFinishing || isDestroyed) {
            return
        }
        chromeAttachAttempts = 0
        ensureChromeAttached()
        window.decorView.post(ensureChromeAttachedRunnable)
    }

    private fun ensureChromeAttached() {
        if (!chromeAttachEnabled || isFinishing || isDestroyed) {
            return
        }
        val anchor = popupAnchor() ?: run {
            retryChromeAttach()
            return
        }
        installComposeOwners(anchor)
        val controlsAttached = ensureControlsOverlayPopup(anchor)
        val drawerAttached = ensureDrawerChromePopup(anchor)
        if (!controlsAttached || !drawerAttached) {
            retryChromeAttach()
        }
    }

    private fun retryChromeAttach() {
        if (chromeAttachAttempts >= MENU_CHROME_MAX_ATTACH_ATTEMPTS) {
            Log.w(TAG, "Chrome attach skipped because Android window token was unavailable")
            return
        }
        chromeAttachAttempts += 1
        window.decorView.postDelayed(ensureChromeAttachedRunnable, MENU_CHROME_ATTACH_RETRY_DELAY_MS)
    }

    private fun removePendingChromeAttachCallbacks() {
        window.decorView.removeCallbacks(ensureChromeAttachedRunnable)
    }

    private fun ensureControlsOverlayPopup(anchor: View): Boolean {
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
        if (showPopupAtLocation(popup, anchor, Gravity.TOP or Gravity.START, 0, 0)) {
            return true
        }
        controlsOverlayPopup = null
        controlsOverlayView = null
        return false
    }

    private fun ensureDrawerChromePopup(anchor: View): Boolean {
        val existing = drawerChromePopup
        if (existing?.isShowing == true && (drawerEdgeHandleView != null || drawerShowing)) {
            return true
        }

        drawerChromePopup?.dismiss()
        val container = FrameLayout(this)
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

        drawerChromeContainer = container
        drawerEdgeHandleView = edgeHandle
        drawerChromePopup = popup
        if (showPopupAtLocation(popup, anchor, Gravity.TOP or Gravity.START, 0, 0)) {
            return true
        }
        drawerChromePopup = null
        drawerChromeContainer = null
        drawerEdgeHandleView = null
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
        DrawerEdgeSwipeHandleView(this, ::showDrawerOverlay).apply {
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

    private fun showDrawerOverlay() {
        if (popupAnchor() == null) {
            lastDrawerStateForTest = "anchor unavailable: decor=${window.decorView.debugViewState()}"
            return
        }
        val popup = drawerChromePopup ?: run {
            lastDrawerStateForTest = "drawer popup unavailable"
            return
        }
        val container = drawerChromeContainer ?: run {
            lastDrawerStateForTest = "drawer container unavailable"
            return
        }
        if (drawerShowing) {
            lastDrawerStateForTest = "already showing"
            return
        }
        lastDrawerStateForTest = "creating"

        val overlay =
            ComposeOwnerFrameLayout(this).apply {
                tag = DRAWER_OVERLAY_TAG
                setTag(R.id.nerust_drawer_content_probe, drawerContentDescription())
                layoutParams =
                    FrameLayout.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT,
                        ViewGroup.LayoutParams.MATCH_PARENT,
                    )
            }
        val drawerContent =
            ComposeView(this).apply {
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
        installComposeOwners(overlay)
        installComposeOwners(drawerContent)
        overlay.addView(drawerContent)

        container.removeAllViews()
        drawerOverlayView = overlay
        drawerComposeView = drawerContent
        drawerEdgeHandleView = null
        drawerShowing = true
        // Resize popup to full screen BEFORE adding Compose content so that
        // the ModalNavigationDrawer measures against the correct width.
        val shown = updateDrawerChromePopupForDrawer(popup)
        if (!shown) {
            clearDrawerWindowReferences()
            restoreDrawerEdgeHandleOverlay()
            lastDrawerStateForTest =
                "showInDrawerPopup=$shown, popup=${popup.debugPopupState()}, overlay=${overlay.debugViewState()}"
            return
        }
        popup.isFocusable = true
        popup.update()
        container.addView(overlay)
        lastDrawerStateForTest =
            "showInDrawerPopup=$shown, popup=${popup.debugPopupState()}, overlay=${overlay.debugViewState()}"
    }

    private fun removeDrawerOverlay(): Boolean {
        if (!drawerShowing) {
            return false
        }
        clearDrawerWindowReferences()
        return restoreDrawerEdgeHandleOverlay()
    }

    private fun showComposeDialog(
        dialogTag: String,
        contentDescription: String,
        onDismiss: () -> Unit,
        ownedByTest: Boolean = false,
        content: @Composable (dismiss: () -> Unit) -> Unit,
    ) {
        dismissComposeDialog(notifyDismiss = true)

        lateinit var dialog: Dialog
        val root =
            ComposeOwnerFrameLayout(this).apply {
                tag = dialogTag
                setTag(R.id.nerust_dialog_content_probe, contentDescription)
                layoutParams =
                    FrameLayout.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT,
                        ViewGroup.LayoutParams.MATCH_PARENT,
                    )
            }
        val composeView =
            ComposeView(this).apply {
                tag = "$dialogTag-compose"
                layoutParams =
                    FrameLayout.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT,
                        ViewGroup.LayoutParams.MATCH_PARENT,
                    )
                setViewCompositionStrategy(ViewCompositionStrategy.DisposeOnDetachedFromWindow)
            }
        dialog =
            Dialog(this).apply {
                window?.setBackgroundDrawable(ColorDrawable(Color.TRANSPARENT))
                setCancelable(true)
                setCanceledOnTouchOutside(true)
            }

        composeView.setContent {
            MaterialTheme {
                content { dialog.dismiss() }
            }
        }

        installComposeOwners(root)
        installComposeOwners(composeView)
        root.addView(composeView)
        dialog.setContentView(root)

        composeDialog = dialog
        composeDialogRootView = root
        composeDialogComposeView = composeView
        composeDialogTag = dialogTag
        composeDialogDismissCallback = onDismiss
        composeDialogOwnedByTest = ownedByTest
        dialog.setOnDismissListener {
            val dismissCallback = composeDialogDismissCallback
            clearComposeDialogWindowReferences()
            dismissCallback?.invoke()
        }
        try {
            dialog.show()
            dialog.window?.setLayout(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            )
            lastDialogStateForTest = "showing $dialogTag"
        } catch (_: WindowManager.BadTokenException) {
            dialog.setOnDismissListener(null)
            clearComposeDialogWindowReferences()
            lastDialogStateForTest = "show failed for $dialogTag: bad token"
            onDismiss()
        } catch (_: IllegalStateException) {
            dialog.setOnDismissListener(null)
            clearComposeDialogWindowReferences()
            lastDialogStateForTest = "show failed for $dialogTag: illegal state"
            onDismiss()
        }
    }

    private fun dismissComposeDialog(notifyDismiss: Boolean) {
        val dialog = composeDialog ?: return
        val dismissCallback = composeDialogDismissCallback
        dialog.setOnDismissListener(null)
        clearComposeDialogWindowReferences()
        dialog.dismiss()
        if (notifyDismiss) {
            dismissCallback?.invoke()
        }
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

    private fun updateDrawerChromePopupForDrawer(popup: PopupWindow): Boolean =
        updatePopupWindow(
            popup,
            0,
            0,
            ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.MATCH_PARENT,
        )

    private fun restoreDrawerEdgeHandleOverlay(): Boolean {
        val popup = drawerChromePopup ?: return false
        val container = drawerChromeContainer ?: return false
        container.removeAllViews()
        val edgeHandle = createDrawerEdgeHandleOverlay()
        container.addView(edgeHandle)
        drawerEdgeHandleView = edgeHandle
        popup.isFocusable = false
        popup.update()
        return updatePopupWindow(
            popup,
            0,
            0,
            dp(DRAWER_EDGE_HANDLE_WIDTH_DP),
            ViewGroup.LayoutParams.MATCH_PARENT,
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

    private fun clearComposeDialogWindowReferences() {
        composeDialog = null
        composeDialogRootView = null
        composeDialogComposeView = null
        composeDialogTag = null
        composeDialogDismissCallback = null
        composeDialogOwnedByTest = false
    }

    private fun View?.isShownInWindowForTest(): Boolean =
        this != null && visibility == View.VISIBLE && isAttachedToWindow && windowToken != null

    private fun View?.debugViewState(): String =
        if (this == null) {
            "null"
        } else {
            "visibility=$visibility, attached=$isAttachedToWindow, token=${windowToken != null}, shown=$isShown"
        }

    private fun Dialog?.debugDialogState(): String =
        if (this == null) "null" else "showing=$isShowing"

    private fun PopupWindow?.debugPopupState(): String =
        if (this == null) "null" else "showing=$isShowing"

    private fun dismissChromePopups() {
        clearDrawerWindowReferences()
        dismissComposeDialog(notifyDismiss = true)
        drawerChromePopup?.dismiss()
        drawerChromePopup = null
        drawerChromeContainer = null
        drawerEdgeHandleView = null
        controlsOverlayPopup?.dismiss()
        controlsOverlayPopup = null
        controlsOverlayView = null
    }

    private fun dp(value: Int): Int =
        (value * resources.displayMetrics.density).toInt()

    private fun drawerContentDescription(): String =
        (listOf(DRAWER_TITLE) + DRAWER_ACTIONS.map { it.label }).joinToString("\n")

    private fun romLibraryContentDescription(entryNames: List<String>): String =
        (listOf("ROM Library", "Import new ROM…") + entryNames).joinToString("\n")

    private fun settingsContentDescription(
        settings: List<AndroidSetting>,
        selections: List<Int>,
    ): String =
        buildList {
            add("Settings")
            settings.forEachIndexed { index, setting ->
                val value = setting.choices.getOrElse(selections.getOrElse(index) { 0 }) { "?" }
                add("${setting.label}: $value")
            }
        }.joinToString("\n")

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
        init {
            // Load the native library via the app classloader so the JVM can
            // resolve `external fun` declarations on this class.  NativeActivity
            // loads the library later via native dlopen which bypasses Java's
            // classloader registration; without this explicit load, standard JNI
            // name lookup fails with UnsatisfiedLinkError.
            System.loadLibrary("main")
        }

        private const val TAG = "Nerust"
        private const val DRAWER_EDGE_HANDLE_WIDTH_DP = 24
        private const val MENU_CHROME_ATTACH_RETRY_DELAY_MS = 100L
        private const val MENU_CHROME_MAX_ATTACH_ATTEMPTS = 100
        private const val ROM_PICKER_REQUEST_CODE = 0x4E45
        // Must match `android/library.rs::IMPORT_ACTION_ID`.
        private const val IMPORT_ACTION_ID = "__import__"
        @Volatile
        private var activeActivityForTest: MainActivity? = null

        fun createRomPickerIntentForTest(): Intent = createRomPickerIntent()

        fun currentActivityForTest(): MainActivity? =
            activeActivityForTest?.takeUnless { it.isDestroyed || it.isFinishing }
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
            modifier =
                Modifier
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
        modifier =
            Modifier
                .semantics { contentDescription = label }
                .padding(NavigationDrawerItemDefaults.ItemPadding),
    )
}

@Composable
private fun NerustDialogHost(content: @Composable () -> Unit) {
    Box(
        modifier =
            Modifier
                .fillMaxSize()
                .padding(24.dp),
        contentAlignment = Alignment.Center,
    ) {
        content()
    }
}

@Composable
private fun NerustDialogCard(
    title: String,
    buttons: @Composable RowScope.() -> Unit,
    body: @Composable ColumnScope.() -> Unit,
) {
    Surface(
        modifier = Modifier.widthIn(min = 280.dp, max = 420.dp),
        shape = MaterialTheme.shapes.extraLarge,
        tonalElevation = 6.dp,
    ) {
        Column(modifier = Modifier.padding(24.dp)) {
            Text(text = title, style = MaterialTheme.typography.headlineSmall)
            Spacer(modifier = Modifier.height(16.dp))
            body()
            Spacer(modifier = Modifier.height(24.dp))
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.End,
                content = buttons,
            )
        }
    }
}

@Composable
private fun NerustRomLibraryDialogCard(
    entryNames: List<String>,
    onDismissRequest: () -> Unit,
    onImport: () -> Unit,
    onSelectEntry: (Int) -> Unit,
) {
    NerustDialogHost {
        NerustDialogCard(
            title = "ROM Library",
            buttons = {
                TextButton(onClick = onDismissRequest) {
                    Text("Cancel")
                }
            },
        ) {
            LazyColumn(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .heightIn(max = 360.dp),
            ) {
                item {
                    DialogListButton(label = "Import new ROM…", onClick = onImport)
                    if (entryNames.isNotEmpty()) {
                        HorizontalDivider()
                    }
                }
                itemsIndexed(entryNames) { index, entryName ->
                    DialogListButton(label = entryName) { onSelectEntry(index) }
                    if (index < entryNames.lastIndex) {
                        HorizontalDivider()
                    }
                }
            }
        }
    }
}

@Composable
private fun NerustSettingsDialogCard(
    settings: List<AndroidSetting>,
    initialSelections: List<Int>,
    onDismissRequest: () -> Unit,
    onSave: (List<Int>) -> Unit,
) {
    val selections =
        remember(settings, initialSelections) {
            mutableStateListOf<Int>().apply {
                settings.forEachIndexed { index, setting ->
                    val maxIndex = max(0, setting.choices.lastIndex)
                    add(initialSelections.getOrElse(index) { 0 }.coerceIn(0, maxIndex))
                }
            }
        }
    var activeSettingIndex by remember { mutableStateOf<Int?>(null) }

    NerustDialogHost {
        Box(contentAlignment = Alignment.Center) {
            NerustDialogCard(
                title = "Settings",
                buttons = {
                    TextButton(onClick = onDismissRequest) {
                        Text("Cancel")
                    }
                    TextButton(onClick = { onSave(selections.toList()) }) {
                        Text("Save")
                    }
                },
            ) {
                LazyColumn(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .heightIn(max = 360.dp),
                ) {
                    itemsIndexed(settings) { index, setting ->
                        val selectedValue = setting.choices.getOrElse(selections[index]) { "?" }
                        DialogSettingButton(
                            label = setting.label,
                            value = selectedValue,
                            key = setting.key,
                        ) {
                            activeSettingIndex = index
                        }
                        if (index < settings.lastIndex) {
                            HorizontalDivider()
                        }
                    }
                }
            }

            activeSettingIndex?.let { settingIndex ->
                NerustDialogCard(
                    title = settings[settingIndex].label,
                    buttons = {
                        TextButton(onClick = { activeSettingIndex = null }) {
                            Text("Cancel")
                        }
                    },
                ) {
                    LazyColumn(
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .heightIn(max = 320.dp),
                    ) {
                        itemsIndexed(settings[settingIndex].choices) { choiceIndex, choiceLabel ->
                            DialogChoiceButton(
                                label = choiceLabel,
                                selected = selections[settingIndex] == choiceIndex,
                            ) {
                                selections[settingIndex] = choiceIndex
                                activeSettingIndex = null
                            }
                            if (choiceIndex < settings[settingIndex].choices.lastIndex) {
                                HorizontalDivider()
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun DialogListButton(label: String, onClick: () -> Unit) {
    TextButton(
        onClick = onClick,
        modifier =
            Modifier
                .fillMaxWidth()
                .semantics { contentDescription = label },
    ) {
        Text(text = label, modifier = Modifier.fillMaxWidth())
    }
}

@Composable
private fun DialogSettingButton(
    label: String,
    value: String,
    key: String,
    onClick: () -> Unit,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onClick)
                .padding(vertical = 16.dp)
                .semantics { contentDescription = "$key: $label: $value" },
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(text = label, modifier = Modifier.weight(1f))
        Text(text = value, style = MaterialTheme.typography.bodyMedium)
    }
}

@Composable
private fun DialogChoiceButton(label: String, selected: Boolean, onClick: () -> Unit) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onClick)
                .padding(vertical = 12.dp)
                .semantics { contentDescription = label },
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(text = label, modifier = Modifier.weight(1f))
        RadioButton(selected = selected, onClick = null)
    }
}

private class ControlsOverlayView(context: Context) : View(context) {
    private val fillPaint =
        Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = Color.argb(48, 255, 255, 255)
            style = Paint.Style.FILL
        }
    private val strokePaint =
        Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = Color.argb(160, 255, 255, 255)
            strokeWidth = 2f
            style = Paint.Style.STROKE
        }
    private val textPaint =
        Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = Color.argb(220, 255, 255, 255)
            textAlign = Paint.Align.CENTER
            typeface = Typeface.create(Typeface.DEFAULT, Typeface.BOLD)
        }
    private val arrowPaint =
        Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = Color.argb(220, 255, 255, 255)
            style = Paint.Style.FILL
        }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        val viewWidth = width.toFloat()
        val viewHeight = height.toFloat()
        if (viewWidth <= 0f || viewHeight <= 0f) {
            return
        }

        portraitControlsLayout(viewWidth, viewHeight).forEach { zone ->
            drawZone(canvas, zone.x, zone.y, zone.width, zone.height, zone.label)
        }
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

        when (label) {
            "UP" -> drawArrow(canvas, rect, Direction.UP)
            "DOWN" -> drawArrow(canvas, rect, Direction.DOWN)
            "LEFT" -> drawArrow(canvas, rect, Direction.LEFT)
            "RIGHT" -> drawArrow(canvas, rect, Direction.RIGHT)
            else -> {
                textPaint.textSize = max(12f, min(height * 0.42f, width * 0.28f))
                val centerY = rect.centerY() - (textPaint.descent() + textPaint.ascent()) / 2f
                canvas.drawText(label, rect.centerX(), centerY, textPaint)
            }
        }
    }

    private enum class Direction { UP, DOWN, LEFT, RIGHT }

    private fun drawArrow(canvas: Canvas, rect: RectF, direction: Direction) {
        val size = min(rect.width(), rect.height()) * 0.45f
        val cx = rect.centerX()
        val cy = rect.centerY()
        val path = Path()
        when (direction) {
            Direction.UP -> {
                path.moveTo(cx, cy - size * 0.5f)
                path.lineTo(cx - size * 0.5f, cy + size * 0.35f)
                path.lineTo(cx + size * 0.5f, cy + size * 0.35f)
            }
            Direction.DOWN -> {
                path.moveTo(cx, cy + size * 0.5f)
                path.lineTo(cx - size * 0.5f, cy - size * 0.35f)
                path.lineTo(cx + size * 0.5f, cy - size * 0.35f)
            }
            Direction.LEFT -> {
                path.moveTo(cx - size * 0.5f, cy)
                path.lineTo(cx + size * 0.35f, cy - size * 0.5f)
                path.lineTo(cx + size * 0.35f, cy + size * 0.5f)
            }
            Direction.RIGHT -> {
                path.moveTo(cx + size * 0.5f, cy)
                path.lineTo(cx - size * 0.35f, cy - size * 0.5f)
                path.lineTo(cx - size * 0.35f, cy + size * 0.5f)
            }
        }
        path.close()
        canvas.drawPath(path, arrowPaint)
    }
}

private class DrawerEdgeSwipeHandleView(
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

            else -> super.onTouchEvent(event)
        }

    override fun performClick(): Boolean = super.performClick()
}

internal fun portraitControlsLayout(width: Float, height: Float): List<OverlayZoneSpec> {
    val controlTop = height * 0.54f
    val controlHeight = height - controlTop
    val dpadLeft = width * 0.08f
    val dpadSize = width * 0.28f
    val dpadCenterX = dpadLeft + dpadSize * 0.5f
    val dpadCenterY = controlTop + controlHeight * 0.58f
    val dpadArm = dpadSize * 0.28f
    val dpadExtent = dpadSize * 0.38f
    val actionSize = width * 0.14f
    val actionGap = width * 0.04f
    val actionLeft = width * 0.64f
    val actionTop = dpadCenterY - actionSize * 0.5f
    val centerButtonWidth = width * 0.10f
    val centerButtonHeight = height * 0.038f
    val centerGap = width * 0.03f
    val centerRowWidth = centerButtonWidth * 2f + centerGap
    val centerLeftBound = dpadLeft + dpadSize + width * 0.03f
    val centerRightBound = actionLeft - width * 0.03f
    val centeredStart = (centerLeftBound + centerRightBound - centerRowWidth) * 0.5f
    val centerStartX =
        centeredStart.coerceIn(
            centerLeftBound,
            max(centerLeftBound, centerRightBound - centerRowWidth),
        )
    val centerTop = controlTop + controlHeight * 0.16f

    return listOf(
        OverlayZoneSpec(
            x = dpadCenterX - dpadArm * 0.5f,
            y = dpadCenterY - dpadExtent,
            width = dpadArm,
            height = dpadExtent - dpadArm * 0.5f,
            label = "UP",
        ),
        OverlayZoneSpec(
            x = dpadCenterX - dpadArm * 0.5f,
            y = dpadCenterY + dpadArm * 0.5f,
            width = dpadArm,
            height = dpadExtent - dpadArm * 0.5f,
            label = "DOWN",
        ),
        OverlayZoneSpec(
            x = dpadCenterX - dpadExtent,
            y = dpadCenterY - dpadArm * 0.5f,
            width = dpadExtent - dpadArm * 0.5f,
            height = dpadArm,
            label = "LEFT",
        ),
        OverlayZoneSpec(
            x = dpadCenterX + dpadArm * 0.5f,
            y = dpadCenterY - dpadArm * 0.5f,
            width = dpadExtent - dpadArm * 0.5f,
            height = dpadArm,
            label = "RIGHT",
        ),
        OverlayZoneSpec(
            x = actionLeft,
            y = actionTop,
            width = actionSize,
            height = actionSize,
            label = "B",
        ),
        OverlayZoneSpec(
            x = actionLeft + actionSize + actionGap,
            y = actionTop,
            width = actionSize,
            height = actionSize,
            label = "A",
        ),
        OverlayZoneSpec(
            x = centerStartX,
            y = centerTop,
            width = centerButtonWidth,
            height = centerButtonHeight,
            label = "SELECT",
        ),
        OverlayZoneSpec(
            x = centerStartX + centerButtonWidth + centerGap,
            y = centerTop,
            width = centerButtonWidth,
            height = centerButtonHeight,
            label = "START",
        ),
    )
}
