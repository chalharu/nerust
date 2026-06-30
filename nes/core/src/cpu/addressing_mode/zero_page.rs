use super::{
    super::{Apu, Controller, Core, CpuCartridgeBus, CpuStepState, CpuStepStateEnum, Ppu},
    exit_addressing_mode,
};

pub(crate) struct ZeroPage;

impl CpuStepState for ZeroPage {
    fn exec(
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut dyn CpuCartridgeBus,
        controller: &mut dyn Controller,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        match core.internal_stat.get_step() {
            1 => {
                let addr = usize::from(core.memory.read_next(
                    &mut core.register,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                ));
                core.internal_stat.set_address(addr);
            }
            _ => {
                return exit_addressing_mode(core);
            }
        }
        CpuStepStateEnum::Continue
    }
}
