/// GBA CPU レジスタファイル。R0-R15 + CPSR + SPSRバンク + R13/R14バンク。
#[derive(Debug, Clone)]
pub struct CpuRegisters {
    r: [u32; 16],
    cpsr: u32,
    spsr: [u32; 5],             // FIQ, SVC, ABT, IRQ, UND (SYS/USRはSPSRなし)
    bank_r8_r12: [[u32; 5]; 2], // USR/SYS/IRQ/SVC/ABT/UND 共用 + FIQ
    bank_r13: [u32; 6],         // USR/SYS 共用 + FIQ/SVC/ABT/IRQ/UND
    bank_r14: [u32; 6],
    pc_written: bool,
}

impl Default for CpuRegisters {
    fn default() -> Self {
        Self {
            r: [0; 16],
            cpsr: 0x1F, // SYSモード
            spsr: [0; 5],
            bank_r8_r12: [[0; 5]; 2],
            bank_r13: [0; 6],
            bank_r14: [0; 6],
            pc_written: false,
        }
    }
}

impl CpuRegisters {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn post_bios() -> Self {
        let mut r = Self::default();
        r.r[15] = 0x08000000;
        r.cpsr = 0x0000001F; // SYS, T=0, I/F=0
        // System/User SP
        r.r[13] = 0x03007F00;
        r.bank_r13[0] = 0x03007F00;
        // Banked stacks as set by real BIOS (GBATEK)
        r.bank_r13[2] = 0x03007FA0; // IRQ
        r.bank_r13[3] = 0x03007FE0; // SVC
        r.bank_r13[4] = 0x03007F00; // ABT
        r.bank_r13[5] = 0x03007F00; // UND
        r.bank_r14[2] = 0;
        r.bank_r14[3] = 0;
        r
    }

    // -- PC / SP / LR --

    pub fn pc(&self) -> u32 {
        self.r[15] & !1
    }

    pub fn set_pc(&mut self, v: u32) {
        self.r[15] = if self.cpsr_t() { v & !1 } else { v & !3 };
        self.pc_written = true;
    }

    pub fn clear_pc_written(&mut self) {
        self.pc_written = false;
    }

    pub fn take_pc_written(&mut self) -> bool {
        std::mem::take(&mut self.pc_written)
    }

    pub fn sp(&self) -> u32 {
        self.r[13]
    }

    pub fn set_sp(&mut self, v: u32) {
        self.r[13] = v;
    }

    pub fn lr(&self) -> u32 {
        self.r[14]
    }

    pub fn set_lr(&mut self, v: u32) {
        self.r[14] = v;
    }

    pub fn r(&self, idx: usize) -> u32 {
        self.r[idx & 0xF]
    }

    pub fn set_r(&mut self, idx: usize, v: u32) {
        let idx = idx & 0xF;
        if idx == 15 {
            self.set_pc(v);
        } else {
            self.r[idx] = v;
        }
    }

    /// Read the User/System register bank while remaining in the current privileged mode.
    pub fn user_r(&self, idx: usize) -> u32 {
        match idx & 0xF {
            8..=12 if self.cpsr_mode() == 0x11 => self.bank_r8_r12[0][idx - 8],
            13 if !matches!(self.cpsr_mode(), 0x10 | 0x1F) => self.bank_r13[0],
            14 if !matches!(self.cpsr_mode(), 0x10 | 0x1F) => self.bank_r14[0],
            register => self.r[register],
        }
    }

    /// Write the User/System register bank without switching processor mode.
    pub fn set_user_r(&mut self, idx: usize, value: u32) {
        match idx & 0xF {
            8..=12 if self.cpsr_mode() == 0x11 => self.bank_r8_r12[0][idx - 8] = value,
            13 if !matches!(self.cpsr_mode(), 0x10 | 0x1F) => self.bank_r13[0] = value,
            14 if !matches!(self.cpsr_mode(), 0x10 | 0x1F) => self.bank_r14[0] = value,
            register => self.r[register] = value,
        }
    }

    // -- CPSR --

    pub fn cpsr(&self) -> u32 {
        self.cpsr
    }

    pub fn set_cpsr(&mut self, v: u32) {
        let old_mode = self.cpsr & 0x1F;
        let new_mode = v & 0x1F;
        if old_mode != new_mode {
            self.switch_bank(old_mode, new_mode);
        }
        self.cpsr = v;
    }

    pub fn cpsr_n(&self) -> bool {
        self.cpsr & (1 << 31) != 0
    }
    pub fn cpsr_z(&self) -> bool {
        self.cpsr & (1 << 30) != 0
    }
    pub fn cpsr_c(&self) -> bool {
        self.cpsr & (1 << 29) != 0
    }
    pub fn cpsr_v(&self) -> bool {
        self.cpsr & (1 << 28) != 0
    }
    pub fn cpsr_t(&self) -> bool {
        self.cpsr & (1 << 5) != 0
    }
    pub fn cpsr_mode(&self) -> u8 {
        (self.cpsr & 0x1F) as u8
    }

