package io.github.chalharu.nerust

import android.content.Context
import android.content.Intent
import android.content.pm.ActivityInfo
import android.content.res.Configuration
import android.os.Build
import android.os.SystemClock
import android.util.Log
import android.view.InputDevice
import android.view.KeyEvent
import android.view.MotionEvent
import androidx.test.core.app.ApplicationProvider
import org.json.JSONArray
import org.json.JSONObject
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import org.junit.runner.RunWith
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import kotlin.concurrent.thread

@RunWith(AndroidJUnit4::class)
class MainActivityE2eTest {
    @Test
    fun runtimeMeetsMinimumSupportedApi() {
        assertTrue("Android API 28 or newer is required", Build.VERSION.SDK_INT >= 28)
    }

    @Test
    fun romPickerIntentUsesSafPersistableReadAccess() {
        val intent = createRomPickerIntent()

        assertEquals(Intent.ACTION_OPEN_DOCUMENT, intent.action)
        assertEquals("*/*", intent.type)
        assertTrue(
            "ROM picker should only return openable documents",
            intent.categories?.contains(Intent.CATEGORY_OPENABLE) == true,
        )
        assertTrue(
            "ROM picker should request read and persistable URI grants",
            intent.flags and ROM_PICKER_REQUIRED_FLAGS == ROM_PICKER_REQUIRED_FLAGS,
        )
    }

    @Test
    fun directoryPickerIntentUsesPersistableReadWriteAccess() {
        val intent = createDirectoryPickerIntent()

        assertEquals(Intent.ACTION_OPEN_DOCUMENT_TREE, intent.action)
        assertTrue(
            intent.flags and DIRECTORY_PICKER_REQUIRED_FLAGS == DIRECTORY_PICKER_REQUIRED_FLAGS,
        )
    }

    @Test(timeout = TEST_TIMEOUT_MS)
    fun delayedDismissCannotAcknowledgeReplacementDialog() {
        val activity = launchActivity()
        val firstDismiss = AtomicReference<(() -> Unit)?>(null)
        var oldAcks = 0
        var newAcks = 0
        lateinit var manager: DialogManager
        runOnActivityThread(activity) {
            manager = DialogManager(activity)
            manager.show(
                dialogTag = "audit-first",
                contentDescription = "first",
                onDismiss = { oldAcks++ },
            ) { dismiss -> firstDismiss.set(dismiss) }
        }
        try {
            assertTrue(waitUntil(DIALOG_TIMEOUT_MS) { firstDismiss.get() != null })
            runOnActivityThread(activity) {
                // Dialog.dismiss queues its listener. Replace before that
                // queued notification runs, on the very same UI turn.
                requireNotNull(firstDismiss.get()).invoke()
                manager.show(
                    dialogTag = "audit-replacement",
                    contentDescription = "replacement",
                    onDismiss = { newAcks++ },
                ) { _ -> }
            }
            // The old queued notification runs before this next UI turn.
            runOnActivityThread(activity) {
                assertEquals(1, oldAcks)
                assertEquals(0, newAcks)
                assertTrue(manager.isShowing("audit-replacement"))
                manager.dismissForStop()
                assertEquals(1, newAcks)
            }
        } finally {
            runOnActivityThread(activity) { manager.dismissForStop() }
        }
    }

