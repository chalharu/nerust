package io.github.chalharu.nerust

import android.content.Context
import android.content.Intent
import android.content.pm.ActivityInfo
import android.content.pm.PackageManager
import android.hardware.input.InputManager
import android.os.Bundle
import android.util.Log
import android.view.KeyEvent
import androidx.activity.ComponentActivity
import androidx.activity.OnBackPressedCallback
import androidx.compose.ui.platform.ComposeView
import androidx.compose.ui.platform.ViewCompositionStrategy
import androidx.lifecycle.setViewTreeLifecycleOwner
import androidx.lifecycle.setViewTreeViewModelStoreOwner
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
    ComponentActivity(),
    InputManager.InputDeviceListener {
    private var navActions: SettingsNavActions? = null
    private var resultSent = false
    private var requestId: Long = -1L

    override fun attachBaseContext(newBase: Context) {
        super.attachBaseContext(AppLocale.wrap(newBase))
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
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
        // Gesture/hardware parity: the title-bar back and the OS back run
        // the same stack logic through the dispatcher (predictive-back on
        // API 33+, compat fallback below, one path either way).
        onBackPressedDispatcher.addCallback(
            this,
            object : OnBackPressedCallback(true) {
                override fun handleOnBackPressed() {
                    if (!handleSettingsBack()) dismissWithResult(dismissed = true)
                }
            },
        )
    }

    override fun onResume() {
        super.onResume()
        activeActivityForTest = this
        (getSystemService(INPUT_SERVICE) as InputManager).registerInputDeviceListener(this, null)
        pushDeviceTruth()
    }

    override fun onPause() {
        (getSystemService(INPUT_SERVICE) as InputManager).unregisterInputDeviceListener(this)
        super.onPause()
    }

    override fun onInputDeviceAdded(deviceId: Int) = pushDeviceTruth()

    override fun onInputDeviceRemoved(deviceId: Int) = pushDeviceTruth()

    override fun onInputDeviceChanged(deviceId: Int) = pushDeviceTruth()

    /** Push the enumeration through the existing command pipe; Rust
     * re-projects and the card applies it without marking the draft dirty. */
    internal fun pushDeviceTruth() {
        val scan = scanDeviceTruth(packageManager)
        val keyboards = JSONArray()
        scan.keyboards.forEach { row ->
            keyboards.put(
                JSONObject()
                    .put("member", row.member)
                    .put("name", row.name)
                    .put("builtin", row.builtin),
            )
        }
        val gamepads = JSONArray()
        scan.gamepadNames.forEach { gamepads.put(JSONObject().put("name", it)) }
        val payload =
            JSONObject()
                .put("keyboards", keyboards)
                .put("capable", scan.touchCapable)
                .put("gamepads", gamepads)
        AppLocale.currentTag()?.let { payload.put("lang", it) }
        pushOp("device truth", "device_truth", payload)
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

    override fun onDestroy() {
        if (this == activeActivityForTest) activeActivityForTest = null
        super.onDestroy()
    }

    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        if (isControllerSource(event.source)) return super.dispatchKeyEvent(event)
        return routeSettingsKeyEvent(event.keyCode, event.action) ||
            super.dispatchKeyEvent(event)
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
