/// OAM DMA controller (DMG/CGB).
///
/// Transfer model matches real hardware (validated by mooneye):
/// - Writing $FF46 arms a transfer: the internal destination pointer becomes
///   `0xFF` (warm-up). CPU reads of OAM/VRAM return $FF while the DMA is
///   active and the destination pointer is non-zero, or whenever a previous
///   DMA was restarted.
/// - Each M-cycle advances the destination pointer by one byte. The first
///   M-cycle after arming (pointer 0xFF -> 0x00) performs no byte transfer
///   (warm-up), so OAM is still accessible on that M-cycle.
/// - Bytes are transferred while the pointer is in 0x00..=0x9F. When it
///   reaches 0xA0 the DMA completes (pointer -> 0xA1) on the next M-cycle.
#[derive(Debug, Clone)]
pub struct DmaController {
    /// Internal destination pointer. 0xA1 means inactive.
    dest: u8,
    /// Base source address (high byte of the written value).
    source: u16,
    /// True when a DMA was restarted while one was still running.
    restarting: bool,
    /// Last value written to the DMA register ($FF46, readable).
    register: u8,
}

impl DmaController {
    pub fn new() -> Self {
        Self {
            dest: 0xA1,
            source: 0,
            restarting: false,
            register: 0,
        }
    }

    /// Start a DMA transfer from the given high byte of the source address.
    pub fn start(&mut self, source_high: u8) {
        self.restarting = self.active();
        self.dest = 0xFF;
        self.source = (source_high as u16) << 8;
        self.register = source_high;
    }

    /// The value readable from the DMA register ($FF46).
    pub fn read_register(&self) -> u8 {
        self.register
    }

    /// Set the readable DMA register value without starting a transfer
    /// (used for the post-boot state, where it reads $FF).
    pub fn set_register(&mut self, value: u8) {
        self.register = value;
    }

    pub fn active(&self) -> bool {
        self.dest != 0xA1
    }

    /// OAM/VRAM access locked while DMA is active (non-zero destination).
    pub fn is_oam_locked(&self) -> bool {
        self.active() && (self.dest != 0 || self.restarting)
    }

    /// Advance the DMA by one M-cycle. Returns the (source address, OAM
    /// offset) pair to transfer, or None when no byte is transferred this
    /// M-cycle (warm-up or completion).
    pub fn transfer_step(&mut self) -> Option<(u16, u8)> {
        if self.dest == 0xA1 {
            return None;
        }
        if self.dest >= 0xA0 {
            // Warm-up (0xFF -> 0x00) or completion (0xA0 -> 0xA1).
            self.dest = self.dest.wrapping_add(1);
            return None;
        }
        let offset = self.dest;
        self.dest += 1;
        let src = self.source;
        self.source += 1;
        Some((src, offset))
    }

    pub fn completed(&self) -> bool {
        !self.active()
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
    fn start_sets_active_and_warmup() {
        let mut dma = DmaController::new();
        dma.start(0xC0);
        assert!(dma.active());
        // Warm-up M-cycle: no byte transferred.
        assert_eq!(dma.transfer_step(), None);
        assert!(dma.active());
    }

    #[test]
    fn dma_completes_after_160_transfers_plus_warmup() {
        let mut dma = DmaController::new();
        dma.start(0x00);
        let mut transferred = 0;
        // 1 warm-up + 160 bytes + 1 completion M-cycle.
        for _ in 0..162 {
            if dma.transfer_step().is_some() {
                transferred += 1;
            }
        }
        assert_eq!(transferred, 160);
        assert!(dma.completed());
        assert!(!dma.active());
    }

    #[test]
    fn transfer_step_advances_source_and_offset() {
        let mut dma = DmaController::new();
        dma.start(0x80);
        dma.transfer_step(); // warm-up
        assert_eq!(dma.transfer_step(), Some((0x8000, 0)));
        assert_eq!(dma.transfer_step(), Some((0x8001, 1)));
    }

    #[test]
    fn oam_unlocked_during_warmup_fresh_dma() {
        let mut dma = DmaController::new();
        dma.start(0x80);
        // Fresh DMA: OAM locked while arming (dest 0xFF), then unlocked
        // during the warm-up M-cycle before the first byte transfers.
        assert!(dma.is_oam_locked());
        dma.transfer_step(); // warm-up, dest 0xFF -> 0x00
        assert!(!dma.is_oam_locked());
        dma.transfer_step(); // first byte, dest 0x00 -> 0x01
        assert!(dma.is_oam_locked());
    }

    #[test]
    fn oam_locked_during_restart_warmup() {
        let mut dma = DmaController::new();
        dma.start(0x80);
        dma.transfer_step(); // warm-up
        dma.transfer_step(); // transfer byte 0
        // Restart while running: OAM stays locked through the new warm-up.
        dma.start(0x40);
        assert!(dma.is_oam_locked());
        dma.transfer_step(); // new warm-up
        assert!(dma.is_oam_locked());
    }

    #[test]
    fn register_reads_back_written_value() {
        let mut dma = DmaController::new();
        dma.start(0x9F);
        assert_eq!(dma.read_register(), 0x9F);
    }
}
