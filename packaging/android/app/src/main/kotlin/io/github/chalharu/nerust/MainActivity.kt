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
    private val controllerAxes = mutableMapOf<Int, MutableMap<String, Float>>()
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

    /** Last presence scan: identical hotplug/resume bursts skip the apply. */
    @Volatile private var lastPresenceTruth: DeviceTruth? = null

    /** Synchronous connect set: the async scan below can lag a physical
     * connect, dropping the first key (the gate reads the scan). Listener
     * callbacks and dispatch share the main thread, so a plain set updated
     * here never skews against routing. */
    private var connectedDeviceIds = mutableSetOf<Int>()

    /** Trigger axes present per device (L/R pair): `device.getMotionRange`
     * crosses Binder on the L2/R2 key path, so probe once per device
     * (cached both ways, evicted on add/change/remove). Pair holds
     * LTRIGGER-absent and RTRIGGER-absent separately: L2 must not reuse
     * the R2 verdict. Main thread only (dispatch plus listener
     * callbacks), like the connect set above. */
    private var hasTriggerAxes = mutableMapOf<Int, Pair<Boolean, Boolean>>()

    /** Right-stick primary absent per device (X/Y pair): `getMotionRange`
     * crosses Binder at motion rate, so probe once per device
     * (evicted on add/change/remove like the trigger cache). Pair holds
     * Z-absent and RZ-absent separately: a pad with Z but no RZ must not
     * alias Y. */
    private var rightStickAlias = mutableMapOf<Int, Pair<Boolean, Boolean>>()

    /** Presence-scan epoch: a scan in flight across onPause must not apply
     * after it (overlay/capture gate flipping while paused). The resume
     * scan below re-converges, so dropping is safe. */
    @Volatile private var presenceEpoch = 0L

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
                { handleBackNavigation() },
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
        sendInitialDeviceAdded()
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
        // behavior (volume, Back, hardware keyboards in gameplay). Counted
        // pads divert by device id too: D-pad-only sources would otherwise
        // read as a TV remote and never reach the game.
        // `connectedDeviceIds` is the synchronous routing set (the async
        // counted scan can lag a physical connect and drop the first key).
        // Both hold live ids only for devices with a resolvable descriptor
        // (null-descriptor ghosts never enter, see sendInitialDeviceAdded/onInputDeviceAdded).
        val isController = isControllerSource(event.source) || connectedDeviceIds.contains(event.deviceId)
        if (Log.isLoggable(TAG, Log.DEBUG)) {
            Log.d(TAG, "game key: source=0x${event.source.toString(16)} device=${event.deviceId} keyCode=${event.keyCode} action=${event.action} controller=$isController")
        }
        if (!isController) {
            return super.dispatchKeyEvent(event)
        }
        // Digital-only triggers ride a helper: pads that report L2/R2 as
        // keys with no analog trigger axes would otherwise lose them
        // (unconsumed keys fall to the platform, Rust only reads axes).
        if (mirrorDigitalTrigger(event)) return true
        val key = controllerKeyFor(event.keyCode)
        if (key == null) {
            if (Log.isLoggable(TAG, Log.DEBUG)) {
                Log.d(TAG, "unmapped controller key: source=0x${event.source.toString(16)} keyCode=${event.keyCode}")
            }
            return super.dispatchKeyEvent(event)
        }
        val pressed = event.action == KeyEvent.ACTION_DOWN
        if (pressed && event.repeatCount > 0) {
            return true
        }
        // Only DOWN/UP carry button state; anything else (e.g. ACTION_MULTIPLE)
        // is not ours to consume.
        if (event.action != KeyEvent.ACTION_DOWN && event.action != KeyEvent.ACTION_UP) {
            return super.dispatchKeyEvent(event)
        }
        updateControllerInput(event.deviceId, key, pressed)
        return true
    }

    /** Digital-trigger mirror: returns true when the event is fully
     * handled here. `null` device capability lookups and non-DOWN/UP
     * actions fall through to the platform path below. */
    private fun mirrorDigitalTrigger(event: KeyEvent): Boolean {
        if (event.keyCode != KeyEvent.KEYCODE_BUTTON_L2 &&
            event.keyCode != KeyEvent.KEYCODE_BUTTON_R2
        ) {
            return false
        }
        val axisKey =
            if (event.keyCode == KeyEvent.KEYCODE_BUTTON_L2) {
                ControllerAxis.TRIGGER_L
            } else {
                ControllerAxis.TRIGGER_R
            }
        if (event.action != KeyEvent.ACTION_DOWN && event.action != KeyEvent.ACTION_UP) return false
        // Range probes cross Binder on the key path: cache both outcomes
        // per device (a device that gains axes re-reports via change).
        // Null devices mirror without caching.
        val deviceId = event.deviceId
        val device = event.device
        val hasAxes =
            if (device == null) {
                false
            } else {
                val present =
                    hasTriggerAxes.getOrPut(deviceId) {
                        (device.getMotionRange(MotionEvent.AXIS_LTRIGGER) != null) to
                            (device.getMotionRange(MotionEvent.AXIS_RTRIGGER) != null)
                    }
                if (event.keyCode == KeyEvent.KEYCODE_BUTTON_L2) !present.first else !present.second
            }
        if (!hasAxes) {
            updateControllerTrigger(deviceId, axisKey, event.action == KeyEvent.ACTION_DOWN)
            return true
        }
        return false
    }

    /** Controller keycode to bridge key, or null to leave to platform. */
    private fun controllerKeyFor(keyCode: Int): String? =
        when (keyCode) {
            KeyEvent.KEYCODE_BUTTON_A -> ControllerKey.BUTTON1
            KeyEvent.KEYCODE_BUTTON_B -> ControllerKey.BUTTON2
            KeyEvent.KEYCODE_BUTTON_X -> ControllerKey.BUTTON3
            KeyEvent.KEYCODE_BUTTON_Y -> ControllerKey.BUTTON4
            KeyEvent.KEYCODE_BUTTON_L1 -> ControllerKey.BUTTON5
            KeyEvent.KEYCODE_BUTTON_R1 -> ControllerKey.BUTTON6
            KeyEvent.KEYCODE_BUTTON_L2 -> ControllerKey.BUTTON7
            KeyEvent.KEYCODE_BUTTON_R2 -> ControllerKey.BUTTON8
            // The digital mirror for axis-less pads never reaches here:
            // mirrorDigitalTrigger consumes those keys into the motion
            // batch above. Keys arriving here drive the trigger buttons
            // (bindings, shortcuts) while the motion axes keep driving
            // the analog store plus shoulder doubling; the session
            // pressed set dedups the dual delivery into one press.
            KeyEvent.KEYCODE_BUTTON_MODE -> ControllerKey.GUIDE
            KeyEvent.KEYCODE_BUTTON_THUMBL -> ControllerKey.L3
            KeyEvent.KEYCODE_BUTTON_THUMBR -> ControllerKey.R3
            // Capture/share/create (Switch capture, Xbox share, PS create):
            // one position button like the rest, so capture, gameplay
            // overrides, and shortcuts all resolve through the same path.
            // `BUTTON_Z` is intentionally not mapped here: it is the
            // Z-labeled / upper-row third button, not capture.
            KeyEvent.KEYCODE_BUTTON_C -> ControllerKey.CAPTURE
            KeyEvent.KEYCODE_BUTTON_START -> ControllerKey.START
            KeyEvent.KEYCODE_BUTTON_SELECT -> ControllerKey.SELECT
            KeyEvent.KEYCODE_DPAD_UP -> ControllerKey.UP
            KeyEvent.KEYCODE_DPAD_DOWN -> ControllerKey.DOWN
            KeyEvent.KEYCODE_DPAD_LEFT -> ControllerKey.LEFT
            KeyEvent.KEYCODE_DPAD_RIGHT -> ControllerKey.RIGHT
            else -> null
        }

    override fun onGenericMotionEvent(event: MotionEvent): Boolean {
        if (event.source and InputDevice.SOURCE_JOYSTICK != InputDevice.SOURCE_JOYSTICK) {
            if (Log.isLoggable(TAG, Log.DEBUG)) {
                Log.d(TAG, "non-joystick motion: source=0x${event.source.toString(16)} device=${event.deviceId}")
            }
            return super.onGenericMotionEvent(event)
        }
        // Uncounted joystick sources (joystick mice, sensors) never divert:
        // consuming them here would eat input the platform owns.
        if (!connectedDeviceIds.contains(event.deviceId)) {
            return super.onGenericMotionEvent(event)
        }
        val hatX = event.getAxisValue(MotionEvent.AXIS_HAT_X)
        val hatY = event.getAxisValue(MotionEvent.AXIS_HAT_Y)
        // The hat is genuinely digital; sticks ride analog floats and Rust
        // derives dpad from them, so Kotlin never digitizes sticks here.
        updateControllerInput(event.deviceId, ControllerKey.LEFT, hatX < -CONTROLLER_AXIS_THRESHOLD)
        updateControllerInput(event.deviceId, ControllerKey.RIGHT, hatX > CONTROLLER_AXIS_THRESHOLD)
        updateControllerInput(event.deviceId, ControllerKey.UP, hatY < -CONTROLLER_AXIS_THRESHOLD)
        updateControllerInput(event.deviceId, ControllerKey.DOWN, hatY > CONTROLLER_AXIS_THRESHOLD)
        // One JNI crossing per motion callback: every axis rides one
        // batched motion event instead of one crossing per axis. Axis ids
        // ride the hoisted table (see MOTION_AXES), so the batch loop
        // stays a single pass with no per-callback locals.
        val crossed = updateControllerAxes(event.deviceId, event)
        if (crossed != null) {
            sensorController.lastControllerDeviceId = event.deviceId
            chromeManager.noteControllerInput()
            sensorController.noteControllerInput()
            sendMotionInput(event.deviceId, crossed)
        }
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

    /** Stable device identity for Rust port assignment: the OS descriptor
     * survives id reuse and reconnects, the raw id does not. Devices that
     * vanish before a descriptor read contribute nothing. */
    private fun sendDeviceAdded(deviceId: Int) {
        // A vanished device contributes no identity; loud here so a pad
        // that never registers stays diagnosable in logcat.
        val descriptor = InputDevice.getDevice(deviceId)?.descriptor ?: run {
            Log.w(TAG, "device_added without descriptor: deviceId=$deviceId")
            return
        }
        sendMenuAction(
            menuEvent(MenuAction.DEVICE_ADDED) {
                put("device_id", deviceId)
                put("descriptor", descriptor)
            },
        )
    }

    private fun sendDeviceRemoved(deviceId: Int) {
        sendMenuAction(
            menuEvent(MenuAction.DEVICE_REMOVED) {
                put("device_id", deviceId)
            },
        )
    }

    /** Single batched motion-event producer: only crossed axes ride the
     * envelope (Rust treats a missing axis as neutral already). Deadzone
     * and dpad substitution live in Rust, never here. */
    private fun sendMotionInput(
        deviceId: Int,
        axes: Map<String, Float>,
    ) {
        sendMenuAction(
            menuEvent(MenuAction.MOTION) {
                put("device_id", deviceId)
                for ((axis, value) in axes) {
                    put(axis, value)
                }
            },
        )
    }

    /** Digital-trigger mirror for pads without analog trigger axes (see
     * dispatch): keys never reach Rust, so the press lands in the cached
     * axis map and ships like any motion batch. */
    private fun updateControllerTrigger(
        deviceId: Int,
        axis: String,
        pressed: Boolean,
    ) {
        val axes = controllerAxes.getOrPut(deviceId) { mutableMapOf() }
        val value = if (pressed) 1f else 0f
        if (axes[axis] == value) return
        axes[axis] = value
        sensorController.lastControllerDeviceId = deviceId
        chromeManager.noteControllerInput()
        sensorController.noteControllerInput()
        sendMotionInput(deviceId, mapOf(axis to value))
    }

    /** Right-stick sample with the RX/RY alias: pads reporting RX/RY
     * never ship Z/RZ, so fall back when the primary range is absent.
     * Cached per device (Binder probe once, not per sample); never keyed
     * on value so calibration noise cannot disable a real axis or shadow
     * it. Null devices never alias and never poison the cache: a vanished
     * device must not pin RX/RY forever. */
    private fun rightStickSample(event: MotionEvent, deviceId: Int, axis: String, id: Int): Float {
        if (axis != ControllerAxis.RIGHT_X && axis != ControllerAxis.RIGHT_Y) {
            return event.getAxisValue(id)
        }
        val device = event.device ?: return event.getAxisValue(id)
        val alias =
            rightStickAlias.getOrPut(deviceId) {
                (device.getMotionRange(MotionEvent.AXIS_Z) == null) to
                    (device.getMotionRange(MotionEvent.AXIS_RZ) == null)
            }
        val useAlias = if (axis == ControllerAxis.RIGHT_X) alias.first else alias.second
        if (!useAlias) return event.getAxisValue(id)
        val aliasAxis = if (axis == ControllerAxis.RIGHT_X) MotionEvent.AXIS_RX else MotionEvent.AXIS_RY
        return event.getAxisValue(aliasAxis)
    }

    /** First-stage Kotlin dedup for the motion stream: motion events fire
     * at sensor rate and Rust only wants crossings, so a callback with no
     * sub-epsilon crossing sends nothing. Neutral first-sends stay local
     * too — Rust treats a missing axis as neutral already. Drift between
     * this epsilon and the Rust per-pad deadzone still crosses JNI per
     * sample (accepted chatter: gating here would need the Rust deadzone
     * pushed to Kotlin). The Rust deadzone/store is the second stage;
     * this cache only saves JNI chatter. Returns the crossed axes (null
     * when nothing crossed), so only changed values ride the envelope.
     * No pair array per callback: names come from the hoisted table. */
    private fun updateControllerAxes(deviceId: Int, event: MotionEvent): Map<String, Float>? {
        val axes = controllerAxes.getOrPut(deviceId) { mutableMapOf() }
        // Digital-trigger mirror owns the trigger slots on axis-less pads:
        // sampling an absent axis here would read 0 and emit a spurious
        // release while the mirrored press is held. Per-axis: skip only
        // the side whose range is absent.
        val present = hasTriggerAxes[deviceId]
        var crossed: MutableMap<String, Float>? = null
        for ((axis, id) in MOTION_AXES) {
            if (axis == ControllerAxis.TRIGGER_L && present?.first == false) continue
            if (axis == ControllerAxis.TRIGGER_R && present?.second == false) continue
            val value = rightStickSample(event, deviceId, axis, id)
            val last = axes[axis]
            if (last != null && kotlin.math.abs(value - last) < CONTROLLER_AXIS_EPSILON) continue
            if (last == null && value == 0f) continue
            axes[axis] = value
            (crossed ?: mutableMapOf<String, Float>().also { crossed = it })[axis] = value
        }
        return crossed
    }

    private fun updateControllerInput(
        deviceId: Int,
        key: String,
        pressed: Boolean,
    ) {
        val keys = controllerPressed.getOrPut(deviceId) { mutableSetOf() }
        val changed = if (pressed) keys.add(key) else keys.remove(key)
        if (!changed) return
        if (Log.isLoggable(TAG, Log.DEBUG)) {
            Log.d(TAG, "controller key: deviceId=$deviceId key=$key pressed=$pressed")
        }
        if (pressed) {
            sensorController.lastControllerDeviceId = deviceId
            chromeManager.noteControllerInput()
            sensorController.noteControllerInput()
        }
        sendControllerInput(deviceId, key, pressed)
        if (keys.isEmpty()) controllerPressed.remove(deviceId)
    }

    private fun releaseController(deviceId: Int) {
        rightStickAlias.remove(deviceId)
        controllerPressed.remove(deviceId)?.toList()?.forEach { key ->
            sendControllerInput(deviceId, key, false)
        }
        // Analog holds converge the same way: one batched recenter so
        // the Rust substitute releases even when the device is gone.
        // Explicit zeros: a missing axis reads as neutral only when never
        // sent, so the recenter names every axis.
        if (controllerAxes.remove(deviceId) == null) return
        sendMotionInput(deviceId, RECENTER_AXES)
    }

    /** Mirror of the Rust tracker for edge-detect and device-removal synthesis;
     * focus loss converges via session clear. `internal` (not private) because
     * the capture window calls this when it loses window focus so a focus
     * steal never strands a hold. */
    internal fun releaseAllControllers() {
        (controllerPressed.keys + controllerAxes.keys).toList().forEach(::releaseController)
    }

    /** Boot sweep: pads connected before resume never fire
     * `onInputDeviceAdded`, so enumerate them once. Counted-only: the
     * routing set, the presence scan, and Rust port assignment all share
     * the same verdict, so ghosts and joystick mice never divert.
     * Clears first: an unplug-while-paused (listener unregistered in
     * onPause) must not leave a stale divert. Listener registration
     * comes first; a racing add re-sends the same descriptor, and Rust
     * collapses duplicates to one index. */
    private fun sendInitialDeviceAdded() {
        connectedDeviceIds.clear()
        hasTriggerAxes.clear()
        rightStickAlias.clear()
        for (deviceId in InputDevice.getDeviceIds()) {
            val device = InputDevice.getDevice(deviceId) ?: continue
            if (!isCountedGamepad(device)) continue
            // Null-descriptor ghosts never enter routing: without an
            // identity Rust cannot port them, so diverting would be
            // dead input.
            if (device.descriptor == null) continue
            connectedDeviceIds.add(deviceId)
            sendDeviceAdded(deviceId)
        }
    }

    override fun onInputDeviceAdded(deviceId: Int) {
        hasTriggerAxes.remove(deviceId)
        rightStickAlias.remove(deviceId)
        val device = InputDevice.getDevice(deviceId)
        // Same counted verdict as the boot sweep; uncounted devices never
        // enter routing and only refresh the overlay gate.
        if (device == null || device.descriptor == null || !isCountedGamepad(device)) {
            refreshControllerPresence()
            return
        }
        connectedDeviceIds.add(deviceId)
        sendDeviceAdded(deviceId)
        refreshControllerPresence()
    }

    override fun onInputDeviceChanged(deviceId: Int) {
        hasTriggerAxes.remove(deviceId)
        rightStickAlias.remove(deviceId)
        // Re-evaluate membership: a device can gain or lose gamepad
        // signals across a change.
        val device = InputDevice.getDevice(deviceId)
        if (device != null && device.descriptor != null && isCountedGamepad(device)) {
            connectedDeviceIds.add(deviceId)
        } else {
            connectedDeviceIds.remove(deviceId)
        }
        refreshControllerPresence()
    }

    override fun onInputDeviceRemoved(deviceId: Int) {
        connectedDeviceIds.remove(deviceId)
        hasTriggerAxes.remove(deviceId)
        rightStickAlias.remove(deviceId)
        releaseController(deviceId)
        sendDeviceRemoved(deviceId)
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
     * would save yet never draw.
     * Scan runs on the shared background lane; only the chrome apply posts
     * back. Identical scans skip the apply via the last-result cache. */
    private fun refreshControllerPresence() {
        val epoch = presenceEpoch
        BridgeIo.runIo {
            val scan =
                try {
                    scanDeviceTruth(packageManager)
                } catch (e: Exception) {
                    Log.e(TAG, "device scan failed", e)
                    return@runIo
                }
            if (scan == lastPresenceTruth) return@runIo
            lastPresenceTruth = scan
            val touchCapable = scan.touchCapable
            val hasGamepads = scan.gamepads.isNotEmpty()
            BridgeIo.postMain {
                if (epoch != presenceEpoch || isDestroyed || isFinishing) return@postMain
                chromeManager.setTouchCapable(touchCapable)
                if (hasGamepads && Log.isLoggable(TAG, Log.DEBUG)) {
                    // Debug breadcrumb for Auto-hide reports: which device hid it.
                    Log.d(TAG, "overlay auto-hide counts gamepad: ${scan.gamepads.map { it.name }.joinToString()}")
                }
                chromeManager.setControllerConnected(hasGamepads)
            }
        }
    }

    override fun onPause() {
        // Invalidate in-flight presence scans first: their postMain apply
        // must not flip the overlay/capture gate while paused. The resume
        // scan re-converges.
        presenceEpoch++
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
        slotListener = null
        rightStickAlias.clear()
        hasTriggerAxes.clear()
        connectedDeviceIds.clear()
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_DESTROY)
        store.clear()
        onActivityDestroyed(userInitiatedExit)
        super.onDestroy()
    }

    /** JNI bridge for menu events from Rust-side handlers. */
    internal fun sendMenuAction(action: String) {
        onMenuAction(action)
    }

    /** Forward the cartridge acceleration sample through the menu lane:
     * same queue as keys and axes, so the UI thread never touches the
     * session and sample ordering is preserved. Decoded by the
     * `cartridge_motion` gate into one motion apply. */
    internal fun forwardCartridgeAcceleration(xG: Float, yG: Float) {
        sendMenuAction(
            menuEvent(MenuAction.CARTRIDGE_MOTION) {
                put("accel_x", xG)
                put("accel_y", yG)
            },
        )
    }

    /** Back stack shared with the gamepad-input capture window (each
     * window has its own dispatcher, so the capture view calls this
     * directly instead of reaching the activity dispatcher). `internal`
     * (not private) for that cross-window call; not a public API leak. */
    internal fun handleBackNavigation() {
        if (dialogManager.isShowing(ERROR_DIALOG_TAG)) {
            dialogManager.dismiss(notifyDismiss = true)
            return
        }
        if (chromeManager.removeDrawerOverlay()) return
        userInitiatedExit = true
        finish()
    }

    // Deprecated upstream; kept because minSdk 28 predates the
    // OnBackInvokedDispatcher replacement (API 33+ path lives above).
    // Always run the shared stack directly: routing API 33+ through
    // super.onBackPressed() re-enters the platform dispatcher instead of
    // the stack the activity owns (the gesture path above already covers
    // real system Back on 33+).
    @SuppressLint("GestureBackNavigation")
    @Suppress("DEPRECATION")
    override fun onBackPressed() {
        // API 33+ owns Back through the dispatcher above; running the
        // stack here too would double-handle one press (dismiss then
        // finish). Below 33 this is the only path.
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) return
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

    fun configureRumbleMotion(
        motionEnabled: Boolean,
        rumbleEnabled: Boolean,
        rumbleStrengthPercent: Int,
        rumbleTarget: String,
    ) {
        sensorController.configure(motionEnabled, rumbleEnabled, rumbleStrengthPercent, rumbleTarget)
    }

    fun setRumble(intensity: Int) {
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
                showErrorDialog(-1L, "Storage permission was denied.")
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

    /** Gamepad lane hooks: the emulator enumerates no pads, so tests drive
     * the connect/remove menu actions with a fake id and descriptor. The
     * payload shape matches the production senders exactly, so decoder,
     * session porting, and shortcut routing are all exercised. */
    fun dispatchDeviceAddedForTest(deviceId: Int, descriptor: String) {
        sendMenuAction(
            menuEvent(MenuAction.DEVICE_ADDED) {
                put("device_id", deviceId)
                put("descriptor", descriptor)
            },
        )
    }

    fun dispatchDeviceRemovedForTest(deviceId: Int) {
        sendMenuAction(
            menuEvent(MenuAction.DEVICE_REMOVED) {
                put("device_id", deviceId)
            },
        )
    }

    /** Routing-set seed for motion dispatch tests: onGenericMotionEvent
     * only diverts ids the OS enumerated, which never happens on the
     * emulator. Scoped to one fake id per test; remove afterwards. */
    fun addTestControllerRouteForTest(deviceId: Int) {
        connectedDeviceIds.add(deviceId)
    }

    fun removeTestControllerRouteForTest(deviceId: Int) {
        connectedDeviceIds.remove(deviceId)
    }

    /** Show a DTO-driven Android settings screen. Presents the versioned sections from Rust.
     * Save and dismiss return request-scoped JSON; edits ride synchronous per-row commands. */
    fun showSettingsDialog(payload: String) {
        val document =
            try {
                JSONObject(payload)
            } catch (e: JSONException) {
                Log.e(TAG, "showSettingsDialog: malformed payload", e)
                showErrorDialog(-1L, "Settings failed to open.")
                return
            }
        if (document.optInt("schemaVersion", -1) != SETTINGS_SCHEMA_VERSION) {
            Log.e(TAG, "showSettingsDialog: unsupported schemaVersion")
            val requestId = document.optLong("requestId", -1L)
            showErrorDialog(requestId, "Unsupported settings schema")
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
     * resolves without the LANG env Android never sets. Terminal-only:
     * Rust applies the saved in-app choice itself, so the vault must
     * never carry it (a saved English would otherwise resolve
     * SystemDefault to English on a Japanese terminal). */
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
        // Malformed payloads carry no gate: dismiss runs no callback so a
        // -1 id never wedges the Rust dialog gate.
        val onDismiss: () -> Unit =
            if (requestId >= 0) ({ onErrorDialogDismissed(requestId) }) else ({})
        dialogManager.show(
            dialogTag = ERROR_DIALOG_TAG,
            contentDescription = message,
            onDismiss = onDismiss,
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

    /** Slot-list query for the drawer submenu (vault mirror, published
     * after every session command). Plain wrapper like the pause query:
     * the external stays private so the JVM name never mangles. */
    private fun querySlotList(): String? =
        try {
            nativeSlotList()
        } catch (e: Exception) {
            Log.e(TAG, "nativeSlotList failed", e)
            null
        }

    private external fun nativeSlotList(): String

    /** Slot-list push target: SlotActivity installs its row updater while
     * open. Rust pushes the refreshed list after every slot op lands, so
     * no polling. Cleared on destroy; a dead screen never receives. */
    fun onSlotListChanged(json: String) {
        slotListener?.invoke(json)
    }

    /** Drawer slot screen: a standalone activity over the live session.
     * Select/create finish back to the game; delete stays and refreshes
     * from the Rust push after the op lands. The initial query runs on the
     * shared background lane; only the activity start posts back. */
    internal fun showSlotMenu() {
        BridgeIo.runIo {
            val raw = querySlotList() ?: return@runIo
            BridgeIo.postMain {
                if (isDestroyed || isFinishing) return@postMain
                startActivity(SlotActivity.createIntent(this, raw, requestedOrientation))
            }
        }
    }

    private external fun onFilePickerResult(uri: String?)

    private external fun onMenuAction(action: String)

    private external fun onErrorDialogDismissed(requestId: Long)

    private external fun onDirectoryPickerResult(uri: String?)

    private external fun onActivityDestroyed(userInitiatedExit: Boolean)

    private external fun onSystemInsetsChanged(insets: IntArray)

    companion object {
        private const val TAG = "Nerust"

        /** See onSlotListChanged. */
        @Volatile
        internal var slotListener: ((String) -> Unit)? = null

        init {
            // Load the native library via the app classloader so the JVM resolves `external fun`.
            // NativeActivity's later dlopen bypasses Java's classloader registration.
            Log.i(TAG, "MainActivity companion: loading native library main")
            System.loadLibrary("main")
            Log.i(TAG, "MainActivity companion: loaded native library main")
        }

        private const val CONTROLLER_AXIS_THRESHOLD = 0.5f
        private const val CONTROLLER_AXIS_EPSILON = 0.01f

        /** Motion axes as (name, id) pairs: one table so names and ids
         * never skew. RX/RY ride as right-stick aliases per sample. */
        private val MOTION_AXES =
            arrayOf(
                ControllerAxis.LEFT_X to MotionEvent.AXIS_X,
                ControllerAxis.LEFT_Y to MotionEvent.AXIS_Y,
                ControllerAxis.RIGHT_X to MotionEvent.AXIS_Z,
                ControllerAxis.RIGHT_Y to MotionEvent.AXIS_RZ,
                ControllerAxis.TRIGGER_L to MotionEvent.AXIS_LTRIGGER,
                ControllerAxis.TRIGGER_R to MotionEvent.AXIS_RTRIGGER,
            )
        /** Analog recenter: every axis at neutral, sent as one batch when
         * a device vanishes so Rust substitutes release. Derived from the
         * single motion table so the set cannot skew. */
        private val RECENTER_AXES = MOTION_AXES.associate { it.first to 0f }
        private const val ROM_PICKER_REQUEST_CODE = 0x4E45
        private const val DIRECTORY_PICKER_REQUEST_CODE = 0x4E46

        @Volatile
        private var activeActivityForTest: MainActivity? = null

        fun createRomPickerIntentForTest(): Intent = createRomPickerIntent()

        fun createDirectoryPickerIntentForTest(): Intent = createDirectoryPickerIntent()

        fun currentActivityForTest(): MainActivity? = activeActivityForTest?.takeUnless { it.isDestroyed || it.isFinishing }
    }
}


