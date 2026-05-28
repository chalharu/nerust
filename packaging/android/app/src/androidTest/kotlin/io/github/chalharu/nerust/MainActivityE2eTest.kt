package io.github.chalharu.nerust

import android.content.Context
import android.content.Intent
import android.os.SystemClock
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.FixMethodOrder
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.MethodSorters
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

@RunWith(AndroidJUnit4::class)
@FixMethodOrder(MethodSorters.NAME_ASCENDING)
class MainActivityE2eTest {
    @Test
    fun romPickerIntentUsesSafPersistableReadAccess() {
        val intent = MainActivity.createRomPickerIntentForTest()

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

    @Test(timeout = TEST_TIMEOUT_MS)
    fun appStartsAndDrawerOpensWithoutVisibleMenuButton() {
        val activity = launchActivity()

        SystemClock.sleep(STARTUP_STABILITY_DELAY_MS)
        assertDrawerHandleAvailable(activity)

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
    }

    @Test(timeout = TEST_TIMEOUT_MS)
    fun composeRomLibraryDialogAppearsWithExpectedEntries() {
        val activity = launchActivity()

        runOnActivityThread(activity) {
            require(!activity.isDestroyed) { "MainActivity should remain alive before opening ROM Library" }
            activity.showRomLibraryDialogForTest(
                arrayOf("Super Mario Bros.", "Metroid"),
                arrayOf("mario", "metroid"),
            )
        }

        assertChromeViewAvailable(
            activity,
            ROM_LIBRARY_DIALOG_TAG,
            DIALOG_TIMEOUT_MS,
            "ROM library dialog should be attached after requesting it",
        )

        runOnActivityThread(activity) {
            val dialogRoot =
                requireNotNull(activity.findChromeViewForTest(ROM_LIBRARY_DIALOG_TAG)) {
                    "ROM library dialog root should be available"
                }
            assertEquals(
                "ROM Library\nImport new ROM…\nSuper Mario Bros.\nMetroid",
                dialogRoot.getTag(R.id.nerust_dialog_content_probe),
            )
            activity.dismissComposeDialogForTest()
        }
    }

    @Test(timeout = TEST_TIMEOUT_MS)
    fun composeSettingsDialogAppearsWithCurrentSelections() {
        val activity = launchActivity()

        runOnActivityThread(activity) {
            require(!activity.isDestroyed) { "MainActivity should remain alive before opening Settings" }
            activity.showSettingsDialogForTest(
                arrayOf("video_filter", "touch_overlay"),
                arrayOf("Video Filter", "Touch Overlay"),
                arrayOf("CRT\tLCD", "On\tOff"),
                arrayOf("1", "0"),
            )
        }

        assertChromeViewAvailable(
            activity,
            SETTINGS_DIALOG_TAG,
            DIALOG_TIMEOUT_MS,
            "Settings dialog should be attached after requesting it",
        )

        runOnActivityThread(activity) {
            val dialogRoot =
                requireNotNull(activity.findChromeViewForTest(SETTINGS_DIALOG_TAG)) {
                    "Settings dialog root should be available"
                }
            assertEquals(
                "Settings\nVideo Filter: LCD\nTouch Overlay: On",
                dialogRoot.getTag(R.id.nerust_dialog_content_probe),
            )
            activity.dismissComposeDialogForTest()
        }
    }

    @Test
    fun portraitControlsOverlayMatchesExpectedArrangement() {
        val zones = portraitControlsLayout(1080f, 1920f).associateBy { it.label }
        val up = requireNotNull(zones["UP"])
        val down = requireNotNull(zones["DOWN"])
        val left = requireNotNull(zones["LEFT"])
        val right = requireNotNull(zones["RIGHT"])
        val select = requireNotNull(zones["SELECT"])
        val start = requireNotNull(zones["START"])
        val b = requireNotNull(zones["B"])
        val a = requireNotNull(zones["A"])

        assertEquals(up.x, down.x, 0.01f)
        assertEquals(up.width, down.width, 0.01f)
        assertEquals(left.y, right.y, 0.01f)
        assertEquals(left.height, right.height, 0.01f)
        assertEquals(a.y, b.y, 0.01f)
        assertEquals(a.height, b.height, 0.01f)
        assertTrue("B should sit to the right of the D-pad", b.x > right.x + right.width)
        assertTrue("A should sit to the right of B", a.x > b.x + b.width)
        assertTrue("Select should sit above the face buttons", select.y + select.height < b.y)
        assertTrue("Start should sit above the face buttons", start.y + start.height < a.y)
        assertTrue("Select should sit between the D-pad and face buttons", select.x > left.x + left.width)
        assertTrue("Start should sit between the D-pad and face buttons", start.x + start.width < b.x)
    }

    @Test(timeout = TEST_TIMEOUT_MS)
    fun menuRomLibraryActionInvokesNativeCallbackWithoutCrashing() {
        exerciseMenuAction(MENU_ACTION_OPEN_LIBRARY)
    }

    @Test(timeout = TEST_TIMEOUT_MS)
    fun menuSettingsActionInvokesNativeCallbackWithoutCrashing() {
        exerciseMenuAction(MENU_ACTION_OPEN_SETTINGS)
    }

    private fun launchActivity(clearTask: Boolean = true): MainActivity {
        sharedActivity
            ?.takeUnless { it.isDestroyed || it.isFinishing }
            ?.let { activity ->
                try {
                    runOnActivityThread(activity) {
                        activity.resetChromeStateForTest()
                    }
                    assertDrawerHandleAvailable(activity)
                    return activity
                } catch (_: Throwable) {
                    sharedActivity = null
                }
            }

        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = ApplicationProvider.getApplicationContext<Context>()
        val monitor = instrumentation.addMonitor(MainActivity::class.java.name, null, false)
        val launchIntent =
            requireNotNull(context.packageManager.getLaunchIntentForPackage(context.packageName)) {
                "Launch intent for ${context.packageName} was not found"
            }
        val launchFlags =
            if (clearTask) {
                Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK
            } else {
                Intent.FLAG_ACTIVITY_NEW_TASK
            }
        launchIntent.addFlags(launchFlags)

        try {
            context.startActivity(launchIntent)
            val activity =
                (monitor.waitForActivityWithTimeout(STARTUP_TIMEOUT_MS) as? MainActivity)
                    ?: throw IllegalArgumentException("MainActivity should be launched")
            sharedActivity = activity
            assertDrawerHandleAvailable(activity)
            return activity
        } finally {
            instrumentation.removeMonitor(monitor)
        }
    }

    private fun assertDrawerHandleAvailable(activity: MainActivity) {
        assertChromeViewAvailable(
            activity,
            DRAWER_EDGE_HANDLE_TAG,
            STARTUP_TIMEOUT_MS,
            "Drawer edge handle should be attached after startup",
        )
    }

    private fun exerciseMenuAction(action: String) {
        val activity = launchActivity()

        SystemClock.sleep(STARTUP_STABILITY_DELAY_MS)
        runOnActivityThread(activity) {
            require(!activity.isDestroyed) { "MainActivity should remain alive before dispatching $action" }
            activity.dispatchMenuActionForTest(action)
        }
        SystemClock.sleep(MENU_ACTION_SETTLE_DELAY_MS)
        runOnActivityThread(activity) {
            require(!activity.isDestroyed) { "MainActivity should remain alive after dispatching $action" }
        }
    }

    private fun assertChromeViewAvailable(
        activity: MainActivity,
        tag: String,
        timeoutMs: Long,
        failureMessage: String,
    ) {
        if (waitUntil(timeoutMs) { chromeViewIsShowing(activity, tag) }) {
            return
        }
        fail("$failureMessage; ${chromeDebugState(activity, tag)}")
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

    private companion object {
        const val DIALOG_TIMEOUT_MS = 5_000L
        const val DRAWER_COMPOSE_TAG = "nerust-drawer-compose"
        const val DRAWER_EDGE_HANDLE_TAG = "nerust-drawer-edge-handle"
        const val DRAWER_OVERLAY_TAG = "nerust-drawer-overlay"
        const val DRAWER_TIMEOUT_MS = 5_000L
        const val EXPECTED_DRAWER_CONTENT =
            "Nerust\nROM Library\nSettings\nPause / Resume\nSave State\nLoad State\nReset"
        const val MENU_ACTION_OPEN_LIBRARY = "open_library"
        const val MENU_ACTION_OPEN_SETTINGS = "open_settings"
        const val MENU_BUTTON_TAG = "nerust-menu-button"
        const val MENU_ACTION_SETTLE_DELAY_MS = 1_000L
        const val POLL_INTERVAL_MS = 50L
        const val ROM_LIBRARY_DIALOG_TAG = "nerust-rom-library-dialog"
        const val ROM_PICKER_REQUIRED_FLAGS =
            Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION
        const val SETTINGS_DIALOG_TAG = "nerust-settings-dialog"
        const val STARTUP_STABILITY_DELAY_MS = 2_000L
        const val STARTUP_TIMEOUT_MS = 60_000L
        const val TEST_TIMEOUT_MS = 180_000L
        const val UI_THREAD_TIMEOUT_MS = 10_000L

        @Volatile
        var sharedActivity: MainActivity? = null
    }
}
