package io.github.chalharu.nerust

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.hardware.input.InputManager
import android.os.Bundle
import android.util.Log
import android.view.InputDevice
import android.view.KeyEvent
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
        val sections = parseSettingsSections(payload)
        val issues = parseIssues(payload)
        val capture = parseCapture(payload)
        val contentDescription =
            settingsContentDescription(
                sections,
                getString(R.string.settings_title),
                getString(R.string.settings_on),
                getString(R.string.settings_off),
            )
        // Same probe IDs the dialog era used, so E2E keeps reading them.
        window.decorView.setTag(R.id.nerust_dialog_content_probe, contentDescription)
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
                onNavStateChanged = { actions ->
                    navActions = actions
                    // First publish lands after composition; hotplug and
                    // resume pushes below converge to the same state.
                    pushKeyboardEnumeration()
                },
                onSave = {
                    sendResult(dismissed = false)
                    finish()
                },
            )
        }
    }

    override fun onResume() {
        super.onResume()
        activeActivityForTest = this
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_RESUME)
        (getSystemService(INPUT_SERVICE) as InputManager).registerInputDeviceListener(this, null)
        pushKeyboardEnumeration()
    }

    override fun onPause() {
        (getSystemService(INPUT_SERVICE) as InputManager).unregisterInputDeviceListener(this)
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_PAUSE)
        super.onPause()
    }

    override fun onInputDeviceAdded(deviceId: Int) = pushKeyboardEnumeration()

    override fun onInputDeviceRemoved(deviceId: Int) = pushKeyboardEnumeration()

    override fun onInputDeviceChanged(deviceId: Int) = pushKeyboardEnumeration()

    /** Enumerate physical keyboards (Kotlin `InputDevice`, Rust owns labels):
     * the built-in row (device id 0) plus one row per external descriptor.
     * BT/USB are indistinguishable via public API so rows stay agnostic;
     * virtual devices are excluded; duplicate HID descriptors merge. */
    internal fun enumerateKeyboards(): JSONArray {
        val out = JSONArray()
        val seen = mutableSetOf<String>()
        val devices =
            InputDevice.getDeviceIds().toList().mapNotNull { InputDevice.getDevice(it) }.filter {
                it.sources and InputDevice.SOURCE_KEYBOARD == InputDevice.SOURCE_KEYBOARD
            }
        val builtin = devices.firstOrNull { it.id == 0 }
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
                    (android.os.Build.VERSION.SDK_INT >= 29 && device.isVirtual)
            if (virtual) continue
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
    internal fun pushKeyboardEnumeration() {
        val nav = navActions ?: return
        val raw =
            try {
                nativeSettingsCommand(
                    settingsCommandJson("keyboards", 0, 0, "keyboards", enumerateKeyboards()),
                )
            } catch (e: Exception) {
                Log.e(TAG, "keyboard enumeration push failed", e)
                return
            }
        raw?.let { nav.refreshPayload(it) }
    }

    override fun onDestroy() {
        lifecycleRegistry.handleLifecycleEvent(Lifecycle.Event.ON_DESTROY)
        if (this == activeActivityForTest) activeActivityForTest = null
        super.onDestroy()
    }

    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        if (!isControllerSource(event.source)) {
            return routeSettingsKeyEvent(event.keyCode, event.action) ||
                super.dispatchKeyEvent(event)
        }
        return super.dispatchKeyEvent(event)
    }

    @Suppress("DEPRECATION")
    @Deprecated("Back resolves inside the settings nav stack first")
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
        if (action != KeyEvent.ACTION_DOWN && action != KeyEvent.ACTION_UP) return true
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
        ): Intent =
            Intent(context, SettingsActivity::class.java).apply {
                putExtra(EXTRA_SCHEMA_VERSION, schemaVersion)
                putExtra(EXTRA_REQUEST_ID, requestId)
                putExtra(EXTRA_PAYLOAD, payload)
            }

        fun currentActivityForTest(): SettingsActivity? =
            activeActivityForTest?.takeUnless { it.isDestroyed || it.isFinishing }

        private fun isControllerSource(source: Int): Boolean =
            source and InputDevice.SOURCE_GAMEPAD == InputDevice.SOURCE_GAMEPAD ||
                source and InputDevice.SOURCE_DPAD == InputDevice.SOURCE_DPAD ||
                source and InputDevice.SOURCE_JOYSTICK == InputDevice.SOURCE_JOYSTICK
    }
}
