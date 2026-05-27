package io.github.chalharu.nerust

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.ext.junit.rules.ActivityScenarioRule
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class MainActivityE2eTest {
    private val composeRule = createEmptyComposeRule()
    private val activityRule = ActivityScenarioRule(MainActivity::class.java)

    @get:Rule
    val ruleChain: RuleChain = RuleChain.outerRule(composeRule).around(activityRule)

    @Test
    fun appStartsAndDrawerMenuIsAvailable() {
        composeRule.waitUntil(timeoutMillis = STARTUP_TIMEOUT_MS) {
            try {
                composeRule
                    .onAllNodesWithContentDescription("Menu")
                    .fetchSemanticsNodes()
                    .isNotEmpty()
            } catch (_: IllegalStateException) {
                false
            }
        }

        composeRule.onNodeWithContentDescription("Menu").assertIsDisplayed().performClick()
        composeRule.onNodeWithText("Nerust").assertIsDisplayed()

        listOf(
            "ROM Library",
            "Settings",
            "Pause / Resume",
            "Save State",
            "Load State",
            "Reset",
        ).forEach { label ->
            composeRule.onNodeWithContentDescription(label).assertIsDisplayed()
        }
    }

    private companion object {
        const val STARTUP_TIMEOUT_MS = 45_000L
    }
}
