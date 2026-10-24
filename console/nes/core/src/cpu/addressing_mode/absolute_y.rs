use nerust_input_traits::ControllerHub;

use super::{
    super::{Apu, Core, CpuCartridgeBus, CpuStepState, CpuStepStateEnum, Ppu, page_crossed},
    exit_addressing_mode,
};

pub(crate) struct AbsoluteY;

impl CpuStepState for AbsoluteY {
    fn exec(
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut dyn CpuCartridgeBus,
        hub: &mut dyn ControllerHub,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        match core.internal_stat.get_step() {
            1 => {
                let addr = usize::from(core.memory.read_next(
                    &mut core.register,
                    ppu,
                    cartridge,
                    hub,
                    apu,
                    &mut core.interrupt,
                ));
                core.internal_stat.set_tempaddr(addr);
            }
            2 => {
                let address_high = core.memory.read_next(
                    &mut core.register,
                    ppu,
                    cartridge,
                    hub,
                    apu,
                    &mut core.interrupt,
                );
                core.internal_stat.set_tempaddr(
                    core.internal_stat.get_tempaddr() | usize::from(address_high) << 8,
                );
                core.internal_stat.set_address(
                    core.internal_stat
                        .get_tempaddr()
                        .wrapping_add(usize::from(core.register.get_y()))
                        & 0xFFFF,
                );
            }
            3 => {
                if !page_crossed(
                    core.internal_stat.get_tempaddr(),
                    core.internal_stat.get_address(),
                ) {
                    return exit_addressing_mode(core);
                }
                // dummy read
                core.memory.read_dummy_cross(
                    core.internal_stat.get_tempaddr(),
                    core.internal_stat.get_address(),
                    ppu,
                    cartridge,
                    hub,
                    apu,
                    &mut core.interrupt,
                );
            }
            _ => {
                return exit_addressing_mode(core);
            }
        }
        CpuStepStateEnum::Continue
    }
}
