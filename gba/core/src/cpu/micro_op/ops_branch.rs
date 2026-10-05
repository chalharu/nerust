//! Micro-op operand types: branch and multiply-commit effects.

/// Multiply commit descriptor: raw word (`thumb` form in low 16 bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MulEffect {
    pub instr: u32,
    pub thumb: bool,
}
/// Final execute-stage effect of a direct branch. The two preceding
/// `Internal` ops model pipeline refill cycles; BL writes LR only when
/// this final op executes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BranchEffect {
    pub offset: u32,
    pub link: bool,
}
