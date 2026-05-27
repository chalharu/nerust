package io.github.chalharu.nerust

import android.view.View
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class MainActivityE2eTest {
    @get:Rule
    val composeRule = createAndroidComposeRule<MainActivity>()

    @Test
    fun appStartsAndDrawerMenuIsAvailable() {
        composeRule.runOnUiThread {
            val menuButton = requireNotNull(
                composeRule.activity.window.decorView.findViewWithTag<View>(MENU_BUTTON_TAG),
            ) {
                "Menu button should be attached after startup"
            }

            assertEquals("Menu", menuButton.contentDescription?.toString())
            assertTrue("Menu button should be visible", menuButton.isShown)
            assertTrue("Menu button should handle clicks", menuButton.performClick())
        }
        composeRule.waitForIdle()
        composeRule.waitUntil(DRAWER_TIMEOUT_MS) {
            drawerOverlayVisible()
        }

        composeRule.runOnUiThread {
            val drawerOverlay = requireNotNull(
                composeRule.activity.window.decorView.findViewWithTag<View>(DRAWER_OVERLAY_TAG),
            ) {
                "Drawer overlay should be attached after tapping Menu"
            }
            assertTrue("Drawer overlay should be visible", drawerOverlay.isShown)
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
    }

    private fun drawerOverlayVisible(): Boolean {
        var visible = false
        composeRule.runOnUiThread {
            visible = composeRule.activity.window.decorView
                .findViewWithTag<View>(DRAWER_OVERLAY_TAG)
                ?.isShown == true
        }
        return visible
    }

    private companion object {
        const val DRAWER_OVERLAY_TAG = "nerust-drawer-overlay"
        const val DRAWER_TIMEOUT_MS = 5_000L
        const val MENU_BUTTON_TAG = "nerust-menu-button"
    }
}
