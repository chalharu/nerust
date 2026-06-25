use std::{path::PathBuf, time::SystemTime};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateSlotSummary {
    pub schema_version: u32,
    pub slot_id: u64,
    pub path: PathBuf,
    pub saved_at: SystemTime,
    pub has_thumbnail: bool,
    pub emulator_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedStateSlot {
    pub summary: StateSlotSummary,
    pub machine_state: Vec<u8>,
    pub thumbnail_png: Option<Vec<u8>>,
}