    @Suppress("DEPRECATION")
    @Test(timeout = TEST_TIMEOUT_MS)
    fun appLoadsGbcAndSupportsDrawerDialogsAndMenuActions() {
        val activity = launchActivity()

        runOnActivityThread(activity) {
            activity.loadRomUriForTest(TestRomProvider.ROM_URI.toString())
        }
        assertTrue(
            "GBC document URI should be detected and loaded",
            waitUntil(STARTUP_TIMEOUT_MS) { activity.lastLoadedSystemForTest() == "gbc" },
        )

        val stateFilesBefore = stateSlotFiles()
        runOnActivityThread(activity) {
            activity.dispatchMenuActionForTest(MenuAction.SAVE_STATE)
        }
        assertTrue(
            "GBC Save State should create a state archive",
            waitUntil(DIALOG_TIMEOUT_MS) { (stateSlotFiles() - stateFilesBefore).isNotEmpty() },
        )

        SystemClock.sleep(STARTUP_STABILITY_DELAY_MS)
        assertDrawerHandleAvailable(activity)

        runOnActivityThread(activity) {
            activity.updateFloatingDpad(true, 400f, 1300f, 500f, 1200f)
            assertArrayEquals(
                floatArrayOf(1f, 400f, 1300f, 500f, 1200f),
                requireNotNull(activity.floatingDpadStateForTest()),
                0.01f,
            )
            activity.updateFloatingDpad(false, 0f, 0f, 0f, 0f)
            assertEquals(0f, requireNotNull(activity.floatingDpadStateForTest())[0], 0.01f)
        }

        runOnActivityThread(activity) {
            require(!activity.isDestroyed) { "MainActivity should remain alive before opening drawer" }
            assertNull(
                "Visible menu button should be removed in swipe-only drawer mode",
                activity.findChromeViewForTest(MENU_BUTTON_TAG),
            )
            activity.openDrawerForTest()
        }

        assertChromeViewAvailable(
            activity,
            DRAWER_OVERLAY_TAG,
            DRAWER_TIMEOUT_MS,
            "Drawer overlay should be attached after opening the drawer",
        )

        runOnActivityThread(activity) {
            require(!activity.isDestroyed) { "MainActivity should remain alive after opening drawer" }
            val drawerOverlay =
                requireNotNull(activity.findChromeViewForTest(DRAWER_OVERLAY_TAG)) {
                    "Drawer overlay should be attached after opening drawer"
                }
            assertTrue("Drawer overlay should be showing", activity.isChromeViewShowingForTest(DRAWER_OVERLAY_TAG))
            assertEquals(EXPECTED_DRAWER_CONTENT, drawerOverlay.getTag(R.id.nerust_drawer_content_probe))

            val drawerComposeView =
                requireNotNull(activity.findChromeViewForTest(DRAWER_COMPOSE_TAG)) {
                    "Drawer ComposeView should be attached after opening drawer"
                }
            assertTrue("Drawer ComposeView should be showing", activity.isChromeViewShowingForTest(DRAWER_COMPOSE_TAG))
            activity.onBackPressed()
        }

        runOnActivityThread(activity) {
            require(!activity.isDestroyed) { "MainActivity should remain alive before opening Settings" }
            launchSettingsForTest(
                listOf(
                    SettingsSection(
                        id = "video",
                        label = "Video",
                        rows =
                            listOf(
                                SettingsRow(
                                    key = "video_filter",
                                    label = "Video Filter",
                                    kind = "choice",
                                    choices = listOf(SettingsChoice("CRT"), SettingsChoice("LCD")),
                                    selected = 1,
                                    number = 0L,
                                    step = 1L,
                                    unit = "",
                                    text = "",
                                ),
                            ),
                    ),
                    SettingsSection(
                        id = "controls",
                        label = "Controls",
                        rows =
                            listOf(
                                SettingsRow(
                                    key = "touch_overlay",
                                    label = "Touch Overlay",
                                    kind = "toggle",
                                    choices = listOf(SettingsChoice("On"), SettingsChoice("Off")),
                                    selected = 0,
                                    number = 0L,
                                    step = 1L,
                                    unit = "",
                                    text = "",
                                ),
                            ),
                    ),
                ),
            )
        }

        assertTrue(
            "SettingsActivity should start after requesting it",
            waitUntil(DIALOG_TIMEOUT_MS) { SettingsActivity.currentActivity() != null },
        )

        runOnActivityThread(activity) {
            val settingsRoot =
                requireNotNull(activity.findChromeViewForTest(SETTINGS_SCREEN_TAG)) {
                    "SettingsActivity decor should be available"
                }
            assertTrue(
                "Settings should run in its own activity",
                SettingsActivity.currentActivity() != null,
            )
            assertEquals(
                "Settings\nVideo Filter: LCD\nTouch Overlay: On",
                settingsRoot.getTag(R.id.nerust_settings_content_probe),
            )
            assertEquals(
                "Video: 1 setting\nControls: 1 setting",
                settingsRoot.getTag(R.id.nerust_settings_hierarchy_probe),
            )
            activity.dismissComposeDialogForTest()
        }
        assertTrue(
            "SettingsActivity should finish after dismissal",
            waitUntil(DIALOG_TIMEOUT_MS) { SettingsActivity.currentActivity() == null },
        )

        runOnActivityThread(activity) {
            activity.resetChromeStateForTest()
        }
        runOnActivityThread(activity) {
            require(!activity.isDestroyed) { "MainActivity should remain alive before dispatching open_settings" }
            activity.dispatchMenuActionForTest(MenuAction.OPEN_SETTINGS)
        }
        assertTrue(
            "SettingsActivity should start after dispatching open_settings",
            waitUntil(DIALOG_TIMEOUT_MS) { SettingsActivity.currentActivity() != null },
        )
        runOnActivityThread(activity) {
            activity.resetChromeStateForTest()
        }
        assertTrue(
            "SettingsActivity should finish after reset",
            waitUntil(DIALOG_TIMEOUT_MS) { SettingsActivity.currentActivity() == null },
        )

        runOnActivityThread(activity) {
            activity.moveTaskToBack(true)
        }
        assertTrue(
            "MainActivity should suspend before process restart",
            waitUntil(DIALOG_TIMEOUT_MS) { MainActivity.currentActivity() == null },
        )
        SystemClock.sleep(LIFECYCLE_SAVE_DELAY_MS)
    }

