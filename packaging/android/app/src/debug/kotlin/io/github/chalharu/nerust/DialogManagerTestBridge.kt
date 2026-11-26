package io.github.chalharu.nerust

/** Instrumentation-test teardown for [DialogManager]. Debug source set only:
 * none of this ships in release. Teardown never acks the Rust gate: firing
 * here would resolve a dialog the test never dismissed. */

fun DialogManager.dismissForTest() {
    dismiss(notifyDismiss = false)
}

fun DialogManager.resetForTest() {
    // Test teardown never acks the Rust gate: firing here would resolve
    // a dialog the test never dismissed.
    dismiss(notifyDismiss = false)
}
