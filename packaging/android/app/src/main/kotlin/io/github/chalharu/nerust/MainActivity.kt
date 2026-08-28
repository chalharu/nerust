package io.github.chalharu.nerust

import android.annotation.SuppressLint
import android.app.Dialog
import android.app.NativeActivity
import android.content.pm.ActivityInfo
import android.content.Context
import android.content.Intent
import android.media.AudioManager
import android.net.Uri
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import android.graphics.Typeface
import android.graphics.drawable.ColorDrawable
import android.hardware.input.InputManager
import android.os.Build
import android.os.Bundle
import android.os.SystemClock
import android.util.Log
import android.util.Base64
import android.view.Gravity
import android.view.HapticFeedbackConstants
import android.view.InputDevice
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.window.OnBackInvokedDispatcher
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
import androidx.documentfile.provider.DocumentFile
import androidx.savedstate.SavedStateRegistry
import androidx.savedstate.SavedStateRegistryController
import androidx.savedstate.SavedStateRegistryOwner
import androidx.savedstate.setViewTreeSavedStateRegistryOwner
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min
import kotlinx.coroutines.launch
import org.json.JSONObject

private const val CONTROLS_OVERLAY_TAG = "nerust-controls-overlay"
private const val DRAWER_COMPOSE_TAG = "nerust-drawer-compose"
private const val DRAWER_EDGE_HANDLE_TAG = "nerust-drawer-edge-handle"
private const val DRAWER_OVERLAY_TAG = "nerust-drawer-overlay"
private const val MENU_ACTION_EXIT = "exit"
private const val MENU_ACTION_LOAD_STATE = "load_state"
private const val MENU_ACTION_OPEN_ROM = "open_rom"
private const val MENU_ACTION_OPEN_SETTINGS = "open_settings"
private const val MENU_ACTION_RESET = "reset"
private const val MENU_ACTION_SAVE_STATE = "save_state"
private const val MENU_ACTION_TOGGLE_PAUSE = "toggle_pause"
private const val MENU_ACTION_UNLOAD = "unload"
private const val MENU_BUTTON_TAG = "nerust-menu-button"
private const val SETTINGS_DIALOG_TAG = "nerust-settings-dialog"
private const val DRAWER_TITLE = "Nerust"
private const val DIALOG_PRESENTATION_FULL_SCREEN = "full_screen"

internal fun screenOrientationRequest(value: Int): Int? =
    when (value) {
        0 -> ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED
        1 -> ActivityInfo.SCREEN_ORIENTATION_PORTRAIT
        2 -> ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE
        else -> null
    }

private data class DrawerAction(val label: String, val action: String)

private data class AndroidSetting(
    val key: String,
    val section: String,
    val label: String,
    val choices: List<String>,
)

private data class AndroidSettingsSection(
    val id: String,
    val label: String,
    val settingIndices: List<Int>,
)

internal data class OverlayZoneSpec(
    val x: Float,
    val y: Float,
    val width: Float,
    val height: Float,
    val label: String,
)

private val DRAWER_ACTIONS = listOf(
    DrawerAction("Open ROM", MENU_ACTION_OPEN_ROM),
    DrawerAction("Settings", MENU_ACTION_OPEN_SETTINGS),
    DrawerAction("Pause / Resume", MENU_ACTION_TOGGLE_PAUSE),
    DrawerAction("Save State", MENU_ACTION_SAVE_STATE),
    DrawerAction("Load State", MENU_ACTION_LOAD_STATE),
    DrawerAction("Reset", MENU_ACTION_RESET),
    DrawerAction("Unload ROM", MENU_ACTION_UNLOAD),
    DrawerAction("Exit", MENU_ACTION_EXIT),
)

private fun createRomPickerIntent(): Intent =
    Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
        addCategory(Intent.CATEGORY_OPENABLE)
        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION)
        type = "*/*"
    }

private fun createDirectoryPickerIntent(): Intent =
    Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).apply {
        addFlags(
            Intent.FLAG_GRANT_READ_URI_PERMISSION or
                Intent.FLAG_GRANT_WRITE_URI_PERMISSION or
                Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION,
        )
    }