    /** Launch the standalone settings screen with canned sections. */
    private fun launchSettingsForTest(sections: List<SettingsSection>) {
        // Mirror the production envelope: SettingsActivity unwraps "payload".
        val context = ApplicationProvider.getApplicationContext<Context>()
        val envelope = JSONObject().put("payload", sectionsToJson(sections))
        context.startActivity(
            SettingsActivity.createIntent(context, SETTINGS_SCHEMA_VERSION, -1L, envelope.toString())
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
        )
    }

    private fun sectionsToJson(sections: List<SettingsSection>): JSONObject {
        val rowsOf = { rows: List<SettingsRow> ->
            val array = JSONArray()
            rows.forEach { row ->
                val choices = JSONArray()
                row.choices.forEach { choice ->
                    choices.put(
                        JSONObject()
                            .put("label", choice.label)
                            .put("enabled", choice.enabled)
                            .put("reason", choice.reason),
                    )
                }
                array.put(
                    JSONObject()
                        .put("key", row.key)
                        .put("label", row.label)
                        .put("kind", row.kind)
                        .put("choices", choices)
                        .put("selected", row.selected)
                        .put("number", row.number)
                        .put("step", row.step)
                        .put("min", row.min)
                        .put("max", row.max)
                        .put("unit", row.unit)
                        .put("text", row.text)
                        .put("target", row.target)
                        .put("description", row.description)
                        .put("detail_target", row.detailTarget)
                        .put("visible", row.visible),
                )
            }
            array
        }
        val array = JSONArray()
        sections.forEach { section ->
            array.put(
                JSONObject()
                    .put("id", section.id)
                    .put("label", section.label)
                    .put("rows", rowsOf(section.rows))
                    .put("visible", section.visible)
                    .put("sub", section.sub),
            )
        }
        return JSONObject()
            .put("sections", array)
            .put("issues", JSONArray())
    }

    @Test(timeout = TEST_TIMEOUT_MS)
    fun gbcDocumentUriRestoresAfterProcessRestart() {
        val activity = launchActivity()

        assertTrue(
            "GBC document URI should restore after process restart",
            waitUntil(STARTUP_TIMEOUT_MS) { activity.lastLoadedSystemForTest() == "gbc" },
        )
    }

