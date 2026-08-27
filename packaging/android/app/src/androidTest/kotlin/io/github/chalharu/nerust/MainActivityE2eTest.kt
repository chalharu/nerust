package io.github.chalharu.nerust

import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.SystemClock
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
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
    fun runtimeMeetsMinimumSupportedApi() {
        assertTrue("Android API 28 or newer is required", Build.VERSION.SDK_INT >= 28)
    }

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

    @Test
    fun directoryPickerIntentUsesPersistableReadWriteAccess() {
        val intent = MainActivity.createDirectoryPickerIntentForTest()

        assertEquals(Intent.ACTION_OPEN_DOCUMENT_TREE, intent.action)
        assertTrue(
            intent.flags and DIRECTORY_PICKER_REQUIRED_FLAGS == DIRECTORY_PICKER_REQUIRED_FLAGS,
        )
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
            assertEquals(
                DIALOG_PRESENTATION_FULL_SCREEN,
                dialogRoot.getTag(R.id.nerust_dialog_presentation_probe),
            )
            activity.dismissComposeDialogForTest()
        }

        runOnActivityThread(activity) {
            activity.resetChromeStateForTest()
        }
        exerciseMenuAction(
            activity,
            MENU_ACTION_OPEN_SETTINGS,
            SETTINGS_DIALOG_TAG,
            "Settings dialog should be attached after dispatching open_settings",
        )
    }

    @Test(timeout = TEST_TIMEOUT_MS)
    fun gbcDocumentUriRestoresAfterProcessRestart() {
        val activity = launchActivity()

        assertTrue(
            "GBC document URI should restore after process restart",
            waitUntil(STARTUP_TIMEOUT_MS) { activity.lastLoadedSystemForTest() == "gbc" },
        )
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
                MainActivity.currentActivityForTest()
            }
        if (activity == null) {
            val current = MainActivity.currentActivityForTest()
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

    private fun exerciseMenuAction(
        activity: MainActivity,
        action: String,
        expectedDialogTag: String,
        expectedDialogMessage: String,
    ) {
        SystemClock.sleep(STARTUP_STABILITY_DELAY_MS)
        runOnActivityThread(activity) {
            require(!activity.isDestroyed) { "MainActivity should remain alive before dispatching $action" }
            activity.dispatchMenuActionForTest(action)
        }
        assertChromeViewAvailable(activity, expectedDialogTag, DIALOG_TIMEOUT_MS, expectedDialogMessage)
        runOnActivityThread(activity) {
            activity.resetChromeStateForTest()
        }
        if (!waitUntil(DIALOG_TIMEOUT_MS) { !safeChromeViewIsShowing(activity, expectedDialogTag) }) {
            fail("Dialog for $action should be dismissed after handling the action; ${safeChromeDebugState(activity, expectedDialogTag)}")
        }
        runOnActivityThread(activity) {
            require(!activity.isDestroyed) { "MainActivity should remain alive after dispatching $action" }
        }
    }

    @Test
    fun portraitControlsOverlayMatchesExpectedArrangement() {
        val zones = controlsLayout(1080f, 1920f).associateBy { it.label }
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

    @Test
    fun landscapeControlsStayInsideSafeScreenBounds() {
        val width = 1920f
        val height = 1080f
        val zones = controlsLayout(width, height)
        assertEquals(8, zones.size)
        zones.forEach { zone ->
            assertTrue(zone.x >= 0f && zone.y >= 0f)
            assertTrue(zone.x + zone.width <= width)
            assertTrue(zone.y + zone.height <= height)
        }
        val byLabel = zones.associateBy { it.label }
        assertTrue(requireNotNull(byLabel["RIGHT"]).x < requireNotNull(byLabel["B"]).x)
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
        const val DIALOG_PRESENTATION_FULL_SCREEN = "full_screen"
        const val DRAWER_COMPOSE_TAG = "nerust-drawer-compose"
        const val DRAWER_EDGE_HANDLE_TAG = "nerust-drawer-edge-handle"
        const val DRAWER_OVERLAY_TAG = "nerust-drawer-overlay"
        const val DRAWER_TIMEOUT_MS = 5_000L
        const val EXPECTED_DRAWER_CONTENT =
            "Nerust\nOpen ROM\nSettings\nPause / Resume\nSave State\nLoad State\nReset\nUnload ROM\nExit"
        const val MENU_ACTION_OPEN_SETTINGS = "open_settings"
        const val MENU_BUTTON_TAG = "nerust-menu-button"
        const val POLL_INTERVAL_MS = 50L
        const val ROM_PICKER_REQUIRED_FLAGS =
            Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION
        const val DIRECTORY_PICKER_REQUIRED_FLAGS =
            Intent.FLAG_GRANT_READ_URI_PERMISSION or
                Intent.FLAG_GRANT_WRITE_URI_PERMISSION or
                Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION
        const val SETTINGS_DIALOG_TAG = "nerust-settings-dialog"
        const val STARTUP_STABILITY_DELAY_MS = 2_000L
        const val STARTUP_TIMEOUT_MS = 60_000L
        const val TEST_TIMEOUT_MS = 180_000L
        const val UI_THREAD_TIMEOUT_MS = 10_000L

    }
}
