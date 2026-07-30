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

    /// Read HDMA registers FF51-FF54.
    pub fn read_register(&self, addr: u16) -> u8 {
        match addr {
            0xFF51 => (self.src >> 8) as u8,
            0xFF52 => (self.src & 0xFF) as u8,
            0xFF53 => (self.dst >> 8) as u8,
            0xFF54 => (self.dst & 0xFF) as u8,
            _ => 0xFF,
        }
    }

    /// Write HDMA registers FF51-FF54.
    pub fn write_register(&mut self, addr: u16, value: u8) {
        match addr {
            0xFF51 => self.src = (self.src & 0x00FF) | ((value as u16) << 8),
            0xFF52 => self.src = (self.src & 0xFF00) | (value as u16),
            0xFF53 => self.dst = (self.dst & 0x00FF) | ((value as u16) << 8),
            0xFF54 => self.dst = (self.dst & 0xFF00) | (value as u16),
            _ => {}
        }
    }
    /// Bit 7 = 0 while active, 1 when idle/completed/cancelled.
    /// Lower 7 bits = remaining - 1 (or $7F when idle).
    pub fn read_status(&self) -> u8 {
        if self.active {
            self.remaining - 1  // bit 7 = 0
        } else if self.remaining > 0 {
            // Cancelled: return remaining with bit 7 = 1
            (self.remaining - 1) | 0x80
        } else {
            // Completed or never started
            0xFF
        }
    }

    /// Cancel the current transfer. Keeps remaining for status read.
    pub fn cancel(&mut self) {
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
    /// Returns true when a new HBlank entry triggers an HDMA transfer.
    pub fn set_hblank(&mut self, on: bool) -> bool {
        if self.hblank_mode && self.active {
            if on && !self.hblank_transferred {
                self.hblank_transferred = true;
                return true;
            }
            if !on {
                self.hblank_transferred = false;
            }
        }
        false
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
