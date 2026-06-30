use super::{
    super::{Apu, Controller, Core, CpuCartridgeBus, CpuStepState, CpuStepStateEnum, Ppu},
    exit_addressing_mode,
};

pub(crate) struct Immediate;

impl CpuStepState for Immediate {
    fn exec(
        core: &mut Core,
        _ppu: &mut Ppu,
        _cartridge: &mut dyn CpuCartridgeBus,
        _controller: &mut dyn Controller,
        _apu: &mut Apu,
    ) -> CpuStepStateEnum {
        let pc = core.register.get_pc();
        core.register.set_pc(pc.wrapping_add(1));
        core.internal_stat.set_address(usize::from(pc));
        exit_addressing_mode(core)
    }
}
