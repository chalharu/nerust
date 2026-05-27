package io.github.chalharu.nerust

import android.os.SystemClock
import android.view.View
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class MainActivityE2eTest {
    @Test
    fun appStartsAndDrawerMenuIsAvailable() {
        val scenario = ActivityScenario.launch(MainActivity::class.java)
        try {
            assertTrue("Menu button should be attached after startup", waitUntil(STARTUP_TIMEOUT_MS) {
                taggedView(scenario, MENU_BUTTON_TAG)?.isShown == true
            })

            scenario.onActivity { activity ->
                val menuButton = requireNotNull(
                    activity.window.decorView.findViewWithTag<View>(MENU_BUTTON_TAG),
                ) {
                    "Menu button should be attached after startup"
                }

                assertEquals("Menu", menuButton.contentDescription?.toString())
                assertTrue("Menu button should be visible", menuButton.isShown)
                assertTrue("Menu button should handle clicks", menuButton.performClick())
            }

            assertTrue("Drawer overlay should be attached after tapping Menu", waitUntil(DRAWER_TIMEOUT_MS) {
                taggedView(scenario, DRAWER_OVERLAY_TAG)?.isShown == true
            })

            scenario.onActivity { activity ->
                val drawerOverlay = requireNotNull(
                    activity.window.decorView.findViewWithTag<View>(DRAWER_OVERLAY_TAG),
                ) {
                    "Drawer overlay should be attached after tapping Menu"
                }
                assertTrue("Drawer overlay should be visible", drawerOverlay.isShown)
                assertEquals(EXPECTED_DRAWER_CONTENT, drawerOverlay.getTag(R.id.nerust_drawer_content_probe))

                val drawerComposeView = requireNotNull(
                    activity.window.decorView.findViewWithTag<View>(DRAWER_COMPOSE_TAG),
                ) {
                    "Drawer ComposeView should be attached after tapping Menu"
                }
                assertTrue("Drawer ComposeView should be visible", drawerComposeView.isShown)
            }
        } finally {
            scenario.close()
        }
    }

    private fun taggedView(scenario: ActivityScenario<MainActivity>, tag: String): View? {
        var view: View? = null
        scenario.onActivity { activity ->
            view = activity.window.decorView.findViewWithTag(tag)
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
