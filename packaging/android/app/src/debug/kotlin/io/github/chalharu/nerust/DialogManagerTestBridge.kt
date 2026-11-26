package io.github.chalharu.nerust

/** Instrumentation-test teardown for [DialogManager]. Debug source set only:
 * none of this ships in release. Teardown never acks the Rust gate: firing
 * here would resolve a dialog the test never dismissed. */

internal fun DialogManager.dismissForTest() {
    dismiss(notifyDismiss = false)
}

internal fun DialogManager.resetForTest() {
    // Test teardown never acks the Rust gate: firing here would resolve
    // a dialog the test never dismissed.
    dismiss(notifyDismiss = false)
}
