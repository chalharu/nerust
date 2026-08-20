/// OAM DMA controller (DMG/CGB).
///
/// Transfers 160 bytes from any source address to OAM[$FE00..$FE9F].
/// Taking ~160 M-cycles (640 T-cycles for DMG, 320 for CGB double speed).
#[derive(Debug, Clone)]
pub struct DmaController {
    active: bool,
    source: u16,
    offset: u8,
}

impl DmaController {
    pub fn new() -> Self {
        Self {
            active: false,
            source: 0,
            offset: 0,
        }
    }

    /// Start a DMA transfer from the given high byte of the source address.
    pub fn start(&mut self, source_high: u8) {
        self.active = true;
        self.source = (source_high as u16) << 8;
        self.offset = 0;
    }

    pub fn active(&self) -> bool {
        self.active
    }

    /// Perform one transfer step. Returns (source_addr, oam_offset) for the
    /// caller to read from the source and write to OAM.
    pub fn transfer_step(&mut self) -> (u16, u8) {
        let src = self.source;
        let off = self.offset;
        self.offset += 1;
        if self.offset >= 160 {
            self.active = false;
        }
        (src, off)
    }

    pub fn completed(&self) -> bool {
        !self.active
    }

    /// OAM access locked while DMA is active.
    pub fn is_oam_locked(&self) -> bool {
        self.active
    }
}

impl Default for DmaController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_sets_active_and_source() {
        let mut dma = DmaController::new();
        dma.start(0xC0);
        assert!(dma.active());
        assert_eq!(dma.transfer_step(), (0xC000, 0));
    }

    #[test]
    fn dma_completes_after_160_steps() {
        let mut dma = DmaController::new();
        dma.start(0x00);
        for _ in 0..160 {
            dma.transfer_step();
        }
        assert!(dma.completed());
        assert!(!dma.active());
    }

    #[test]
    fn transfer_step_advances_offset() {
        let mut dma = DmaController::new();
        dma.start(0x80);
        assert_eq!(dma.transfer_step(), (0x8000, 0));
        assert_eq!(dma.transfer_step(), (0x8000, 1));
    }
}
