use crate::cpu_core::Lr35902Cpu;
use crate::cpu_core::StepResult;
use crate::cpu_opcodes::CpuStepState;
use crate::cpu_opcodes::helpers::reg;
use crate::memory::GbcMemoryBus;
use crate::ppu::OamBugKind;

// ── PUSH (4 M-cycles) ──────────────────────────────────────

pub(crate) struct Push<const R: u8>;
impl<const R: u8> CpuStepState for Push<R> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            bus.trigger_oam_bug(core.registers().sp(), OamBugKind::Write, 0);
            return StepResult::Continue;
        }
        if step == 1 {
            let v = if R == 3 {
                core.registers().af()
            } else {
                match R {
                    reg::BC => core.registers().bc(),
                    reg::DE => core.registers().de(),
                    reg::R16_HL => core.registers().hl(),
                    _ => 0,
                }
            };
            core.set_operand(0, (v >> 8) as u8);
            core.set_operand(1, v as u8);
            bus.trigger_oam_bug(core.registers().sp().wrapping_sub(1), OamBugKind::Write, 0);
            return StepResult::Continue;
        }
        if step == 2 {
            let _t = core.registers().sp().wrapping_sub(1);
            core.registers_mut().set_sp(_t);
            bus.write(core.registers().sp(), core.operand(0));
            bus.trigger_oam_bug(core.registers().sp().wrapping_sub(1), OamBugKind::Write, 0);
            return StepResult::Continue;
        }
        let _t = core.registers().sp().wrapping_sub(1);
        core.registers_mut().set_sp(_t);
        bus.write(core.registers().sp(), core.operand(1));
        StepResult::Exit
    }
}

// ── POP (3 M-cycles) ───────────────────────────────────────

pub(crate) struct Pop<const R: u8>;
impl<const R: u8> CpuStepState for Pop<R> {
    fn exec(core: &mut Lr35902Cpu, bus: &mut GbcMemoryBus, step: u8) -> StepResult {
        if step == 0 {
            bus.trigger_oam_bug(core.registers().sp(), OamBugKind::ReadInc, 0);
            return StepResult::Continue;
        }
        if step == 1 {
            let v = bus.read(core.registers().sp());
            core.set_operand(0, v);
            let _t = core.registers().sp().wrapping_add(1);
            core.registers_mut().set_sp(_t);
            bus.trigger_oam_bug(core.registers().sp(), OamBugKind::Read, 0);
            return StepResult::Continue;
        }
        let lo = core.operand(0);
        let hi = bus.read(core.registers().sp());
        let _t = core.registers().sp().wrapping_add(1);
        core.registers_mut().set_sp(_t);
        let v = ((hi as u16) << 8) | lo as u16;
        if R == 3 {
            core.registers_mut().set_af(v)
        } else {
            match R {
                reg::BC => core.registers_mut().set_bc(v),
                reg::DE => core.registers_mut().set_de(v),
                reg::R16_HL => core.registers_mut().set_hl(v),
                _ => {}
            }
        }
        StepResult::Exit
    }
}
