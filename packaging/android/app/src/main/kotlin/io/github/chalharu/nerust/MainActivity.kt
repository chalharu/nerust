package io.github.chalharu.nerust

import android.annotation.SuppressLint
import android.app.NativeActivity
import android.content.Context
import android.content.Intent
import android.content.pm.ActivityInfo
import android.content.pm.PackageManager
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
import org.json.JSONException
import org.json.JSONObject

private const val SETTINGS_SCREEN_TAG = "nerust-settings-screen"
private const val ERROR_DIALOG_TAG = "nerust-error-dialog"

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
    private val safRepository = SafRepository(this)
    // Test-only dialog probe: release builds skip the write so the
    // production show path pays one branch and keeps no test state.
    internal var lastDialogStateForTest = "not requested"
        set(value) {
            if (BuildConfig.DEBUG) field = value
        }

    // Root-window safe insets in physical pixels, order [top, right, bottom, left].
    // Observed only: the listener refreshes and pushes this cache, so the render thread reads only.
    @Volatile private var cachedSafeInsetsPx: IntArray = intArrayOf(0, 0, 0, 0)

    @Volatile private var lastLoadedSystemForTest: String? = null

    /** Set only on user-initiated finish paths; native exits then, never on destroy. */
    private var userInitiatedExit = false

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

    override fun attachBaseContext(newBase: Context) {
        super.attachBaseContext(AppLocale.wrap(newBase))
    }

    override fun onResume() {
        super.onResume()
        Log.i(TAG, "onResume")
        pushSystemLocaleToVault()
        activeActivityForTest = this
        chromeManager.onResume()
        sensorController.onResume()
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_RESUME)
        (getSystemService(INPUT_SERVICE) as InputManager).registerInputDeviceListener(this, null)
        refreshControllerPresence()
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
        sendMenuAction(menuEvent(MenuAction.ORIENTATION_CHANGED))
    }

    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        // Settings capture lives in SettingsActivity with its own dispatch;
        // here only gamepad sources divert, everything else keeps platform
        // behavior (volume, Back, hardware keyboards in gameplay).
        val isController = isControllerSource(event.source)
        if (!isController) {
            return super.dispatchKeyEvent(event)
        }
        val key =
            when (event.keyCode) {
                KeyEvent.KEYCODE_BUTTON_A -> ControllerKey.BUTTON1
                KeyEvent.KEYCODE_BUTTON_B -> ControllerKey.BUTTON2
                KeyEvent.KEYCODE_BUTTON_L1 -> ControllerKey.BUTTON5
                KeyEvent.KEYCODE_BUTTON_R1 -> ControllerKey.BUTTON6
                KeyEvent.KEYCODE_BUTTON_START -> ControllerKey.START
                KeyEvent.KEYCODE_BUTTON_SELECT -> ControllerKey.SELECT
                KeyEvent.KEYCODE_DPAD_UP -> ControllerKey.UP
                KeyEvent.KEYCODE_DPAD_DOWN -> ControllerKey.DOWN
                KeyEvent.KEYCODE_DPAD_LEFT -> ControllerKey.LEFT
                KeyEvent.KEYCODE_DPAD_RIGHT -> ControllerKey.RIGHT
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
        updateControllerInput(event.deviceId, ControllerKey.LEFT, horizontal < -CONTROLLER_AXIS_THRESHOLD)
        updateControllerInput(event.deviceId, ControllerKey.RIGHT, horizontal > CONTROLLER_AXIS_THRESHOLD)
        updateControllerInput(event.deviceId, ControllerKey.UP, vertical < -CONTROLLER_AXIS_THRESHOLD)
        updateControllerInput(event.deviceId, ControllerKey.DOWN, vertical > CONTROLLER_AXIS_THRESHOLD)
        return true
    }

    /** Single controller-event producer: press, release, and removal
     * synthesis share one extras shape. */
    private fun sendControllerInput(deviceId: Int, key: String, pressed: Boolean) {
        sendMenuAction(
            menuEvent(MenuAction.CONTROLLER) {
                put("device_id", deviceId)
                put("key", key)
                put("pressed", pressed)
            },
        )
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
        sendControllerInput(deviceId, key, pressed)
        if (keys.isEmpty()) controllerPressed.remove(deviceId)
    }

    private fun releaseController(deviceId: Int) {
        controllerPressed.remove(deviceId)?.toList()?.forEach { key ->
            sendControllerInput(deviceId, key, false)
        }
    }

    /** Mirror of the Rust tracker for edge-detect and device-removal synthesis; focus loss converges via session clear. */
    private fun releaseAllControllers() {
        controllerPressed.keys.toList().forEach(::releaseController)
    }

    override fun onInputDeviceAdded(deviceId: Int) {
        refreshControllerPresence()
    }

    override fun onInputDeviceChanged(deviceId: Int) {
        refreshControllerPresence()
    }

    override fun onInputDeviceRemoved(deviceId: Int) {
        releaseController(deviceId)
        sensorController.noteControllerRemoved(deviceId)
        refreshControllerPresence()
    }

    /** Auto overlay rule, evaluated live: non-touch terminals never draw
     * buttons; while a gamepad is connected Auto hides them. Keyboards
     * stay out of this gate on purpose: they are shareable auxiliary
     * input (and the emulator always enumerates one), so mere presence
     * must not evict the touch controls.
     * Touch capability mirrors the settings push (feature flag or a
     * touchscreen device): settings and runtime must agree, or "Always"
     * would save yet never draw. */
    private fun refreshControllerPresence() {
        val scan = scanDeviceTruth(packageManager)
        chromeManager.setTouchCapable(scan.touchCapable)
        if (scan.gamepadNames.isNotEmpty()) {
            // Debug breadcrumb for Auto-hide reports: which device hid it.
            Log.d(TAG, "overlay auto-hide counts gamepad: ${scan.gamepadNames.joinToString()}")
        }
        chromeManager.setControllerConnected(scan.gamepadNames.isNotEmpty())
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
        safRepository.close()
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_DESTROY)
        store.clear()
        onActivityDestroyed(userInitiatedExit)
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
            userInitiatedExit = true
            finish()
        }
    }

    // Deprecated upstream; kept because minSdk 28 predates the
    // OnBackInvokedDispatcher replacement (API 33+ path lives above).
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

    /** Touch-overlay configuration pushed from Rust. Kotlin draws the pushed
     * zones and joystick only; it keeps no geometry of its own. */
    fun configureControlsOverlay(
        visibility: String,
        opacityPercent: Int,
        haptics: Boolean,
        zones: String,
        joystick: String,
    ) {
        chromeManager.configureControlsOverlay(
            visibility,
            opacityPercent,
            haptics,
            zones,
            joystick,
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

    fun controlsZoneCountForTest(): Int = chromeManager.controlsZoneCountForTest()

    fun drawerScrollMaxForTest(): Int? = chromeManager.drawerScrollMaxForTest()

    /** Cached root-window safe insets in physical pixels: [top, right, bottom, left].
     * Read by Rust without JNI; refreshed on the UI thread, so the render thread never blocks. */
    fun getSafeInsetsPx(): IntArray = cachedSafeInsetsPx.clone()

    private fun installSafeInsetsListener() {
        val decor = window?.decorView ?: return
        decor.setOnApplyWindowInsetsListener { _, insets ->
            // Unthrottled layout passes flood JNI; push only on change.
            val next = windowInsetsToSafePx(insets)
            if (!cachedSafeInsetsPx.contentEquals(next)) {
                cachedSafeInsetsPx = next
                onSystemInsetsChanged(next)
            }
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
    fun deleteSafFile(
        treeUri: String,
        relativePath: String,
    ): String = safRepository.deleteSafFile(treeUri, relativePath)

    fun openSafWriteSession(
        treeUri: String,
        relativePath: String,
    ): Long = safRepository.openSafWriteSession(treeUri, relativePath)

    fun writeSafWriteSession(sessionId: Long, bytes: ByteArray): Boolean =
        safRepository.writeSafWriteSession(sessionId, bytes)

    fun closeSafWriteSession(sessionId: Long, commit: Boolean): Boolean =
        safRepository.closeSafWriteSession(sessionId, commit)

    fun listSafFilesWithSizes(
        treeUri: String,
        relativePath: String,
    ): String = safRepository.listSafFilesWithSizes(treeUri, relativePath)

    /** Streaming SAF read: one server-side stream, sequential chunks. */
    fun openSafReadSession(
        treeUri: String,
        relativePath: String,
    ): Long = safRepository.openSafReadSession(treeUri, relativePath)

    fun readSafReadSession(
        sessionId: Long,
        maxBytes: Int,
    ): ByteArray? = safRepository.readSafReadSession(sessionId, maxBytes)

    fun closeSafReadSession(sessionId: Long) {
        safRepository.closeSafReadSession(sessionId)
    }

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
            SETTINGS_SCREEN_TAG -> {
                val settings = SettingsActivity.currentActivityForTest()
                settings != null && !settings.isFinishing && !settings.isDestroyed
            }

            else -> {
                window.decorView.findViewWithTag<View>(tag).isShownInWindowForTest()
            }
        }

    fun findChromeViewForTest(tag: String): View? =
        chromeManager.findViewForTest(tag) ?: when (tag) {
            SETTINGS_SCREEN_TAG -> {
                SettingsActivity.currentActivityForTest()?.window?.decorView
            }

            else -> {
                window.decorView.findViewWithTag(tag)
            }
        }

    fun chromeDebugStateForTest(tag: String): String =
        "tag=$tag, destroyed=$isDestroyed, finishing=$isFinishing, decor=${window.decorView.debugViewState()}, " +
            chromeManager.debugStateForTest() + dialogManager.debugState() +
            ", settings=${SettingsActivity.currentActivityForTest()?.isFinishing}" +
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
        SettingsActivity.currentActivityForTest()?.finishForReplacement()
            ?: dialogManager.dismissForTest()
    }

    fun resetChromeStateForTest() {
        SettingsActivity.currentActivityForTest()?.finish()
        dialogManager.resetForTest()
        chromeManager.removeDrawerOverlay()
    }

    /** Show a DTO-driven Android settings screen. Presents the versioned sections from Rust.
     * Save and dismiss return request-scoped JSON; edits ride synchronous per-row commands. */
    fun showSettingsDialog(payload: String) {
        val document =
            try {
                JSONObject(payload)
            } catch (e: JSONException) {
                Log.e(TAG, "showSettingsDialog: malformed payload", e)
                return
            }
        if (document.optInt("schemaVersion", -1) != SETTINGS_SCHEMA_VERSION) {
            Log.e(TAG, "showSettingsDialog: unsupported schemaVersion")
            val requestId = document.optLong("requestId", -1L)
            if (requestId >= 0) {
                showErrorDialog(requestId, "Unsupported settings schema")
            }
            return
        }
        val requestId = document.getLong("requestId")
        // SettingsActivity unwraps and validates the same versioned envelope.
        // Pass the live orientation lock so the settings screen opens in the
        // same orientation as the game screen (see onCreate).
        startActivity(
            SettingsActivity.createIntent(
                this,
                SETTINGS_SCHEMA_VERSION,
                requestId,
                document.toString(),
                requestedOrientation,
            ),
        )
    }

    /** Persist the saved in-app language. No recreate: drawer, dialogs
     * and the edge handle inflate from the wrapped context on every show,
     * so they follow the choice live. Recreating the NativeActivity would
     * destroy the Rust event loop and freeze the app. */
    fun applyAppLanguage(tag: String) {
        AppLocale.setSavedTag(this, tag)
    }

    /** Push the terminal locale tag into the Rust vault so SystemDefault
     * resolves without the LANG env Android never sets. */
    private fun pushSystemLocaleToVault() {
        val tag = AppLocale.currentTag() ?: return
        try {
            pushSystemLocale(tag)
        } catch (e: Exception) {
            Log.e(TAG, "pushSystemLocale failed", e)
        }
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

    /** Dumb host for the Rust dialog gate: always present what Rust requests.
     * A showing settings screen finishes with its own dismissal result so
     * Rust resolves it before the error shows. */
    fun showErrorDialog(requestId: Long, message: String) {
        SettingsActivity.currentActivityForTest()?.finishForReplacement()
        dialogManager.show(
            dialogTag = ERROR_DIALOG_TAG,
            contentDescription = message,
            ownedByTest = false,
            onDismiss = { onErrorDialogDismissed(requestId) },
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

    private external fun pushSystemLocale(tag: String)

    /** Pause truth for the drawer (vault, published after every session
     * command). Plain wrapper: `internal external` would mangle the JVM
     * name (`$app` suffix) and break native registration, so the external
     * stays private like every other query. */
    internal fun isEmulationPaused(): Boolean = nativeIsEmulationPaused()

    private external fun nativeIsEmulationPaused(): Boolean

    private external fun onFilePickerResult(uri: String?)

    private external fun onMenuAction(action: String)

    private external fun onErrorDialogDismissed(requestId: Long)

    private external fun onDirectoryPickerResult(uri: String?)

    private external fun onActivityDestroyed(userInitiatedExit: Boolean)

    private external fun onCartridgeAcceleration(
        xG: Float,
        yG: Float,
    )

    private external fun onSystemInsetsChanged(insets: IntArray)

    companion object {
        private const val TAG = "Nerust"

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


