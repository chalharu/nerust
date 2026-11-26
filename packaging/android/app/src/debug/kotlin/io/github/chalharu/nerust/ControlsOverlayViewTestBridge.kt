package io.github.chalharu.nerust

/** Instrumentation-test probe for [ControlsOverlayView]. Debug source set
 * only: none of this ships in release. */

fun ControlsOverlayView.floatingDpadStateForTest(): FloatArray =
    floatArrayOf(
        if (floatingDpadActive) 1f else 0f,
        floatingDpadCenterX,
        floatingDpadCenterY,
        floatingDpadKnobX,
        floatingDpadKnobY,
    )
