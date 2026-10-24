//! Micro-op operand types: block-transfer and branch effects.

/// Block-batch opener: `is_load` selects the GBALoad/GBAStore word
/// convention, `fetch_width` (4 ARM, 2 Thumb) the erase-floor N.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BlockStartEffect {
    pub is_load: bool,
    pub fetch_width: u8,
}
/// Instruction-end commit for a block transfer. `sp` goes through
/// `set_sp` (NOT `set_r`, which also spills to the user bank inside
/// the LDM^ conflict window); `writeback` is the LDM/STM base update via `set_r`; `ldm_conflict`
/// arms the post-LDM^ bank-conflict window (ARM S-bit loads to the
/// user bank outside USR/SYS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BlockEndEffect {
    pub sp: Option<u32>,
    pub writeback: Option<(usize, u32)>,
    pub ldm_conflict: bool,
    /// First word address, for the instruction-scoped fetch-stream break
    /// (open-bus blocks pre-pay nothing, mapped blocks break).
    pub first_addr: u32,
}
