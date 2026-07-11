use nerust_input_traits::ControllerHub;

use super::{
    super::{Apu, Core, CpuCartridgeBus, CpuStepState, CpuStepStateEnum, Ppu},
    exit_addressing_mode,
};

pub(crate) struct AbsoluteIndirect;

impl CpuStepState for AbsoluteIndirect {
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
                let address_high = usize::from(core.memory.read_next(
                    &mut core.register,
                    ppu,
                    cartridge,
                    hub,
                    apu,
                    &mut core.interrupt,
                ));
                core.internal_stat
                    .set_address((address_high << 8) | core.internal_stat.get_tempaddr());
            }
            3 => {
                let addr = usize::from(core.memory.read(
                    core.internal_stat.get_address(),
                    ppu,
                    cartridge,
                    hub,
                    apu,
                    &mut core.interrupt,
                ));
                core.internal_stat.set_tempaddr(addr);
            }
            4 => {
                let address_high = usize::from(core.memory.read(
                    (core.internal_stat.get_address().wrapping_add(1) & 0xFF)
                        | (core.internal_stat.get_address() & 0xFF00),
                    ppu,
                    cartridge,
                    hub,
                    apu,
                    &mut core.interrupt,
                ));
                core.internal_stat
                    .set_address((address_high << 8) | core.internal_stat.get_tempaddr());
            }
            _ => {
                return exit_addressing_mode(core);
            }
        }
        CpuStepStateEnum::Continue
    }
}