    @Test(timeout = TEST_TIMEOUT_MS)
    fun appLaunchesAfterTermination() {
        assertDrawerHandleAvailable(launchActivity())
    }

    @Suppress("DEPRECATION")
    @Test(timeout = TEST_TIMEOUT_MS)
    fun backTerminatesApplicationProcess() {
        val activity = launchActivity()
        watchStuckShutdown("back", activity)
        runOnActivityThread(activity) {
            activity.onBackPressed()
        }
        // Termination itself is verified by e2e.sh (process gone): the test
        // shares the app process, so a clean exit ends the test run here.
        // Only a lingering process reaches the return below.
        SystemClock.sleep(TERMINATION_GRACE_MS)
    }

    @Test(timeout = TEST_TIMEOUT_MS)
    fun exitTerminatesApplicationProcess() {
        val activity = launchActivity()
        watchStuckShutdown("exit", activity)
        runOnActivityThread(activity) {
            activity.dispatchMenuActionForTest(MenuAction.EXIT)
        }
        // Same contract as backTerminatesApplicationProcess above.
        SystemClock.sleep(TERMINATION_GRACE_MS)
    }

    @Test(timeout = TEST_TIMEOUT_MS)
    fun unloadClearsTouchOverlay() {
        val activity = launchActivity()

        runOnActivityThread(activity) {
            activity.loadRomUriForTest(TestRomProvider.ROM_URI.toString())
        }
        assertTrue(
            "GBC document URI should be detected and loaded",
            waitUntil(STARTUP_TIMEOUT_MS) { activity.lastLoadedSystemForTest() == "gbc" },
        )
        // The load path rebuilds the overlay; the JNI push lands asynchronously.
        val zonesBefore =
            waitUntilValue(DIALOG_TIMEOUT_MS) {
                activity.controlsZoneCountForTest().takeIf { it > 0 }
            }
        requireNotNull(zonesBefore) { "Touch overlay should carry zones while a ROM is loaded" }

        runOnActivityThread(activity) {
            activity.dispatchMenuActionForTest(MenuAction.UNLOAD)
        }
        assertTrue(
            "Unload should clear pushed touch zones",
            waitUntil(DIALOG_TIMEOUT_MS) { activity.controlsZoneCountForTest() == 0 },
        )
    }

    @Suppress("DEPRECATION")
    @Test(timeout = TEST_TIMEOUT_MS)
    fun drawerScrollsInLandscape() {
        val activity = launchActivity()

        runOnActivityThread(activity) {
            activity.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE
        }
        assertTrue(
            "Activity should rotate to landscape",
            waitUntil(STARTUP_TIMEOUT_MS) {
                activity.resources.configuration.orientation == Configuration.ORIENTATION_LANDSCAPE
            },
        )
        assertDrawerHandleAvailable(activity)

        try {
            runOnActivityThread(activity) {
                activity.openDrawerForTest()
            }
            assertChromeViewAvailable(
                activity,
                DRAWER_OVERLAY_TAG,
                DRAWER_TIMEOUT_MS,
                "Drawer overlay should be attached in landscape",
            )
            // The sheet overflows the landscape height, so the published
            // range must be measured (finite) and non-zero (scrollable).
            val scrollMax =
                waitUntilValue(DRAWER_TIMEOUT_MS) {
                    activity.drawerScrollMaxForTest()?.takeIf { it != Int.MAX_VALUE }
                }
            requireNotNull(scrollMax) { "Drawer sheet should publish a measured scroll range" }
            assertTrue("Drawer sheet should scroll in landscape (max=$scrollMax)", scrollMax > 0)

            runOnActivityThread(activity) {
                val drawerOverlay =
                    requireNotNull(activity.findChromeViewForTest(DRAWER_OVERLAY_TAG)) {
                        "Drawer overlay should be attached in landscape"
                    }
                assertEquals(
                    EXPECTED_DRAWER_CONTENT,
                    drawerOverlay.getTag(R.id.nerust_drawer_content_probe),
                )
                activity.onBackPressed()
            }
        } finally {
            runOnActivityThread(activity) {
                if (!activity.isDestroyed) {
                    activity.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_PORTRAIT
                }
            }
        }
    }

