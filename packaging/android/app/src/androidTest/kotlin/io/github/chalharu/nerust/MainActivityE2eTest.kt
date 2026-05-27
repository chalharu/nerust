package io.github.chalharu.nerust

import android.view.View
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.ComposeTestRule
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class MainActivityE2eTest {
    @get:Rule
    val composeRule: ComposeTestRule = createEmptyComposeRule()

    @Test
    fun appStartsAndDrawerMenuIsAvailable() {
        val scenario = ActivityScenario.launch(MainActivity::class.java)
        try {
            composeRule.waitUntil(STARTUP_TIMEOUT_MS) {
                taggedView(scenario, MENU_BUTTON_TAG)?.isShown == true
            }

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

            composeRule.waitUntil(DRAWER_TIMEOUT_MS) {
                taggedView(scenario, DRAWER_OVERLAY_TAG)?.isShown == true
            }

            scenario.onActivity { activity ->
                val drawerOverlay = requireNotNull(
                    activity.window.decorView.findViewWithTag<View>(DRAWER_OVERLAY_TAG),
                ) {
                    "Drawer overlay should be attached after tapping Menu"
                }
                assertTrue("Drawer overlay should be visible", drawerOverlay.isShown)
            }

            composeRule.waitUntil(DRAWER_TIMEOUT_MS) {
                composeTextDisplayed("Nerust")
            }
            composeRule.onNodeWithText("Nerust", useUnmergedTree = true).assertIsDisplayed()
            listOf(
                "ROM Library",
                "Settings",
                "Pause / Resume",
                "Save State",
                "Load State",
                "Reset",
            ).forEach { label ->
                composeRule.onNodeWithText(label, useUnmergedTree = true).assertIsDisplayed()
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

    private fun composeTextDisplayed(text: String): Boolean {
        try {
            composeRule.onNodeWithText(text, useUnmergedTree = true).assertIsDisplayed()
            return true
        } catch (_: AssertionError) {
            return false
        } catch (_: IllegalStateException) {
            return false
        }
    }

    private companion object {
        const val DRAWER_OVERLAY_TAG = "nerust-drawer-overlay"
        const val DRAWER_TIMEOUT_MS = 5_000L
        const val MENU_BUTTON_TAG = "nerust-menu-button"
        const val STARTUP_TIMEOUT_MS = 60_000L
    }
}
