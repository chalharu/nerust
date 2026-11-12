//! Android settings views: ViewModel-DTO-driven wire model and stateless orchestration.
//!
//! The Android settings screen is a dumb renderer over [`SettingsViewModel`]
//! projections. Every entry builds a fresh view model from the snapshot the
//! caller passes in, applies at most one command, and drops it before
//! returning: no resident editor crosses the JNI boundary, so the `!Send`
//! editor never leaves its calling thread.
//!
//! Onion: transport lives in `wire`, projection in `projection`, edits in
//! `commands`, capture in `capture`. Kotlin only renders sections/rows and
//! echoes `(key, index, number)` commands back.

mod capture;
mod commands;
mod projection;
mod wire;

// Open-payload dependencies only; submodules refer to each other through
// `super::`, and `tests` imports the submodules directly.
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_settings_core::model::language::AppLanguage;
use projection::project_fresh;
use wire::{projection, with_vm_deps};

#[cfg(not(target_os = "android"))]
use wire::{CaptureSlot, GamepadInfo, I18n, KeyboardInfo, ViewPayload};
// Android-facing surface: the bridge, vault, and save paths name these
// through `settings_views::`. Only the JNI build (the `android` module)
// uses them; host tests import the submodules directly.
#[cfg(target_os = "android")]
pub(crate) use capture::apply_hardware_key;
#[cfg(target_os = "android")]
pub(crate) use commands::{apply_command_to, finish_snapshot};
#[cfg(target_os = "android")]
pub(crate) use wire::{
    CaptureSlot, GamepadInfo, I18n, KeyboardInfo, Projection, ViewPayload, app_language_tag,
    install_vm_deps, system_language_from_tag,
};

#[cfg(test)]
mod tests;

/// Open payload for Kotlin: schema version, sections, live issues from a
/// non-fatal `finish` probe, and the pending key-capture state.
/// `keyboards` is the vault's enumerated physical-keyboard list;
/// `touch_capable` is `None` until Kotlin reports touch capability;
/// `system_locale` is the vault-pushed system `Locale` (`None` until pushed).
/// Returns the payload struct: the JNI edge serializes exactly once, so the
/// wire bytes stay identical without a parse/concat round-trip.
pub(super) fn open_payload(
    snapshot: &SettingsSnapshot,
    keyboards: &[KeyboardInfo],
    touch_capable: Option<bool>,
    gamepads: &[GamepadInfo],
    system_locale: Option<AppLanguage>,
) -> Result<ViewPayload, String> {
    with_vm_deps(|deps| {
        let i18n = I18n::resolve(snapshot.shared.general.language, system_locale);
        let truth = projection(keyboards, touch_capable, gamepads, i18n);
        let capture: CaptureSlot = None;
        project_fresh(snapshot, deps, &capture, truth)
    })
}
