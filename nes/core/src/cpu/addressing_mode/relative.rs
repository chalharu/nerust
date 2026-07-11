use nerust_input_traits::ControllerHub;

use super::{
    super::{Apu, Core, CpuCartridgeBus, CpuStepState, CpuStepStateEnum, Ppu},
    exit_addressing_mode,
};

pub(crate) struct Relative;

impl CpuStepState for Relative {
    fn exec(
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut dyn CpuCartridgeBus,
        hub: &mut dyn ControllerHub,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        match core.internal_stat.get_step() {
            1 => {
                let offset = u16::from(core.memory.read_next(
                    &mut core.register,
                    ppu,
                    cartridge,
                    hub,
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