    pub fn set_cpsr_n(&mut self, v: bool) {
        if v {
            self.cpsr |= 1 << 31;
        } else {
            self.cpsr &= !(1 << 31);
        }
    }
    pub fn set_cpsr_z(&mut self, v: bool) {
        if v {
            self.cpsr |= 1 << 30;
        } else {
            self.cpsr &= !(1 << 30);
        }
    }
    pub fn set_cpsr_c(&mut self, v: bool) {
        if v {
            self.cpsr |= 1 << 29;
        } else {
            self.cpsr &= !(1 << 29);
        }
    }
    pub fn set_cpsr_v(&mut self, v: bool) {
        if v {
            self.cpsr |= 1 << 28;
        } else {
            self.cpsr &= !(1 << 28);
        }
    }

    // -- SPSR --

    pub fn spsr(&self) -> u32 {
        let idx = Self::spsr_index(self.cpsr_mode());
        if let Some(i) = idx { self.spsr[i] } else { 0 }
    }

    pub fn set_spsr(&mut self, v: u32) {
        if let Some(i) = Self::spsr_index(self.cpsr_mode()) {
            self.spsr[i] = v;
        }
    }

    pub fn enter_exception(
        &mut self,
        mode: u8,
        vector: u32,
        return_address: u32,
        disable_irq: bool,
    ) {
        let old_cpsr = self.cpsr;
        let mut new_cpsr = (old_cpsr & !(0x1F | (1 << 5))) | u32::from(mode);
        if disable_irq {
            new_cpsr |= 1 << 7;
        }
        self.set_cpsr(new_cpsr);
        self.set_spsr(old_cpsr);
        self.set_lr(return_address);
        self.set_pc(vector);
    }

    // -- Mode switch --

    fn mode_to_bank_index(mode: u32) -> Option<usize> {
        match mode {
            0x10 | 0x1F => Some(0), // USR/SYS
            0x11 => Some(1),        // FIQ
            0x12 => Some(2),        // IRQ
            0x13 => Some(3),        // SVC
            0x17 => Some(4),        // ABT
            0x1B => Some(5),        // UND
            _ => None,
        }
    }

    fn spsr_index(mode: u8) -> Option<usize> {
        match mode as u32 {
            0x11 => Some(0), // FIQ
            0x13 => Some(1), // SVC
            0x17 => Some(2), // ABT
            0x12 => Some(3), // IRQ
            0x1B => Some(4), // UND
            _ => None,
        }
    }

    fn switch_bank(&mut self, old_mode: u32, new_mode: u32) {
        let old_fiq = old_mode == 0x11;
        let new_fiq = new_mode == 0x11;
        if old_fiq != new_fiq {
            let old_idx = usize::from(old_fiq);
            let new_idx = usize::from(new_fiq);
            self.bank_r8_r12[old_idx].copy_from_slice(&self.r[8..13]);
            self.r[8..13].copy_from_slice(&self.bank_r8_r12[new_idx]);
        }

        let old_idx = Self::mode_to_bank_index(old_mode);
        let new_idx = Self::mode_to_bank_index(new_mode);
        if let Some(o) = old_idx {
            self.bank_r13[o] = self.r[13];
            self.bank_r14[o] = self.r[14];
        }
        if let Some(n) = new_idx {
            self.r[13] = self.bank_r13[n];
            self.r[14] = self.bank_r14[n];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_bios_registers() {
        let r = CpuRegisters::post_bios();
        assert_eq!(r.pc(), 0x08000000);
        assert_eq!(r.sp(), 0x03007F00);
        assert_eq!(r.cpsr() & 0x1F, 0x1F);
        assert!(!r.cpsr_t());
    }

    #[test]
    fn bank_switch_fiq() {
        let mut r = CpuRegisters::default();
        r.set_r(8, 0x8888);
        r.set_r(13, 0x1111);
        r.set_r(14, 0x2222);
        r.set_cpsr(0x11); // FIQ
        assert_eq!(r.r(8), 0);
        assert_ne!(r.sp(), 0x1111);
        r.set_r(8, 0x9999);
        r.set_r(13, 0x3333);
        r.set_cpsr(0x1F); // back to SYS
        assert_eq!(r.r(8), 0x8888);
        assert_eq!(r.sp(), 0x1111);
        r.set_cpsr(0x11);
        assert_eq!(r.r(8), 0x9999);
        assert_eq!(r.sp(), 0x3333);
    }

    #[test]
    fn non_fiq_modes_share_r8_r12() {
        let mut r = CpuRegisters::default();
        r.set_r(8, 0x1234);
        r.set_cpsr(0x12); // IRQ
        assert_eq!(r.r(8), 0x1234);
        r.set_r(8, 0x5678);
        r.set_cpsr(0x13); // SVC
        assert_eq!(r.r(8), 0x5678);
    }

    #[test]
    fn cpsr_flags() {
        let mut r = CpuRegisters::default();
        r.set_cpsr_n(true);
        r.set_cpsr_z(true);
        assert!(r.cpsr_n());
        assert!(r.cpsr_z());
        r.set_cpsr_n(false);
        assert!(!r.cpsr_n());
    }
}
