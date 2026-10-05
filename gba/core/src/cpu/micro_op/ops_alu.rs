//! Micro-op operand types: ALU-immediate effects.

/// Register effect of an ALU-immediate instruction, fully decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AluEffect {
    pub op: AluImmOp,
    pub rd: usize,
    pub rn: usize,
    pub imm: u32,
    pub set_flags: bool,
    /// Thumb MOV-imm forces N=0; ARM MOVS takes N from bit 31.
    /// V is preserved by MOV in both modes.
    pub thumb_mov: bool,
    /// Shifter carry-out for S with a rotated immediate
    /// (bit(rot-1) of the imm8). None selects the live CPSR carry
    /// (rot == 0, or flag-neutral without S).
    pub carry: Option<bool>,
}

/// Covered ALU-immediate operations (full ARM DP-imm set; Thumb uses
/// Mov/Cmp/Add/Sub).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AluImmOp {
    Mov,
    Add,
    Sub,
    Cmp,
    And,
    Eor,
    Rsb,
    Adc,
    Sbc,
    Rsc,
    Tst,
    Teq,
    Cmn,
    Orr,
    Bic,
    Mvn,
}

impl AluImmOp {
    /// TST/TEQ/CMP/CMN update flags without writing Rd.
    pub(crate) fn is_flag_only(self) -> bool {
        matches!(
            self,
            AluImmOp::Tst | AluImmOp::Teq | AluImmOp::Cmp | AluImmOp::Cmn
        )
    }

    /// Logical operations preserve V; arithmetic operations replace it.
    pub(crate) fn replaces_v(self) -> bool {
        matches!(
            self,
            AluImmOp::Add
                | AluImmOp::Sub
                | AluImmOp::Rsb
                | AluImmOp::Adc
                | AluImmOp::Sbc
                | AluImmOp::Rsc
                | AluImmOp::Cmp
                | AluImmOp::Cmn
        )
    }
}
