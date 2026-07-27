//! Per-opcode state machine trait + handler table.
//!
//! Each instruction is a zero-sized struct implementing `CpuStepState`.
//! The `exec()` method is called once per M-cycle with the current step
//! number (1-based). It returns `Continue` to stay in the same state
//! for the next M-cycle, or `Exit` when the instruction completes.

use crate::cpu::{Lr35902Cpu, StepResult};
use crate::memory::GbcMemoryBus;

mod alu;
mod cb;
mod control;
mod inc_dec;
mod load;
mod misc;
mod stack;

// ── Trait ──────────────────────────────────────────────────

/// Interface for decomposing an instruction into M-cycle steps.
///
/// Implementors receive the current step number (1-based) and perform
/// the work for that M-cycle. Memory reads/writes happen here at the
/// correct M-cycle boundary.
pub(crate) trait CpuStepState {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult;
}

// ── Handler table ──────────────────────────────────────────

pub(crate) type HandlerFn = fn(&mut Lr35902Cpu, &mut GbcMemoryBus, u8) -> StepResult;

/// Returns the function pointer table mapping opcode → handler.
pub(crate) fn handler_table() -> [HandlerFn; 256] {
    let mut t: [HandlerFn; 256] = [misc::Invalid::exec; 256];

    // Block 0 (0x00-0x3F)
    t[0x00] = misc::Nop::exec;
    t[0x01] = load::LdR16D16::<0>::exec; // BC
    t[0x02] = load::LdR16memA::<0>::exec;
    t[0x03] = inc_dec::IncR16::<0>::exec;
    t[0x04] = inc_dec::IncR8::<0>::exec; // B (bits 3-5 = 000)
    t[0x05] = inc_dec::DecR8::<0>::exec;
    t[0x06] = load::LdR8D8::<0>::exec;
    t[0x07] = misc::Rlca::exec;
    t[0x08] = load::LdA16Sp::exec;
    t[0x09] = alu::AddHlR16::<0>::exec;
    t[0x0A] = load::LdAR16mem::<0>::exec;
    t[0x0B] = inc_dec::DecR16::<0>::exec;
    t[0x0C] = inc_dec::IncR8::<1>::exec;
    t[0x0D] = inc_dec::DecR8::<1>::exec;
    t[0x0E] = load::LdR8D8::<1>::exec;
    t[0x0F] = misc::Rrca::exec;

    t[0x10] = misc::Stop::exec;
    t[0x11] = load::LdR16D16::<2>::exec; // DE
    t[0x12] = load::LdR16memA::<2>::exec;
    t[0x13] = inc_dec::IncR16::<2>::exec;
    t[0x14] = inc_dec::IncR8::<2>::exec;
    t[0x15] = inc_dec::DecR8::<2>::exec;
    t[0x16] = load::LdR8D8::<2>::exec;
    t[0x17] = misc::Rla::exec;
    t[0x18] = control::Jr::exec;
    t[0x19] = alu::AddHlR16::<2>::exec;
    t[0x1A] = load::LdAR16mem::<2>::exec;
    t[0x1B] = inc_dec::DecR16::<2>::exec;
    t[0x1C] = inc_dec::IncR8::<3>::exec;
    t[0x1D] = inc_dec::DecR8::<3>::exec;
    t[0x1E] = load::LdR8D8::<3>::exec;
    t[0x1F] = misc::Rra::exec;

    for (op, h) in [
        (0x20, control::JrCond::<0>::exec as HandlerFn),
        (0x28, control::JrCond::<1>::exec as HandlerFn),
        (0x30, control::JrCond::<2>::exec as HandlerFn),
        (0x38, control::JrCond::<3>::exec as HandlerFn),
    ] {
        t[op] = h;
    }
    t[0x21] = load::LdR16D16::<6>::exec;
    t[0x22] = load::LdHliA::exec;
    t[0x23] = inc_dec::IncR16::<6>::exec;
    t[0x24] = inc_dec::IncR8::<4>::exec;
    t[0x25] = inc_dec::DecR8::<4>::exec;
    t[0x26] = load::LdR8D8::<4>::exec;
    t[0x27] = misc::Daa::exec;
    t[0x28] = control::JrCond::<1>::exec; // Z
    t[0x29] = alu::AddHlR16::<6>::exec;
    t[0x2A] = load::LdAHli::exec;
    t[0x2B] = inc_dec::DecR16::<6>::exec;
    t[0x2C] = inc_dec::IncR8::<5>::exec;
    t[0x2D] = inc_dec::DecR8::<5>::exec;
    t[0x2E] = load::LdR8D8::<5>::exec;
    t[0x2F] = misc::Cpl::exec;
    t[0x30] = control::JrCond::<2>::exec;
    t[0x31] = load::LdR16D16::<8>::exec;
    t[0x32] = load::LdHldA::exec;
    t[0x33] = inc_dec::IncSp::exec;
    t[0x34] = inc_dec::IncHlIndirect::exec;
    t[0x35] = inc_dec::DecHlIndirect::exec;
    t[0x36] = load::LdHlD8::exec;
    t[0x37] = misc::Scf::exec;
    t[0x38] = control::JrCond::<3>::exec;
    t[0x39] = alu::AddHlSp::exec;
    t[0x3A] = load::LdAHld::exec;
    t[0x3B] = inc_dec::DecSp::exec;
    t[0x3C] = inc_dec::IncR8::<7>::exec;
    t[0x3D] = inc_dec::DecR8::<7>::exec;
    t[0x3E] = load::LdR8D8::<7>::exec;
    t[0x3F] = misc::Ccf::exec;

    // Block 1 (0x40-0x7F): LD r8, r8
    for op in 0x40..=0x7F {
        t[op] = load::LdR8R8::exec;
    }
    t[0x76] = misc::Halt::exec;

    // Block 2 (0x80-0xBF): ALU A, r8
    for op in 0x80..=0xBF {
        t[op] = alu::AluAR8::exec;
    }

    // Block 3 (0xC0-0xFF)
    for (op, h) in [
        (0xC0, control::RetCond::<0>::exec as HandlerFn),
        (0xC8, control::RetCond::<1>::exec as HandlerFn),
        (0xD0, control::RetCond::<2>::exec as HandlerFn),
        (0xD8, control::RetCond::<3>::exec as HandlerFn),
    ] {
        t[op] = h;
    }
    t[0xC1] = stack::Pop::<0>::exec;
    for (op, h) in [
        (0xC2, control::JpCond::<0>::exec as HandlerFn),
        (0xCA, control::JpCond::<1>::exec as HandlerFn),
        (0xD2, control::JpCond::<2>::exec as HandlerFn),
        (0xDA, control::JpCond::<3>::exec as HandlerFn),
    ] {
        t[op] = h;
    }
    t[0xC3] = control::JpA16::exec;
    for (op, h) in [
        (0xC4, control::CallCond::<0>::exec as HandlerFn),
        (0xCC, control::CallCond::<1>::exec as HandlerFn),
        (0xD4, control::CallCond::<2>::exec as HandlerFn),
        (0xDC, control::CallCond::<3>::exec as HandlerFn),
    ] {
        t[op] = h;
    }
    t[0xC5] = stack::Push::<0>::exec;
    t[0xC6] = alu::AluAD8::<0>::exec;
    t[0xC7] = control::Rst::<0>::exec;
    t[0xC9] = control::Ret::exec;
    t[0xCB] = cb::CbPrefix::exec;
    t[0xCD] = control::Call::exec;
    t[0xCE] = alu::AluAD8::<1>::exec;
    t[0xCF] = control::Rst::<1>::exec;
    t[0xD1] = stack::Pop::<2>::exec;
    t[0xD5] = stack::Push::<2>::exec;
    t[0xD6] = alu::AluAD8::<2>::exec;
    t[0xD7] = control::Rst::<2>::exec;
    t[0xD9] = control::Reti::exec;
    t[0xDE] = alu::AluAD8::<3>::exec;
    t[0xDF] = control::Rst::<3>::exec;
    t[0xE0] = misc::LdhA8A::exec;
    t[0xE1] = stack::Pop::<6>::exec;
    t[0xE2] = misc::LdCA::exec;
    t[0xE5] = stack::Push::<6>::exec;
    t[0xE6] = alu::AluAD8::<4>::exec;
    t[0xE7] = control::Rst::<4>::exec;
    t[0xE8] = alu::AddSpE::exec;
    t[0xE9] = control::JpHl::exec;
    t[0xEA] = load::LdA16A::exec;
    t[0xEE] = alu::AluAD8::<5>::exec;
    t[0xEF] = control::Rst::<5>::exec;
    t[0xF0] = misc::LdhAA8::exec;
    t[0xF1] = stack::Pop::<7>::exec;
    t[0xF2] = misc::LdAC::exec;
    t[0xF3] = misc::Di::exec;
    t[0xF5] = stack::Push::<7>::exec;
    t[0xF6] = alu::AluAD8::<6>::exec;
    t[0xF7] = control::Rst::<6>::exec;
    t[0xF8] = load::LdHlSpE::exec;
    t[0xF9] = misc::LdSpHl::exec;
    t[0xFA] = load::LdAA16::exec;
    t[0xFB] = misc::Ei::exec;
    t[0xFE] = alu::AluAD8::<7>::exec;
    t[0xFF] = control::Rst::<7>::exec;

    // Invalid opcodes with per-opcode byte + M-cycle counts
    t[0xD3] = misc::InvalidOp::<1, 0>::exec;
    t[0xDB] = misc::InvalidOp::<1, 0>::exec;
    t[0xDD] = misc::InvalidOp::<1, 0>::exec;
    t[0xE3] = misc::InvalidOp::<2, 2>::exec;
    t[0xE4] = misc::InvalidOp::<2, 2>::exec;
    t[0xEB] = misc::InvalidOp::<3, 3>::exec;
    t[0xEC] = misc::InvalidOp::<3, 3>::exec;
    t[0xED] = misc::InvalidOp::<2, 2>::exec;
    t[0xF4] = misc::InvalidOp::<3, 3>::exec;
    t[0xFC] = misc::InvalidOp::<3, 3>::exec;
    t[0xFD] = misc::InvalidOp::<3, 3>::exec;

    t
}
