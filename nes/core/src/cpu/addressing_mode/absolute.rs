use super::{
    super::{Apu, ControllerHub, Core, CpuCartridgeBus, CpuStepState, CpuStepStateEnum, Ppu},
    exit_addressing_mode,
};

pub(crate) struct Absolute;

impl CpuStepState for Absolute {
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
                core.internal_stat.set_address(addr);
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
                core.internal_stat.set_address(
                    core.internal_stat.get_address() | (usize::from(address_high) << 8),
                );
            }
            _ => {
                return exit_addressing_mode(core);
            }
        }
        CpuStepStateEnum::Continue
    }
}
