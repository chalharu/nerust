package io.github.chalharu.nerust

import android.annotation.SuppressLint
import android.app.NativeActivity
import android.content.Intent
import android.content.pm.ActivityInfo
import android.content.res.Configuration
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
import kotlin.math.max

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
    // Observed only: the listener refreshes and pushes this cache, so the render thread reads only.
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
        // PopupWindow chrome does not reliably survive rotation: rebuild it explicitly.
        // The Compose settings dialog re-lays itself out; an open drawer is closed.
        chromeManager.onConfigurationChanged()
        // Tell Rust to rebuild its touch overlay even if no winit Resized event arrives.
        // The Rust re-push converges with the re-attach above.
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
        chromeManager.onDestroy()
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

    /** Touch-overlay configuration pushed from Rust. Kotlin draws the pushed zones,
     * falling back to its local `controlsLayout()` copy when the payload is absent or malformed. */
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

    /** Cached root-window safe insets in physical pixels: [top, right, bottom, left].
     * Read by Rust without JNI; refreshed on the UI thread, so the render thread never blocks. */
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

    /** Show or refresh the small FPS label at the bottom-right corner. Runs on the Java main thread.
     * `visible` is false when no ROM is loaded or emulation is paused. */
    fun updateFpsOverlay(fps: Float, visible: Boolean) {
        chromeManager.updateFpsOverlay(fps, visible)
    }

    /** SAF bridge, delegated to [SafRepository]. Chunked methods bound every JNI crossing to 64KB. */
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

    fun readSafFileChunkBytes(
        treeUri: String,
        relativePath: String,
        offset: Long,
        length: Long,
    ): ByteArray? = safRepository.readSafFileChunkBytes(treeUri, relativePath, offset, length)

    fun writeSafFileChunkBytes(
        treeUri: String,
        relativePath: String,
        bytes: ByteArray,
        append: Boolean,
    ): String = safRepository.writeSafFileChunkBytes(treeUri, relativePath, bytes, append)

    fun listSafFilesWithSizes(
        treeUri: String,
        relativePath: String,
    ): String = safRepository.listSafFilesWithSizes(treeUri, relativePath)

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

    /** Show a modal Android settings dialog. Presents the versioned settings JSON from Rust.
     * Save and dismiss return request-scoped JSON keyed by stable setting IDs. */
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

    /** Show a modal Android error dialog. Presents an aggregated OSD error batch as scrollable text.
     * Dismissal reports back through onErrorDialogDismissed so Rust can resume and show the next error. */
    fun showErrorDialog(message: String) {
        // Never evict the settings dialog. Ack as dismissed so Rust keeps the batch queued.
        if (dialogManager.isShowing(SETTINGS_DIALOG_TAG)) {
            Log.i(TAG, "showErrorDialog: suppressed while settings dialog is showing")
            onErrorDialogDismissed()
            return
        }
        dialogManager.show(
            dialogTag = ERROR_DIALOG_TAG,
            contentDescription = message,
            presentation = DIALOG_PRESENTATION_FULL_SCREEN,
            ownedByTest = false,
            onDismiss = { onErrorDialogDismissed() },
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

    private external fun onErrorDialogDismissed()

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
            // Load the native library via the app classloader so the JVM resolves `external fun`.
            // NativeActivity's later dlopen bypasses Java's classloader registration.
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


