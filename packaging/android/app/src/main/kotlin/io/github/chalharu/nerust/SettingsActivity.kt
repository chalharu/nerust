package io.github.chalharu.nerust

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.pm.ActivityInfo
import android.content.pm.PackageManager
import android.hardware.input.InputManager
import android.os.Build
import android.os.Bundle
import android.util.Log
import android.view.InputDevice
import android.view.KeyEvent
import android.window.OnBackInvokedCallback
import android.window.OnBackInvokedDispatcher
import androidx.compose.ui.platform.ComposeView
import androidx.compose.ui.platform.ViewCompositionStrategy
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
import org.json.JSONArray
import org.json.JSONObject

private const val EXTRA_SCHEMA_VERSION = "nerust.extra.SCHEMA_VERSION"
private const val EXTRA_REQUEST_ID = "nerust.extra.REQUEST_ID"
private const val EXTRA_PAYLOAD = "nerust.extra.PAYLOAD"
private const val EXTRA_ORIENTATION = "nerust.extra.ORIENTATION"

/** Standalone settings screen. MainActivity pushes one versioned payload per
 * open (see `request_show_settings_dialog`); edits ride synchronous per-row
 * commands against the Rust vault draft, save/dismiss return request-scoped
 * JSON. Being an Activity (not a Dialog window) `dispatchKeyEvent` fires
 * directly, so no dialog key hook is needed for hardware-key capture. */