    @Test(timeout = TEST_TIMEOUT_MS)
    fun gamepadButtonKeysDivertThroughDispatch() {
        val activity = launchActivity()
        val down = gamepadKey(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_BUTTON_A)
        val up = gamepadKey(KeyEvent.ACTION_UP, KeyEvent.KEYCODE_BUTTON_A)
        val unmapped = gamepadKey(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_BUTTON_Z)
        assertTrue("gamepad A press should divert to the session", dispatchKeyOnActivityThread(activity, down))
        assertTrue("gamepad A release should divert to the session", dispatchKeyOnActivityThread(activity, up))
        assertFalse(
            "unmapped gamepad key should fall through to the platform",
            dispatchKeyOnActivityThread(activity, unmapped),
        )
    }

    @Test(timeout = TEST_TIMEOUT_MS)
    fun gamepadGuideButtonTogglesPauseEndToEnd() {
        val activity = launchActivity()
        runOnActivityThread(activity) {
            activity.loadRomUriForTest(TestRomProvider.ROM_URI.toString())
        }
        assertTrue(
            "GBC document URI should be detected and loaded",
            waitUntil(STARTUP_TIMEOUT_MS) { activity.lastLoadedSystemForTest() == "gbc" },
        )
        runOnActivityThread(activity) {
            if (activity.isEmulationPaused()) activity.dispatchMenuActionForTest(MenuAction.TOGGLE_PAUSE)
        }
        assertTrue(
            "emulation should start running",
            waitUntil(DIALOG_TIMEOUT_MS) { !activity.isEmulationPaused() },
        )
        runOnActivityThread(activity) {
            activity.dispatchDeviceAddedForTest(E2E_PAD_ID, E2E_PAD_DESCRIPTOR)
        }
        pressGuideButton(activity)
        assertTrue(
            "Guide press should pause through the pad shortcut",
            waitUntil(DIALOG_TIMEOUT_MS) { activity.isEmulationPaused() },
        )
        // Removal parks the pad in Rust while Kotlin still diverts on the
        // gamepad source. Positive proof: resume first, then press — a
        // still-connected pad would toggle back to paused.
        runOnActivityThread(activity) {
            activity.dispatchDeviceRemovedForTest(E2E_PAD_ID)
        }
        runOnActivityThread(activity) {
            activity.dispatchMenuActionForTest(MenuAction.TOGGLE_PAUSE)
        }
        assertTrue(
            "cleanup should resume emulation",
            waitUntil(DIALOG_TIMEOUT_MS) { !activity.isEmulationPaused() },
        )
        pressGuideButton(activity)
        SystemClock.sleep(PAUSE_SETTLE_MS)
        assertFalse("removed pad must not drive shortcuts", activity.isEmulationPaused())
    }

    @Test(timeout = TEST_TIMEOUT_MS)
    fun gamepadMotionBatchDivertsThroughDispatch() {
        val activity = launchActivity()
        runOnActivityThread(activity) {
            activity.addTestControllerRouteForTest(E2E_PAD_ID)
        }
        try {
            val motion = joystickMove(E2E_PAD_ID, MotionEvent.AXIS_X, 0.8f)
            try {
                assertTrue(
                    "joystick motion should divert to the session",
                    dispatchMotionOnActivityThread(activity, motion),
                )
            } finally {
                motion.recycle()
            }
        } finally {
            runOnActivityThread(activity) {
                activity.removeTestControllerRouteForTest(E2E_PAD_ID)
            }
        }
    }

