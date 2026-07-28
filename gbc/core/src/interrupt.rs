/// Five interrupt sources available on DMG/CGB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum InterruptKind {
    VBlank = 0,
    LcdStat = 1,
    Timer = 2,
    Serial = 3,
    Joypad = 4,
}

impl InterruptKind {
    pub fn bit(self) -> u8 {
        1 << (self as u8)
    }

    pub fn vector(self) -> u16 {
        0x40 + (self as u16) * 8
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HaltState {
    Running,
    Halted { bug_triggered: bool },
    Stopped,
}

pub struct InterruptController {
    ime: bool,
    ie: u8,
    if_: u8,
    halted: HaltState,
}

impl InterruptController {
    pub fn new() -> Self {
        Self {
            ime: false,
            ie: 0,
            if_: 0xE1,
            halted: HaltState::Running,
        }
    }

    pub fn request(&mut self, kind: InterruptKind) {
        self.if_ |= kind.bit();
    }

    pub fn acknowledge(&mut self) -> Option<InterruptKind> {
        if !self.ime {
            // IME=0: still wake from halt if an interrupt is pending
            let pending = self.ie & self.if_ & 0x1F;
            if pending != 0 && matches!(self.halted, HaltState::Halted { .. }) {
                self.halted = HaltState::Running;
            }
            return None;
        }

        let fired = self.ie & self.if_ & 0x1F;
        if fired == 0 {
            return None;
        }

        self.ime = false;
        self.halted = HaltState::Running;

        let n = fired.trailing_zeros();
        let kind = match n {
            0 => InterruptKind::VBlank,
            1 => InterruptKind::LcdStat,
            2 => InterruptKind::Timer,
            3 => InterruptKind::Serial,
            4 => InterruptKind::Joypad,
            _ => return None,
        };

        self.if_ &= !kind.bit();
        Some(kind)
    }

    pub fn interrupt_pending(&self) -> bool {
        self.ime && (self.ie & self.if_ & 0x1F) != 0
    }

    pub fn halt(&mut self) {
        let bug = !self.ime && (self.ie & self.if_ & 0x1F) != 0;
        self.halted = HaltState::Halted { bug_triggered: bug };
    }

    pub fn stop(&mut self) {
        self.halted = HaltState::Stopped;
    }

    pub fn wake_by_joypad(&mut self, buttons: u8) -> bool {
        if matches!(self.halted, HaltState::Stopped) && (buttons & 0x0F) != 0x0F {
            self.halted = HaltState::Running;
            true
        } else {
            false
        }
    }

    pub fn read_ie(&self) -> u8 {
        self.ie
    }

    pub fn write_ie(&mut self, v: u8) {
        self.ie = v;
    }

    pub fn read_if(&self) -> u8 {
        self.if_ | 0xE0
    }

    pub fn write_if(&mut self, v: u8) {
        self.if_ = v & 0x1F;
    }

    pub fn is_halted(&self) -> bool {
        !matches!(self.halted, HaltState::Running)
    }

    pub fn is_halted_or_stopped(&self) -> bool {
        self.is_halted()
    }

    pub fn is_halt_bug_active(&self) -> bool {
        matches!(
            self.halted,
            HaltState::Halted {
                bug_triggered: true
            }
        )
    }

    pub fn set_ime(&mut self, v: bool) {
        self.ime = v;
    }

    pub fn get_ime(&self) -> bool {
        self.ime
    }
}

impl Default for InterruptController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_sets_if_bit() {
        let mut ic = InterruptController::new();
        ic.request(InterruptKind::VBlank);
        assert_eq!(ic.if_ & 0x01, 0x01);
        ic.request(InterruptKind::Timer);
        assert_eq!(ic.if_ & 0x04, 0x04);
    }

    #[test]
    fn acknowledge_returns_none_when_no_interrupt_enabled() {
        let mut ic = InterruptController::new();
        ic.set_ime(true);
        ic.request(InterruptKind::VBlank);
        assert!(ic.acknowledge().is_none());
    }

    #[test]
    fn acknowledge_returns_none_when_ime_disabled() {
        let mut ic = InterruptController::new();
        ic.write_ie(0x01);
        ic.request(InterruptKind::VBlank);
        assert!(ic.acknowledge().is_none());
    }

    #[test]
    fn acknowledge_clears_ime_and_if_bit() {
        let mut ic = InterruptController::new();
        ic.write_ie(0x01);
        ic.request(InterruptKind::VBlank);
        ic.set_ime(true);
        let kind = ic.acknowledge().expect("should ack");
        assert_eq!(kind, InterruptKind::VBlank);
        assert!(!ic.get_ime());
        assert_eq!(ic.if_ & 0x01, 0x00);
    }

    #[test]
    fn acknowledge_respects_priority_order() {
        let mut ic = InterruptController::new();
        ic.write_ie(0x1F); // all enabled
        ic.request(InterruptKind::Timer);
        ic.request(InterruptKind::VBlank);
        ic.set_ime(true);

        let kind = ic.acknowledge().expect("should ack");
        assert_eq!(kind, InterruptKind::VBlank);
        assert_eq!(ic.if_, 0xE0 | 0x04); // Timer still pending
    }

    #[test]
    fn halt_sets_halt_state() {
        let mut ic = InterruptController::new();
        assert!(!ic.is_halted());
        ic.halt();
        assert!(ic.is_halted());
    }

    #[test]
    fn halt_bug_detected_when_ime_disabled_and_irq_pending() {
        let mut ic = InterruptController::new();
        ic.write_ie(0x01);
        ic.request(InterruptKind::VBlank);
        ic.halt();
        assert!(ic.is_halt_bug_active());
    }

    #[test]
    fn stop_then_wake_by_joypad() {
        let mut ic = InterruptController::new();
        ic.stop();
        assert!(ic.is_halted());

        // All buttons released → no wake
        assert!(!ic.wake_by_joypad(0xFF));

        // Any button pressed → wake
        assert!(ic.wake_by_joypad(0xFE));
        assert!(!ic.is_halted());
    }

    #[test]
    fn read_if_has_upper_bits_set() {
        let mut ic = InterruptController::new();
        ic.write_if(0x00);
        assert_eq!(ic.read_if(), 0xE0);
    }

    #[test]
    fn interrupt_pending_with_ime_and_enabled_irq() {
        let mut ic = InterruptController::new();
        ic.write_ie(0x01);
        ic.request(InterruptKind::VBlank);
        ic.set_ime(true);
        assert!(ic.interrupt_pending());
    }

    #[test]
    fn no_interrupt_pending_when_ime_disabled() {
        let mut ic = InterruptController::new();
        ic.write_ie(0x01);
        ic.request(InterruptKind::VBlank);
        assert!(!ic.interrupt_pending());
    }

    #[test]
    fn if_write_masks_upper_bits() {
        let mut ic = InterruptController::new();
        ic.write_if(0xFF);
        assert_eq!(ic.read_if() & 0x1F, 0x1F);
    }

    #[test]
    fn ie_write_preserves_full_byte() {
        let mut ic = InterruptController::new();
        ic.write_ie(0xFF);
        assert_eq!(ic.read_ie(), 0xFF);
    }

    #[test]
    fn wake_by_joypad_no_effect_on_halted_not_stopped() {
        let mut ic = InterruptController::new();
        ic.halt();
        assert!(!ic.wake_by_joypad(0x00));
        assert!(ic.is_halted()); // still halted
    }

    #[test]
    fn halt_bug_not_triggered_when_ime_is_set() {
        let mut ic = InterruptController::new();
        ic.set_ime(true);
        ic.write_ie(0x01);
        ic.request(InterruptKind::VBlank);
        ic.halt();
        assert!(!ic.is_halt_bug_active());
    }

    #[test]
    fn stop_then_wake_by_any_button_press() {
        let mut ic = InterruptController::new();
        ic.stop();
        assert!(ic.wake_by_joypad(0x00));
        assert!(!ic.is_halted());
    }

    #[test]
    fn stop_no_wake_when_all_released() {
        let mut ic = InterruptController::new();
        ic.stop();
        assert!(!ic.wake_by_joypad(0x0F));
    }
}
