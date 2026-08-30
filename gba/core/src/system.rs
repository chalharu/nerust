use crate::memory::GbaMemoryBus;

pub struct GbaSystem {
    pub bus: GbaMemoryBus,
    tick: u64,
}

impl GbaSystem {
    pub fn new() -> Self {
        Self {
            bus: GbaMemoryBus::new(),
            tick: 0,
        }
    }

    pub fn bus(&self) -> &GbaMemoryBus {
        &self.bus
    }

    pub fn bus_mut(&mut self) -> &mut GbaMemoryBus {
        &mut self.bus
    }

    /// 1 T-cycle 進行。VCOUNT 更新は Bus 側に委譲。
    /// TODO(gba-tick-frame): Phase 3では常に false。Phase 8で frame_done を返す。
    pub fn step_tcycle(&mut self) -> bool {
        self.tick = self.tick.wrapping_add(1);
        self.bus.tick()
    }
}

impl Default for GbaSystem {
    fn default() -> Self {
        Self::new()
    }
}
