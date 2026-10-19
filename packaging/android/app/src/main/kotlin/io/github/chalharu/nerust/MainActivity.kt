package io.github.chalharu.nerust

import android.annotation.SuppressLint
import android.app.NativeActivity
import android.content.Intent
import android.content.pm.ActivityInfo
import android.content.res.Configuration
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.Typeface
import android.hardware.input.InputManager
import android.media.AudioManager
import android.os.Build
import android.os.Bundle
import android.util.Log
import android.view.InputDevice
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.View
import android.view.WindowInsets as AndroidWindowInsets
import android.window.OnBackInvokedDispatcher
import androidx.compose.foundation.clickable
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
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
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
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
import org.json.JSONObject
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min

private const val MENU_ACTION_ERROR_DIALOG_DISMISSED = "error_dialog_dismissed"
private const val SETTINGS_DIALOG_TAG = "nerust-settings-dialog"
private const val ERROR_DIALOG_TAG = "nerust-error-dialog"
private const val DIALOG_PRESENTATION_FULL_SCREEN = "full_screen"

internal fun screenOrientationRequest(value: Int): Int? =
    when (value) {
        0 -> ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED
        1 -> ActivityInfo.SCREEN_ORIENTATION_PORTRAIT
        2 -> ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE
        else -> null
    }

