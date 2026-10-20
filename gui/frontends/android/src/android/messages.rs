use std::collections::BTreeMap;

use nerust_input_traits::AbstractKey;

/// Menu button presses from the Kotlin overflow menu.
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
    OrientationChanged,
    Reset,
    SaveState,
    TogglePause,
    Unload,
}

/// Single Kotlin→Rust event channel. Menu presses and the error-dialog
/// dismiss ride the same bridge but decode separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrontendEvent {
    Menu(MenuAction),
    ErrorDialogDismissed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RomPickerResult {
    Cancelled,
    Selected(String),
    TreeSelected(String),
    /// Worker-thread completion: bytes fetched off-thread; digest rides along.
    RomBytes {
        uri: String,
        file_name: String,
        bytes: Vec<u8>,
        digest: [u8; 32],
    },
    /// Worker-thread failure: surfaces on the event thread through the
    /// normal picker-error path (error dialog queue).
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SettingsDialogResult {
    Dismissed,
    Applied(BTreeMap<String, usize>),
}
