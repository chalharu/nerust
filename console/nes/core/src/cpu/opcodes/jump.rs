use super::{
    super::{
        Apu, ControllerHub, Core, CpuCartridgeBus, CpuStepState, CpuStepStateEnum, Ppu,
        read_dummy_current,
    },
    exit_opcode,
};

pub(crate) struct Jmp;

impl CpuStepState for Jmp {
    fn exec(
        core: &mut Core,
        _ppu: &mut Ppu,
        _cartridge: &mut dyn CpuCartridgeBus,
        _hub: &mut dyn ControllerHub,
        _apu: &mut Apu,
    ) -> CpuStepStateEnum {
        core.register
            .set_pc(core.internal_stat.get_address() as u16);
        exit_opcode(core)
    }
}

pub(crate) struct Jsr;

impl CpuStepState for Jsr {
    fn exec(
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut dyn CpuCartridgeBus,
        hub: &mut dyn ControllerHub,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        match core.internal_stat.get_step() {
            1 => {
                let sp = usize::from(core.register.get_sp());
                let _ = core
                    .memory
                    .read(0x100 | sp, ppu, cartridge, hub, apu, &mut core.interrupt);

                core.internal_stat
                    .set_tempaddr(core.register.get_pc().wrapping_sub(1) as usize);
            }
            2 => {
                let hi = (core.internal_stat.get_tempaddr() >> 8) as u8;
                let sp = usize::from(core.register.get_sp());
                core.register.set_sp((sp.wrapping_sub(1) & 0xFF) as u8);
                core.memory.write(
                    0x100 | sp,
                    hi,
                    ppu,
                    cartridge,
                    hub,
                    apu,
                    &mut core.interrupt,
                );
            }
            3 => {
                let low = (core.internal_stat.get_tempaddr() & 0xFF) as u8;
                let sp = usize::from(core.register.get_sp());
                core.register.set_sp((sp.wrapping_sub(1) & 0xFF) as u8);
                core.memory.write(
                    0x100 | sp,
                    low,
                    ppu,
                    cartridge,
                    hub,
                    apu,
                    &mut core.interrupt,
                );
                core.register
                    .set_pc(core.internal_stat.get_address() as u16);
            }
            _ => {
                return exit_opcode(core);
            }
        }
        CpuStepStateEnum::Continue
    }
}

pub(crate) struct Rts;

impl CpuStepState for Rts {
    fn exec(
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut dyn CpuCartridgeBus,
        hub: &mut dyn ControllerHub,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        match core.internal_stat.get_step() {
            1 => {
                // dummy read
                read_dummy_current(core, ppu, cartridge, hub, apu);
            }
            2 => {
                // dummy read
                let sp = usize::from(core.register.get_sp());
                let _ = core
                    .memory
                    .read(sp | 0x100, ppu, cartridge, hub, apu, &mut core.interrupt);

                core.register.set_sp((sp.wrapping_add(1) & 0xFF) as u8);
            }
            3 => {
                let sp = usize::from(core.register.get_sp());
                core.internal_stat
                    .set_tempaddr(usize::from(core.memory.read(
                        sp | 0x100,
                        ppu,
                        cartridge,
                        hub,
                        apu,
                        &mut core.interrupt,
                    )));

                core.register.set_sp((sp.wrapping_add(1) & 0xFF) as u8);
            }
            4 => {
                let sp = usize::from(core.register.get_sp());
                let high =
                    core.memory
                        .read(sp | 0x100, ppu, cartridge, hub, apu, &mut core.interrupt);
                core.internal_stat
                    .set_tempaddr(core.internal_stat.get_tempaddr() | usize::from(high) << 8);
            }
            5 => {
                core.register
                    .set_pc(core.internal_stat.get_tempaddr() as u16);
                let _ = core.memory.read_next(
                    &mut core.register,
                    ppu,
                    cartridge,
                    hub,
                    apu,
                    &mut core.interrupt,
                );
            }
            _ => {
                return exit_opcode(core);
            }
        }
        CpuStepStateEnum::Continue
    }
}