private data class AndroidSetting(
    val key: String,
    val section: String,
    val label: String,
    val choices: List<String>,
    val edges: List<String> = emptyList(),
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

internal data class FloatingJoystickSpec(
    val centerX: Float,
    val centerY: Float,
    val baseRadius: Float,
    val knobRadius: Float,
    val activationBounds: RectF,
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

class MainActivity :
    NativeActivity(),
    LifecycleOwner,
    SavedStateRegistryOwner,
    ViewModelStoreOwner,
    InputManager.InputDeviceListener {
    private val lifecycleRegistry = LifecycleRegistry(this)
    private val registryController = SavedStateRegistryController.create(this)
    private val store = ViewModelStore()
    private val controllerPressed = mutableMapOf<Int, MutableSet<String>>()
    private val sensorController = SensorController(this)
    private val chromeManager = ChromeManager(this)
    private val dialogManager = DialogManager(this)
    private val safRepository by lazy { SafRepository(this) }
    internal var lastDialogStateForTest = "not requested"

    // Root-window safe insets in physical pixels, order [top, right, bottom, left].
    // Observed only: the listener refreshes this cache and pushes it to Rust
    // via onSystemInsetsChanged, so the render thread reads the cache only.
    @Volatile private var cachedSafeInsetsPx: IntArray = intArrayOf(0, 0, 0, 0)

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
        sensorController.onCreate()
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_CREATE)
        installSafeInsetsListener()
        chromeManager.onResume()
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
        chromeManager.onResume()
        sensorController.onResume()
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_RESUME)
        (getSystemService(INPUT_SERVICE) as InputManager).registerInputDeviceListener(this, null)
        chromeManager.scheduleChromeAttach()
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        Log.i(TAG, "onWindowFocusChanged: hasFocus=$hasFocus")
        sensorController.onWindowFocusChanged(hasFocus)
        if (hasFocus) {
            chromeManager.scheduleChromeAttach()
        }
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        Log.i(TAG, "onConfigurationChanged: orientation=${newConfig.orientation}")
        // The manifest handles rotation in-place (no Activity recreate),
        // but PopupWindow chrome does not reliably survive it: the popup
        // can keep a stale frame, lose its window, or fall behind the
        // recreated native surface while still reporting isShowing=true,
        // so the re-attach machinery never fires again and the touch
        // controls stay invisible. Rebuild the popup chrome explicitly
        // for the new configuration. The Compose settings dialog is a
        // proper Dialog window that re-lays itself out, so it is left
        // alone; an open navigation drawer is closed (one tap reopens).
        chromeManager.onConfigurationChanged()
        // Tell Rust to rebuild its touch overlay (hit zones) for the new
        // window size even if no winit Resized event arrives; the Rust
        // side also re-pushes the overlay config, which converges with
        // the re-attach above.
        sendMenuAction("orientation_changed")
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
                KeyEvent.KEYCODE_BUTTON_L1 -> "button5"
                KeyEvent.KEYCODE_BUTTON_R1 -> "button6"
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

    private fun updateControllerInput(
        deviceId: Int,
        key: String,
        pressed: Boolean,
    ) {
        val keys = controllerPressed.getOrPut(deviceId) { mutableSetOf() }
        val changed = if (pressed) keys.add(key) else keys.remove(key)
        if (!changed) return
        if (pressed) {
            sensorController.lastControllerDeviceId = deviceId
            chromeManager.noteControllerInput()
            sensorController.noteControllerInput()
        }
        sendMenuAction("controller:$deviceId:$key:${if (pressed) 1 else 0}")
        if (keys.isEmpty()) controllerPressed.remove(deviceId)
    }

    private fun releaseController(deviceId: Int) {
        controllerPressed.remove(deviceId)?.toList()?.forEach { key ->
            sendMenuAction("controller:$deviceId:$key:0")
        }
    }

    private fun releaseAllControllers() {
        controllerPressed.keys.toList().forEach(::releaseController)
    }

    override fun onInputDeviceAdded(deviceId: Int) = Unit

    override fun onInputDeviceChanged(deviceId: Int) = Unit

    override fun onInputDeviceRemoved(deviceId: Int) {
        releaseController(deviceId)
        sensorController.noteControllerRemoved(deviceId)
    }

    override fun onPause() {
        if (activeActivityForTest === this) {
            activeActivityForTest = null
        }
        Log.i(TAG, "onPause")
        chromeManager.onPause()
        sensorController.onPause()
        releaseAllControllers()
        (getSystemService(INPUT_SERVICE) as InputManager).unregisterInputDeviceListener(this)
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_PAUSE)
        super.onPause()
    }

    override fun onStop() {
        Log.i(TAG, "onStop")
        dialogManager.dismissForStop()
        chromeManager.onStop()
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
                "lastDrawerState=${chromeManager.lastDrawerStateForTest} lastDialogState=$lastDialogStateForTest",
        )
        chromeManager.onStop()
        dialogManager.dismissForStop()
        sensorController.onDestroy()
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_DESTROY)
        store.clear()
        onActivityDestroyed()
        super.onDestroy()
    }

    /** JNI bridge for menu events from Rust-side handlers. */
    internal fun sendMenuAction(action: String) {
        onMenuAction(action)
    }

    /** Forward the cartridge acceleration sample to the native bridge. */
    internal fun forwardCartridgeAcceleration(xG: Float, yG: Float) {
        onCartridgeAcceleration(xG, yG)
    }

    private fun handleBackNavigation() {
        if (!chromeManager.removeDrawerOverlay()) {
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

    /**
     * Touch-overlay configuration pushed from Rust.
     *
     * Geometry (hit zones) is owned by Rust (`touch_layout.rs`): [zones]
     * carries the resolved relative rects (`LABEL:x,y,w,h;...`) computed from
     * the same scale/offset/visibility parameters. Kotlin draws the pushed
     * zones and only falls back to its local `controlsLayout()` copy when
     * the payload is absent or malformed. Keep the fractions in sync.
     */
    fun configureControlsOverlay(
        visibility: String,
        opacityPercent: Int,
        scalePercent: Int,
        verticalOffsetPercent: Int,
        haptics: Boolean,
        shouldersVisible: Boolean,
        zones: String,
    ) {
        chromeManager.configureControlsOverlay(
            visibility,
            opacityPercent,
            scalePercent,
            verticalOffsetPercent,
            haptics,
            shouldersVisible,
            zones,
        )
    }

    fun performControlHaptic() {
        chromeManager.performControlHaptic()
    }

    fun configureCartridgePeripherals(
        motionEnabled: Boolean,
        rumbleEnabled: Boolean,
        rumbleStrengthPercent: Int,
        rumbleTarget: String,
    ) {
        sensorController.configure(motionEnabled, rumbleEnabled, rumbleStrengthPercent, rumbleTarget)
    }

    fun setCartridgeRumble(intensity: Int) {
        sensorController.setIntensity(intensity)
    }

    fun updateFloatingDpad(
        active: Boolean,
        centerX: Float,
        centerY: Float,
        knobX: Float,
        knobY: Float,
    ) {
        chromeManager.updateFloatingDpad(active, centerX, centerY, knobX, knobY)
    }

    fun floatingDpadStateForTest(): FloatArray? = chromeManager.floatingDpadStateForTest()

    /**
     * Show or refresh the small FPS label at the bottom-right corner.
     *
     * Called from Rust at ~2Hz while emulating. `visible` is false when no
     * ROM is loaded or emulation is paused, hiding the label instead of
     * showing stale numbers. Runs on the Java main thread.
     *
     * Rendered in a dedicated non-touchable popup (like the other chrome):
     * a plain decorView child is not reliably composited above the native
     * game surface.
     */
    /**
     * Cached root-window safe insets in physical pixels: [top, right, bottom, left].
     * Read by Rust without JNI; the cache is refreshed on the UI thread by
     * [installSafeInsetsListener], which also pushes to Rust natively, so
     * the render thread never blocks on a JNI query.
     */
    fun getSafeInsetsPx(): IntArray = cachedSafeInsetsPx.clone()

    private fun installSafeInsetsListener() {
        val decor = window?.decorView ?: return
        decor.setOnApplyWindowInsetsListener { _, insets ->
            cachedSafeInsetsPx = windowInsetsToSafePx(insets)
            onSystemInsetsChanged(cachedSafeInsetsPx)
            insets
        }
        decor.requestApplyInsets()
    }

    private fun windowInsetsToSafePx(insets: AndroidWindowInsets): IntArray {
        var left = 0
        var top = 0
        var right = 0
        var bottom = 0
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            val bars = insets.getInsets(AndroidWindowInsets.Type.systemBars())
            left = bars.left
            top = bars.top
            right = bars.right
            bottom = bars.bottom
        } else {
            @Suppress("DEPRECATION")
            with(insets) {
                left = systemWindowInsetLeft
                top = systemWindowInsetTop
                right = systemWindowInsetRight
                bottom = systemWindowInsetBottom
            }
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            insets.displayCutout?.let { cutout ->
                left = maxOf(left, cutout.safeInsetLeft)
                top = maxOf(top, cutout.safeInsetTop)
                right = maxOf(right, cutout.safeInsetRight)
                bottom = maxOf(bottom, cutout.safeInsetBottom)
            }
        }
        return intArrayOf(top, right, bottom, left)
    }

    fun updateFpsOverlay(fps: Float, visible: Boolean) {
        chromeManager.updateFpsOverlay(fps, visible)
    }

    /**
     * SAF bridge, delegated to [SafRepository]. The whole-file methods stay
     * for APKs predating chunked I/O; the chunked methods bound every JNI
     * crossing to 64KB (`writeSafFileChunk` appends with `"wa"`).
     */
    fun readSafFile(
        treeUri: String,
        relativePath: String,
    ): String? = safRepository.readSafFile(treeUri, relativePath)

    fun writeSafFile(
        treeUri: String,
        relativePath: String,
        encoded: String,
    ): String = safRepository.writeSafFile(treeUri, relativePath, encoded)

    fun deleteSafFile(
        treeUri: String,
        relativePath: String,
    ): String = safRepository.deleteSafFile(treeUri, relativePath)

    fun listSafFiles(
        treeUri: String,
        relativePath: String,
    ): String = safRepository.listSafFiles(treeUri, relativePath)

    fun getSafFileSize(
        treeUri: String,
        relativePath: String,
    ): Long = safRepository.getSafFileSize(treeUri, relativePath)

    fun readSafFileChunk(
        treeUri: String,
        relativePath: String,
        offset: Long,
        length: Long,
    ): String? = safRepository.readSafFileChunk(treeUri, relativePath, offset, length)

    fun writeSafFileChunk(
        treeUri: String,
        relativePath: String,
        encoded: String,
        append: Boolean,
    ): String = safRepository.writeSafFileChunk(treeUri, relativePath, encoded, append)

    @Suppress("DEPRECATION")
    override fun onActivityResult(
        requestCode: Int,
        resultCode: Int,
        data: Intent?,
    ) {
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
                data
                    ?.flags
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
        chromeManager.isViewShowingForTest(tag) ?: when (tag) {
            SETTINGS_DIALOG_TAG,
            -> {
                dialogManager.isShowing(tag)
            }

            else -> {
                window.decorView.findViewWithTag<View>(tag).isShownInWindowForTest()
            }
        }

    fun findChromeViewForTest(tag: String): View? =
        chromeManager.findViewForTest(tag) ?: when (tag) {
            SETTINGS_DIALOG_TAG,
            -> {
                dialogManager.rootViewForTag(tag)
            }

            else -> {
                window.decorView.findViewWithTag(tag)
            }
        }

    fun chromeDebugStateForTest(tag: String): String =
        "tag=$tag, destroyed=$isDestroyed, finishing=$isFinishing, decor=${window.decorView.debugViewState()}, " +
            chromeManager.debugStateForTest() + dialogManager.debugState() +
            ", lastDialog=$lastDialogStateForTest"

    fun dispatchMenuActionForTest(action: String) {
        chromeManager.dispatchMenuAction(action)
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
        chromeManager.showDrawerOverlay()
    }

    fun dismissComposeDialogForTest() {
        dialogManager.dismissForTest()
    }

    fun resetChromeStateForTest() {
        dialogManager.resetForTest()
        chromeManager.removeDrawerOverlay()
    }

    fun showSettingsDialogForTest(
        keys: Array<String>,
        labels: Array<String>,
        choiceStrings: Array<String>,
        currentIndices: Array<String>,
        sections: Array<String> = emptyArray(),
        edgeStrings: Array<String> = emptyArray(),
    ) {
        showSettingsDialogInternal(
            content =
                SettingsDialogContent(
                    keys = keys.toList(),
                    labels = labels.toList(),
                    choiceStrings = choiceStrings.toList(),
                    currentIndices = currentIndices.toList(),
                    sections = sections.toList(),
                    edgeStrings = edgeStrings.toList(),
                ),
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
        val edgeStrings =
            Array(fieldDocuments.size) { index ->
                fieldDocuments[index].second.optJSONArray("edges")?.let { edges ->
                    (0 until edges.length()).joinToString("\t") { edges.getString(it) }
                } ?: ""
            }
        showSettingsDialogInternal(
            content =
                SettingsDialogContent(
                    keys = keys.toList(),
                    labels = labels.toList(),
                    choiceStrings = choiceStrings.toList(),
                    currentIndices = currentIndices.toList(),
                    sections = sections.toList(),
                    edgeStrings = edgeStrings.toList(),
                ),
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

    private data class SettingsDialogContent(
        val keys: List<String>,
        val labels: List<String>,
        val choiceStrings: List<String>,
        val currentIndices: List<String>,
        val sections: List<String>,
        val edgeStrings: List<String>,
    )

    private fun showSettingsDialogInternal(
        content: SettingsDialogContent,
        requestId: Long?,
        ownedByTest: Boolean,
    ) {
        val keys = content.keys
        val labels = content.labels
        val choiceStrings = content.choiceStrings
        val currentIndices = content.currentIndices
        val sections = content.sections
        val edgeStrings = content.edgeStrings
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
                    edges =
                        edgeStrings
                            .getOrNull(index)
                            ?.split('\t')
                            ?.filter(String::isNotEmpty)
                            ?: emptyList(),
                )
            }
        val initialSelections =
            settings.mapIndexed { index, setting ->
                val maxIndex = max(0, setting.choices.lastIndex)
                (currentIndices.getOrNull(index)?.toIntOrNull() ?: 0).coerceIn(0, maxIndex)
            }
        var resultSent = false

        dialogManager.show(
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
        dialogManager.rootViewForTag(SETTINGS_DIALOG_TAG)?.setTag(
            R.id.nerust_settings_hierarchy_probe,
            buildSettingsSections(settings).joinToString("\n") { section ->
                "${section.label}: ${settingsCountLabel(section.settingIndices.size)}"
            },
        )
    }

    /**
     * Show a modal Android error dialog.
     *
     * Presents an aggregated OSD error batch as scrollable text with a single
     * dismiss button. Called from the Rust JNI bridge on the Java main thread.
     * Dismissal (including replacement by another dialog) reports back through
     * the menu string bridge so Rust can resume its auto-pause and show the
     * next queued error. A long batch scrolls instead of clipping.
     */
    fun showErrorDialog(message: String) {
        // Never evict the settings dialog: Rust gates on bridge truth,
        // but a racing show must not destroy in-progress settings input
        // either. Ack as dismissed so Rust keeps the batch queued behind
        // settings instead of losing it.
        if (dialogManager.isShowing(SETTINGS_DIALOG_TAG)) {
            Log.i(TAG, "showErrorDialog: suppressed while settings dialog is showing")
            chromeManager.dispatchMenuAction(MENU_ACTION_ERROR_DIALOG_DISMISSED)
            return
        }
        dialogManager.show(
            dialogTag = ERROR_DIALOG_TAG,
            contentDescription = message,
            presentation = DIALOG_PRESENTATION_FULL_SCREEN,
            ownedByTest = false,
            onDismiss = { chromeManager.dispatchMenuAction(MENU_ACTION_ERROR_DIALOG_DISMISSED) },
        ) { dismiss ->
            NerustErrorDialogCard(
                message = message,
                onDismissRequest = dismiss,
            )
        }
    }

    internal fun installComposeOwners(root: View) {
        listOf(window.decorView, root).forEach { view ->
            view.setViewTreeLifecycleOwner(this)
            view.setViewTreeSavedStateRegistryOwner(this)
            view.setViewTreeViewModelStoreOwner(this)
        }
    }

    private fun View?.isShownInWindowForTest(): Boolean =
        this != null && visibility == View.VISIBLE && isAttachedToWindow && windowToken != null

    private fun View?.debugViewState(): String =
        if (this == null) {
            "null"
        } else {
            "visibility=$visibility, attached=$isAttachedToWindow, token=${windowToken != null}, shown=$isShown"
        }

    private external fun onFilePickerResult(uri: String?)

    private external fun onMenuAction(action: String)

    private external fun onDirectoryPickerResult(uri: String?)

    private external fun onSettingsDialogResult(result: String?)

    private external fun onActivityDestroyed()

    private external fun onCartridgeAcceleration(
        xG: Float,
        yG: Float,
    )

    private external fun onSystemInsetsChanged(insets: IntArray)

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

        private const val CONTROLLER_AXIS_THRESHOLD = 0.5f
        private const val ROM_PICKER_REQUEST_CODE = 0x4E45
        private const val DIRECTORY_PICKER_REQUEST_CODE = 0x4E46

        @Volatile
        private var activeActivityForTest: MainActivity? = null

        fun createRomPickerIntentForTest(): Intent = createRomPickerIntent()

        fun createDirectoryPickerIntentForTest(): Intent = createDirectoryPickerIntent()

        fun currentActivityForTest(): MainActivity? = activeActivityForTest?.takeUnless { it.isDestroyed || it.isFinishing }
    }
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
private fun NerustErrorDialogCard(
    message: String,
    onDismissRequest: () -> Unit,
) {
    NerustDialogHost {
        NerustDialogCard(
            title = "Error",
            buttons = {
                TextButton(onClick = onDismissRequest) { Text("OK") }
            },
        ) {
            SelectionContainer {
                Text(
                    text = message,
                    modifier =
                        Modifier
                            .heightIn(max = 480.dp)
                            .verticalScroll(rememberScrollState()),
                )
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
            val visibleSettingCount =
                activeSection
                    ?.let { visibleSettingIndices(settings, it, selections).size }
            SettingsNavigationHeader(
                title =
                    activeSettingIndex?.let { settings[it].label }
                        ?: activeSection?.label
                        ?: "Settings",
                subtitle =
                    when {
                        activeSettingIndex != null -> "Choose a value"
                        activeSection != null -> settingsCountLabel(visibleSettingCount ?: 0)
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

                activeSection != null -> {
                    val visibleSection =
                        activeSection.copy(
                            settingIndices =
                                visibleSettingIndices(settings, activeSection, selections),
                        )
                    SettingsSectionList(
                        section = visibleSection,
                        settings = settings,
                        selections = selections,
                        onSettingClick = { activeSettingIndex = it },
                    )
                }

                else -> {
                    SettingsCategoryList(
                        sections = sections,
                        settings = settings,
                        selections = selections,
                        onSectionClick = { activeSectionId = it },
                    )
                }
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
            val visibleIndices = visibleSettingIndices(settings, section, selections)
            val summary =
                visibleIndices.take(2).joinToString(" · ") { settingIndex ->
                    val setting = settings[settingIndex]
                    val value = setting.choices.getOrElse(selections[settingIndex]) { "?" }
                    "${setting.label}: $value"
                }
            SettingsNavigationRow(
                title = section.label,
                value = settingsCountLabel(visibleIndices.size),
                supportingText = summary,
                semanticsLabel = "${section.label}: ${settingsCountLabel(visibleIndices.size)}",
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
        "system.gba" -> "Game Boy Advance"
        "general" -> "General"
        else -> id.substringAfterLast('.').replace('_', ' ').replaceFirstChar(Char::uppercase)
    }

private fun settingsCountLabel(count: Int): String = "$count ${if (count == 1) "setting" else "settings"}"

// Same-edge follow (mirrors desktop): the notification area mode
// governs the edge, so the status row hides while both position
// pickers share the top or bottom band. Edge classes come from the
// Rust payload ("top"/"bottom" per choice); nothing here assumes
// choice indices.
private fun visibleSettingIndices(
    settings: List<AndroidSetting>,
    section: AndroidSettingsSection,
    selections: List<Int>,
): List<Int> {
    val notifPos = settings.indexOfFirst { it.key == "notifications.position" }
    val statusPos = settings.indexOfFirst { it.key == "status.position" }
    val statusArea = settings.indexOfFirst { it.key == "status.area_mode" }
    if (notifPos < 0 || statusPos < 0 || statusArea < 0) return section.settingIndices
    fun edgeOf(settingIndex: Int): String {
        val setting = settings.getOrNull(settingIndex) ?: return ""
        return setting.edges.getOrElse(selections.getOrElse(settingIndex) { 0 }) { "" }
    }
    val notifEdge = edgeOf(notifPos)
    val statusEdge = edgeOf(statusPos)
    if (notifEdge.isEmpty() || statusEdge.isEmpty()) return section.settingIndices
    if (notifEdge != statusEdge) return section.settingIndices
    return section.settingIndices.filter { it != statusArea }
}

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
private fun DialogChoiceButton(
    label: String,
    selected: Boolean,
    onClick: () -> Unit,
) {
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
