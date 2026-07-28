use serde::Deserialize;

/// Input pad state for a test event.
#[derive(Debug, Clone, Deserialize)]
pub struct PadEntry {
    pub button: String,
    pub state: String, // "press" or "release"
}

/// Expected memory value at a given address.
#[derive(Debug, Clone, Deserialize)]
pub struct MemoryEntry {
    pub address: String, // hex, e.g. "0xC000"
    pub value: String,   // hex, e.g. "0x42"
}

/// Expected serial output hash.
#[derive(Debug, Clone, Deserialize)]
pub struct HashEntry {
    pub hash: String, // hex-encoded CRC32 or SHA256
}

/// A single test event: run for `cycles` M-cycles, then verify.
#[derive(Debug, Clone, Deserialize)]
pub struct RomEvent {
    pub cycles: usize,
    pub serial: Option<HashEntry>,
    pub frame: Option<HashEntry>,
    pub audio: Option<HashEntry>,
    pub memory: Option<Vec<MemoryEntry>>,
    pub pad: Option<Vec<PadEntry>>,
}
