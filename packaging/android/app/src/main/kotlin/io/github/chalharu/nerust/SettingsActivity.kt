package io.github.chalharu.nerust

import android.content.Context
import android.content.Intent
import android.content.pm.ActivityInfo
import android.content.pm.PackageManager
import android.hardware.input.InputManager
import android.os.Bundle
import android.util.Log
import android.view.InputDevice
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
    @Volatile
    private var navActions: SettingsNavActions? = null
    private var resultSent = false
    private var requestId: Long = -1L

    /** Last pushed scan: identical hotplug/resume bursts skip the JNI push. */
    @Volatile
    private var lastDeviceTruth: DeviceTruth? = null

    /** Truth-order pad descriptors from the last scan: capture matches the
     * event device descriptor against this order (descriptors, from the
     * scan `pad:<descriptor>` members, survive id reuse and reconnects;
     * raw device ids do not). Null (keys, uncounted devices) means no
     * per-pad routing. */
    @Volatile
    private var gamepadOrder: List<String> = emptyList()

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
     * re-projects and the card applies it without marking the draft dirty.
     * Scan + JNI run on the shared background lane (never the UI thread);
     * only the nav apply posts back. Identical scans skip the push via the
     * last-result cache. Payload shape is unchanged. */
    internal fun pushDeviceTruth() {
        // The card only exists after the first composition publish. Caching
        // a scan the card can never receive would suppress the real first
        // push (identical-scan skip), stranding device-dependent rows
        // (connected pads, capture enablement) empty until the next hotplug.
        if (navActions == null) return
        BridgeIo.runIo {
            val scan =
                try {
                    scanDeviceTruth(packageManager)
                } catch (e: Exception) {
                    Log.e(TAG, "device scan failed", e)
                    return@runIo
                }
            if (scan == lastDeviceTruth) return@runIo
            lastDeviceTruth = scan
            gamepadOrder = scan.gamepads.map { it.member.substringAfter("pad:") }
            if (Log.isLoggable(TAG, Log.DEBUG)) {
                Log.d(TAG, "device truth scan: keyboards=${scan.keyboards.size} gamepads=${scan.gamepads.map { it.name }}")
            }
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
            scan.gamepads.forEach { gamepads.put(JSONObject().put("member", it.member).put("name", it.name)) }
            val payload =
                JSONObject()
                    .put("keyboards", keyboards)
                    .put("capable", scan.touchCapable)
                    .put("gamepads", gamepads)
            // Terminal locale only: Rust applies the saved in-app choice
            // itself (see MainActivity.pushSystemLocaleToVault).
            AppLocale.currentTag()?.let { payload.put("lang", it) }
            pushOp("device truth", "device_truth", payload)
        }
    }

    /** One push through the command pipe; Rust re-projects and the card
     * applies it without marking the draft dirty. JNI runs on the shared
     * background lane; only the nav apply posts back to the main thread. */
    private fun pushOp(label: String, op: String, payload: JSONObject) {
        if (navActions == null) return
        val command = settingsCommandJson(op, payload)
        BridgeIo.runIo {
            val raw =
                try {
                    nativeSettingsCommand(command)
                } catch (e: Exception) {
                    Log.e(TAG, "$label push failed", e)
                    return@runIo
                }
            if (raw == null) return@runIo
            BridgeIo.postMain { navActions?.refreshPayload(raw) }
        }
    }

    override fun onDestroy() {
        if (this == activeActivityForTest) activeActivityForTest = null
        super.onDestroy()
    }

    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        // Capture wait takes every hardware key first, keyboard and pad
        // alike; unmapped keys fall through to the platform. Outside
        // capture, controller sources keep platform behavior.
        val capturing = navActions?.isCapturing == true
        val controller = isControllerSource(event.source)
        if ((controller || capturing) && Log.isLoggable(TAG, Log.DEBUG)) {
            Log.d(
                TAG,
                "settings key: source=0x${event.source.toString(16)} keyCode=${event.keyCode} action=${event.action} capturing=$capturing",
            )
        }
        if (controller && !capturing) {
            return super.dispatchKeyEvent(event)
        }
        // Descriptor identity: the live id is recycled on reconnect, so
        // the event device resolves to its descriptor and only counted
        // pads in truth order route per-pad; keys and uncounted devices
        // pass null (no per-pad routing, Rust decides the fallback).
        val eventDescriptor = InputDevice.getDevice(event.deviceId)?.descriptor
        val padDescriptor = eventDescriptor?.takeIf { it in gamepadOrder }
        return routeSettingsKeyEvent(event.keyCode, padDescriptor, event.action) ||
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
     * when Rust consumed the key; releases and unmapped keys fall through.
     * `padDescriptor` is the truth-order pad descriptor (null for keys and
     * uncounted devices): per-pad mapping rows only accept their own pad's
     * descriptor, which survives id reuse and reconnects. */
    private fun routeSettingsKeyEvent(
        keyCode: Int,
        padDescriptor: String?,
        action: Int,
    ): Boolean {
        val nav = navActions ?: return false
        if (!nav.isCapturing) return false
        // Non-key actions (e.g. ACTION_MULTIPLE) are not bindings: let the
        // platform handle them instead of swallowing the press.
        if (action != KeyEvent.ACTION_DOWN && action != KeyEvent.ACTION_UP) return false
        val raw =
            try {
                nativeHardwareKey(keyCode, padDescriptor, action == KeyEvent.ACTION_DOWN)
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
        padDescriptor: String?,
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