class MainActivity : NativeActivity(), LifecycleOwner, SavedStateRegistryOwner, ViewModelStoreOwner,
    InputManager.InputDeviceListener {
    private val lifecycleRegistry = LifecycleRegistry(this)
    private val registryController = SavedStateRegistryController.create(this)
    private val store = ViewModelStore()
    private val controllerPressed = mutableMapOf<Int, MutableSet<String>>()
    private val ensureChromeAttachedRunnable = Runnable { ensureChromeAttached() }
    private val restoreControllerOverlayRunnable = Runnable {
        controllerOverlayHiddenUntil = 0L
        scheduleChromeAttach()
    }
    private var chromeAttachAttempts = 0
    private var chromeAttachEnabled = false
    private var controlsOverlayPopup: PopupWindow? = null
    private var controlsOverlayView: View? = null
    private var controllerOverlayHiddenUntil = 0L
    private var controlsVisibility = "auto"
    private var controlsOpacityPercent = 65
    private var controlsScalePercent = 100
    private var controlsVerticalOffsetPercent = 0
    private var controlsHaptics = true
    private var drawerChromePopup: PopupWindow? = null
    private var drawerChromeContainer: FrameLayout? = null
    private var drawerEdgeHandleView: View? = null
    private var drawerShowing = false
    private var drawerFullScreenPopup: PopupWindow? = null
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
    @Volatile private var lastLoadedSystemForTest: String? = null

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
        Log.i(TAG, "onCreate: savedInstanceState=${savedInstanceState != null}")
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            onBackInvokedDispatcher.registerOnBackInvokedCallback(
                OnBackInvokedDispatcher.PRIORITY_DEFAULT,
                ::handleBackNavigation,
            )
        }
        volumeControlStream = AudioManager.STREAM_MUSIC
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_CREATE)
        scheduleChromeAttach()
    }

    override fun onStart() {
        super.onStart()
        Log.i(TAG, "onStart")
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_START)
    }

    override fun onResume() {
        super.onResume()
        Log.i(TAG, "onResume")
        activeActivityForTest = this
        chromeAttachEnabled = true
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_RESUME)
        (getSystemService(INPUT_SERVICE) as InputManager).registerInputDeviceListener(this, null)
        scheduleChromeAttach()
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        Log.i(TAG, "onWindowFocusChanged: hasFocus=$hasFocus")
        if (hasFocus) {
            scheduleChromeAttach()
        }
    }

    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        val source = event.source
        val isController =
            source and InputDevice.SOURCE_GAMEPAD == InputDevice.SOURCE_GAMEPAD ||
                source and InputDevice.SOURCE_DPAD == InputDevice.SOURCE_DPAD ||
                source and InputDevice.SOURCE_JOYSTICK == InputDevice.SOURCE_JOYSTICK
        if (!isController) {
            return super.dispatchKeyEvent(event)
        }
        val key =
            when (event.keyCode) {
                KeyEvent.KEYCODE_BUTTON_A -> "button1"
                KeyEvent.KEYCODE_BUTTON_B -> "button2"
                KeyEvent.KEYCODE_BUTTON_START -> "start"
                KeyEvent.KEYCODE_BUTTON_SELECT -> "select"
                KeyEvent.KEYCODE_DPAD_UP -> "up"
                KeyEvent.KEYCODE_DPAD_DOWN -> "down"
                KeyEvent.KEYCODE_DPAD_LEFT -> "left"
                KeyEvent.KEYCODE_DPAD_RIGHT -> "right"
                else -> return super.dispatchKeyEvent(event)
            }
        val pressed = event.action == KeyEvent.ACTION_DOWN
        if (pressed && event.repeatCount > 0) {
            return true
        }
        if (event.action != KeyEvent.ACTION_DOWN && event.action != KeyEvent.ACTION_UP) {
            return true
        }
        updateControllerInput(event.deviceId, key, pressed)
        return true
    }

    override fun onGenericMotionEvent(event: MotionEvent): Boolean {
        if (event.source and InputDevice.SOURCE_JOYSTICK != InputDevice.SOURCE_JOYSTICK) {
            return super.onGenericMotionEvent(event)
        }
        val horizontal = event.getAxisValue(MotionEvent.AXIS_HAT_X)
        val vertical = event.getAxisValue(MotionEvent.AXIS_HAT_Y)
        updateControllerInput(event.deviceId, "left", horizontal < -CONTROLLER_AXIS_THRESHOLD)
        updateControllerInput(event.deviceId, "right", horizontal > CONTROLLER_AXIS_THRESHOLD)
        updateControllerInput(event.deviceId, "up", vertical < -CONTROLLER_AXIS_THRESHOLD)
        updateControllerInput(event.deviceId, "down", vertical > CONTROLLER_AXIS_THRESHOLD)
        return true
    }

    private fun updateControllerInput(deviceId: Int, key: String, pressed: Boolean) {
        val keys = controllerPressed.getOrPut(deviceId) { mutableSetOf() }
        val changed = if (pressed) keys.add(key) else keys.remove(key)
        if (!changed) return
        if (pressed) hideControlsForControllerInput()
        onMenuAction("controller:$deviceId:$key:${if (pressed) 1 else 0}")
        if (keys.isEmpty()) controllerPressed.remove(deviceId)
    }

    private fun releaseController(deviceId: Int) {
        controllerPressed.remove(deviceId)?.toList()?.forEach { key ->
            onMenuAction("controller:$deviceId:$key:0")
        }
    }

    private fun releaseAllControllers() {
        controllerPressed.keys.toList().forEach(::releaseController)
    }

    override fun onInputDeviceAdded(deviceId: Int) = Unit

    override fun onInputDeviceChanged(deviceId: Int) = Unit

    override fun onInputDeviceRemoved(deviceId: Int) {
        releaseController(deviceId)
    }

    override fun onPause() {
        if (activeActivityForTest === this) {
            activeActivityForTest = null
        }
        Log.i(TAG, "onPause")
        chromeAttachEnabled = false
        releaseAllControllers()
        (getSystemService(INPUT_SERVICE) as InputManager).unregisterInputDeviceListener(this)
        removePendingChromeAttachCallbacks()
        window.decorView.removeCallbacks(restoreControllerOverlayRunnable)
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_PAUSE)
        super.onPause()
    }

    override fun onStop() {
        Log.i(TAG, "onStop")
        removePendingChromeAttachCallbacks()
        dismissChromePopups()
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_STOP)
        super.onStop()
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        Log.i(TAG, "onSaveInstanceState")
        registryController.performSave(outState)
    }

    override fun onDestroy() {
        if (activeActivityForTest === this) {
            activeActivityForTest = null
        }
        Log.i(
            TAG,
            "onDestroy: isFinishing=$isFinishing isDestroyed=$isDestroyed " +
                "lastDrawerState=$lastDrawerStateForTest lastDialogState=$lastDialogStateForTest",
        )
        chromeAttachEnabled = false
        removePendingChromeAttachCallbacks()
        dismissChromePopups()
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_DESTROY)
        store.clear()
        onActivityDestroyed()
        super.onDestroy()
    }

    private fun handleBackNavigation() {
        if (!removeDrawerOverlay()) {
            finish()
        }
    }

    @Deprecated("Used on Android 12 and earlier; API 33+ uses OnBackInvokedDispatcher")
    @SuppressLint("GestureBackNavigation")
    @Suppress("DEPRECATION")
    override fun onBackPressed() {
        handleBackNavigation()
    }

    @Suppress("DEPRECATION")
    fun startRomPicker() {
        Log.i(TAG, "startRomPicker")
        startActivityForResult(createRomPickerIntent(), ROM_PICKER_REQUEST_CODE)
    }

    @Suppress("DEPRECATION")
    fun startDirectoryPicker() {
        startActivityForResult(createDirectoryPickerIntent(), DIRECTORY_PICKER_REQUEST_CODE)
    }

    fun configureControlsOverlay(
        visibility: String,
        opacityPercent: Int,
        scalePercent: Int,
        verticalOffsetPercent: Int,
        haptics: Boolean,
    ) {
        controlsVisibility = visibility
        controlsOpacityPercent = opacityPercent.coerceIn(0, 100)
        controlsScalePercent = scalePercent.coerceIn(50, 150)
        controlsVerticalOffsetPercent = verticalOffsetPercent.coerceIn(-30, 30)
        controlsHaptics = haptics
        controlsOverlayPopup?.dismiss()
        controlsOverlayPopup = null
        controlsOverlayView = null
        scheduleChromeAttach()
    }

    fun performControlHaptic() {
        if (controlsHaptics) {
            window.decorView.performHapticFeedback(HapticFeedbackConstants.KEYBOARD_TAP)
        }
    }

    fun readSafFile(treeUri: String, relativePath: String): String? {
        val file = resolveSafDocument(treeUri, relativePath, create = false) ?: return null
        return contentResolver.openInputStream(file.uri)?.use { input ->
            Base64.encodeToString(input.readBytes(), Base64.NO_WRAP)
        }
    }

    fun writeSafFile(treeUri: String, relativePath: String, encoded: String): String {
        val file = resolveSafDocument(treeUri, relativePath, create = true) ?: return "error"
        val bytes = Base64.decode(encoded, Base64.NO_WRAP)
        contentResolver.openOutputStream(file.uri, "rwt")?.use { it.write(bytes) } ?: return "error"
        return "ok"
    }

    fun deleteSafFile(treeUri: String, relativePath: String): String {
        val file = resolveSafDocument(treeUri, relativePath, create = false) ?: return "missing"
        return if (file.delete()) "ok" else "error"
    }

    fun listSafFiles(treeUri: String, relativePath: String): String {
        val directory = resolveSafDirectory(treeUri, relativePath, create = false)
            ?: return "[]"
        return org.json.JSONArray(directory.listFiles().mapNotNull(DocumentFile::getName)).toString()
    }

    private fun resolveSafDocument(treeUri: String, relativePath: String, create: Boolean): DocumentFile? {
        val segments = relativePath.split('/').filter(String::isNotBlank)
        val fileName = segments.lastOrNull() ?: return null
        val directory = resolveSafDirectory(treeUri, segments.dropLast(1).joinToString("/"), create)
            ?: return null
        return directory.findFile(fileName)
            ?: if (create) directory.createFile("application/octet-stream", fileName) else null
    }

    private fun resolveSafDirectory(treeUri: String, relativePath: String, create: Boolean): DocumentFile? {
        var current = DocumentFile.fromTreeUri(this, Uri.parse(treeUri)) ?: return null
        for (segment in relativePath.split('/').filter(String::isNotBlank)) {
            current = resolveSafChildDirectory(current, segment, create) ?: return null
        }
        return current
    }

    private fun resolveSafChildDirectory(
        parent: DocumentFile,
        name: String,
        create: Boolean,
    ): DocumentFile? =
        parent.findFile(name)?.takeIf(DocumentFile::isDirectory)
            ?: if (create) parent.createDirectory(name) else null

    @Suppress("DEPRECATION")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != ROM_PICKER_REQUEST_CODE && requestCode != DIRECTORY_PICKER_REQUEST_CODE) {
            return
        }
        Log.i(TAG, "onActivityResult: requestCode=$requestCode resultCode=$resultCode uri=${data?.data}")

        val uri = if (resultCode == RESULT_OK) data?.data else null
        if (uri != null) {
            val requestedFlags =
                if (requestCode == DIRECTORY_PICKER_REQUEST_CODE) {
                    Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                } else {
                    Intent.FLAG_GRANT_READ_URI_PERMISSION
                }
            val takeFlags =
                data?.flags
                    ?.and(requestedFlags)
                    ?.takeIf { it != 0 }
                    ?: requestedFlags
            try {
                contentResolver.takePersistableUriPermission(uri, takeFlags)
            } catch (error: SecurityException) {
                Log.w(TAG, "Failed to keep Android ROM URI permission", error)
            }
        }

        if (requestCode == DIRECTORY_PICKER_REQUEST_CODE) {
            onDirectoryPickerResult(uri?.toString())
        } else {
            onFilePickerResult(uri?.toString())
        }
    }

    fun isChromeViewShowingForTest(tag: String): Boolean =
        when (tag) {
            CONTROLS_OVERLAY_TAG -> controlsOverlayPopup?.isShowing == true &&
                controlsOverlayView.isShownInWindowForTest()
            DRAWER_COMPOSE_TAG -> drawerShowing && drawerFullScreenPopup?.isShowing == true &&
                drawerComposeView.isShownInWindowForTest()
            DRAWER_EDGE_HANDLE_TAG -> !drawerShowing && drawerChromePopup?.isShowing == true &&
                drawerEdgeHandleView.isShownInWindowForTest()
            DRAWER_OVERLAY_TAG -> drawerShowing && drawerFullScreenPopup?.isShowing == true &&
                drawerOverlayView.isShownInWindowForTest()
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
            "drawerShowing=$drawerShowing, drawerFullScreenPopup=${drawerFullScreenPopup.debugPopupState()}, " +
            "drawerOverlay=${drawerOverlayView.debugViewState()}, " +
            "drawerCompose=${drawerComposeView.debugViewState()}, dialogTag=$composeDialogTag, " +
            "dialog=${composeDialog.debugDialogState()}, dialogRoot=${composeDialogRootView.debugViewState()}, " +
            "dialogCompose=${composeDialogComposeView.debugViewState()}, lastDrawer=$lastDrawerStateForTest, " +
            "lastDialog=$lastDialogStateForTest"

    fun dispatchMenuActionForTest(action: String) {
        dispatchMenuAction(action)
    }

    fun loadRomUriForTest(uri: String) {
        lastLoadedSystemForTest = null
        Log.i(TAG, "loadRomUriForTest: submitting URI $uri")
        onFilePickerResult(uri)
    }

    fun lastLoadedSystemForTest(): String? = lastLoadedSystemForTest

    fun notifyRomLoaded(systemId: String) {
        lastLoadedSystemForTest = systemId
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

    fun showSettingsDialogForTest(
        keys: Array<String>,
        labels: Array<String>,
        choiceStrings: Array<String>,
        currentIndices: Array<String>,
        sections: Array<String> = emptyArray(),
    ) {
        showSettingsDialogInternal(
            keys = keys,
            labels = labels,
            choiceStrings = choiceStrings,
            currentIndices = currentIndices,
            sections = sections,
            requestId = null,
            ownedByTest = true,
        )
    }


    /**
     * Show a modal Android settings dialog.
     *
    * Presents the versioned settings JSON supplied by Rust. Tapping a row opens
    * a Compose choice picker. Save and dismiss return request-scoped JSON with
    * values keyed by stable setting IDs.
     *
     * Called from the Rust JNI bridge on the Java main thread.
     */
    fun showSettingsDialog(payload: String) {
        val document = JSONObject(payload)
        require(document.getInt("schemaVersion") == SETTINGS_SCHEMA_VERSION) {
            "Unsupported settings schema"
        }
        val requestId = document.getLong("requestId")
        val sectionDocuments = document.getJSONArray("sections")
        val fieldDocuments = mutableListOf<Pair<String, JSONObject>>()
        for (sectionIndex in 0 until sectionDocuments.length()) {
            val section = sectionDocuments.getJSONObject(sectionIndex)
            val sectionId = section.getString("id")
            val fields = section.getJSONArray("fields")
            for (fieldIndex in 0 until fields.length()) {
                fieldDocuments += sectionId to fields.getJSONObject(fieldIndex)
            }
        }
        val keys = Array(fieldDocuments.size) { fieldDocuments[it].second.getString("key") }
        val labels = Array(fieldDocuments.size) { fieldDocuments[it].second.getString("label") }
        val sections = Array(fieldDocuments.size) { fieldDocuments[it].first }
        val choiceStrings =
            Array(fieldDocuments.size) { index ->
                val options = fieldDocuments[index].second.getJSONArray("options")
                (0 until options.length()).joinToString("\t") { options.getString(it) }
            }
        val currentIndices =
            Array(fieldDocuments.size) { fieldDocuments[it].second.getInt("value").toString() }
        showSettingsDialogInternal(
            keys = keys,
            labels = labels,
            choiceStrings = choiceStrings,
            currentIndices = currentIndices,
            sections = sections,
            requestId = requestId,
            ownedByTest = false,
        )
    }

    fun applyScreenOrientation(value: Int) {
        val orientation = screenOrientationRequest(value)
        if (orientation == null) {
            Log.w(TAG, "Ignoring invalid screen orientation value: $value")
            return
        }
        if (requestedOrientation != orientation) {
            requestedOrientation = orientation
        }
    }

    private fun showSettingsDialogInternal(
        keys: Array<String>,
        labels: Array<String>,
        choiceStrings: Array<String>,
        currentIndices: Array<String>,
        sections: Array<String> = emptyArray(),
        requestId: Long?,
        ownedByTest: Boolean,
    ) {
        val settings =
            labels.indices.map { index ->
                AndroidSetting(
                    key = keys.getOrNull(index) ?: "setting_$index",
                    section = sections.getOrNull(index) ?: "general",
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
            presentation = DIALOG_PRESENTATION_FULL_SCREEN,
            ownedByTest = ownedByTest,
            onDismiss = {
                if (!resultSent && requestId != null) {
                    onSettingsDialogResult(
                        JSONObject()
                            .put("schemaVersion", SETTINGS_SCHEMA_VERSION)
                            .put("requestId", requestId)
                            .put("dismissed", true)
                            .put("values", JSONObject())
                            .toString(),
                    )
                }
            },
        ) { dismiss ->
            NerustSettingsDialogCard(
                settings = settings,
                initialSelections = initialSelections,
                onDismissRequest = dismiss,
                onSave = { selections ->
                    resultSent = true
                    if (requestId != null) {
                        val values = JSONObject()
                        keys.forEachIndexed { index, key -> values.put(key, selections[index]) }
                        val result =
                            JSONObject()
                                .put("schemaVersion", SETTINGS_SCHEMA_VERSION)
                                .put("requestId", requestId)
                                .put("values", values)
                        onSettingsDialogResult(result.toString())
                    }
                    dismiss()
                },
            )
        }
        composeDialogRootView?.setTag(
            R.id.nerust_settings_hierarchy_probe,
            buildSettingsSections(settings).joinToString("\n") { section ->
                "${section.label}: ${settingsCountLabel(section.settingIndices.size)}"
            },
        )
    }

    private fun scheduleChromeAttach() {
        if (!chromeAttachEnabled || isFinishing || isDestroyed) {
            Log.i(
                TAG,
                "scheduleChromeAttach: skipped (enabled=$chromeAttachEnabled finishing=$isFinishing destroyed=$isDestroyed)",
            )
            return
        }
        chromeAttachAttempts = 0
        Log.i(TAG, "scheduleChromeAttach: decor=${window.decorView.debugViewState()}")
        ensureChromeAttached()
        window.decorView.post(ensureChromeAttachedRunnable)
    }

    private fun ensureChromeAttached() {
        if (!chromeAttachEnabled || isFinishing || isDestroyed) {
            Log.i(
                TAG,
                "ensureChromeAttached: skipped (enabled=$chromeAttachEnabled finishing=$isFinishing destroyed=$isDestroyed)",
            )
            return
        }
        val anchor = popupAnchor() ?: run {
            Log.i(TAG, "ensureChromeAttached: anchor unavailable decor=${window.decorView.debugViewState()}")
            retryChromeAttach()
            return
        }
        installComposeOwners(anchor)
        val controlsAttached = ensureControlsOverlayPopup(anchor)
        val drawerAttached = ensureDrawerChromePopup(anchor)
        Log.i(
            TAG,
            "ensureChromeAttached: controlsAttached=$controlsAttached drawerAttached=$drawerAttached " +
                "anchor=${anchor.debugViewState()}",
        )
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
        Log.i(
            TAG,
            "retryChromeAttach: scheduling attempt $chromeAttachAttempts/$MENU_CHROME_MAX_ATTACH_ATTEMPTS",
        )
        window.decorView.postDelayed(ensureChromeAttachedRunnable, MENU_CHROME_ATTACH_RETRY_DELAY_MS)
    }

    private fun removePendingChromeAttachCallbacks() {
        window.decorView.removeCallbacks(ensureChromeAttachedRunnable)
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
        window.decorView.removeCallbacks(restoreControllerOverlayRunnable)
        window.decorView.postDelayed(restoreControllerOverlayRunnable, CONTROLLER_OVERLAY_HIDE_MS)
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
        if (showPopupAtLocation("drawer-edge-handle", popup, anchor, Gravity.TOP or Gravity.START, 0, 0)) {
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
        ControlsOverlayView(
            this,
            controlsOpacityPercent,
            controlsScalePercent,
            controlsVerticalOffsetPercent,
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
        val anchor = popupAnchor() ?: run {
            lastDrawerStateForTest = "anchor unavailable: decor=${window.decorView.debugViewState()}"
            Log.w(TAG, "showDrawerOverlay: anchor unavailable decor=${window.decorView.debugViewState()}")
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

    private fun removeDrawerOverlay(): Boolean {
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

    private fun showComposeDialog(
        dialogTag: String,
        contentDescription: String,
        presentation: String,
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
                setTag(R.id.nerust_dialog_presentation_probe, presentation)
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
            Log.i(TAG, "showComposeDialog: showing $dialogTag")
            dialog.show()
            dialog.window?.setLayout(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            )
            lastDialogStateForTest = "showing $dialogTag"
            Log.i(TAG, "showComposeDialog: showing $dialogTag succeeded")
        } catch (error: WindowManager.BadTokenException) {
            dialog.setOnDismissListener(null)
            clearComposeDialogWindowReferences()
            lastDialogStateForTest = "show failed for $dialogTag: bad token"
            Log.w(TAG, "showComposeDialog: failed for $dialogTag with bad token", error)
            onDismiss()
        } catch (error: IllegalStateException) {
            dialog.setOnDismissListener(null)
            clearComposeDialogWindowReferences()
            lastDialogStateForTest = "show failed for $dialogTag: illegal state"
            Log.w(TAG, "showComposeDialog: failed for $dialogTag with illegal state", error)
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
        popupName: String,
        popup: PopupWindow,
        anchor: View,
        gravity: Int,
        x: Int,
        y: Int,
    ): Boolean =
        try {
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

    private fun updatePopupWindow(popup: PopupWindow, x: Int, y: Int, width: Int, height: Int): Boolean =
        try {
            popup.width = width
            popup.height = height
            popup.update(x, y, width, height)
            Log.i(TAG, "updatePopupWindow: updated popup to ($x,$y ${width}x$height)")
            true
        } catch (error: WindowManager.BadTokenException) {
            Log.w(TAG, "updatePopupWindow: failed with bad token", error)
            false
        } catch (error: IllegalStateException) {
            Log.w(TAG, "updatePopupWindow: failed with illegal state", error)
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
        drawerFullScreenPopup?.setOnDismissListener(null)
        drawerFullScreenPopup?.dismiss()
        drawerFullScreenPopup = null
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

    private external fun onDirectoryPickerResult(uri: String?)

    private external fun onSettingsDialogResult(result: String?)

    private external fun onActivityDestroyed()

    companion object {
        private const val TAG = "Nerust"
        private const val SETTINGS_SCHEMA_VERSION = 1

        init {
            // Load the native library via the app classloader so the JVM can
            // resolve `external fun` declarations on this class.  NativeActivity
            // loads the library later via native dlopen which bypasses Java's
            // classloader registration; without this explicit load, standard JNI
            // name lookup fails with UnsatisfiedLinkError.
            Log.i(TAG, "MainActivity companion: loading native library main")
            System.loadLibrary("main")
            Log.i(TAG, "MainActivity companion: loaded native library main")
        }

        private const val DRAWER_EDGE_HANDLE_WIDTH_DP = 24
        private const val MENU_CHROME_ATTACH_RETRY_DELAY_MS = 100L
        private const val MENU_CHROME_MAX_ATTACH_ATTEMPTS = 100
        private const val CONTROLLER_OVERLAY_HIDE_MS = 5_000L
        private const val CONTROLLER_AXIS_THRESHOLD = 0.5f
        private const val ROM_PICKER_REQUEST_CODE = 0x4E45
        private const val DIRECTORY_PICKER_REQUEST_CODE = 0x4E46
        @Volatile
        private var activeActivityForTest: MainActivity? = null

        fun createRomPickerIntentForTest(): Intent = createRomPickerIntent()

        fun createDirectoryPickerIntentForTest(): Intent = createDirectoryPickerIntent()

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
private fun NerustFullScreenDialogHost(content: @Composable ColumnScope.() -> Unit) {
    Surface(modifier = Modifier.fillMaxSize()) {
        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(WindowInsets.safeDrawing.asPaddingValues())
                    .padding(horizontal = 24.dp, vertical = 16.dp),
            content = content,
        )
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
private fun NerustSettingsDialogCard(
    settings: List<AndroidSetting>,
    initialSelections: List<Int>,
    onDismissRequest: () -> Unit,
    onSave: (List<Int>) -> Unit,
) {
    val selections = rememberSettingsSelections(settings, initialSelections)
    val sections = remember(settings) { buildSettingsSections(settings) }
    var activeSectionId by remember { mutableStateOf<String?>(null) }
    var activeSettingIndex by remember { mutableStateOf<Int?>(null) }

    Surface(modifier = Modifier.fillMaxSize()) {
        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(WindowInsets.safeDrawing.asPaddingValues()),
        ) {
            val activeSection = sections.find { it.id == activeSectionId }
            SettingsNavigationHeader(
                title = activeSettingIndex?.let { settings[it].label }
                    ?: activeSection?.label
                    ?: "Settings",
                subtitle = when {
                    activeSettingIndex != null -> "Choose a value"
                    activeSection != null -> settingsCountLabel(activeSection.settingIndices.size)
                    else -> "Choose a category"
                },
                canNavigateBack = activeSection != null || activeSettingIndex != null,
                onNavigateBack = {
                    if (activeSettingIndex != null) {
                        activeSettingIndex = null
                    } else {
                        activeSectionId = null
                    }
                },
                onDismissRequest = onDismissRequest,
                onSave = { onSave(selections.toList()) },
            )
            HorizontalDivider()

            when {
                activeSettingIndex != null -> {
                    val settingIndex = requireNotNull(activeSettingIndex)
                    SettingsChoiceList(
                        setting = settings[settingIndex],
                        selectedIndex = selections[settingIndex],
                        onSelect = { choiceIndex ->
                            selections[settingIndex] = choiceIndex
                            activeSettingIndex = null
                        },
                    )
                }
                activeSection != null -> SettingsSectionList(
                    section = activeSection,
                    settings = settings,
                    selections = selections,
                    onSettingClick = { activeSettingIndex = it },
                )
                else -> SettingsCategoryList(
                    sections = sections,
                    settings = settings,
                    selections = selections,
                    onSectionClick = { activeSectionId = it },
                )
            }
        }
    }
}

@Composable
private fun SettingsNavigationHeader(
    title: String,
    subtitle: String,
    canNavigateBack: Boolean,
    onNavigateBack: () -> Unit,
    onDismissRequest: () -> Unit,
    onSave: () -> Unit,
) {
    Column(modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (canNavigateBack) {
                TextButton(onClick = onNavigateBack) { Text("Back") }
            } else {
                TextButton(onClick = onDismissRequest) { Text("Cancel") }
            }
            TextButton(onClick = onSave) { Text("Save") }
        }
        Text(text = title, style = MaterialTheme.typography.headlineSmall)
        Text(text = subtitle, style = MaterialTheme.typography.bodyMedium)
    }
}

@Composable
private fun SettingsCategoryList(
    sections: List<AndroidSettingsSection>,
    settings: List<AndroidSetting>,
    selections: List<Int>,
    onSectionClick: (String) -> Unit,
) {
    LazyColumn(modifier = Modifier.fillMaxSize()) {
        itemsIndexed(sections) { index, section ->
            val summary = section.settingIndices.take(2).joinToString(" · ") { settingIndex ->
                val setting = settings[settingIndex]
                val value = setting.choices.getOrElse(selections[settingIndex]) { "?" }
                "${setting.label}: $value"
            }
            SettingsNavigationRow(
                title = section.label,
                value = settingsCountLabel(section.settingIndices.size),
                supportingText = summary,
                semanticsLabel = "${section.label}: ${settingsCountLabel(section.settingIndices.size)}",
                onClick = { onSectionClick(section.id) },
            )
            if (index < sections.lastIndex) HorizontalDivider()
        }
    }
}

@Composable
private fun SettingsSectionList(
    section: AndroidSettingsSection,
    settings: List<AndroidSetting>,
    selections: List<Int>,
    onSettingClick: (Int) -> Unit,
) {
    LazyColumn(modifier = Modifier.fillMaxSize()) {
        itemsIndexed(section.settingIndices) { index, settingIndex ->
            val setting = settings[settingIndex]
            val value = setting.choices.getOrElse(selections[settingIndex]) { "?" }
            SettingsNavigationRow(
                title = setting.label,
                value = value,
                semanticsLabel = "${setting.key}: ${setting.label}: $value",
                onClick = { onSettingClick(settingIndex) },
            )
            if (index < section.settingIndices.lastIndex) HorizontalDivider()
        }
    }
}

@Composable
private fun SettingsChoiceList(
    setting: AndroidSetting,
    selectedIndex: Int,
    onSelect: (Int) -> Unit,
) {
    LazyColumn(modifier = Modifier.fillMaxSize()) {
        itemsIndexed(setting.choices) { choiceIndex, choiceLabel ->
            DialogChoiceButton(
                label = choiceLabel,
                selected = selectedIndex == choiceIndex,
            ) {
                onSelect(choiceIndex)
            }
            if (choiceIndex < setting.choices.lastIndex) HorizontalDivider()
        }
    }
}

@Composable
private fun SettingsNavigationRow(
    title: String,
    value: String,
    semanticsLabel: String,
    supportingText: String? = null,
    onClick: () -> Unit,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onClick)
                .padding(horizontal = 24.dp, vertical = 18.dp)
                .semantics { contentDescription = semanticsLabel },
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(text = title, style = MaterialTheme.typography.titleMedium)
            supportingText?.takeIf(String::isNotBlank)?.let {
                Text(text = it, style = MaterialTheme.typography.bodySmall)
            }
        }
        Text(text = value, style = MaterialTheme.typography.bodyMedium)
    }
}

private fun buildSettingsSections(settings: List<AndroidSetting>): List<AndroidSettingsSection> =
    settings
        .withIndex()
        .groupBy({ it.value.section }, { it.index })
        .map { (id, indices) ->
            AndroidSettingsSection(id, settingsSectionLabel(id), indices)
        }

private fun settingsSectionLabel(id: String): String =
    when (id) {
        "audio" -> "Audio"
        "video" -> "Video"
        "controls" -> "Controls"
        "storage" -> "Storage"
        "system.nes" -> "Nintendo Entertainment System"
        "system.gbc" -> "Game Boy Color"
        "general" -> "General"
        else -> id.substringAfterLast('.').replace('_', ' ').replaceFirstChar(Char::uppercase)
    }

private fun settingsCountLabel(count: Int): String =
    "$count ${if (count == 1) "setting" else "settings"}"

@Composable
private fun rememberSettingsSelections(
    settings: List<AndroidSetting>,
    initialSelections: List<Int>,
) = remember(settings, initialSelections) {
    mutableStateListOf<Int>().apply {
        settings.forEachIndexed { index, setting ->
            val maxIndex = max(0, setting.choices.lastIndex)
            add(initialSelections.getOrElse(index) { 0 }.coerceIn(0, maxIndex))
        }
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

@SuppressLint("ViewConstructor")
private class ControlsOverlayView(
    context: Context,
    opacityPercent: Int,
    private val scalePercent: Int,
    private val verticalOffsetPercent: Int,
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
    private val arrowPaint =
        Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = Color.argb((220 * opacity).toInt(), 255, 255, 255)
            style = Paint.Style.FILL
        }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        val viewWidth = width.toFloat()
        val viewHeight = height.toFloat()
        if (viewWidth <= 0f || viewHeight <= 0f) {
            return
        }

        controlsLayout(viewWidth, viewHeight, scalePercent, verticalOffsetPercent).forEach { zone ->
            drawZone(canvas, zone.x, zone.y, zone.width, zone.height, zone.label)
        }
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

@SuppressLint("ViewConstructor")
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

internal fun controlsLayout(
    width: Float,
    height: Float,
    scalePercent: Int = 100,
    verticalOffsetPercent: Int = 0,
): List<OverlayZoneSpec> {
    val portrait = height >= width
    val base = min(width, height)
    val scale = scalePercent.coerceIn(50, 150) / 100f
    val verticalOffset = height * verticalOffsetPercent.coerceIn(-30, 30) / 100f
    val controlTop = if (portrait) height * 0.54f else 0f
    val controlHeight = height - controlTop
    val dpadLeft = base * 0.08f
    val dpadSize = base * 0.28f * scale
    val dpadCenterX = dpadLeft + dpadSize * 0.5f
    val dpadCenterY =
        (if (portrait) controlTop + controlHeight * 0.58f else height * 0.65f) + verticalOffset
    val dpadArm = dpadSize * 0.28f
    val dpadExtent = dpadSize * 0.42f
    val actionSize = base * 0.14f * scale
    val actionGap = base * 0.04f
    val actionLeft = if (portrait) width * 0.64f else width - base * 0.08f - actionSize * 2f - actionGap
    val actionTop = dpadCenterY - actionSize * 0.5f
    val centerButtonWidth = base * 0.10f * scale
    val centerButtonHeight = base * 0.068f * scale
    val centerGap = base * 0.03f
    val centerRowWidth = centerButtonWidth * 2f + centerGap
    val centerStartX = (width - centerRowWidth) * 0.5f
    val centerTop =
        (if (portrait) controlTop + controlHeight * 0.16f else height * 0.82f) + verticalOffset

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
