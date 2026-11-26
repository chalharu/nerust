package io.github.chalharu.nerust

import android.view.View

/** Instrumentation-test probes for [ChromeManager]. Debug source set only:
 * none of this ships in release. All reads observe production state; no
 * method mutates behavior or branches on test. */

fun ChromeManager.floatingDpadStateForTest(): FloatArray? =
    (controlsOverlayView as? ControlsOverlayView)?.floatingDpadStateForTest()

/** Pushed touch-zone count: 0 while unloaded or hidden. Unload assertions use this. */
fun ChromeManager.controlsZoneCountForTest(): Int = controlsPushedZones.size

/** Drawer sheet scroll range in pixels, null while the drawer is closed. */
fun ChromeManager.drawerScrollMaxForTest(): Int? = drawerScrollState?.maxValue

fun ChromeManager.isViewShowingForTest(tag: String): Boolean? =
    when (tag) {
        CONTROLS_OVERLAY_TAG -> {
            controlsOverlayPopup?.isShowing == true &&
                controlsOverlayView.isShownInWindowForTest()
        }
        DRAWER_COMPOSE_TAG -> {
            drawerShowing && drawerFullScreenPopup?.isShowing == true &&
                drawerComposeView.isShownInWindowForTest()
        }
        DRAWER_EDGE_HANDLE_TAG -> {
            !drawerShowing && drawerChromePopup?.isShowing == true &&
                drawerEdgeHandleView.isShownInWindowForTest()
        }
        DRAWER_OVERLAY_TAG -> {
            drawerShowing && drawerFullScreenPopup?.isShowing == true &&
                drawerOverlayView.isShownInWindowForTest()
        }
        FPS_OVERLAY_TAG -> {
            fpsOverlayPopup?.isShowing == true &&
                fpsOverlayView.isShownInWindowForTest()
        }
        else -> null
    }

fun ChromeManager.findViewForTest(tag: String): View? =
    when (tag) {
        CONTROLS_OVERLAY_TAG -> controlsOverlayView
        DRAWER_COMPOSE_TAG -> drawerComposeView
        DRAWER_EDGE_HANDLE_TAG -> drawerEdgeHandleView
        DRAWER_OVERLAY_TAG -> drawerOverlayView
        FPS_OVERLAY_TAG -> fpsOverlayView
        else -> null
    }

fun ChromeManager.debugStateForTest(): String =
    "chromeAttachEnabled=$chromeAttachEnabled, attachAttempts=$chromeAttachAttempts, attachGiveUps=$chromeAttachGiveUps, " +
        "controlsPopup=${controlsOverlayPopup.debugPopupState()}, controlsView=${controlsOverlayView.debugViewState()}, " +
        "drawerPopup=${drawerChromePopup.debugPopupState()}, drawerHandle=${drawerEdgeHandleView.debugViewState()}, " +
        "drawerShowing=$drawerShowing, drawerFullScreenPopup=${drawerFullScreenPopup.debugPopupState()}, " +
        "drawerOverlay=${drawerOverlayView.debugViewState()}, drawerCompose=${drawerComposeView.debugViewState()}, " +
        "lastDrawer=$lastDrawerState, fpsVisible=$fpsVisible, fpsValue=$fpsValue, " +
        "fpsPopup=${fpsOverlayPopup.debugPopupState()}, fpsView=${fpsOverlayView.debugViewState()}"

private fun View?.isShownInWindowForTest(): Boolean =
    this != null && visibility == View.VISIBLE && isAttachedToWindow && windowToken != null
