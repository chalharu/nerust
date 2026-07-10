use super::{
    super::{Apu, ControllerHub, Core, CpuCartridgeBus, CpuStepState, CpuStepStateEnum, Ppu},
    exit_opcode,
};

pub(crate) struct Nop;

impl CpuStepState for Nop {
    fn exec(
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut dyn CpuCartridgeBus,
        hub: &mut dyn ControllerHub,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        match core.internal_stat.get_step() {
            1 => {
                let pc = core.register.get_pc() as usize;
                let _ = core
                    .memory
                    .read(pc, ppu, cartridge, hub, apu, &mut core.interrupt);
            }
            _ => {
                return exit_opcode(core);
            }
        }
        CpuStepStateEnum::Continue
    }
}

pub(crate) struct Kil;

impl CpuStepState for Kil {
    fn exec(
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut dyn CpuCartridgeBus,
        hub: &mut dyn ControllerHub,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        match core.internal_stat.get_step() {
            1 | 2 => {
                let pc = core.register.get_pc() as usize;
                let _ = core
                    .memory
                    .read(pc, ppu, cartridge, hub, apu, &mut core.interrupt);
            }
            _ => {
                return exit_opcode(core);
            }
        }
        CpuStepStateEnum::Continue
    }
}
