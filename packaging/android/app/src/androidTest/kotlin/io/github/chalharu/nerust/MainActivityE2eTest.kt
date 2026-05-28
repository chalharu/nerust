package io.github.chalharu.nerust

import android.app.Instrumentation
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.SystemClock
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Assume.assumeTrue
import org.junit.FixMethodOrder
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.MethodSorters

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

    @Test
    fun appStartsAndDrawerMenuIsAvailable() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = ApplicationProvider.getApplicationContext<Context>()
        val monitor = instrumentation.addMonitor(MainActivity::class.java.name, null, false)
        var launchedActivity: MainActivity? = null

        try {
            val activity = launchMainActivity(instrumentation, context, monitor).also { launchedActivity = it }

            instrumentation.runOnMainSync {
                require(!activity.isDestroyed) { "MainActivity should remain alive before opening Menu" }
                val menuButton = requireNotNull(
                    activity.findChromeViewForTest(MENU_BUTTON_TAG),
                ) {
                    "Menu button should be attached after startup"
                }

                assertEquals("Menu", menuButton.contentDescription?.toString())
                assertTrue("Menu button should be showing", activity.isChromeViewShowingForTest(MENU_BUTTON_TAG))
                assertTrue("Menu button should handle clicks", menuButton.performClick())
            }
            instrumentation.waitForIdleSync()

            assertChromeViewAvailable(
                instrumentation,
                activity,
                DRAWER_OVERLAY_TAG,
                DRAWER_TIMEOUT_MS,
                "Drawer overlay should be attached after tapping Menu",
            )

            instrumentation.runOnMainSync {
                require(!activity.isDestroyed) { "MainActivity should remain alive after opening Menu" }
                val drawerOverlay = requireNotNull(
                    activity.findChromeViewForTest(DRAWER_OVERLAY_TAG),
                ) {
                    "Drawer overlay should be attached after tapping Menu"
                }
                assertTrue("Drawer overlay should be showing", activity.isChromeViewShowingForTest(DRAWER_OVERLAY_TAG))
                assertEquals(EXPECTED_DRAWER_CONTENT, drawerOverlay.getTag(R.id.nerust_drawer_content_probe))

                val drawerComposeView = requireNotNull(
                    activity.findChromeViewForTest(DRAWER_COMPOSE_TAG),
                ) {
                    "Drawer ComposeView should be attached after tapping Menu"
                }
                assertTrue("Drawer ComposeView should be showing", activity.isChromeViewShowingForTest(DRAWER_COMPOSE_TAG))
            }
        } finally {
            instrumentation.removeMonitor(monitor)
            launchedActivity?.let { activity ->
                finishActivity(instrumentation, activity)
            }
        }
    }

    @Test
    fun lifecycleRecreateKeepsMenuAvailable() {
        assumeTrue(
            "NativeActivity recreate e2e is covered on the latest API emulator",
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.VANILLA_ICE_CREAM,
        )
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = ApplicationProvider.getApplicationContext<Context>()
        var firstActivity: MainActivity? = null
        var recreatedActivity: MainActivity? = null

        try {
            val firstMonitor = instrumentation.addMonitor(MainActivity::class.java.name, null, false)
            val first = try {
                launchMainActivity(instrumentation, context, firstMonitor)
            } finally {
                instrumentation.removeMonitor(firstMonitor)
            }.also { firstActivity = it }

            val recreateMonitor = instrumentation.addMonitor(MainActivity::class.java.name, null, false)
            try {
                instrumentation.runOnMainSync {
                    first.recreate()
                }
                instrumentation.waitForIdleSync()
                val recreated =
                    requireNotNull(recreateMonitor.waitForActivityWithTimeout(STARTUP_TIMEOUT_MS) as? MainActivity) {
                        "MainActivity should be recreated"
                    }
                recreatedActivity = recreated
                assertTrue("MainActivity should be destroyed before recreation completes", waitUntil(STARTUP_TIMEOUT_MS) {
                    first.isDestroyed
                })
                firstActivity = null
                assertMenuButtonAvailable(instrumentation, recreated)
            } finally {
                instrumentation.removeMonitor(recreateMonitor)
            }
        } finally {
            listOfNotNull(recreatedActivity, firstActivity).distinct().forEach { activity ->
                finishActivity(instrumentation, activity)
            }
        }
    }

    private fun launchMainActivity(
        instrumentation: Instrumentation,
        context: Context,
        monitor: Instrumentation.ActivityMonitor,
        clearTask: Boolean = true,
    ): MainActivity {
        val launchIntent =
            requireNotNull(context.packageManager.getLaunchIntentForPackage(context.packageName)) {
                "Launch intent for ${context.packageName} was not found"
            }
        val launchFlags = if (clearTask) {
            Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK
        } else {
            Intent.FLAG_ACTIVITY_NEW_TASK
        }
        launchIntent.addFlags(launchFlags)
        context.startActivity(launchIntent)
        val activity = requireNotNull(monitor.waitForActivityWithTimeout(STARTUP_TIMEOUT_MS) as? MainActivity) {
            "MainActivity should be launched"
        }
        instrumentation.waitForIdleSync()
        assertMenuButtonAvailable(instrumentation, activity)
        return activity
    }

    private fun finishActivity(instrumentation: Instrumentation, activity: MainActivity) {
        instrumentation.runOnMainSync {
            if (!activity.isFinishing && !activity.isDestroyed) {
                activity.finish()
            }
        }
        instrumentation.waitForIdleSync()
        assertTrue("MainActivity should be destroyed after finish", waitUntil(STARTUP_TIMEOUT_MS) {
            activity.isDestroyed
        })
    }

    private fun assertMenuButtonAvailable(instrumentation: Instrumentation, activity: MainActivity) {
        assertChromeViewAvailable(
            instrumentation,
            activity,
            MENU_BUTTON_TAG,
            STARTUP_TIMEOUT_MS,
            "Menu button should be attached after startup",
        )
    }

    private fun assertChromeViewAvailable(
        instrumentation: Instrumentation,
        activity: MainActivity,
        tag: String,
        timeoutMs: Long,
        failureMessage: String,
    ) {
        if (waitUntil(timeoutMs) { chromeViewIsShowing(instrumentation, activity, tag) }) {
            return
        }
        fail("$failureMessage; ${chromeDebugState(instrumentation, activity, tag)}")
    }

    private fun chromeViewIsShowing(
        instrumentation: Instrumentation,
        activity: MainActivity,
        tag: String,
    ): Boolean {
        var showing = false
        instrumentation.runOnMainSync {
            if (!activity.isDestroyed) {
                showing = activity.isChromeViewShowingForTest(tag)
            }
        }
        return showing
    }

    private fun chromeDebugState(
        instrumentation: Instrumentation,
        activity: MainActivity,
        tag: String,
    ): String {
        var state = "activity state unavailable"
        instrumentation.runOnMainSync {
            state = activity.chromeDebugStateForTest(tag)
        }
        return state
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
        const val DRAWER_COMPOSE_TAG = "nerust-drawer-compose"
        const val DRAWER_OVERLAY_TAG = "nerust-drawer-overlay"
        const val DRAWER_TIMEOUT_MS = 5_000L
        const val EXPECTED_DRAWER_CONTENT = "Nerust\nROM Library\nSettings\nPause / Resume\nSave State\nLoad State\nReset"
        const val MENU_BUTTON_TAG = "nerust-menu-button"
        const val POLL_INTERVAL_MS = 50L
        const val ROM_PICKER_REQUIRED_FLAGS =
            Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION
        const val STARTUP_TIMEOUT_MS = 60_000L
    }
}
