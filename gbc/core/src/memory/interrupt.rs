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
        let fired = self.ie & self.if_ & 0x1F;
        if fired == 0 {
            return None;
        }

        self.ime = false;

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
