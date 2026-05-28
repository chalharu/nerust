package io.github.chalharu.nerust

import android.app.Instrumentation
import android.content.Context
import android.content.Intent
import android.os.SystemClock
import android.view.View
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class MainActivityE2eTest {
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
                val decorView = activity.window.decorView
                val menuButton = requireNotNull(
                    decorView.findViewWithTag<View>(MENU_BUTTON_TAG),
                ) {
                    "Menu button should be attached after startup"
                }

                assertEquals("Menu", menuButton.contentDescription?.toString())
                assertTrue("Menu button should be visible", menuButton.isShown)
                assertTrue("Menu button should handle clicks", menuButton.performClick())
            }
            instrumentation.waitForIdleSync()

            assertTrue("Drawer overlay should be attached after tapping Menu", waitUntil(DRAWER_TIMEOUT_MS) {
                taggedView(instrumentation, activity, DRAWER_OVERLAY_TAG)?.isShown == true
            })

            instrumentation.runOnMainSync {
                require(!activity.isDestroyed) { "MainActivity should remain alive after opening Menu" }
                val decorView = activity.window.decorView
                val drawerOverlay = requireNotNull(
                    decorView.findViewWithTag<View>(DRAWER_OVERLAY_TAG),
                ) {
                    "Drawer overlay should be attached after tapping Menu"
                }
                assertTrue("Drawer overlay should be visible", drawerOverlay.isShown)
                assertEquals(EXPECTED_DRAWER_CONTENT, drawerOverlay.getTag(R.id.nerust_drawer_content_probe))

                val drawerComposeView = requireNotNull(
                    decorView.findViewWithTag<View>(DRAWER_COMPOSE_TAG),
                ) {
                    "Drawer ComposeView should be attached after tapping Menu"
                }
                assertTrue("Drawer ComposeView should be visible", drawerComposeView.isShown)
            }
        } finally {
            instrumentation.removeMonitor(monitor)
            launchedActivity?.let { activity ->
                instrumentation.runOnMainSync {
                    if (!activity.isFinishing && !activity.isDestroyed) {
                        activity.finish()
                    }
                }
                instrumentation.waitForIdleSync()
            }
        }
    }

    @Test
    fun activityDestroyAndRelaunchKeepsMenuAvailable() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = ApplicationProvider.getApplicationContext<Context>()
        var firstActivity: MainActivity? = null
        var relaunchedActivity: MainActivity? = null

        try {
            val firstMonitor = instrumentation.addMonitor(MainActivity::class.java.name, null, false)
            val first = try {
                launchMainActivity(instrumentation, context, firstMonitor)
            } finally {
                instrumentation.removeMonitor(firstMonitor)
            }.also { firstActivity = it }

            finishActivity(instrumentation, first)
            assertTrue("MainActivity should be destroyed before relaunch", waitUntil(STARTUP_TIMEOUT_MS) {
                first.isDestroyed
            })
            firstActivity = null

            val relaunchMonitor = instrumentation.addMonitor(MainActivity::class.java.name, null, false)
            try {
                relaunchedActivity = launchMainActivity(instrumentation, context, relaunchMonitor)
            } finally {
                instrumentation.removeMonitor(relaunchMonitor)
            }
        } finally {
            listOfNotNull(relaunchedActivity, firstActivity).distinct().forEach { activity ->
                finishActivity(instrumentation, activity)
            }
        }
    }

    private fun launchMainActivity(
        instrumentation: Instrumentation,
        context: Context,
        monitor: Instrumentation.ActivityMonitor,
    ): MainActivity {
        val launchIntent =
            requireNotNull(context.packageManager.getLaunchIntentForPackage(context.packageName)) {
                "Launch intent for ${context.packageName} was not found"
            }
        launchIntent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK)
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
    }

    private fun assertMenuButtonAvailable(instrumentation: Instrumentation, activity: MainActivity) {
        assertTrue("Menu button should be attached after startup", waitUntil(STARTUP_TIMEOUT_MS) {
            taggedView(instrumentation, activity, MENU_BUTTON_TAG)?.isShown == true
        })
    }

    private fun taggedView(
        instrumentation: Instrumentation,
        activity: MainActivity,
        tag: String,
    ): View? {
        var view: View? = null
        instrumentation.runOnMainSync {
            if (!activity.isDestroyed) {
                view = activity.window.decorView.findViewWithTag(tag)
            }
        }
        return view
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
        const val STARTUP_TIMEOUT_MS = 60_000L
    }
}
