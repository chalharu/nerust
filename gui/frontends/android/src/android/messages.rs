use std::collections::BTreeMap;

pub(crate) use crate::bridge_events::{FrontendEvent, MenuAction};

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
