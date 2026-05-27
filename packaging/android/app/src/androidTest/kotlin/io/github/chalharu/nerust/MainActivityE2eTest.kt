package io.github.chalharu.nerust

import android.content.Context
import android.content.Intent
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class MainActivityE2eTest {
    @Test
    fun appStartsAndDrawerMenuIsAvailable() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val device = UiDevice.getInstance(instrumentation)
        val context = ApplicationProvider.getApplicationContext<Context>()
        val launchIntent =
            requireNotNull(context.packageManager.getLaunchIntentForPackage(context.packageName)) {
                "Launch intent for ${context.packageName} was not found"
            }

        device.pressHome()
        launchIntent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK)
        context.startActivity(launchIntent)

        val menuButton =
            requireNotNull(device.wait(Until.findObject(By.desc("Menu")), STARTUP_TIMEOUT_MS)) {
                "Menu button should be visible after startup"
            }
        menuButton.click()

        requireNotNull(device.wait(Until.findObject(By.text("Nerust")), DRAWER_TIMEOUT_MS)) {
            "Drawer title should be visible after tapping Menu"
        }

        listOf(
            "ROM Library",
            "Settings",
            "Pause / Resume",
            "Save State",
            "Load State",
            "Reset",
        ).forEach { label ->
            assertTrue(
                "Drawer item '$label' should be visible",
                device.wait(Until.hasObject(By.desc(label)), DRAWER_TIMEOUT_MS),
            )
        }
    }

    private companion object {
        const val STARTUP_TIMEOUT_MS = 60_000L
        const val DRAWER_TIMEOUT_MS = 5_000L
    }
}