    @Test(timeout = TEST_TIMEOUT_MS)
    fun cartridgeMotionConfigReachesKotlin() {
        val activity = launchActivity()
        runOnActivityThread(activity) {
            activity.loadRomUriForTest(TestRomProvider.MBC7_ROM_URI.toString())
        }
        assertTrue(
            "MBC7 document URI should be detected and loaded",
            waitUntil(STARTUP_TIMEOUT_MS) { activity.lastLoadedSystemForTest() == "gbc" },
        )
        // Full configure hop: MBC7 ROM arms the accelerometer demand in
        // Rust, the session sync forwards it over JNI, and Kotlin enables
        // motion plus rumble. The motion sample flow itself needs a real
        // sensor; MBC7 latch coverage stays in gbc_core host tests.
        val config =
            waitUntilValue(DIALOG_TIMEOUT_MS) {
                activity.rumbleMotionConfigForTest().takeIf { it.first }
            }
        requireNotNull(config) { "MBC7 load should enable motion in Kotlin" }
        assertTrue("rumble stays enabled from settings", config.second)
    }

    @Test(timeout = TEST_TIMEOUT_MS)
    fun cartridgeAccelerationLaneForwardsSamples() {
        val activity = launchActivity()
        // Wiring guard: quantize+clamp in Kotlin, menu lane, Rust apply.
        // In-range, over-range (clamped to the schema range), and zero
        // samples must all cross without crashing; observable cartridge
        // effects stay with the MBC7 host tests and real-device smoke.
        runOnActivityThread(activity) {
            activity.forwardCartridgeAcceleration(0.5f, -0.25f)
            activity.forwardCartridgeAcceleration(9.5f, -9.5f)
            activity.forwardCartridgeAcceleration(0.0f, 0.0f)
        }
    }

    /** Watchdog for a stuck shutdown: dumps every thread while the test still runs.
     * Destroy is expected within seconds. A linger without destroy — or a
     * destroy that never completes — names the blocker, so always dump:
     * the shared process means these stacks cover the app UI thread too. */
    private fun watchStuckShutdown(tag: String, activity: MainActivity) {
        thread(isDaemon = true, name = "shutdown-watchdog-$tag") {
            SystemClock.sleep(SHUTDOWN_WATCHDOG_MS)
            val destroyed = activity.isDestroyed
            Log.w(TAG, "$tag: shutdown watchdog fired (destroyed=$destroyed); dumping threads")
            for ((stuck, stack) in Thread.getAllStackTraces()) {
                Log.w(TAG, "$tag: thread=${stuck.name} state=${stuck.state}\n\t${stack.joinToString("\n\t")}")
            }
        }
    }

