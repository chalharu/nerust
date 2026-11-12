//! Android settings key capture: hardware keycodes plus the capture gate.
//!
//! Keycodes arrive from Kotlin as Android keycodes; the gate derives from
//! the vault device truth so rows and the command path cannot disagree.

use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_input_traits::Key;

use super::projection::project_fresh;
use super::wire::{
    CaptureSlot, Projection, ViewPayload, build_vm, sync_snapshot_from_vm, with_vm_deps,
};

/// Hardware key routing for capture (and future keyboard play): Android
/// keycodes to `Key`. Only pressed events with an active capture mutate.
/// Returns the refreshed payload plus whether the key was consumed as a
/// binding. Releases and unmapped keycodes leave the capture pending and
/// report `false` so the platform can handle them (volume, Back, ...).
pub(crate) fn apply_hardware_key(
    keycode: i32,
    pressed: bool,
    snapshot: &mut SettingsSnapshot,
    capture: &mut CaptureSlot,
    truth: Projection,
) -> Result<(Option<ViewPayload>, bool), String> {
    with_vm_deps(|deps| {
        let i18n = truth.i18n;
        // Borrow first: releases and unmapped keys (volume, Back, ...) must
        // not pay a target+prompt clone on the hot path.
        if pressed
            && let Some(key) = android_keycode_to_key(keycode)
            && let Some((target, _)) = capture.as_ref()
        {
            let target = target.clone();
            let vm = build_vm(snapshot, deps, i18n)?;
            vm.capture
                .start_capture(target)
                .map_err(|error| error.to_string())?;
            vm.capture.apply_captured_key(key);
            sync_snapshot_from_vm(snapshot, &vm, i18n);
            *capture = None;
            Ok((Some(project_fresh(snapshot, deps, capture, truth)?), true))
        } else {
            // No-op: Kotlin only reads the payload when consumed is true,
            // so skip the full re-projection (releases arrive per key).
            Ok((None, false))
        }
    })
}

/// Android keycode to emulated key. The table is sorted by keycode, so one
/// binary search replaces the old linear scan on every key event.
const ANDROID_KEYCODE_TABLE: &[(i32, Key)] = {
    use nerust_input_traits::Key;
    &[
        (7, Key::Digit0),
        (8, Key::Digit1),
        (9, Key::Digit2),
        (10, Key::Digit3),
        (11, Key::Digit4),
        (12, Key::Digit5),
        (13, Key::Digit6),
        (14, Key::Digit7),
        (15, Key::Digit8),
        (16, Key::Digit9),
        (19, Key::ArrowUp),
        (20, Key::ArrowDown),
        (21, Key::ArrowLeft),
        (22, Key::ArrowRight),
        (29, Key::KeyA),
        (30, Key::KeyB),
        (31, Key::KeyC),
        (32, Key::KeyD),
        (33, Key::KeyE),
        (34, Key::KeyF),
        (35, Key::KeyG),
        (36, Key::KeyH),
        (37, Key::KeyI),
        (38, Key::KeyJ),
        (39, Key::KeyK),
        (40, Key::KeyL),
        (41, Key::KeyM),
        (42, Key::KeyN),
        (43, Key::KeyO),
        (44, Key::KeyP),
        (45, Key::KeyQ),
        (46, Key::KeyR),
        (47, Key::KeyS),
        (48, Key::KeyT),
        (49, Key::KeyU),
        (50, Key::KeyV),
        (51, Key::KeyW),
        (52, Key::KeyX),
        (53, Key::KeyY),
        (54, Key::KeyZ),
        (55, Key::Comma),
        (56, Key::Period),
        (57, Key::AltLeft),
        (58, Key::AltRight),
        (59, Key::ShiftLeft),
        (60, Key::ShiftRight),
        (61, Key::Tab),
        (62, Key::Space),
        (66, Key::Enter),
        (67, Key::Backspace),
        (68, Key::Backquote),
        (69, Key::Minus),
        (70, Key::Equal),
        (71, Key::BracketLeft),
        (72, Key::BracketRight),
        (73, Key::Backslash),
        (74, Key::Semicolon),
        (75, Key::Quote),
        (76, Key::Slash),
        (92, Key::PageUp),
        (93, Key::PageDown),
        (111, Key::Escape),
        (112, Key::Delete),
        (113, Key::ControlLeft),
        (114, Key::ControlRight),
        (115, Key::CapsLock),
        (122, Key::Home),
        (123, Key::End),
        (124, Key::Insert),
        (131, Key::F1),
        (132, Key::F2),
        (133, Key::F3),
        (134, Key::F4),
        (135, Key::F5),
        (136, Key::F6),
        (137, Key::F7),
        (138, Key::F8),
        (139, Key::F9),
        (140, Key::F10),
        (141, Key::F11),
        (142, Key::F12),
        (144, Key::Numpad0),
        (145, Key::Numpad1),
        (146, Key::Numpad2),
        (147, Key::Numpad3),
        (148, Key::Numpad4),
        (149, Key::Numpad5),
        (150, Key::Numpad6),
        (151, Key::Numpad7),
        (152, Key::Numpad8),
        (153, Key::Numpad9),
        (154, Key::NumpadDivide),
        (155, Key::NumpadMultiply),
        (156, Key::NumpadSubtract),
        (157, Key::NumpadAdd),
        (158, Key::NumpadDecimal),
        (163, Key::NumpadEnter),
        (164, Key::NumpadEqual),
    ]
};

fn android_keycode_to_key(keycode: i32) -> Option<Key> {
    ANDROID_KEYCODE_TABLE
        .binary_search_by_key(&keycode, |(code, _)| *code)
        .ok()
        .map(|index| ANDROID_KEYCODE_TABLE[index].1)
}
