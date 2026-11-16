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

use super::keycode::{android_keycode_to_button, android_keycode_to_key};
use super::projection::project_fresh;
use super::vm::{Projection, VmDeps, build_vm, sync_snapshot_from_vm, with_vm_deps};
use super::wire::{CaptureSlot, ViewPayload, rank_for_member};

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
            // Rank, not live position: the target pad is a settings
            // rank, so the pressing pad must resolve the same way or a
            // reconnect cycle would reject its own pad's buttons.
            let pad =
                pad_descriptor.and_then(|descriptor| rank_for_member(truth.pad_order, descriptor));
            let (key, button) = if let Some(want) = target.pad() {
                // Per-pad mapping rows take only their own pad's
                // buttons: a press from another pad (or a keyboard key)
                // leaves the capture pending instead of eating it. A
                // D-pad press lands its position, never its arrow twin.
                if pad != Some(want) {
                    return Ok((None, false));
                }
                (
                    None,
                    u32::try_from(keycode)
                        .ok()
                        .and_then(android_keycode_to_button),
                )
            } else if target.action().is_some() {
                // Shortcuts take either side: a D-pad press lands both
                // the arrow key and the pad position in their columns.
                (
                    android_keycode_to_key(keycode),
                    u32::try_from(keycode)
                        .ok()
                        .and_then(android_keycode_to_button),
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

/// Press level turning an analog trigger into a captured button.
/// Gameplay fires at 0.9 (see session `TRIGGER_PRESS`); capture takes a
/// deliberate half-press so short-travel triggers still land without
/// brushing the stick noise floor.
const MOTION_CAPTURE_PRESS: f32 = 0.5;

/// Shared commit tail for captured buttons: start, apply, write back,
/// clear the pending slot, and re-project. Key and motion entries converge
/// here so the two can never apply differently.
fn commit_captured_button(
    deps: &VmDeps,
    snapshot: &mut SettingsSnapshot,
    capture: &mut CaptureSlot,
    truth: Projection,
    target: CaptureTarget,
    button: StandardButton,
) -> Result<(Option<ViewPayload>, bool), String> {
    let vm = build_vm(snapshot, deps, truth.i18n)?;
    vm.capture
        .start_capture(target)
        .map_err(|error| error.to_string())?;
    vm.capture.apply_captured_pad(button);
    sync_snapshot_from_vm(snapshot, &vm, truth.i18n);
    *capture = None;
    Ok((Some(project_fresh(snapshot, deps, capture, truth)?), true))
}

/// Motion capture for analog-only triggers (ZL/ZR on pads that send no
/// key event, only `AXIS_LTRIGGER`/`AXIS_RTRIGGER`): axis values to the
/// trigger button through the same rank gate as hardware keys. Only
/// shortcut and per-pad mapping rows accept it (game rows stay key-only,
/// like the key path); below-threshold values leave the capture pending
/// and report `false`.
pub(crate) fn apply_motion_capture(
    trigger_l: f32,
    trigger_r: f32,
    pad_descriptor: Option<&str>,
    snapshot: &mut SettingsSnapshot,
    capture: &mut CaptureSlot,
    truth: Projection,
) -> Result<(Option<ViewPayload>, bool), String> {
    // Dominant axis wins; ties read left. Simultaneous full squeezes
    // land one deterministic button instead of depending on event order.
    let button = if trigger_l >= MOTION_CAPTURE_PRESS && trigger_l >= trigger_r {
        Some(StandardButton::LeftTrigger)
    } else if trigger_r >= MOTION_CAPTURE_PRESS {
        Some(StandardButton::RightTrigger)
    } else {
        None
    };
    let Some(button) = button else {
        return Ok((None, false));
    };
    with_vm_deps(|deps| {
        let Some((target, _)) = capture.as_ref() else {
            return Ok((None, false));
        };
        let target = target.clone();
        // Rank, not live position: same gate as the key path, so a
        // reconnect cycle cannot skew per-pad routing here either.
        let pad =
            pad_descriptor.and_then(|descriptor| rank_for_member(truth.pad_order, descriptor));
        // Same gate as the key path: per-pad rows take only their own
        // pad's press, shortcuts take any pad, game rows take none.
        let granted = if let Some(want) = target.pad() {
            (pad == Some(want)).then_some(button)
        } else if target.action().is_some() {
            Some(button)
        } else {
            None
        };
        let Some(button) = granted else {
            return Ok((None, false));
        };
        commit_captured_button(deps, snapshot, capture, truth, target, button)
    })
}
