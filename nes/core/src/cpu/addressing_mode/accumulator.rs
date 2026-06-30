use super::{
    super::{Apu, Controller, Core, CpuCartridgeBus, CpuStepState, CpuStepStateEnum, Ppu},
    exit_addressing_mode,
};

pub(crate) struct Accumulator;

impl CpuStepState for Accumulator {
    fn exec(
        core: &mut Core,
        _ppu: &mut Ppu,
        _cartridge: &mut dyn CpuCartridgeBus,
        _controller: &mut dyn Controller,
        _apu: &mut Apu,
    ) -> CpuStepStateEnum {
        exit_addressing_mode(core)
    }
}
