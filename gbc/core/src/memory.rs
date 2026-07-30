use crate::{
    apu::GbcApu,
    cartridge::Cartridge,
    cpu_core::Lr35902Cpu,
    dma::DmaController,
    interrupt::{InterruptController, InterruptKind},
    ppu::GbcPpu,
    serial::Serial,
    timer::Timer,
};

const WRAM_SIZE: usize = 0x8000;
const HRAM_SIZE: usize = 0x7F;

/// Top-level memory bus for the Game Boy / GBC.
///
/// Owns all hardware devices and address-space routing. All device fields
/// are private; external access is through facade methods.
pub struct GbcMemoryBus {
    cartridge: Cartridge,
    wram: Box<[u8; WRAM_SIZE]>,
    wram_bank: u8,
    hram: [u8; HRAM_SIZE],
    boot_rom: [u8; 0x100],
    boot_rom_mapped: bool,

    ppu: GbcPpu,
    apu: GbcApu,
    interrupt: InterruptController,
    timer: Timer,
    dma: DmaController,
    serial: Serial,
    joypad: u8,

    double_speed: bool,
    speed_switch_pending: bool,

    // CGB HDMA registers (FF51-FF55) — stub for v1
    hdma1: u8,
    hdma2: u8,
    hdma3: u8,
    hdma4: u8,
    hdma5: u8,

    /// Sub-cycle timing: T-cycles consumed by the current CPU instruction
    /// before this step. Set by the main loop before each cpu.step() call.
    /// Used by PPU write_register for sub-M-cycle mode3 pixel_x estimation.
    pub(crate) cpu_cycle_offset: u32,

    /// T-cycle accumulator for CPU/PPU synchronization.
    /// Each step_tcycle() increments this; CPU runs every 4th T-cycle.
    tick: u32,

    /// Double-speed PPU toggle: alternates every T-cycle to halve the
    /// effective PPU rate in double-speed mode.
    ppu_ds_toggle: bool,
    /// DMA transfer sub-cycle counter: 1 byte per 4 T-cycles.
    dma_tcounter: u8,
}

impl GbcMemoryBus {
    pub fn new(boot_rom: [u8; 0x100], boot_rom_mapped: bool) -> Self {
        Self {
            cartridge: Cartridge::default(),
            wram: Box::new([0; WRAM_SIZE]),
            wram_bank: 1,
            hram: [0; HRAM_SIZE],
            boot_rom,
            boot_rom_mapped,

            ppu: GbcPpu::default(),
            apu: GbcApu::default(),
            interrupt: InterruptController::new(),
            timer: Timer::new(),
            dma: DmaController::new(),
            serial: Serial::new(),
            joypad: 0xFF,

            double_speed: false,
            speed_switch_pending: false,

            hdma1: 0xFF,
            hdma2: 0xFF,
            hdma3: 0xFF,
            hdma4: 0xFF,
            hdma5: 0xFF,
            cpu_cycle_offset: 0,
            tick: 0,
            ppu_ds_toggle: false,
            dma_tcounter: 0,
        }
    }

    pub fn set_cartridge(&mut self, cartridge: Cartridge) {
        self.cartridge = cartridge;
    }

    // ── read / write ──────────────────────────────────────────

    pub fn read(&self, addr: u16) -> u8 {
        // OAM DMA: only OAM/VRAM are locked. Other regions accessible normally.
        if self.dma.is_oam_locked() && matches!(addr, 0x8000..=0x9FFF | 0xFE00..=0xFE9F) {
            return 0xFF;
        }

        match addr {
            0xFE00..=0xFE9F => self.ppu.read_oam((addr & 0xFF) as u8),
            0xFEA0..=0xFEFF => 0x00,
            0xFF00 => self.joypad | 0xC0,
            0xFF01 => self.serial.read_sb(),
            0xFF02 => self.serial.read_sc(),
            0xFF04..=0xFF07 => self.timer.read(addr),
            0xFF0F => self.interrupt.read_if(),
            0xFF10..=0xFF3F => self.apu.read_register(addr),
             0xFF40..=0xFF4B | 0xFF4F | 0xFF6C => self.ppu.read_register(addr),
            0xFF50 => {
                if self.boot_rom_mapped {
                    0xFE
                } else {
                    0xFF
                }
            }
            0xFF68..=0xFF6B => self.ppu.read_palette(addr),
            0xFF4D => self.read_key1(),
            0xFF51 => self.hdma1,
            0xFF52 => self.hdma2,
            0xFF53 => self.hdma3,
            0xFF54 => self.hdma4,
            0xFF55 => self.hdma5,
            0xFF70 => self.wram_bank,
            0xFF80..=0xFFFE => self.hram[(addr - 0xFF80) as usize],
            0xFFFF => self.interrupt.read_ie(),
            _ => self.read_storage(addr),
        }
    }

