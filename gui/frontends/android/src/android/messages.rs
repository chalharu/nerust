use std::collections::BTreeMap;

use nerust_input_traits::AbstractKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuAction {
    ControllerInput {
        device_id: i32,
        key: AbstractKey,
        pressed: bool,
    },
    Exit,
    LoadState,
    OpenRom,
    OpenSettings,
    Reset,
    SaveState,
    TogglePause,
    Unload,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RomPickerResult {
    Cancelled,
    Selected(String),
    TreeSelected(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SettingsDialogResult {
    Dismissed,
    Applied(BTreeMap<String, usize>),
}
