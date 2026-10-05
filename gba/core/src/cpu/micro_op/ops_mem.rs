//! Micro-op operand types: memory-access descriptors.

/// Thumb LDR (literal): word-aligned pool address plus dest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PcRelRead {
    pub addr: u32,
    pub rd: usize,
}

/// One data access. `addr = base +/- offset` is resolved at interpret
/// time from live registers (matches handler evaluation order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MemAccess {
    pub width: u8,
    pub rd: usize,
    pub rn: usize,
    pub offset: u32,
    pub offset_register: Option<usize>,
    pub subtract: bool,
    pub is_sp: bool,
    pub signed_load: bool,
    pub post_indexed: bool,
    pub writeback: bool,
    /// Thumb LDRSH odd-address bus quirk (halfword read + ROM merge +
    /// high-byte sign). ARM LDRSH at odd addresses is a plain
    /// sign-extended byte read.
    pub halfword_odd_quirk: bool,
    /// Precomputed store word (ARM STR of R15: instruction+12).
    /// Snapshot at expansion; registers are frozen across one
    /// instruction, matching live in-handler evaluation.
    pub store_value: Option<u32>,
}