    /// Cartridge, VRAM, WRAM, HRAM, and boot ROM — shared between
    /// `read()` (CPU access) and `read_raw()` (DMA access).
    fn read_storage(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x00FF if self.boot_rom_mapped => self.boot_rom[addr as usize],
            0x0000..=0x7FFF => self.cartridge.read_rom(addr),
            0x8000..=0x9FFF => self.ppu.read_vram(addr),
            0xA000..=0xBFFF => self.cartridge.read_ram(addr),
            0xC000..=0xDFFF => self.wram[addr as usize & 0x1FFF],
            0xE000..=0xFDFF => self.read_storage(addr - 0x2000),
            _ => 0xFF,
        }
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        // OAM DMA: only OAM/VRAM writes are dropped. Other regions writable.
        if self.dma.is_oam_locked() && matches!(addr, 0x8000..=0x9FFF | 0xFE00..=0xFE9F) {
            return;
        }

        match addr {
            0x0000..=0x7FFF => self.cartridge.write_rom(addr, value),
            0x8000..=0x9FFF => self.ppu.write_vram(addr, value),
            0xA000..=0xBFFF => self.cartridge.write_ram(addr, value),
            0xC000..=0xDFFF => self.wram[addr as usize & 0x1FFF] = value,
            0xE000..=0xFDFF => self.write(addr - 0x2000, value),
            0xFE00..=0xFE9F => self.ppu.write_oam((addr & 0xFF) as u8, value),
            0xFF00 => self.joypad = (self.joypad & 0x30) | (value & 0x30),
            0xFF01 => self.serial.write_sb(value),
            0xFF02 => {
                if self.serial.write_sc(value) {
                    self.interrupt.request(InterruptKind::Serial);
                }
            }
            0xFF04..=0xFF07 => self.timer.write(addr, value),
            0xFF0F => self.interrupt.write_if(value),
            0xFF10..=0xFF3F => self.apu.write_register(addr, value),
               0xFF40..=0xFF45 | 0xFF47..=0xFF4B | 0xFF4F | 0xFF6C => {
                 self.ppu.write_register(addr, value);
             }
            0xFF46 => self.dma.start(value),
            0xFF4D => self.write_key1(value),
            // HDMA registers (CGB) — stub, filled in Phase 7
            0xFF51 => self.hdma1 = value,
            0xFF52 => self.hdma2 = value,
            0xFF53 => self.hdma3 = value,
            0xFF54 => self.hdma4 = value,
            0xFF55 => self.hdma5 = value,
            0xFF50 => {
                if value & 0x01 != 0 {
                    self.boot_rom_mapped = false;
                }
            }
            0xFF68..=0xFF6B => self.ppu.write_palette(addr, value),
            0xFF70 => {
                self.wram_bank = if value & 0x07 == 0 { 1 } else { value & 0x07 };
            }
            0xFF80..=0xFFFE => self.hram[(addr - 0xFF80) as usize] = value,
            0xFFFF => self.interrupt.write_ie(value),
            _ => {}
        }
    }

    // ── step_devices (legacy, used by test code) ─────────────

    /// Advance PPU+timer+DMA by 1 T-cycle (no CPU). Used by tests
    /// that step PPU directly without a CPU context.
    pub fn step_devices_tcycle(&mut self) -> bool {
        self.tick = self.tick.wrapping_add(1);
        let video = 1u32;
        let ppu_res = self.ppu.step(video);
        if ppu_res.lcd_stat { self.interrupt.request(InterruptKind::LcdStat); }
        if ppu_res.vblank { self.interrupt.request(InterruptKind::VBlank); }
        self.apu.step(video);
        if self.timer.step(1).overflow { self.interrupt.request(InterruptKind::Timer); }
        if self.dma.active() {
            self.dma_tcounter += 1;
            if self.dma_tcounter >= 4 {
                self.dma_tcounter = 0;
                let (src, offset) = self.dma.transfer_step();
                let byte = self.read_raw(src.wrapping_add(offset as u16));
                self.ppu.write_oam(offset, byte);
            }
        }
        ppu_res.frame_done
    }

    /// Legacy batch interface: advance by `cycles` T-cycles (no CPU).
    pub fn step_devices(&mut self, cycles: u32) -> bool {
        let mut done = false;
        for _ in 0..cycles { if self.step_devices_tcycle() { done = true; } }
        done
    }

    /// Advance ALL devices (including CPU) by 1 T-cycle.
    /// Call this 4 times per CPU M-cycle for proper T-cycle synchronization.
    /// Returns true if a PPU frame completed.
    pub fn step_tcycle(&mut self, cpu: &mut Lr35902Cpu) -> bool {
        self.tick = self.tick.wrapping_add(1);
        let t1 = self.tick % 4;
        self.cpu_cycle_offset = t1;

        let video = 1u32;
        let ppu_res = self.ppu.step(video);
        if ppu_res.lcd_stat { self.interrupt.request(InterruptKind::LcdStat); }
        if ppu_res.vblank { self.interrupt.request(InterruptKind::VBlank); }
        self.apu.step(video);
        if self.timer.step(1).overflow { self.interrupt.request(InterruptKind::Timer); }
        if self.dma.active() {
            self.dma_tcounter += 1;
            if self.dma_tcounter >= 4 {
                self.dma_tcounter = 0;
                let (src, offset) = self.dma.transfer_step();
                let byte = self.read_raw(src.wrapping_add(offset as u16));
                self.ppu.write_oam(offset, byte);
            }
        }
        // CPU runs every 4th T-cycle (end of M-cycle)
        if t1 == 3 {
            // cpu_cycle_offset is already set to 3 (T4 write for DMG).
            // cpu.step() processes 1 full M-cycle; write timing is
            // adjusted by mode3_pixel_x via cpu_cycle_offset.
            cpu.step(self);
        }
        ppu_res.frame_done
    }

    // ── facade methods ───────────────────────────────────────

    pub fn serial_output(&self) -> &[u8] {
        self.serial.output()
    }

    pub fn set_joypad(&mut self, state: u8) {
        self.joypad = state;
    }

    pub fn flush_audio(&mut self) -> Vec<f32> {
        self.apu.flush_samples()
    }

    pub fn render_frame(&self, fb: &mut nerust_render_traits::FrameBuffer) {
        self.ppu.render(fb);
    }

    pub fn acknowledge_interrupt(&mut self) -> Option<InterruptKind> {
        self.interrupt.acknowledge()
    }

    pub fn is_halted_or_stopped(&self) -> bool {
        self.interrupt.is_halted_or_stopped()
    }

    pub fn is_halt_bug_active(&self) -> bool {
        self.interrupt.is_halt_bug_active()
    }

    pub fn clear_halt_bug(&mut self) {
        self.interrupt.clear_halt_bug();
    }

    pub fn set_cgb_mode(&mut self, enabled: bool) {
        self.ppu.cgb_mode = enabled;
        self.ppu.cgb_game = enabled;
        if enabled {
            // Initialize CGB palettes with CGB boot ROM defaults.
            // When the boot ROM is skipped, these provide visible colors
            // for DMG compatibility mode instead of all-black.
            self.ppu.init_default_cgb_palettes();
            // CGB game mode (bit 7 = 1), opri not changed
            self.ppu.raw_set_key0(0x80);
        } else {
            self.ppu.raw_set_key0(0x00);
        }
    }

    /// Set whether the GAME itself is CGB-native (bit 7 of ROM header $143).
    /// This controls CGB-only rendering behavior independent of hardware mode.
    pub fn set_cgb_game(&mut self, enabled: bool) {
        self.ppu.cgb_game = enabled;
        // KEY0: bit 2 = DMG emulation mode, blocks CGB palette register writes
        // opri is not changed here (preserves initialization value)
        self.ppu.raw_set_key0(if enabled { 0x80 } else { 0x04 });
    }

    pub fn is_dma_active(&self) -> bool {
        self.dma.active()
    }

    pub fn stop(&mut self) {
        self.timer.reset_div();
        if self.speed_switch_pending {
            self.speed_switch_pending = false;
            self.double_speed = !self.double_speed;
        }
        self.interrupt.stop();
    }

    pub fn halt_cpu(&mut self) {
        self.interrupt.halt();
    }

    pub fn set_ime(&mut self, v: bool) {
        self.interrupt.set_ime(v);
    }

    pub fn ime_enabled(&self) -> bool {
        self.interrupt.get_ime()
    }

    // ── DMA access ───────────────────────────────────────────

    fn read_raw(&self, addr: u16) -> u8 {
        self.read_storage(addr)
    }

    pub fn read_dma(&self, addr: u16) -> Option<u8> {
        if (0xFF80..=0xFFFE).contains(&addr) {
            Some(self.hram[(addr - 0xFF80) as usize])
        } else {
            None
        }
    }

    pub fn write_dma(&mut self, addr: u16, value: u8) -> bool {
        if (0xFF80..=0xFFFE).contains(&addr) {
            self.hram[(addr - 0xFF80) as usize] = value;
            true
        } else {
            false
        }
    }

    // ── CGB double-speed ─────────────────────────────────────

    fn read_key1(&self) -> u8 {
        let mut val = 0x7E;
        if self.double_speed {
            val |= 0x80;
        }
        if self.speed_switch_pending {
            val |= 0x01;
        }
        val
    }

    fn write_key1(&mut self, value: u8) {
        if value & 0x01 != 0 {
            self.speed_switch_pending = true;
        }
    }
}

