use gtk::prelude::*;
use nerust_contract_options::Mmc3IrqVariant;
use nerust_contract_settings::{desktop::StoragePolicy, input::KeyboardKey, nes::NesVideoFilter};
use nerust_gui_shell::settings::bindings::keys::keyboard_key_from_id;
use std::path::PathBuf;

pub(super) fn keyboard_key_from_combo(combo: &gtk::ComboBoxText) -> Option<KeyboardKey> {
    combo.active_id().as_deref().and_then(keyboard_key_from_id)
}

pub(super) fn parse_optional_path(text: &str) -> Option<PathBuf> {
    optional_string(text).map(PathBuf::from)
}

fn optional_string(text: &str) -> Option<String> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

pub(super) fn path_text(path: Option<&PathBuf>) -> String {
    path.map(|path| path.display().to_string())
        .unwrap_or_else(|| "Default".into())
}

pub(super) fn bool_text(value: bool) -> &'static str {
    if value { "On" } else { "Off" }
}

pub(super) fn option_u32_text(value: Option<u32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "Default".into())
}

pub(super) fn value_or_none(spin: &gtk::SpinButton) -> Option<u32> {
    let value = spin.value() as u32;
    (value > 0).then_some(value)
}

pub(super) fn storage_policy_id(policy: StoragePolicy) -> &'static str {
    match policy {
        StoragePolicy::RomSidecar => "rom_sidecar",
        StoragePolicy::AppData => "app_data",
        StoragePolicy::CustomRoots => "custom_roots",
    }
}

pub(super) fn storage_policy_label(policy: StoragePolicy) -> &'static str {
    match policy {
        StoragePolicy::RomSidecar => "ROM sidecar",
        StoragePolicy::AppData => "App data",
        StoragePolicy::CustomRoots => "Custom roots",
    }
}

pub(super) fn storage_policy_from_combo(combo: &gtk::ComboBoxText) -> StoragePolicy {
    match combo.active_id().as_deref() {
        Some("app_data") => StoragePolicy::AppData,
        Some("custom_roots") => StoragePolicy::CustomRoots,
        _ => StoragePolicy::RomSidecar,
    }
}

pub(super) fn mmc3_variant_id(value: Option<Mmc3IrqVariant>) -> &'static str {
    match value {
        None => "default",
        Some(Mmc3IrqVariant::Sharp) => "sharp",
        Some(Mmc3IrqVariant::Nec) => "nec",
    }
}

pub(super) fn mmc3_variant_label(value: Option<Mmc3IrqVariant>) -> &'static str {
    match value {
        None => "Default",
        Some(Mmc3IrqVariant::Sharp) => "Sharp",
        Some(Mmc3IrqVariant::Nec) => "NEC",
    }
}

pub(super) fn mmc3_variant_from_combo(combo: &gtk::ComboBoxText) -> Option<Mmc3IrqVariant> {
    match combo.active_id().as_deref() {
        Some("sharp") => Some(Mmc3IrqVariant::Sharp),
        Some("nec") => Some(Mmc3IrqVariant::Nec),
        _ => None,
    }
}

pub(super) fn nes_filter_id(value: NesVideoFilter) -> &'static str {
    match value {
        NesVideoFilter::None => "none",
        NesVideoFilter::NtscRgb => "ntsc_rgb",
        NesVideoFilter::NtscComposite => "ntsc_composite",
        NesVideoFilter::NtscSVideo => "ntsc_svideo",
    }
}

pub(super) fn nes_filter_label(value: NesVideoFilter) -> &'static str {
    match value {
        NesVideoFilter::None => "None",
        NesVideoFilter::NtscRgb => "NTSC RGB",
        NesVideoFilter::NtscComposite => "NTSC Composite",
        NesVideoFilter::NtscSVideo => "NTSC S-Video",
    }
}

pub(super) fn nes_filter_from_combo(combo: &gtk::ComboBoxText) -> NesVideoFilter {
    match combo.active_id().as_deref() {
        Some("none") => NesVideoFilter::None,
        Some("ntsc_rgb") => NesVideoFilter::NtscRgb,
        Some("ntsc_svideo") => NesVideoFilter::NtscSVideo,
        _ => NesVideoFilter::NtscComposite,
    }
}
