/// HDMA / GDMA controller (CGB only).
///
/// GDMA: triggered by bit 7 = 0 in $FF55. Transfers all blocks immediately.
/// HDMA: triggered by bit 7 = 1. Transfers 16 bytes per HBlank period.
#[derive(Debug, Clone)]
pub struct HdmaController {
    pub src: u16,
    pub dst: u16,
    /// Remaining length in blocks (1 block = 16 bytes).
    /// 0 = idle / completed.
    pub remaining: u8,
    /// Bit 7 of $FF55 as written: 1 = HDMA (HBlank), 0 = GDMA (immediate).
    pub hblank_mode: bool,
    /// True while a GDMA or HDMA transfer is active.
    pub active: bool,
    /// True when a new HBlank has been entered since last transfer.
    hblank_transferred: bool,
}

impl HdmaController {
    pub fn new() -> Self {
        Self {
            src: 0,
            dst: 0,
            remaining: 0,
            hblank_mode: false,
            active: false,
            hblank_transferred: false,
        }
    }

    pub fn set_source_raw(&mut self, src: u16) {
        self.src = src;
    }

    pub fn set_dest_raw(&mut self, dst: u16) {
        self.dst = dst;
    }

    /// Start a transfer. `value` is the byte written to $FF55:
    ///   bit 7: 0=GDMA, 1=HDMA
    ///   bits 0-6: length = (value & 0x7F) + 1 blocks (16 bytes each)
    pub fn start(&mut self, value: u8) {
        self.hblank_mode = value & 0x80 != 0;
        self.remaining = (value & 0x7F) + 1;
        self.active = true;
        self.hblank_transferred = false;
    }

    pub fn active(&self) -> bool {
        self.active
    }

    /// Read $FF55: returns remaining length | status.
    /// Bit 7 = 0 if active, 1 if completed.
    pub fn read_status(&self) -> u8 {
        if self.active {
            self.remaining - 1
        } else {
            0xFF
        }
    }

    /// Cancel the current transfer (write to $FF55 while active or on mode change).
    pub fn cancel(&mut self) {
        self.remaining = 0;
        self.active = false;
    }

    /// Advance src/dst after a block transfer. Returns false when complete.
    pub fn advance(&mut self) -> bool {
        self.src = self.src.wrapping_add(16);
        self.dst = self.dst.wrapping_add(16);
        self.remaining = self.remaining.saturating_sub(1);
        if self.remaining == 0 {
            self.active = false;
            false
        } else {
            true
        }
    }

    /// Called from step_devices to track HBlank transitions.
    pub fn set_hblank(&mut self, on: bool) {
        if self.hblank_mode && self.active {
            if on && !self.hblank_transferred {
                self.hblank_transferred = true;
            } else if !on {
                self.hblank_transferred = false;
            }
        }
    }

    /// Whether we should transfer a block this step (HDMA in new HBlank).
    pub fn should_transfer_hblank(&self) -> bool {
        self.active && self.hblank_mode && self.hblank_transferred
    }

    /// For GDMA: returns true as long as active.
    pub fn should_transfer_gdma(&self) -> bool {
        self.active && !self.hblank_mode
    }
}

impl Default for HdmaController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gdma_start_sets_active() {
        let mut hdma = HdmaController::new();
        hdma.set_source_raw(0x0000);
        hdma.set_dest_raw(0x8000);
        hdma.start(7); // length = 8 blocks
        assert!(hdma.active);
        assert_eq!(hdma.remaining, 8);
        assert!(!hdma.hblank_mode);
    }

    #[test]
    fn hdma_start_sets_hblank_mode() {
        let mut hdma = HdmaController::new();
        hdma.start(0x80 | 3); // HDMA, length = 4
        assert!(hdma.active);
        assert!(hdma.hblank_mode);
        assert_eq!(hdma.remaining, 4);
    }

    #[test]
    fn read_status_returns_remaining_minus_1_when_active() {
        let mut hdma = HdmaController::new();
        hdma.set_source_raw(0x0000);
        hdma.set_dest_raw(0x8000);
        hdma.start(7); // remaining = 8
        assert_eq!(hdma.read_status(), 7);
    }

    #[test]
    fn completed_read_status_returns_0xff() {
        let hdma = HdmaController::new();
        assert_eq!(hdma.read_status(), 0xFF);
    }
}
