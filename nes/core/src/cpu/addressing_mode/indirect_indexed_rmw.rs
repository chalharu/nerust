use super::{
    super::{Apu, ControllerHub, Core, CpuCartridgeBus, CpuStepState, CpuStepStateEnum, Ppu},
    exit_addressing_mode,
};

pub(crate) struct IndirectIndexedRMW;

impl CpuStepState for IndirectIndexedRMW {
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
                core.internal_stat.set_data(core.memory.read(
                    core.internal_stat.get_tempaddr(),
                    ppu,
                    cartridge,
                    hub,
                    apu,
                    &mut core.interrupt,
                ));
            }
            3 => {
                let address_high = usize::from(core.memory.read(
                    core.internal_stat.get_tempaddr().wrapping_add(1) & 0xFF,
                    ppu,
                    cartridge,
                    hub,
                    apu,
                    &mut core.interrupt,
                ));
                core.internal_stat
                    .set_tempaddr((address_high << 8) | usize::from(core.internal_stat.get_data()));
                core.internal_stat.set_address(
                    core.internal_stat
                        .get_tempaddr()
                        .wrapping_add(usize::from(core.register.get_y()))
                        & 0xFFFF,
                );
            }
            4 => {
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
