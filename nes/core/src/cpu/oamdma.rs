use super::{Apu, ControllerHub, Core, CpuCartridgeBus, Ppu, read_dummy_current};

#[derive(serde::Serialize, serde::Deserialize, PartialEq, Eq, Clone, Copy)]
pub(crate) enum OamDmaStateEnumValue {
    Step0,
    Step1,
    Step2,
    None,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub(crate) struct OamDmaStateValue {
    offset: u8,
    count: u8,
    value: u8,
}

impl OamDmaStateValue {
    pub(crate) fn new() -> Self {
        Self {
            offset: 0,
            count: 0,
            value: 0,
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct OamDmaState {
    #[serde(skip, default = "make_state_pool")]
    state_pool: [Box<dyn OamDmaStepState>; OamDmaStateEnumValue::None as usize],
    state: OamDmaStateEnumValue,
    value: OamDmaStateValue,
}

fn make_state_pool() -> [Box<dyn OamDmaStepState>; OamDmaStateEnumValue::None as usize] {
    [
        Box::new(OamDma),
        Box::new(OamDmaStep1),
        Box::new(OamDmaStep2),
    ]
}

impl Clone for OamDmaState {
    fn clone(&self) -> Self {
        Self {
            state_pool: make_state_pool(),
            state: self.state,
            value: self.value.clone(),
        }
    }
}

impl OamDmaState {
    pub(crate) fn new() -> OamDmaState {
        Self {
            state_pool: make_state_pool(),
            state: OamDmaStateEnumValue::None,
            value: OamDmaStateValue::new(),
        }
    }

    pub(crate) fn has_transaction(&self) -> bool {
        self.state != OamDmaStateEnumValue::None
    }

    pub(crate) fn start_transaction(&mut self, offset: u8) {
        self.state = OamDmaStateEnumValue::Step0;
        self.value.offset = offset;
        self.value.count = 255;
    }

    pub(crate) fn reset(&mut self) {
        self.state = OamDmaStateEnumValue::None;
    }

    pub(crate) fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut dyn CpuCartridgeBus,
        hub: &mut dyn ControllerHub,
        apu: &mut Apu,
    ) {
        self.state = self.state_pool[self.state as usize].next(
            core,
            ppu,
            cartridge,
            hub,
            apu,
            &mut self.value,
        );
    }

    #[allow(dead_code, reason = "OAM DMA state query")]
    pub(crate) fn count(&self) -> Option<u8> {
        if self.state == OamDmaStateEnumValue::None {
            None
        } else {
            Some(self.value.count)
        }
    }
}

pub(crate) trait OamDmaStepState {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut dyn CpuCartridgeBus,
        hub: &mut dyn ControllerHub,
        apu: &mut Apu,
        value: &mut OamDmaStateValue,
    ) -> OamDmaStateEnumValue;
}

pub(crate) struct OamDma;

impl OamDmaStepState for OamDma {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut dyn CpuCartridgeBus,
        hub: &mut dyn ControllerHub,
        apu: &mut Apu,
        _value: &mut OamDmaStateValue,
    ) -> OamDmaStateEnumValue {
        // dummy read
        read_dummy_current(core, ppu, cartridge, hub, apu);
        if core.cycles & 1 != 0 {
            OamDmaStateEnumValue::Step0
        } else {
            OamDmaStateEnumValue::Step1
        }
    }
}

struct OamDmaStep1;

impl OamDmaStepState for OamDmaStep1 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut dyn CpuCartridgeBus,
        hub: &mut dyn ControllerHub,
        apu: &mut Apu,
        value: &mut OamDmaStateValue,
    ) -> OamDmaStateEnumValue {
        if core.cycles & 1 == 0 {
            read_dummy_current(core, ppu, cartridge, hub, apu);
            return OamDmaStateEnumValue::Step1;
        }
        value.value = core.memory.read(
            usize::from(value.offset) * 0x100 + usize::from(255 - value.count),
            ppu,
            cartridge,
            hub,
            apu,
            &mut core.interrupt,
        );
        OamDmaStateEnumValue::Step2
    }
}

struct OamDmaStep2;

impl OamDmaStepState for OamDmaStep2 {
    fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut dyn CpuCartridgeBus,
        hub: &mut dyn ControllerHub,
        apu: &mut Apu,
        value: &mut OamDmaStateValue,
    ) -> OamDmaStateEnumValue {
        if core.cycles & 1 != 0 {
            read_dummy_current(core, ppu, cartridge, hub, apu);
            return OamDmaStateEnumValue::Step2;
        }
        core.memory.write(
            0x2004,
            value.value,
            ppu,
            cartridge,
            hub,
            apu,
            &mut core.interrupt,
        );
        if value.count == 0 {
            OamDmaStateEnumValue::None
        } else {
            value.count -= 1;
            OamDmaStateEnumValue::Step1
        }
    }
}
