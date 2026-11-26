package io.github.chalharu.nerust

import android.content.Intent
import android.util.Log
import android.view.View

/** Instrumentation-test entry points for [MainActivity]. Debug source set
 * only: none of this ships in release. Each hook drives the same production
 * path the UI uses (menu lane, routing set, session), never a test-only
 * branch, so the tests cannot pass through a shortcut reality misses.
 *
 * The connect/remove menu actions stay covered by the verify-contract
 * producer gate, which scans this file alongside the main sources.
 * Pure observers (pause state, cartridge samples) need no hook: tests call
 * the `internal` members directly.
 */

internal fun MainActivity.lastLoadedSystemForTest(): String? = lastLoadedSystem

internal fun MainActivity.loadRomUriForTest(uri: String) {
    lastLoadedSystem = null
    Log.i("Nerust", "loadRomUriForTest: submitting URI $uri")
    onFilePickerResult(uri)
}

internal fun MainActivity.dispatchMenuActionForTest(action: String) {
    chromeManager.dispatchMenuAction(action)
}

internal fun MainActivity.openDrawerForTest() {
    chromeManager.showDrawerOverlay()
}

internal fun MainActivity.dismissComposeDialogForTest() {
    SettingsActivity.currentActivity()?.finishForReplacement()
        ?: dialogManager.dismissForTest()
}

internal fun MainActivity.resetChromeStateForTest() {
    SettingsActivity.currentActivity()?.finish()
    dialogManager.resetForTest()
    chromeManager.removeDrawerOverlay()
}

internal fun MainActivity.floatingDpadStateForTest(): FloatArray? = chromeManager.floatingDpadStateForTest()

internal fun MainActivity.controlsZoneCountForTest(): Int = chromeManager.controlsZoneCountForTest()

internal fun MainActivity.drawerScrollMaxForTest(): Int? = chromeManager.drawerScrollMaxForTest()

internal fun MainActivity.isChromeViewShowingForTest(tag: String): Boolean =
    chromeManager.isViewShowingForTest(tag) ?: when (tag) {
        SETTINGS_SCREEN_TAG -> {
            val settings = SettingsActivity.currentActivity()
            settings != null && !settings.isFinishing && !settings.isDestroyed
        }

        else -> {
            window.decorView.findViewWithTag<View>(tag).isShownInWindowForTest()
        }
    }

internal fun MainActivity.findChromeViewForTest(tag: String): View? =
    chromeManager.findViewForTest(tag) ?: when (tag) {
        SETTINGS_SCREEN_TAG -> {
            SettingsActivity.currentActivity()?.window?.decorView
        }

        else -> {
            window.decorView.findViewWithTag(tag)
        }
    }

internal fun MainActivity.chromeDebugStateForTest(tag: String): String =
    "tag=$tag, destroyed=$isDestroyed, finishing=$isFinishing, decor=${window.decorView.debugViewState()}, " +
        chromeManager.debugStateForTest() + dialogManager.debugState() +
        ", settings=${SettingsActivity.currentActivity()?.isFinishing}" +
        ", lastDialog=$lastDialogState"

/** Gamepad lane hooks: the emulator enumerates no pads, so tests drive
 * the connect/remove menu actions with a fake id and descriptor. The
 * payload shape matches the production senders exactly, so decoder,
 * session porting, and shortcut routing are all exercised. */
internal fun MainActivity.dispatchDeviceAddedForTest(deviceId: Int, descriptor: String) {
    sendMenuAction(
        menuEvent(MenuAction.DEVICE_ADDED) {
            put("device_id", deviceId)
            put("descriptor", descriptor)
        },
    )
}

internal fun MainActivity.dispatchDeviceRemovedForTest(deviceId: Int) {
    sendMenuAction(
        menuEvent(MenuAction.DEVICE_REMOVED) {
            put("device_id", deviceId)
        },
    )
}

/** Routing-set seed for motion dispatch tests: onGenericMotionEvent
 * only diverts ids the OS enumerated, which never happens on the
 * emulator. Scoped to one fake id per test; remove afterwards. */
internal fun MainActivity.addTestControllerRouteForTest(deviceId: Int) {
    connectedDeviceIds.add(deviceId)
}

internal fun MainActivity.removeTestControllerRouteForTest(deviceId: Int) {
    connectedDeviceIds.remove(deviceId)
}

internal fun MainActivity.Companion.createRomPickerIntentForTest(): Intent = createRomPickerIntent()

internal fun MainActivity.Companion.createDirectoryPickerIntentForTest(): Intent = createDirectoryPickerIntent()

internal fun MainActivity.Companion.currentActivity(): MainActivity? =
    activeActivity?.takeUnless { it.isDestroyed || it.isFinishing }

private fun View?.isShownInWindowForTest(): Boolean =
    this != null && visibility == View.VISIBLE && isAttachedToWindow && windowToken != null
