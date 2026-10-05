//! Micro-op engine tests: execute-effect pins (absolute values) plus
//! expansion shape/gate/manifest tests. All live here, next to the
//! engine, importing the pieces explicitly.
use super::MicroOp;
use super::apply::apply_op;
use super::expand_arm::expand_arm;
use super::expand_thumb::expand_thumb;
use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;
use std::collections::VecDeque;

/// Drive expanded ops without retire (no flush/pc-advance): pins
/// execute effects at absolute values.
fn run_ops(
    regs: &mut CpuRegisters,
    bus: &mut GbaMemoryBus,
    ops: super::MicroOpVec,
    pc: u32,
    is_thumb: bool,
) {
    let mut queue: VecDeque<MicroOp> = ops.into_iter().collect();
    while let Some(op) = queue.pop_front() {
        apply_op(regs, bus, op, pc, is_thumb);
    }
}

fn run_arm(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u32) {
    let pc = regs.pc();
    let ops = expand_arm(instr, regs).expect("arm expands");
    run_ops(regs, bus, ops, pc, false);
}

fn run_thumb(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u16) {
    let pc = regs.pc();
    let ops = expand_thumb(instr, regs).expect("thumb expands");
    run_ops(regs, bus, ops, pc, true);
}

mod apply;
mod expand;