class SettingsActivity :
    Activity(),
    LifecycleOwner,
    SavedStateRegistryOwner,
    ViewModelStoreOwner,
    InputManager.InputDeviceListener {
    private val lifecycleRegistry = LifecycleRegistry(this)
    private val registryController = SavedStateRegistryController.create(this)
    private val store = ViewModelStore()
    private var navActions: SettingsNavActions? = null
    private var resultSent = false
    private var requestId: Long = -1L
    // Predictive-back callback (API 33+): without it the gesture finishes
    // the activity, bypassing the in-stack back handling below. Typed as
    // Any so class verification never touches the API-33 type on old OS.
    private var backDispatcherCallback: Any? = null

    override val lifecycle: Lifecycle
        get() = lifecycleRegistry

    override val savedStateRegistry: SavedStateRegistry
        get() = registryController.savedStateRegistry

    override val viewModelStore: ViewModelStore
        get() = store

    override fun attachBaseContext(newBase: Context) {
        super.attachBaseContext(AppLocale.wrap(newBase))
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        registryController.performAttach()
        registryController.performRestore(savedInstanceState)
        super.onCreate(savedInstanceState)
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_CREATE)
        // Follow the game screen: without this an unspecified activity falls
        // back to the system default (portrait with auto-rotate off) even
        // while the game is locked landscape, so the setting screen would
        // rotate away from the screen that opened it. MainActivity passes its
        // live requestedOrientation in the envelope; Auto stays unspecified
        // so both follow the sensor together.
        val extraOrientation =
            intent.getIntExtra(EXTRA_ORIENTATION, ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED)
        if (requestedOrientation != extraOrientation) {
            requestedOrientation = extraOrientation
        }
        requestId = intent.getLongExtra(EXTRA_REQUEST_ID, -1L)
        if (intent.getIntExtra(EXTRA_SCHEMA_VERSION, -1) != SETTINGS_SCHEMA_VERSION) {
            Log.e(TAG, "SettingsActivity: unsupported schemaVersion")
            finish()
            return
        }
        val rawPayload = intent.getStringExtra(EXTRA_PAYLOAD)
        val payload =
            try {
                requireNotNull(rawPayload) { "settings payload missing" }
                JSONObject(rawPayload).getJSONObject("payload")
            } catch (e: Exception) {
                Log.e(TAG, "SettingsActivity: malformed payload", e)
                finish()
                return
            }
        val sections =
            try {
                parseSettingsSections(payload)
            } catch (e: JSONException) {
                Log.e(TAG, "SettingsActivity: malformed sections", e)
                finish()
                return
            }
        val issues = parseIssues(payload)
        val capture = parseCapture(payload)
        val contentDescription =
            settingsContentDescription(
                sections,
                getString(R.string.settings_title),
                getString(R.string.settings_on),
                getString(R.string.settings_off),
            )
        window.decorView.setTag(R.id.nerust_settings_content_probe, contentDescription)
        window.decorView.setTag(
            R.id.nerust_settings_hierarchy_probe,
            sections.joinToString("\n") { section ->
                val count = section.rows.count { it.kind != "header" && it.kind != "button" }
                "${section.label}: ${resources.getQuantityString(R.plurals.settings_count, count, count)}"
            },
        )
        val composeView =
            ComposeView(this).apply {
                setViewCompositionStrategy(ViewCompositionStrategy.DisposeOnDetachedFromWindow)
            }
        setContentView(composeView)
        listOf(window.decorView, composeView).forEach { view ->
            view.setViewTreeLifecycleOwner(this)
            view.setViewTreeSavedStateRegistryOwner(this)
            view.setViewTreeViewModelStoreOwner(this)
        }
        composeView.setContent {
            NerustSettingsDialogCard(
                initialSections = sections,
                initialIssues = issues,
                initialCapture = capture,
                onDismissRequest = { dismissWithResult(dismissed = true) },
                onCommand = { command -> nativeSettingsCommand(command) },
                // Live preview (see previewOrientation): rotate this screen on
                // every draft change; save/dismiss semantics stay in Rust.
                onOrientationChanged = { orientation ->
                    if (requestedOrientation != orientation) {
                        requestedOrientation = orientation
                    }
                },
                onNavStateChanged = { actions ->
                    navActions = actions
                    // First publish lands after composition; hotplug and
                    // resume pushes below converge to the same state.
                    pushDeviceTruth()
                },
                onSave = {
                    sendResult(dismissed = false)
                    finish()
                },
            )
        }
        // Gesture/hardware parity: the title-bar back and the OS back must
        // run the same stack logic, so register the predictive-back path
        // next to the deprecated onBackPressed override below (API 33+).
        if (Build.VERSION.SDK_INT >= 33) {
            val callback =
                OnBackInvokedCallback {
                    if (!handleSettingsBack()) dismissWithResult(dismissed = true)
                }
            onBackInvokedDispatcher.registerOnBackInvokedCallback(
                OnBackInvokedDispatcher.PRIORITY_DEFAULT,
                callback,
            )
            backDispatcherCallback = callback
        }
    }

    override fun onResume() {
        super.onResume()
        activeActivityForTest = this
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_RESUME)
        (getSystemService(INPUT_SERVICE) as InputManager).registerInputDeviceListener(this, null)
        pushDeviceTruth()
    }

    override fun onPause() {
        (getSystemService(INPUT_SERVICE) as InputManager).unregisterInputDeviceListener(this)
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_PAUSE)
        super.onPause()
    }

    override fun onInputDeviceAdded(deviceId: Int) = pushDeviceTruth()

    override fun onInputDeviceRemoved(deviceId: Int) = pushDeviceTruth()

    override fun onInputDeviceChanged(deviceId: Int) = pushDeviceTruth()

    /** Enumerate physical keyboards (Kotlin `InputDevice`): the built-in
     * row (device id 0 with letter keys) plus one row per external
     * descriptor. BT/USB are indistinguishable via public API so rows stay
     * agnostic; virtual devices are excluded; duplicate HID descriptors
     * merge. Button clusters (power keys, headset jacks, touch panels,
     * fingerprint sensors) also expose SOURCE_KEYBOARD, and id 0 is no
     * safer than any other id, so every row must carry letter keys to
     * count as a keyboard — built-in and external share one rule. */
    internal fun enumerateKeyboards(): JSONArray {
        val out = JSONArray()
        val seen = mutableSetOf<String>()
        val devices =
            InputDevice.getDeviceIds().toList().mapNotNull { InputDevice.getDevice(it) }.filter {
                it.sources and InputDevice.SOURCE_KEYBOARD == InputDevice.SOURCE_KEYBOARD
            }
        val builtin =
            devices.firstOrNull {
                it.id == 0 && it.hasKeys(KeyEvent.KEYCODE_A)[0]
            }
        if (builtin != null) {
            seen += "kbd:builtin"
            out.put(
                JSONObject()
                    .put("member", "kbd:builtin")
                    .put("name", builtin.name)
                    .put("builtin", true),
            )
        }
        for (device in devices) {
            if (device.id == 0) continue
            // isVirtual() is API 29+; guard it, falling back to the
            // documented negative-id rule for virtual keyboards.
            val virtual =
                device.id < 0 ||
                    (Build.VERSION.SDK_INT >= 29 && device.isVirtual)
            if (virtual) continue
            // gpio-keys, pmic power keys, headset jacks, touch panels and
            // fingerprint sensors all expose SOURCE_KEYBOARD; a letter key
            // is what separates a keyboard from a button cluster.
            if (!device.hasKeys(KeyEvent.KEYCODE_A)[0]) continue
            val member = "kbd:${device.descriptor}"
            if (!seen.add(member)) continue
            out.put(
                JSONObject()
                    .put("member", member)
                    .put("name", device.name)
                    .put("builtin", false),
            )
        }
        return out
    }

    /** Push the enumeration through the existing command pipe; Rust
     * re-projects and the card applies it without marking the draft dirty. */
    internal fun pushDeviceTruth() {
        pushKeyboardEnumeration()
        pushTouchCapability()
        pushGamepads()
        pushSystemLocale()
    }

    /** One push through the command pipe; Rust re-projects and the card
     * applies it without marking the draft dirty. */
    private fun pushOp(label: String, op: String, payload: Any) {
        val nav = navActions ?: return
        val raw =
            try {
                nativeSettingsCommand(settingsCommandJson(op, 0, 0, op, payload))
            } catch (e: Exception) {
                Log.e(TAG, "$label push failed", e)
                return
            }
        raw?.let { nav.refreshPayload(it) }
    }

    internal fun pushKeyboardEnumeration() {
        pushOp("keyboard enumeration", "keyboards", enumerateKeyboards())
    }

    internal fun pushTouchCapability() {
        pushOp("touch capability", "touch", JSONObject().put("capable", hasTouchscreen(packageManager)))
    }

    internal fun pushSystemLocale() {
        val tag = AppLocale.currentTag() ?: return
        pushOp("locale", "locale", JSONObject().put("lang", tag))
    }

    internal fun pushGamepads() {
        val names = JSONArray()
        connectedGamepadNames().forEach { names.put(JSONObject().put("name", it)) }
        pushOp("gamepad", "gamepads", names)
    }

    override fun onDestroy() {
        if (Build.VERSION.SDK_INT >= 33) {
            (backDispatcherCallback as? OnBackInvokedCallback)?.let {
                onBackInvokedDispatcher.unregisterOnBackInvokedCallback(it)
            }
            backDispatcherCallback = null
        }
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_DESTROY)
        if (this == activeActivityForTest) activeActivityForTest = null
        super.onDestroy()
    }

    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        if (isControllerSource(event.source)) return super.dispatchKeyEvent(event)
        return routeSettingsKeyEvent(event.keyCode, event.action) ||
            super.dispatchKeyEvent(event)
    }

    // Deprecated upstream; kept because minSdk 28 predates the
    // OnBackInvokedDispatcher replacement. Back resolves inside the
    // settings nav stack first (see handleSettingsBack).
    @Suppress("DEPRECATION")
    override fun onBackPressed() {
        if (handleSettingsBack()) return
        dismissWithResult(dismissed = true)
    }

    /** Back resolves capture → navigate up → discard confirm → dismiss. */
    internal fun handleSettingsBack(): Boolean {
        val nav = navActions
        return when {
            nav == null -> false
            nav.isConfirming -> {
                nav.dismissConfirm()
                true
            }
            nav.isCapturing -> {
                nav.cancelCapture()
                true
            }
            nav.canNavigateUp -> {
                nav.navigateUp()
                true
            }
            nav.isDirty -> {
                nav.requestCancel()
                true
            }
            else -> false
        }
    }

    /** Route a key event to Rust while a binding capture is pending. True only
     * when Rust consumed the key; releases and unmapped keys fall through. */
    private fun routeSettingsKeyEvent(
        keyCode: Int,
        action: Int,
    ): Boolean {
        val nav = navActions ?: return false
        if (!nav.isCapturing) return false
        // Non-key actions (e.g. ACTION_MULTIPLE) are not bindings: let the
        // platform handle them instead of swallowing the press.
        if (action != KeyEvent.ACTION_DOWN && action != KeyEvent.ACTION_UP) return false
        val raw =
            try {
                nativeHardwareKey(keyCode, action == KeyEvent.ACTION_DOWN)
            } catch (e: Exception) {
                Log.e(TAG, "nativeHardwareKey failed", e)
                null
            }
        val doc =
            raw?.let {
                try {
                    JSONObject(it)
                } catch (e: JSONException) {
                    null
                }
            }
        if (doc != null && doc.optBoolean("consumed", false)) {
            nav.applyPayload(doc.toString())
            return true
        }
        return false
    }

    internal fun dismissWithResult(dismissed: Boolean) {
        sendResult(dismissed)
        finish()
    }

    /** Dumb host for the Rust dialog gate: settings sends its own result, so
     * an outer screen (error dialog) just finishes it. */
    fun finishForReplacement() {
        dismissWithResult(dismissed = true)
    }

    private fun sendResult(dismissed: Boolean) {
        if (resultSent || requestId < 0) return
        resultSent = true
        try {
            onSettingsDialogResult(
                JSONObject()
                    .put("schemaVersion", SETTINGS_SCHEMA_VERSION)
                    .put("requestId", requestId)
                    .put("dismissed", dismissed)
                    .toString(),
            )
        } catch (e: Exception) {
            Log.e(TAG, "onSettingsDialogResult failed", e)
        }
    }

    private external fun onSettingsDialogResult(result: String?)

    private external fun nativeSettingsCommand(command: String): String?

    private external fun nativeHardwareKey(
        keyCode: Int,
        pressed: Boolean,
    ): String?

    companion object {
        private const val TAG = "SettingsActivity"

        @Volatile
        private var activeActivityForTest: SettingsActivity? = null

        fun createIntent(
            context: Context,
            schemaVersion: Int,
            requestId: Long,
            payload: String,
            orientation: Int = ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED,
        ): Intent =
            Intent(context, SettingsActivity::class.java).apply {
                putExtra(EXTRA_SCHEMA_VERSION, schemaVersion)
                putExtra(EXTRA_REQUEST_ID, requestId)
                putExtra(EXTRA_PAYLOAD, payload)
                putExtra(EXTRA_ORIENTATION, orientation)
            }

        fun currentActivityForTest(): SettingsActivity? =
            activeActivityForTest?.takeUnless { it.isDestroyed || it.isFinishing }
    }
}
