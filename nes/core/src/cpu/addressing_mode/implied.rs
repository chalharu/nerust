use nerust_input_traits::ControllerHub;

use super::{
    super::{Apu, Core, CpuCartridgeBus, CpuStepState, CpuStepStateEnum, Ppu},
    exit_addressing_mode,
};

pub(crate) struct Implied;

impl CpuStepState for Implied {
    fn exec(
        core: &mut Core,
        _ppu: &mut Ppu,
        _cartridge: &mut dyn CpuCartridgeBus,
        _hub: &mut dyn ControllerHub,
        _apu: &mut Apu,
    ) -> CpuStepStateEnum {
        exit_addressing_mode(core)
    }
}
