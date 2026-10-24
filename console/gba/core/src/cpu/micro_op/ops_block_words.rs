//! Micro-op operand types: block-transfer word payloads.

/// One block word (PUSH/POP, LDM/STM). `addr` is snapshotted at expansion (queue-fill
/// runs on pre-instruction state); values are read at execution, which
/// is exact because no CPU register changes between the
/// words of one instruction (bus ticks never touch registers).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BlockWord {
    pub addr: u32,
    /// Source/dest low register, or 14 for the PUSH LR slot.
    pub reg: usize,
    pub load: bool,
    pub first: bool,
    /// POP {..,PC} final word: `set_pc` (retire flushes the pipeline).
    pub pc_load: bool,
    /// ARM LDM^ (S bit, no PC in list): loads land in the user bank.
    /// Snapshot at expansion (mode frozen until a PC load, which is
    /// always last and never takes this path).
    pub user_bank: bool,
    /// ARM LDM^ loading PC: restore CPSR from SPSR (skipped in
    /// USR/SYS, which have none). Evaluated at execution, before any
    /// mode change within this instruction.
    pub restore_cpsr: bool,
    /// Precomputed store word (STM base-in-list quirk: non-first
    /// occurrences of the base store the final address). Snapshot at
    /// expansion; registers are frozen across the words of one
    /// instruction, so this matches live in-loop evaluation.
    pub store_value: Option<u32>,
}

/// One empty-list block word (ARM LDM/STM Rlist=0, Thumb PUSH/POP with
/// no registers): the single transferred PC word. Address and store
/// value are snapshotted at expansion (frozen pre-instruction state,
/// like `BlockWord`); the access runs at execution through the same
/// bus calls, including the batch/no-batch shape (Thumb batches, ARM
/// does not) and sequential-touch shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BlockEmptyEffect {
    pub load: bool,
    pub addr: u32,
    /// ARM writeback (W=1): `(base_reg, base +/- 0x40)` via `set_r`.
    /// None when W=0 (ARM) or for Thumb (SP goes through `BlockEnd.sp`,
    /// which uses `set_sp`).
    pub writeback_reg: Option<(usize, u32)>,
    /// Store word for STM/PUSH (PC + 4 ARM / + 2 Thumb), snapshotted
    /// at expansion.
    pub store_value: u32,
    /// ARM LDM^ loading PC: restore CPSR from SPSR. Evaluated at
    /// execution like the `BlockWord` path.
    pub restore_cpsr: bool,
    /// Touch `data_sequential` before the access (Thumb empty paths
    /// set it false; ARM empty paths leave it alone).
    pub reset_sequential: bool,
    /// Charge the fetch-stream break here. True for standalone
    /// (ARM) empties; false when a `BlockEnd` follows (Thumb), which
    /// carries the instruction's single break — a second break would
    /// double-charge N-S.
    pub break_stream: bool,
}
