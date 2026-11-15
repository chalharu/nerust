//! Android settings key capture: hardware keycodes plus the capture gate.
//!
//! Keycodes arrive from Kotlin as Android keycodes; the gate derives from
//! the vault device truth so rows and the command path cannot disagree.
//! Pad buttons carry the Kotlin device descriptor (`None` for keys and
//! uncounted devices): per-pad mapping rows only accept presses from
//! their own pad, so two pads never cross-capture. The descriptor
//! resolves against the same vault list that built the rows, so a Kotlin
//! scan-order index can never skew against the Rust side.

use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_input_traits::StandardButton;
use nerust_settings_core::{editor::CaptureTarget, i18n::UiText};

use super::keycode::android_keycode_to_key;
use super::projection::project_fresh;
use super::wire::{
    CaptureSlot, Projection, ViewPayload, build_vm, pad_index_for_member, sync_snapshot_from_vm,
    with_vm_deps,
};

fn android_keycode_to_pad(keycode: i32) -> Option<StandardButton> {
    u32::try_from(keycode)
        .ok()
        .and_then(super::keycode::android_keycode_to_button)
}

/// Prompt key for one capture target: pad rows name what a press can
/// claim, game rows with no keyboard connected name the dpad twin,
/// everything else keeps the shared prompt. The edge owns this branch
/// (it holds the vault device truth); the viewmodel only tracks the
/// pending target and renders the shared prompt for its own view.
pub(crate) fn capture_prompt(
    target: &CaptureTarget,
    keyboards_present: bool,
    gamepads_present: bool,
) -> UiText {
    if matches!(target, CaptureTarget::PadBinding { .. }) {
        UiText::CapturePromptPad
    } else if matches!(target, CaptureTarget::Binding { .. })
        && !keyboards_present
        && gamepads_present
    {
        UiText::CapturePromptDpadOnly
    } else {
        UiText::CapturePrompt
    }
}

/// Hardware key routing for capture: Android keycodes to `Key`. Only
/// pressed events with an active capture mutate. Returns the refreshed
/// payload plus whether the key was consumed as a binding. Releases and
/// unmapped keycodes leave the capture pending and report `false` so the
/// platform can handle them (volume, Back, ...).
pub(crate) fn apply_hardware_key(
    keycode: i32,
    pad_descriptor: Option<&str>,
    pressed: bool,
    snapshot: &mut SettingsSnapshot,
    capture: &mut CaptureSlot,
    truth: Projection,
) -> Result<(Option<ViewPayload>, bool), String> {
    with_vm_deps(|deps| {
        let i18n = truth.i18n;
        // Borrow first: releases and unmapped keys (volume, Back, ...) must
        // not pay a target+prompt clone on the hot path.
        if pressed && let Some((target, _)) = capture.as_ref() {
            // Key and button decode independently: D-pad codes (19-22)
            // name both an arrow key and a pad position, so overlap is
            // normal. The target gates which side lands; the shared tail
            // applies every granted side, so the two columns never fork.
            let target = target.clone();
            let pad = pad_descriptor
                .and_then(|descriptor| pad_index_for_member(truth.gamepads, descriptor));
            let (key, button) = if let Some(want) = target.pad() {
                // Per-pad mapping rows take only their own pad's
                // buttons: a press from another pad (or a keyboard key)
                // leaves the capture pending instead of eating it. A
                // D-pad press lands its position, never its arrow twin.
                if pad != Some(want) {
                    return Ok((None, false));
                }
                (None, android_keycode_to_pad(keycode))
            } else if target.action().is_some() {
                // Shortcuts take either side: a D-pad press lands both
                // the arrow key and the pad position in their columns.
                (
                    android_keycode_to_key(keycode),
                    android_keycode_to_pad(keycode),
                )
            } else {
                // Game rows take keyboard keys only: pad-only buttons
                // leave the capture pending, while D-pad presses land
                // their arrow-key twin into the shared vocabulary.
                (android_keycode_to_key(keycode), None)
            };
            if key.is_some() || button.is_some() {
                let vm = build_vm(snapshot, deps, i18n)?;
                vm.capture
                    .start_capture(target)
                    .map_err(|error| error.to_string())?;
                if let Some(key) = key {
                    vm.capture.apply_captured_key(key);
                }
                if let Some(button) = button {
                    vm.capture.apply_captured_pad(button);
                }
                sync_snapshot_from_vm(snapshot, &vm, i18n);
                *capture = None;
                return Ok((Some(project_fresh(snapshot, deps, capture, truth)?), true));
            }
        }
        // No-op: Kotlin only reads the payload when consumed is true,
        // so skip the full re-projection (releases arrive per key).
        Ok((None, false))
    })
}