impl Default for GbcMemoryBus {
    fn default() -> Self {
        Self::new([0; 0x100], false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bus() -> GbcMemoryBus {
        GbcMemoryBus::new([0; 0x100], false)
    }

    #[test]
    fn read_joypad_has_upper_bits_set() {
        let bus = bus();
        assert_eq!(bus.read(0xFF00) & 0xC0, 0xC0);
    }

    #[test]
    fn read_timer_div_returns_upper_byte() {
        let bus = bus();
        let v = bus.read(0xFF04);
        assert_eq!(v, 0); // DIV starts at 0 (Timer::new with div=0)
    }

    #[test]
    fn read_interrupt_if_has_upper_bits_set() {
        let bus = bus();
        assert_eq!(bus.read(0xFF0F) & 0xE0, 0xE0);
    }

    #[test]
    fn write_dma_triggers_transfer() {
        let mut bus = bus();
        bus.write(0xFF46, 0xC0);
        assert!(bus.dma.active());
    }

    #[test]
    fn write_boot_rom_disable_unmaps() {
        let mut bus = GbcMemoryBus::new([0x00; 0x100], true);
        assert!(bus.boot_rom_mapped);
        bus.write(0xFF50, 0x01);
        assert!(!bus.boot_rom_mapped);
    }

    #[test]
    fn boot_rom_mapped_reads_from_rom_area() {
        let mut rom = [0u8; 0x100];
        rom[0x42] = 0xAB;
        let bus = GbcMemoryBus::new(rom, true);
        assert_eq!(bus.read(0x0042), 0xAB);
    }

    #[test]
    fn wram_read_write_roundtrip() {
        let mut bus = bus();
        bus.write(0xC000, 0x42);
        assert_eq!(bus.read(0xC000), 0x42);
    }

    #[test]
    fn echo_ram_mirrors_wram() {
        let mut bus = bus();
        bus.write(0xC000, 0x77);
        assert_eq!(bus.read(0xE000), 0x77);
    }

    #[test]
    fn write_timer_div_resets_counter() {
        let mut bus = bus();
        let _before = bus.read(0xFF04);
        bus.write(0xFF04, 0x00);
        assert_eq!(bus.read(0xFF04), 0x00);
    }

    #[test]
    fn write_serial_then_read_sc() {
        let mut bus = bus();
        bus.write(0xFF01, 0x55);
        bus.write(0xFF02, 0x01);
        assert_eq!(bus.read(0xFF02) & 0x7E, 0x7E);
    }

    #[test]
    fn write_to_unused_io_is_noop() {
        let mut bus = bus();
        bus.write(0xFF03, 0x42);
        assert_eq!(bus.read(0xFF0F) & 0xE0, 0xE0);
    }

    #[test]
    fn interrupt_write_flow_through_bus() {
        let mut bus = bus();
        bus.write(0xFFFF, 0x01); // IE ← VBlank enabled
        bus.write(0xFF0F, 0x01); // IF ← VBlank requested
        assert_eq!(bus.read(0xFF0F), 0xE1);
    }

    #[test]
    fn echo_write_mirrors_to_wram() {
        let mut bus = bus();
        bus.write(0xE000, 0xAB);
        assert_eq!(bus.read(0xC000), 0xAB);
    }

    #[test]
    fn joypad_write_preserves_select_bits() {
        let mut bus = bus();
        bus.write(0xFF00, 0x30);
        assert_eq!(bus.read(0xFF00) & 0xF0, 0xF0);
    }

    #[test]
    fn boot_rom_unmapped_reads_cartridge() {
        let bus = GbcMemoryBus::new([0xFF; 0x100], false);
        assert_eq!(bus.read(0x0000), 0x00); // default cartridge ROM is zeroed
    }

    // ── facade method delegation ──────────────────────────

    #[test]
    fn set_joypad_changes_read_value() {
        let mut bus = bus();
        bus.set_joypad(0x00);
        assert_eq!(bus.read(0xFF00), 0xC0);
    }

    #[test]
    fn flush_audio_returns_empty_from_stub() {
        let mut bus = bus();
        assert!(bus.flush_audio().is_empty());
    }

    #[test]
    fn acknowledge_interrupt_delegates() {
        let mut bus = bus();
        assert!(bus.acknowledge_interrupt().is_none());
    }

    #[test]
    fn is_halted_or_stopped_returns_false_by_default() {
        assert!(!bus().is_halted_or_stopped());
    }

    #[test]
    fn is_dma_active_returns_false_by_default() {
        assert!(!bus().is_dma_active());
    }

    // ── DMA constrained access ────────────────────────────

    #[test]
    fn read_dma_returns_some_for_hram() {
        let mut bus = bus();
        bus.write(0xFF80, 0x42);
        assert_eq!(bus.read_dma(0xFF80), Some(0x42));
    }

    #[test]
    fn read_dma_returns_none_below_hram() {
        assert_eq!(bus().read_dma(0xC000), None);
    }

    #[test]
    fn write_dma_returns_true_for_hram() {
        let mut bus = bus();
        assert!(bus.write_dma(0xFF80, 0x42));
        assert_eq!(bus.read_dma(0xFF80), Some(0x42));
    }

    #[test]
    fn write_dma_returns_false_below_hram() {
        assert!(!bus().write_dma(0xC000, 0x42));
    }

    // ── CGB double-speed (KEY1) ───────────────────────────

    #[test]
    fn read_key1_default_is_7e() {
        assert_eq!(bus().read(0xFF4D), 0x7E);
    }

    #[test]
    fn write_key1_sets_pending_flag() {
        let mut bus = bus();
        bus.write(0xFF4D, 0x01);
        assert_eq!(bus.read(0xFF4D), 0x7F);
    }

    // ── WRAM bank select ──────────────────────────────────

    #[test]
    fn wram_bank_default_is_1() {
        assert_eq!(bus().read(0xFF70), 1);
    }

    #[test]
    fn write_wram_bank_select_updates_value() {
        let mut bus = bus();
        bus.write(0xFF70, 0x03);
        assert_eq!(bus.read(0xFF70), 0x03);
    }

    #[test]
    fn write_wram_bank_0_clamps_to_1() {
        let mut bus = bus();
        bus.write(0xFF70, 0x00);
        assert_eq!(bus.read(0xFF70), 1);
    }

    // ── FF50 boot ROM disable ─────────────────────────────

    #[test]
    fn read_ff50_boot_rom_mapped_returns_fe() {
        let bus = GbcMemoryBus::new([0; 0x100], true);
        assert_eq!(bus.read(0xFF50), 0xFE);
    }

    #[test]
    fn read_ff50_boot_rom_unmapped_returns_ff() {
        assert_eq!(bus().read(0xFF50), 0xFF);
    }

    // ── step_devices DMA path ─────────────────────────────

    #[test]
    fn step_devices_runs_dma_transfer() {
        let mut bus = bus();
        // Start DMA from 0xC000 (WRAM area) to OAM
        bus.write(0xFF46, 0xC0);
        assert!(bus.is_dma_active());
        // Step enough cycles for a few DMA transfers
        bus.step_devices(4 * 4); // 4 M-cycles → 1 byte transfer
        assert!(bus.is_dma_active()); // still active after 1 transfer

        // Complete the full 160 transfers
        bus.step_devices(4 * 159);
        assert!(!bus.is_dma_active());
    }

    // ── Stop ───────────────────────────────────────────────

    #[test]
    fn stop_resets_div_and_sets_halted() {
        let mut bus = bus();
        bus.write(0xFF04, 0x00); // reset div first
        bus.step_devices(1000); // advance div
        assert!(bus.read(0xFF04) > 0);
        bus.stop();
        assert_eq!(bus.read(0xFF04), 0);
        assert!(bus.is_halted_or_stopped());
    }

    // ── Default impl ──────────────────────────────────────

    #[test]
    fn default_bus_creates_with_zero_bootrom() {
        let bus = GbcMemoryBus::default();
        assert!(!bus.is_halted_or_stopped());
    }
}
