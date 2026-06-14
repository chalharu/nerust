use super::super::CpuCartridgeBus;
use super::super::{Apu, Controller, Core, CpuStepState, CpuStepStateEnum, Ppu};
use super::exit_addressing_mode;

pub(crate) struct Relative;

impl CpuStepState for Relative {
    fn exec(
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut dyn CpuCartridgeBus,
        controller: &mut dyn Controller,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        match core.internal_stat.get_step() {
            1 => {
                let offset = u16::from(core.memory.read_next(
                    &mut core.register,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                ));
                let pc = core.register.get_pc();
                core.internal_stat.set_address(
                    pc.wrapping_add(offset)
                        .wrapping_sub(if offset < 0x80 { 0 } else { 0x100 })
                        as usize,
                );
            }
            _ => {
                return exit_addressing_mode(core);
            }
        }
        CpuStepStateEnum::Continue
    }
}
