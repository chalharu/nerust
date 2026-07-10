use super::{
    super::{Apu, ControllerHub, Core, CpuCartridgeBus, CpuStepState, CpuStepStateEnum, Ppu},
    exit_addressing_mode,
};

pub(crate) struct ZeroPageX;

impl CpuStepState for ZeroPageX {
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
                let pc = usize::from(core.register.get_pc());
                core.memory.read_dummy_cross(
                    pc,
                    core.internal_stat.get_address(),
                    ppu,
                    cartridge,
                    hub,
                    apu,
                    &mut core.interrupt,
                );
                core.internal_stat.set_address(
                    (core
                        .internal_stat
                        .get_address()
                        .wrapping_add(usize::from(core.register.get_x())))
                        & 0xFF,
                );
            }
            _ => {
                return exit_addressing_mode(core);
            }
        }
        CpuStepStateEnum::Continue
    }
}
