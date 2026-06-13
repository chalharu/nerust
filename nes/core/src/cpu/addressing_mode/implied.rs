use super::super::CpuCartridgeBus;
use super::super::{Apu, Controller, Core, CpuStepState, CpuStepStateEnum, Ppu};
use super::exit_addressing_mode;

pub(crate) struct Implied;

impl CpuStepState for Implied {
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