    private fun launchActivity(): MainActivity {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val launchIntent =
            requireNotNull(context.packageManager.getLaunchIntentForPackage(context.packageName)) {
                "Launch intent for ${context.packageName} was not found"
            }
        launchIntent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK)
        context.startActivity(launchIntent)
        val activity =
            waitUntilValue(STARTUP_TIMEOUT_MS) {
                MainActivity.currentActivity()
            }
        if (activity == null) {
            val current = MainActivity.currentActivity()
            if (current != null) {
                fail("MainActivity relaunch did not restore the drawer handle; ${safeChromeDebugState(current, DRAWER_EDGE_HANDLE_TAG)}")
            }
            throw IllegalArgumentException("MainActivity should be launched")
        }
        runOnActivityThread(activity) {
            activity.resetChromeStateForTest()
        }
        assertDrawerHandleAvailable(activity)
        return activity
    }

    private fun assertDrawerHandleAvailable(activity: MainActivity) {
        assertChromeViewAvailable(
            activity,
            DRAWER_EDGE_HANDLE_TAG,
            STARTUP_TIMEOUT_MS,
            "Drawer edge handle should be attached after startup",
        )
    }

    @Test
    fun pushedZonesParseToDrawableRects() {
        // Geometry lives in Rust; Kotlin only parses the pushed payload.
        val zones = parsePushedZones("B:0.70,0.60,0.10,0.10;A:0.85,0.60,0.10,0.10")
        assertEquals(2, zones.size)
        val byLabel = zones.associateBy { it.label }
        val b = requireNotNull(byLabel["B"])
        val a = requireNotNull(byLabel["A"])
        assertEquals(a.y, b.y, 0.0001f)
        assertEquals(a.height, b.height, 0.0001f)
        assertTrue("A should sit to the right of B", a.x > b.x + b.width)
        assertTrue(parsePushedZones("B:bad;A:0.85,0.60,0.10").isEmpty())
        assertTrue(parsePushedZones("").isEmpty())
    }

    @Test
    fun pushedJoystickParsesHomeGeometry() {
        val joystick = requireNotNull(parsePushedJoystick("0.20,0.70,0.12,0.06"))
        assertTrue(joystick.baseRadius > joystick.knobRadius)
        assertTrue(joystick.knobRadius > 0f)
        assertNull(parsePushedJoystick("0.20,0.70,0.12"))
        assertNull(parsePushedJoystick(""))
    }

    private fun assertChromeViewAvailable(
        activity: MainActivity,
        tag: String,
        timeoutMs: Long,
        failureMessage: String,
    ) {
        if (waitUntil(timeoutMs) { safeChromeViewIsShowing(activity, tag) }) {
            return
        }
        fail("$failureMessage; ${safeChromeDebugState(activity, tag)}")
    }

    private fun chromeViewIsShowing(activity: MainActivity, tag: String): Boolean {
        var showing = false
        runOnActivityThread(activity) {
            if (!activity.isDestroyed) {
                showing = activity.isChromeViewShowingForTest(tag)
            }
        }
        return showing
    }

    private fun chromeDebugState(activity: MainActivity, tag: String): String {
        var state = "activity state unavailable"
        runOnActivityThread(activity) {
            state = activity.chromeDebugStateForTest(tag)
        }
        return state
    }

    private fun safeChromeViewIsShowing(activity: MainActivity, tag: String): Boolean =
        try {
            chromeViewIsShowing(activity, tag)
        } catch (_: Throwable) {
            false
        }

    private fun safeChromeDebugState(activity: MainActivity, tag: String): String =
        try {
            chromeDebugState(activity, tag)
        } catch (error: Throwable) {
            "debug unavailable: ${error.message ?: error::class.java.simpleName}"
        }

    private fun pressGuideButton(activity: MainActivity) {
        val down = gamepadKey(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_BUTTON_MODE)
        val up = gamepadKey(KeyEvent.ACTION_UP, KeyEvent.KEYCODE_BUTTON_MODE)
        assertTrue("Guide press should divert to the session", dispatchKeyOnActivityThread(activity, down))
        assertTrue("Guide release should divert to the session", dispatchKeyOnActivityThread(activity, up))
    }

    private fun gamepadKey(action: Int, keyCode: Int): KeyEvent {
        val now = SystemClock.uptimeMillis()
        return KeyEvent(
            now,
            now,
            action,
            keyCode,
            0,
            0,
            E2E_PAD_ID,
            0,
            0,
            InputDevice.SOURCE_GAMEPAD,
        )
    }

    private fun dispatchKeyOnActivityThread(activity: MainActivity, event: KeyEvent): Boolean {
        val result = AtomicBoolean(false)
        runOnActivityThread(activity) {
            result.set(activity.dispatchKeyEvent(event))
        }
        return result.get()
    }

    private fun dispatchMotionOnActivityThread(activity: MainActivity, event: MotionEvent): Boolean {
        val result = AtomicBoolean(false)
        runOnActivityThread(activity) {
            result.set(activity.onGenericMotionEvent(event))
        }
        return result.get()
    }

    private fun joystickMove(deviceId: Int, axis: Int, value: Float): MotionEvent {
        val properties =
            arrayOf(
                MotionEvent.PointerProperties().apply {
                    id = 0
                    toolType = MotionEvent.TOOL_TYPE_FINGER
                },
            )
        val coords =
            arrayOf(
                MotionEvent.PointerCoords().apply {
                    setAxisValue(axis, value)
                },
            )
        val now = SystemClock.uptimeMillis()
        return MotionEvent.obtain(
            now,
            now,
            MotionEvent.ACTION_MOVE,
            1,
            properties,
            coords,
            0,
            0,
            1f,
            1f,
            deviceId,
            0,
            InputDevice.SOURCE_JOYSTICK or InputDevice.SOURCE_GAMEPAD,
            0,
        )
    }

    private fun runOnActivityThread(activity: MainActivity, action: () -> Unit) {
        val completion = CountDownLatch(1)
        val failure = AtomicReference<Throwable?>()
        activity.runOnUiThread {
            try {
                action()
            } catch (error: Throwable) {
                failure.set(error)
            } finally {
                completion.countDown()
            }
        }
        assertTrue(
            "Timed out waiting for MainActivity UI thread work to complete",
            completion.await(UI_THREAD_TIMEOUT_MS, TimeUnit.MILLISECONDS),
        )
        failure.get()?.let { throw it }
    }

    private fun waitUntil(timeoutMs: Long, condition: () -> Boolean): Boolean {
        val deadline = SystemClock.elapsedRealtime() + timeoutMs
        while (SystemClock.elapsedRealtime() <= deadline) {
            if (condition()) {
                return true
            }
            SystemClock.sleep(POLL_INTERVAL_MS)
        }
        return condition()
    }

    private fun stateSlotFiles(): Set<String> {
        val context = ApplicationProvider.getApplicationContext<Context>()
        return context.filesDir
            .walkTopDown()
            .filter { it.isFile && it.extension == "state" }
            .map { it.absolutePath }
            .toSet()
    }

    private fun <T> waitUntilValue(timeoutMs: Long, supplier: () -> T?): T? {
        val deadline = SystemClock.elapsedRealtime() + timeoutMs
        while (SystemClock.elapsedRealtime() <= deadline) {
            supplier()?.let { return it }
            SystemClock.sleep(POLL_INTERVAL_MS)
        }
        return supplier()
    }

    private companion object {
        const val DIALOG_TIMEOUT_MS = 5_000L
        const val E2E_PAD_DESCRIPTOR = "e2e-test-pad"
        const val E2E_PAD_ID = 4242
        const val PAUSE_SETTLE_MS = 2_000L
        const val DRAWER_COMPOSE_TAG = "nerust-drawer-compose"
        const val DRAWER_EDGE_HANDLE_TAG = "nerust-drawer-edge-handle"
        const val DRAWER_OVERLAY_TAG = "nerust-drawer-overlay"
        const val DRAWER_TIMEOUT_MS = 5_000L
        const val EXPECTED_DRAWER_CONTENT =
            "Nerust\nOpen ROM\nSettings\nPause\nSave State\nLoad State\nSlots\nReset\nUnload ROM\nExit"
        const val MENU_BUTTON_TAG = "nerust-menu-button"
        const val LIFECYCLE_SAVE_DELAY_MS = 1_000L
        const val POLL_INTERVAL_MS = 50L
        const val ROM_PICKER_REQUIRED_FLAGS =
            Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION
        const val DIRECTORY_PICKER_REQUIRED_FLAGS =
            Intent.FLAG_GRANT_READ_URI_PERMISSION or
                Intent.FLAG_GRANT_WRITE_URI_PERMISSION or
                Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION
        const val SETTINGS_SCREEN_TAG = "nerust-settings-screen"
        const val SHUTDOWN_WATCHDOG_MS = 30_000L
        const val TAG = "MainActivityE2eTest"
        const val STARTUP_STABILITY_DELAY_MS = 2_000L
        const val STARTUP_TIMEOUT_MS = 60_000L
        // Grace for a clean exit on a loaded runner before e2e.sh verifies
        // the process is gone. Hangs past this plus the harness wait fail
        // with thread stacks (see expect_process_termination).
        const val TERMINATION_GRACE_MS = 30_000L
        const val TEST_TIMEOUT_MS = 180_000L
        const val UI_THREAD_TIMEOUT_MS = 10_000L

    }
}
