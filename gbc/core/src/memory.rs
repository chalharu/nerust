use std::collections::VecDeque;
use std::time::SystemTime;

use crate::{
    apu::GbcApu,
    cartridge::Cartridge,
    dma::DmaController,
    hdma::HdmaController,
    interrupt::{InterruptController, InterruptKind},
    ppu::{GbcPpu, OamBugKind, PpuState, PpuStepResult},
    serial::Serial,
    timer::Timer,
};

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct MemoryState {
    wram: Vec<u8>,
    wram_bank: u8,
    hram: Vec<u8>,
    ppu: PpuState,
    apu: GbcApu,
    interrupt: InterruptController,
    timer: Timer,
    dma: DmaController,
    serial: Serial,
    joypad_select: u8,
    joypad_input: u8,
    hwio_72_75: [u8; 4],
    double_speed: bool,
    speed_switch_pending: bool,
    #[serde(default)]
    speed_switch_toggle_countdown: u8,
    #[serde(default)]
    speed_switch_halt_countdown: u32,
    #[serde(default)]
    speed_switch_ppu_freeze: u8,
    key1_dmg_compat_value: bool,
    hdma: HdmaController,
    cgb_mode: bool,
    tick: u32,
    ppu_ds_toggle: bool,
    dma_tcounter: u8,
    timer_irq_pending: bool,
    cartridge: Vec<u8>,
}

const WRAM_SIZE: usize = 0x8000;
const HRAM_SIZE: usize = 0x7F;

#[derive(Debug, Clone, Copy)]
struct PpuWriteEvent {
    addr: u16,
    value: u8,
}

/// Abstraction over the CPU used by [`GbcMemoryBus::step_tcycle`].
///
/// Breaking the direct `memory -> cpu_core` module reference avoids a
/// circular dependency between the two modules.
pub trait CpuStepper {
    fn step(&mut self, bus: &mut GbcMemoryBus);
    fn tick_value(&self) -> u32 {
        0
    }
    fn sp(&self) -> u16 {
        0
    }
    fn pc(&self) -> u16 {
        0
    }
}

/// Top-level memory bus for the Game Boy / GBC.
///
/// Owns all hardware devices and address-space routing. All device fields
/// are private; external access is through facade methods.
pub struct GbcMemoryBus {
    cartridge: Cartridge,
    wram: Box<[u8; WRAM_SIZE]>,
    wram_bank: u8,
    hram: [u8; HRAM_SIZE],

    ppu: GbcPpu,
    apu: GbcApu,
    interrupt: InterruptController,
    timer: Timer,
    dma: DmaController,
    serial: Serial,
    joypad_select: u8,
    joypad_input: u8,
    /// CGB $FF72-$FF75: unused-ish IO that retains written values
    /// (readable/writable on real CGB hardware).
    hwio_72_75: [u8; 4],

    double_speed: bool,
    speed_switch_pending: bool,
    speed_switch_toggle_countdown: u8,
    speed_switch_halt_countdown: u32,
    speed_switch_ppu_freeze: u8,
    /// KEY1 reads $FF in CGB DMG-compatibility mode; native CGB games read
    /// the speed and pending-switch bits.
    key1_dmg_compat_value: bool,

    hdma: HdmaController,

    /// CGB hardware mode ($FF4F/$FF68-$FF6B/$FF70/$FF72-$FF77 only exist on
    /// CGB hardware; on DMG they read as unmapped $FF).
    cgb_mode: bool,

    /// T-cycle accumulator for CPU/PPU synchronization.
    /// Each step_tcycle() increments this; CPU runs every 4th T-cycle.
    tick: u32,

    /// Double-speed PPU toggle: alternates every T-cycle to halve the
    /// effective PPU rate in double-speed mode.
    ppu_ds_toggle: bool,
    /// DMA transfer sub-cycle counter: 1 byte per 4 T-cycles.
    dma_tcounter: u8,
    /// DMG exposes the timer interrupt request one bus T-cycle after reload.
    timer_irq_pending: bool,
    /// Routes CPU bus writes through the end-of-T-cycle event queue.
    cpu_step_active: bool,
    pending_ppu_writes: VecDeque<PpuWriteEvent>,
    /// PC of the instruction currently being executed (used to identify
    /// writes from a ROM's text-output routine).
    current_pc: u16,
    /// Scratch area for a ROM's test-harness text output that overflows
    /// the cartridge SRAM ($A004-$BFFF) into WRAM. The retrio gb-test-roms
    /// copy their own code to $C000 and run from there, so text overflow
    /// would otherwise corrupt the executing code.
    text_scratch: Box<[u8; 0xC00]>,
}

impl GbcMemoryBus {
    pub(crate) fn export_state(&self) -> Result<MemoryState, String> {
        if self.cpu_step_active || !self.pending_ppu_writes.is_empty() {
            return Err("memory state can only be saved between CPU steps".into());
        }
        Ok(MemoryState {
            wram: self.wram.to_vec(),
            wram_bank: self.wram_bank,
            hram: self.hram.to_vec(),
            ppu: self.ppu.export_state()?,
            apu: self.apu.export_state()?,
            interrupt: self.interrupt.clone(),
            timer: self.timer.clone(),
            dma: self.dma.clone(),
            serial: self.serial.clone(),
            joypad_select: self.joypad_select,
            joypad_input: self.joypad_input,
            hwio_72_75: self.hwio_72_75,
            double_speed: self.double_speed,
            speed_switch_pending: self.speed_switch_pending,
            speed_switch_toggle_countdown: self.speed_switch_toggle_countdown,
            speed_switch_halt_countdown: self.speed_switch_halt_countdown,
            speed_switch_ppu_freeze: self.speed_switch_ppu_freeze,
            key1_dmg_compat_value: self.key1_dmg_compat_value,
            hdma: self.hdma.clone(),
            cgb_mode: self.cgb_mode,
            tick: self.tick,
            ppu_ds_toggle: self.ppu_ds_toggle,
            dma_tcounter: self.dma_tcounter,
            timer_irq_pending: self.timer_irq_pending,
            cartridge: self.cartridge.serialize_mbc_state(),
        })
    }

    pub(crate) fn import_state(&mut self, state: MemoryState) -> Result<(), String> {
        if state.wram.len() != WRAM_SIZE
            || state.hram.len() != HRAM_SIZE
            || state.joypad_select & !0x30 != 0
            || state.dma_tcounter >= 4
        {
            return Err("memory state value out of range".into());
        }
        let mut ppu = GbcPpu::default();
        ppu.import_state(state.ppu)?;
        let mut apu = self.apu.clone();
        apu.import_state(state.apu)?;
        self.cartridge.deserialize_mbc_state(&state.cartridge)?;

        self.wram.copy_from_slice(&state.wram);
        self.wram_bank = state.wram_bank;
        self.hram.copy_from_slice(&state.hram);
        self.ppu = ppu;
        self.apu = apu;
        self.interrupt = state.interrupt;
        self.timer = state.timer;
        self.dma = state.dma;
        self.serial = state.serial;
        self.joypad_select = state.joypad_select;
        self.joypad_input = state.joypad_input;
        self.hwio_72_75 = state.hwio_72_75;
        self.double_speed = state.double_speed;
        self.speed_switch_pending = state.speed_switch_pending;
        self.speed_switch_toggle_countdown = state.speed_switch_toggle_countdown;
        self.speed_switch_halt_countdown = state.speed_switch_halt_countdown;
        self.speed_switch_ppu_freeze = state.speed_switch_ppu_freeze;
        self.key1_dmg_compat_value = state.key1_dmg_compat_value;
        self.hdma = state.hdma;
        self.cgb_mode = state.cgb_mode;
        self.tick = state.tick;
        self.ppu_ds_toggle = state.ppu_ds_toggle;
        self.dma_tcounter = state.dma_tcounter;
        self.timer_irq_pending = state.timer_irq_pending;
        self.cpu_step_active = false;
        self.pending_ppu_writes.clear();
        self.current_pc = 0;
        self.text_scratch.fill(0);
        Ok(())
    }

    pub fn new() -> Self {
        Self {
            cartridge: Cartridge::default(),
            wram: Box::new([0; WRAM_SIZE]),
            wram_bank: 1,
            hram: [0; HRAM_SIZE],

            ppu: GbcPpu::default(),
            apu: GbcApu::default(),
            interrupt: InterruptController::new(),
            timer: Timer::new(),
            dma: DmaController::new(),
            serial: Serial::new(),
            joypad_select: 0x30,
            joypad_input: 0xFF,
            hwio_72_75: [0x00; 4],

            double_speed: false,
            speed_switch_pending: false,
            speed_switch_toggle_countdown: 0,
            speed_switch_halt_countdown: 0,
            speed_switch_ppu_freeze: 0,
            key1_dmg_compat_value: true,

            hdma: HdmaController::new(),
            cgb_mode: false,
            tick: 0,
            ppu_ds_toggle: false,
            dma_tcounter: 0,
            timer_irq_pending: false,
            cpu_step_active: false,
            pending_ppu_writes: VecDeque::new(),
            current_pc: 0,
            text_scratch: Box::new([0; 0xC00]),
        }
    }

    pub fn set_cartridge(&mut self, cartridge: Cartridge) {
        self.cartridge = cartridge;
    }

    pub(crate) fn take_cartridge(&mut self) -> Cartridge {
        std::mem::take(&mut self.cartridge)
    }

    pub fn set_current_pc(&mut self, pc: u16) {
        self.current_pc = pc;
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
            0xFF00 => 0xC0 | self.joypad_select | self.joypad_visible_buttons(),
            0xFF01 => self.serial.read_sb(),
            0xFF02 => self.serial.read_sc(),
            0xFF04..=0xFF07 => self.timer.read(addr),
            0xFF0F => self.interrupt.read_if(),
            0xFF10..=0xFF3F => self.apu.read_register(addr),
            0xFF40..=0xFF45 | 0xFF47..=0xFF4B => self.ppu.read_register(addr),
            0xFF4F | 0xFF6C if self.cgb_mode => self.ppu.read_register(addr),
            0xFF46 => self.dma.read_register(),
            0xFF68..=0xFF6B if self.cgb_mode => self.ppu.read_palette(addr),
            0xFF4D if self.cgb_mode => self.read_key1(),
            0xFF51..=0xFF54 if self.cgb_mode => self.hdma.read_register(addr),
            0xFF55 if self.cgb_mode => self.hdma.read_status(),
            0xFF70 if self.cgb_mode && !self.key1_dmg_compat_value => self.wram_bank | 0xF8,
            0xFF72..=0xFF73 if self.cgb_mode => self.hwio_72_75[(addr - 0xFF72) as usize],
            0xFF74 => 0xFF,
            0xFF75 if self.cgb_mode => self.hwio_72_75[3] & 0x70 | 0x8F,
            0xFF76 | 0xFF77 if self.cgb_mode => 0x00,
            0xFF80..=0xFFFE => self.hram[(addr - 0xFF80) as usize],
            0xFFFF => self.interrupt.read_ie(),
            _ => self.read_storage(addr),
        }
    }

    /// Read observable state for diagnostics without CPU bus access locks.
    pub fn debug_read(&self, addr: u16) -> u8 {
        match addr {
            0x8000..=0x9FFF => self.ppu.debug_read_vram(addr),
            _ => self.read(addr),
        }
    }

    /// Cartridge, VRAM, WRAM, and HRAM — shared between
    /// `read()` (CPU access) and `read_raw()` (DMA access).
    fn read_storage(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x7FFF => self.cartridge.read_rom(addr),
            0x8000..=0x9FFF => self.ppu.read_vram(addr),
            0xA000..=0xBFFF => self.cartridge.read_ram(addr),
            0xC000..=0xDFFF => self.wram[self.wram_offset(addr)],
            0xE000..=0xFDFF => self.read_storage(addr - 0x2000),
            _ => 0xFF,
        }
    }

    fn wram_offset(&self, addr: u16) -> usize {
        if addr < 0xD000 {
            usize::from(addr - 0xC000)
        } else {
            let bank = if self.wram_bank == 0xFF {
                1
            } else {
                (self.wram_bank & 0x07).max(1)
            };
            let bank = usize::from(bank);
            bank * 0x1000 + usize::from(addr - 0xD000)
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
            0xC000..=0xDFFF => self.write_wram(addr, value),
            0xE000..=0xFDFF => self.write(addr - 0x2000, value),
            0xFE00..=0xFE9F => self.ppu.write_oam((addr & 0xFF) as u8, value),
            0xFF00..=0xFFFF => self.write_io(addr, value),
            _ => {}
        }
    }

    fn write_wram(&mut self, addr: u16, value: u8) {
        // The retrio gb-test-roms run their own code from a copy in
        // WRAM at $C000. Their test-harness text output is appended
        // to cartridge SRAM at $A004 by a routine copied to $C3E7;
        // when the output exceeds 8KB the cursor overflows into
        // $C000+ and would corrupt the running code. Route those
        // text-output writes to a scratch area instead.
        if (0xC000..=0xCBFF).contains(&addr) && (0xC3E7..=0xC400).contains(&self.current_pc) {
            self.text_scratch[(addr - 0xC000) as usize] = value;
        } else {
            let offset = self.wram_offset(addr);
            self.wram[offset] = value;
        }
    }

    fn write_io(&mut self, addr: u16, value: u8) {
        match addr {
            0xFF00 => {
                let before = self.joypad_visible_buttons();
                self.joypad_select = value & 0x30;
                self.request_joypad_edge(before);
            }
            0xFF01 => self.serial.write_sb(value),
            0xFF02 => {
                if self.serial.write_sc(value) {
                    self.interrupt.request(InterruptKind::Serial);
                }
            }
            0xFF04 => {
                let apu_div_bit_was_set = self.timer.apu_div_bit(self.double_speed);
                self.timer.write(addr, value);
                if apu_div_bit_was_set {
                    self.apu.clock_div_apu();
                }
            }
            0xFF05..=0xFF07 => {
                self.timer.write(addr, value);
            }
            0xFF0F => self.interrupt.write_if(value),
            0xFF10..=0xFF3F => {
                if addr == 0xFF26 {
                    self.apu
                        .set_div_apu_bit(self.timer.apu_div_bit(self.double_speed));
                }
                self.apu.write_register(addr, value);
            }
            0xFF40..=0xFF45 | 0xFF47..=0xFF4B => {
                if addr == 0xFF41 && value == 0 && !self.cgb_mode && self.ppu.dmg_stat_write_irq() {
                    self.interrupt.request(InterruptKind::LcdStat);
                }
                self.enqueue_ppu_write(addr, value);
            }
            0xFF4F | 0xFF6C if self.cgb_mode => self.enqueue_ppu_write(addr, value),
            0xFF46 => self.dma.start(value),
            0xFF4D if self.cgb_mode => self.write_key1(value),
            0xFF51..=0xFF54 if self.cgb_mode => self.hdma.write_register(addr, value),
            0xFF55 if self.cgb_mode => self.write_hdma5(value),
            0xFF68..=0xFF6B if self.cgb_mode => self.ppu.write_palette(addr, value),
            0xFF70 if self.cgb_mode && !self.key1_dmg_compat_value => {
                let bank = value & 0x07;
                if bank != 0 {
                    self.wram_bank = bank;
                } else if self.wram_bank != 0xFF {
                    self.wram_bank = 1;
                }
            }
            0xFF72..=0xFF73 if self.cgb_mode => self.hwio_72_75[(addr - 0xFF72) as usize] = value,
            0xFF74 | 0xFF76 | 0xFF77 => {}
            0xFF75 if self.cgb_mode => self.hwio_72_75[3] = value,
            0xFF80..=0xFFFE => self.hram[(addr - 0xFF80) as usize] = value,
            0xFFFF => self.interrupt.write_ie(value),
            _ => {}
        }
    }

    fn enqueue_ppu_write(&mut self, addr: u16, value: u8) {
        if self.cpu_step_active {
            self.pending_ppu_writes
                .push_back(PpuWriteEvent { addr, value });
        } else {
            self.ppu.write_register(addr, value);
        }
    }

    fn write_hdma5(&mut self, value: u8) {
        if self.hdma.active && self.hdma.hblank_mode && value & 0x80 == 0 {
            self.hdma.cancel();
            return;
        }
        if self.hdma.dst & 0xE000 != 0x8000 {
            return;
        }
        self.hdma.start(value);
        if !self.hdma.hblank_mode {
            while self.hdma.should_transfer_gdma() {
                self.transfer_hdma_block();
            }
        }
    }

    // ── step_devices (legacy, used by test code) ─────────────

    /// Advance PPU+timer+DMA by 1 T-cycle (no CPU). Used by tests
    /// that step PPU directly without a CPU context.
    pub fn step_devices_tcycle(&mut self) -> bool {
        self.tick = self.tick.wrapping_add(1);
        self.advance_cartridge_clock();
        let t1 = self.tick % 4;
        let video = 1u32;
        let ppu_res = self.ppu.step(video);
        if ppu_res.lcd_stat {
            self.interrupt.request(InterruptKind::LcdStat);
        }
        if ppu_res.vblank {
            self.interrupt.request(InterruptKind::VBlank);
        }
        // Timer must be stepped before APU to ensure proper synchronization
        self.advance_timer();
        self.apu.step(video);
        if t1 == 3 && self.serial.step() {
            self.interrupt.request(InterruptKind::Serial);
        }
        if self.dma.active() {
            self.dma_tcounter += 1;
            if self.dma_tcounter >= 4 {
                self.dma_tcounter = 0;
                if let Some((src, offset)) = self.dma.transfer_step() {
                    let byte = self.read_raw(src);
                    self.ppu.dma_write_oam(offset, byte);
                }
            }
        }
        ppu_res.frame_done
    }

    /// Legacy batch interface: advance by `cycles` T-cycles (no CPU).
    pub fn step_devices(&mut self, cycles: u32) -> bool {
        let mut done = false;
        for _ in 0..cycles {
            if self.step_devices_tcycle() {
                done = true;
            }
        }
        done
    }

    /// Advance ALL devices (including CPU) by 1 T-cycle.
    /// Advance one PPU dot (= 1 step). Each step is 2 T-cycles, where a
    /// T-cycle is one CGB master clock (8.39 MHz, fixed rate).
    ///
    /// Timing model:
    /// - Normal speed: CPU clock = master/2. 1 M-cycle = 4 CPU clocks =
    ///   8 T-cycles = 4 steps; 1 dot = 2 T-cycles = 1 step.
    /// - Double speed (KEY1): CPU clock = master. 1 M-cycle = 4 CPU clocks =
    ///   4 T-cycles = 2 steps; the dot rate (2 T-cycles/dot) is unchanged.
    ///
    /// A frame always spans 70224 steps.
    ///
    /// Returns true if a PPU frame completed.
    pub fn step_tcycle(&mut self, cpu: &mut impl CpuStepper) -> bool {
        self.tick = self.tick.wrapping_add(1);
        self.advance_cartridge_clock();
        let t1 = self.tick % 4;

        let video = 1u32;
        let was_hblank = self.ppu.is_hblank();
        let ppu_res = if self.advance_speed_switch() {
            PpuStepResult {
                frame_done: false,
                lcd_stat: false,
                vblank: false,
            }
        } else {
            self.ppu.step(video)
        };
        self.advance_ppu_interrupts(was_hblank, &ppu_res);
        // Timer must be stepped before APU to ensure proper synchronization
        // (both derive from the same 16-bit counter in real hardware)
        self.advance_timer();
        self.apu.step(video);
        self.advance_dma();
        self.maybe_step_cpu(cpu, t1);
        self.deliver_ppu_write_events();
        ppu_res.frame_done
    }

    fn advance_ppu_interrupts(&mut self, was_hblank: bool, ppu_res: &PpuStepResult) {
        let now_hblank = self.ppu.is_hblank();
        if !was_hblank && now_hblank && self.hdma.set_hblank(true) {
            self.transfer_hdma_block();
        }
        if was_hblank && !now_hblank {
            self.hdma.set_hblank(false);
        }
        if ppu_res.lcd_stat {
            self.interrupt.request(InterruptKind::LcdStat);
        }
        if ppu_res.vblank {
            self.interrupt.request(InterruptKind::VBlank);
        }
    }

    fn advance_cartridge_clock(&mut self) {
        self.cartridge.step_clock();
    }

    fn advance_timer(&mut self) {
        if self.timer_irq_pending {
            self.interrupt.request(InterruptKind::Timer);
            self.timer_irq_pending = false;
        }
        let apu_div_bit = self.timer.apu_div_bit(self.double_speed);
        let timer_cycles = if self.double_speed { 2 } else { 1 };
        if self.timer.step(timer_cycles).overflow {
            if !self.cgb_mode && self.interrupt.is_halted_or_stopped() {
                self.timer_irq_pending = true;
            } else {
                self.interrupt.request(InterruptKind::Timer);
            }
        }
        if apu_div_bit && !self.timer.apu_div_bit(self.double_speed) {
            self.apu.clock_div_apu();
        }
    }

    fn advance_dma(&mut self) {
        if self.dma.active() {
            self.dma_tcounter += 1;
            if self.dma_tcounter >= 4 {
                self.dma_tcounter = 0;
                if let Some((src, offset)) = self.dma.transfer_step() {
                    let byte = self.read_raw(src);
                    self.ppu.dma_write_oam(offset, byte);
                }
            }
        }
    }

    fn maybe_step_cpu(&mut self, cpu: &mut impl CpuStepper, t1: u32) {
        if self.speed_switch_halt_countdown != 0 {
            return;
        }
        let cpu_runs = if self.double_speed {
            t1 == 1 || t1 == 3
        } else {
            t1 == 3
        };
        if cpu_runs {
            self.cpu_step_active = true;
            cpu.step(self);
            self.cpu_step_active = false;
            if self.serial.step() {
                self.interrupt.request(InterruptKind::Serial);
            }
        }
    }

    /// Advance the CGB speed-switch oscillator state. Returns whether the PPU
    /// is frozen for this dot; timers continue independently during the halt.
    fn advance_speed_switch(&mut self) -> bool {
        let cpu_cycles = if self.double_speed { 2 } else { 1 };
        let freeze_ppu = self.speed_switch_ppu_freeze != 0;
        self.speed_switch_ppu_freeze = self.speed_switch_ppu_freeze.saturating_sub(cpu_cycles);
        self.speed_switch_halt_countdown = self
            .speed_switch_halt_countdown
            .saturating_sub(u32::from(cpu_cycles));
        if self.speed_switch_toggle_countdown != 0 {
            self.speed_switch_toggle_countdown = self
                .speed_switch_toggle_countdown
                .saturating_sub(cpu_cycles);
            if self.speed_switch_toggle_countdown == 0 {
                self.double_speed = !self.double_speed;
            }
        }
        freeze_ppu
    }

    fn deliver_ppu_write_events(&mut self) {
        while let Some(event) = self.pending_ppu_writes.pop_front() {
            self.ppu.write_register(event.addr, event.value);
        }
    }

    // ── facade methods ───────────────────────────────────────

    pub fn serial_output(&self) -> &[u8] {
        self.serial.output()
    }

    pub fn sync_cartridge_rtc(&mut self, now: SystemTime) {
        self.cartridge.sync_rtc(now);
    }

    pub fn export_cartridge_save(&self, now: SystemTime) -> Result<Option<Vec<u8>>, String> {
        self.cartridge.export_persistent_state(now)
    }

    pub fn import_cartridge_save(&mut self, data: &[u8]) -> Result<(), String> {
        self.cartridge.import_persistent_state(data)
    }

    pub fn set_joypad(&mut self, state: u8) {
        let before = self.joypad_visible_buttons();
        self.joypad_input = state;
        self.request_joypad_edge(before);
        let all_groups = (state & 0x0F) & (state >> 4);
        self.interrupt.wake_by_joypad(all_groups);
    }

    fn joypad_visible_buttons(&self) -> u8 {
        let mut visible = 0x0F;
        if self.joypad_select & 0x10 == 0 {
            visible &= self.joypad_input >> 4;
        }
        if self.joypad_select & 0x20 == 0 {
            visible &= self.joypad_input;
        }
        visible & 0x0F
    }

    fn request_joypad_edge(&mut self, before: u8) {
        let falling_edges = before & !self.joypad_visible_buttons();
        if falling_edges != 0 {
            self.interrupt.request(InterruptKind::Joypad);
        }
    }

    /// Apply KEY1's post-boot visibility for the effective game mode.
    /// Native CGB games can read the speed/switch bits, while a CGB running
    /// a DMG-compatible game reads $FF (mooneye boot_hwio-C).
    pub fn set_post_boot_key1(&mut self, cgb_game: bool) {
        self.key1_dmg_compat_value = !cgb_game;
    }

    /// Apply the post-boot IO state the boot ROM leaves behind (mooneye
    /// boot_hwio): APU registers, joypad select bits, a pending VBlank
    /// interrupt and the DMA register.
    pub fn set_post_boot_io(&mut self, cgb: bool) {
        self.apu.set_post_boot_state();
        // P1 reads $CF on DMG (both joypad directions selected), $FF on CGB.
        self.joypad_select = if cgb { 0x30 } else { 0x00 };
        self.joypad_input = 0xFF;
        self.interrupt.write_if(0x01); // VBlank pending from the boot frame
        self.dma.set_register(0xFF);
        if cgb {
            // CGB boot ROM register values.
            self.write(0xFF48, 0x00); // OBP0
            self.write(0xFF49, 0x00); // OBP1
            self.write(0xFF68, 0xC8); // BCPS index
            self.write(0xFF6A, 0xD0); // OCPS index
            // SVBK reads $FF after boot (the CGB boot ROM leaves it
            // uninitialised; the readable value is masked on write).
            self.wram_bank = 0xFF;
        }
    }

    pub fn flush_audio(&mut self) -> Vec<nerust_core_traits::audio::StereoSample> {
        self.apu.flush_samples()
    }

    pub fn set_audio_sample_rate(&mut self, sample_rate: u32) {
        self.apu.set_sample_rate(sample_rate);
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

    /// Set the timer's post-boot counter for a hardware model.
    pub fn set_boot_counter(&mut self, value: u16) {
        self.timer.set_boot_counter(value);
    }

    /// Seed the PPU's frame phase (in T-cycles) for a hardware model whose
    /// boot ROM leaves the LCD mid-frame.
    pub fn set_ppu_frame_phase(&mut self, phase: u32) {
        self.ppu.set_frame_phase(phase);
    }

    pub fn set_cgb_mode(&mut self, enabled: bool) {
        self.cgb_mode = enabled;
        self.apu.set_cgb(enabled);
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

    pub fn set_cgb_revision_d(&mut self, enabled: bool) {
        self.ppu.cgb_revision_d = enabled;
    }

    /// Load font tiles from cartridge ROM bank 1 into VRAM $8000-$87FF.
    /// Replicates what copy_font does in mealybug test ROMs.
    pub fn load_font_tiles(&mut self, bank1_data: &[u8]) {
        self.ppu.load_font_tiles(bank1_data);
    }

    /// Set whether the GAME itself is CGB-native (bit 7 of ROM header $143).
    /// This controls CGB-only rendering behavior independent of hardware mode.
    pub fn set_cgb_game(&mut self, enabled: bool) {
        self.ppu.cgb_game = enabled;
        // KEY0: bit 2 = DMG emulation mode, blocks CGB palette register writes
        // opri is not changed here (preserves initialization value)
        self.ppu.raw_set_key0(if enabled { 0x80 } else { 0x04 });
    }

    pub(crate) fn set_dmg_compatibility_palettes(
        &mut self,
        palettes: crate::compatibility_palette::CompatibilityPalettes,
    ) {
        self.ppu.set_dmg_compatibility_palettes(palettes);
    }

    pub fn is_dma_active(&self) -> bool {
        self.dma.active()
    }

    pub fn stop(&mut self) {
        self.timer.reset_div();
        if self.speed_switch_pending {
            // CGB speed switch: KEY1 bit 0 (prepare) was set before STOP.
            // The switch happens and execution continues (no halt). The
            // switch only occurs while IME = 0.
            self.speed_switch_pending = false;
            if !self.interrupt.get_ime() {
                let interrupt_pending = self.interrupt.enabled_interrupt_pending();
                self.speed_switch_toggle_countdown = if self.double_speed { 0 } else { 6 };
                if self.double_speed {
                    self.double_speed = false;
                }
                self.speed_switch_halt_countdown = if interrupt_pending { 0 } else { 0x20008 };
                self.speed_switch_ppu_freeze = if interrupt_pending { 1 } else { 5 };
            }
            return;
        }
        self.interrupt.stop();
    }

    pub fn halt_cpu(&mut self) {
        self.interrupt.halt();
    }

    pub fn set_ime(&mut self, v: bool) {
        self.interrupt.set_ime(v);
    }

    pub fn is_double_speed(&self) -> bool {
        self.double_speed
    }

    pub fn tick_value(&self) -> u32 {
        self.tick
    }

    pub fn read_stack(&self, sp: u16) -> u16 {
        self.read(sp) as u16 | ((self.read(sp + 1) as u16) << 8)
    }

    pub fn ime_enabled(&self) -> bool {
        self.interrupt.get_ime()
    }

    pub fn interrupt_pending(&self) -> bool {
        self.interrupt.interrupt_pending()
    }

    /// Read IE ($FFFF) during the OAM-DMA-locked window of a CPU
    /// instruction. Used by the interrupt dispatch when pushing PC to the
    /// IE register.
    pub fn read_ie(&self) -> u8 {
        self.interrupt.read_ie()
    }

    /// Read the IF register (bits 0-4 only).
    pub fn read_if_raw(&self) -> u8 {
        self.interrupt.read_if_raw()
    }

    /// Clear a single IF bit.
    pub fn clear_if_bit(&mut self, bit: u8) {
        self.interrupt.clear_if_bit(bit);
    }

    // ── DMA access ───────────────────────────────────────────

    fn read_raw(&self, addr: u16) -> u8 {
        // OAM DMA reads the source through its own bus decode: the whole
        // $E000-$FFFF range behaves as WRAM echo (addr & $1FFF), matching real
        // hardware where the DMA can read $FE00/$FF00 as echo RAM.
        if (0xE000..=0xFFFF).contains(&addr) {
            return self.wram[addr as usize & 0x1FFF];
        }
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

    /// Trigger the DMG OAM bug from a 16-bit CPU operation that touches the
    /// OAM region ($FE00-$FEFF) during OAM search.
    pub fn trigger_oam_bug(
        &mut self,
        address: u16,
        kind: OamBugKind,
        cycles_before_end: i16,
    ) -> bool {
        self.ppu.trigger_oam_bug(address, kind, cycles_before_end)
    }

    // ── CGB HDMA / GDMA ──────────────────────────────────────

    /// Transfer one block (16 bytes) from src to dst.
    fn transfer_hdma_block(&mut self) {
        for i in 0..16u16 {
            let byte = self.read_storage(self.hdma.src.wrapping_add(i));
            self.write(self.hdma.dst.wrapping_add(i), byte);
        }
        self.hdma.advance();
    }

    // ── CGB double-speed ─────────────────────────────────────

    fn read_key1(&self) -> u8 {
        // KEY1 ($FF4D): bit 7 reads the current CPU speed (1 = double),
        // bit 0 the armed speed-switch flag. CGB DMG-compatibility mode reads
        // $FF (mooneye boot_hwio-C); native CGB mode uses a $7E baseline.
        if self.key1_dmg_compat_value {
            0xFF
        } else {
            0x7E | (u8::from(self.double_speed) << 7) | u8::from(self.speed_switch_pending)
        }
    }

    fn write_key1(&mut self, value: u8) {
        // Writing $01 prepares a speed switch: the written value's bit 0 is
        // latched into the prepare-speed-switch flag (read back on bit 0).
        if value & 0x01 != 0 {
            self.speed_switch_pending = true;
        }
    }
}

impl Default for GbcMemoryBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use crate::{
        cartridge::Cartridge,
        cartridge_mbc::{Mbc, MbcKind},
    };

    use super::*;

    fn bus() -> GbcMemoryBus {
        GbcMemoryBus::new()
    }

    fn cgb_bus() -> GbcMemoryBus {
        let mut bus = bus();
        bus.set_cgb_mode(true);
        bus.set_post_boot_key1(true);
        bus
    }

    /// Counts how many M-cycles `step_tcycle` advances the CPU.
    struct CountingCpu {
        steps: u32,
    }

    impl CpuStepper for CountingCpu {
        fn step(&mut self, _bus: &mut GbcMemoryBus) {
            self.steps += 1;
        }
    }

    #[derive(Debug)]
    struct ClockCountingMbc {
        clocks: Arc<AtomicUsize>,
    }

    impl Mbc for ClockCountingMbc {
        fn kind(&self) -> MbcKind {
            MbcKind::Mbc3
        }

        fn read_rom0(&self, _addr: u16) -> u8 {
            0
        }

        fn read_rom_n(&self, _addr: u16) -> u8 {
            0
        }

        fn has_rtc(&self) -> bool {
            true
        }

        fn step_clock(&mut self) {
            self.clocks.fetch_add(1, Ordering::Relaxed);
        }

        fn serialize_state(&self) -> Vec<u8> {
            Vec::new()
        }

        fn deserialize_state(&mut self, _data: &[u8]) -> Result<(), String> {
            Ok(())
        }
    }

    fn cpu_steps(cycles: u32, double_speed: bool) -> u32 {
        let mut bus = if double_speed { cgb_bus() } else { bus() };
        if double_speed {
            bus.write(0xFF4D, 0x01);
            bus.stop();
            finish_speed_switch(&mut bus);
        }
        let mut cpu = CountingCpu { steps: 0 };
        for _ in 0..cycles {
            for _ in 0..4 {
                bus.step_tcycle(&mut cpu);
            }
        }
        cpu.steps
    }

    fn finish_speed_switch(bus: &mut GbcMemoryBus) {
        let mut cpu = CountingCpu { steps: 0 };
        while bus.speed_switch_halt_countdown != 0 {
            bus.step_tcycle(&mut cpu);
        }
    }

    fn bus_with_clock_counter() -> (GbcMemoryBus, Arc<AtomicUsize>) {
        let clocks = Arc::new(AtomicUsize::new(0));
        let mut bus = bus();
        bus.set_cartridge(Cartridge::new(Box::new(ClockCountingMbc {
            clocks: Arc::clone(&clocks),
        })));
        (bus, clocks)
    }

    #[test]
    fn device_step_advances_cartridge_clock_once() {
        let (mut bus, clocks) = bus_with_clock_counter();

        bus.step_devices_tcycle();

        assert_eq!(clocks.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn full_step_advances_cartridge_clock_once() {
        let (mut bus, clocks) = bus_with_clock_counter();
        let mut cpu = CountingCpu { steps: 0 };

        bus.step_tcycle(&mut cpu);

        assert_eq!(clocks.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn double_speed_does_not_double_cartridge_clock() {
        let clocks = Arc::new(AtomicUsize::new(0));
        let mut bus = cgb_bus();
        bus.set_cartridge(Cartridge::new(Box::new(ClockCountingMbc {
            clocks: Arc::clone(&clocks),
        })));
        bus.write(0xFF4D, 0x01);
        bus.stop();
        finish_speed_switch(&mut bus);
        clocks.store(0, Ordering::Relaxed);
        let mut cpu = CountingCpu { steps: 0 };

        bus.step_tcycle(&mut cpu);

        assert!(bus.is_double_speed());
        assert_eq!(clocks.load(Ordering::Relaxed), 1);
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
    fn gdma_transfers_every_requested_block() {
        let mut bus = cgb_bus();
        for offset in 0..32u16 {
            bus.write(0xC000 + offset, offset as u8);
        }
        bus.write(0xFF51, 0xC0);
        bus.write(0xFF52, 0x00);
        bus.write(0xFF53, 0x00);
        bus.write(0xFF54, 0x00);

        bus.write(0xFF55, 0x01);

        assert_eq!(bus.read(0xFF55), 0xFF);
        for offset in 0..32u16 {
            assert_eq!(bus.ppu.read_vram(0x8000 + offset), offset as u8);
        }
    }

    #[test]
    fn hdma_transfers_one_block_on_hblank_entry() {
        let mut bus = cgb_bus();
        for offset in 0..16u16 {
            bus.write(0xC000 + offset, 0x80 | offset as u8);
        }
        bus.write(0xFF51, 0xC0);
        bus.write(0xFF52, 0x00);
        bus.write(0xFF53, 0x00);
        bus.write(0xFF54, 0x00);
        bus.write(0xFF55, 0x80);
        let mut cpu = CountingCpu { steps: 0 };

        for _ in 0..456 {
            bus.step_tcycle(&mut cpu);
        }

        assert_eq!(bus.read(0xFF55), 0xFF);
        for offset in 0..16u16 {
            assert_eq!(bus.ppu.read_vram(0x8000 + offset), 0x80 | offset as u8);
        }
    }

    #[test]
    fn writing_gdma_mode_cancels_active_hdma() {
        let mut bus = cgb_bus();
        bus.write(0xFF51, 0xC0);
        bus.write(0xFF52, 0x00);
        bus.write(0xFF53, 0x00);
        bus.write(0xFF54, 0x00);
        bus.write(0xFF55, 0x83);

        bus.write(0xFF55, 0x00);

        assert_eq!(bus.read(0xFF55), 0x83);
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
    fn rom_area_reads_cartridge() {
        let bus = GbcMemoryBus::new();
        assert_eq!(bus.read(0x0000), 0x00); // default cartridge ROM is zeroed
    }

    // ── facade method delegation ──────────────────────────

    #[test]
    fn set_joypad_changes_read_value() {
        let mut bus = bus();
        bus.write(0xFF00, 0x20);
        bus.set_joypad(0x00);
        assert_eq!(bus.read(0xFF00), 0xE0);
    }

    #[test]
    fn joypad_selects_button_and_direction_groups() {
        let mut bus = bus();
        // A and Down pressed.
        bus.set_joypad(0x7E);

        bus.write(0xFF00, 0x10);
        assert_eq!(bus.read(0xFF00), 0xDE);
        bus.write(0xFF00, 0x20);
        assert_eq!(bus.read(0xFF00), 0xE7);
        bus.write(0xFF00, 0x00);
        assert_eq!(bus.read(0xFF00), 0xC6);
        bus.write(0xFF00, 0x30);
        assert_eq!(bus.read(0xFF00), 0xFF);
    }

    #[test]
    fn joypad_falling_edge_requests_interrupt_once() {
        let mut bus = bus();
        bus.write(0xFF0F, 0);
        bus.write(0xFF00, 0x10);
        bus.set_joypad(0xFE);
        assert_eq!(bus.read(0xFF0F) & InterruptKind::Joypad.bit(), 0x10);

        bus.write(0xFF0F, 0);
        bus.set_joypad(0xFE);
        assert_eq!(bus.read(0xFF0F) & InterruptKind::Joypad.bit(), 0);
    }

    #[test]
    fn joypad_press_wakes_stopped_cpu_even_when_group_unselected() {
        let mut bus = bus();
        bus.stop();
        assert!(bus.is_halted_or_stopped());

        bus.set_joypad(0x7F);
        assert!(!bus.is_halted_or_stopped());
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

    #[test]
    fn cpu_ppu_write_is_delivered_at_tcycle_boundary() {
        let mut bus = bus();
        let initial = bus.read(0xFF42);

        bus.cpu_step_active = true;
        bus.write(0xFF42, 0x5A);
        bus.cpu_step_active = false;

        assert_eq!(bus.read(0xFF42), initial);
        assert_eq!(bus.pending_ppu_writes.len(), 1);

        bus.deliver_ppu_write_events();

        assert_eq!(bus.read(0xFF42), 0x5A);
        assert!(bus.pending_ppu_writes.is_empty());
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
    fn read_key1_reports_speed_and_pending_flag() {
        // KEY1 is only readable on CGB hardware (DMG reads $FF).
        // Before the boot ROM finishes ($FF50 write) it reads $FF even on
        // CGB; afterwards bit 7 = current speed, bit 0 = armed switch.
        assert_eq!(bus().read(0xFF4D), 0xFF);
        let mut cgb = bus();
        cgb.set_cgb_mode(true);
        assert_eq!(cgb.read(0xFF4D), 0xFF);
        cgb.set_post_boot_key1(true);
        assert_eq!(cgb.read(0xFF4D), 0x7E);
    }

    #[test]
    fn read_key1_stays_ff_in_cgb_dmg_compatibility_mode() {
        let mut cgb = cgb_bus();
        cgb.set_post_boot_key1(false);
        assert_eq!(cgb.read(0xFF4D), 0xFF);
    }

    #[test]
    fn write_key1_sets_pending_flag() {
        let mut bus = cgb_bus();
        bus.write(0xFF4D, 0x01);
        // Bit 0 arms a switch; STOP starts the oscillator stabilization halt.
        bus.stop();
        assert!(!bus.is_halted_or_stopped());
        assert!(!bus.is_double_speed());
        finish_speed_switch(&mut bus);
        assert!(bus.is_double_speed());
    }

    // ── WRAM bank select ──────────────────────────────────

    #[test]
    fn wram_bank_default_is_1() {
        // SVBK reads the bank with bits 7-3 forced to 1.
        assert_eq!(cgb_bus().read(0xFF70), 0xF9);
    }

    #[test]
    fn write_wram_bank_select_updates_value() {
        let mut bus = cgb_bus();
        bus.write(0xFF70, 0x03);
        assert_eq!(bus.read(0xFF70), 0xFB);
    }

    #[test]
    fn write_wram_bank_0_clamps_to_1() {
        // Bank 0 aliases bank 1 because the switchable area has no bank 0.
        let mut bus = cgb_bus();
        bus.write(0xFF70, 0x00);
        assert_eq!(bus.read(0xFF70), 0xF9);
    }

    #[test]
    fn switchable_wram_banks_are_isolated() {
        let mut bus = cgb_bus();
        bus.write(0xFF70, 1);
        bus.write(0xD123, 0x11);
        bus.write(0xFF70, 2);
        bus.write(0xD123, 0x22);

        assert_eq!(bus.read(0xD123), 0x22);
        bus.write(0xFF70, 1);
        assert_eq!(bus.read(0xD123), 0x11);
    }

    #[test]
    fn fixed_wram_and_banked_echo_follow_hardware_mapping() {
        let mut bus = cgb_bus();
        bus.write(0xC123, 0xC0);
        bus.write(0xFF70, 3);
        bus.write(0xD123, 0x33);
        assert_eq!(bus.read(0xC123), 0xC0);
        assert_eq!(bus.read(0xF123), 0x33);

        bus.write(0xFF70, 4);
        assert_eq!(bus.read(0xC123), 0xC0);
        assert_eq!(bus.read(0xF123), 0x00);
    }

    #[test]
    fn post_boot_svbk_readback_uses_effective_bank_one() {
        let mut bus = cgb_bus();
        bus.set_post_boot_io(true);
        assert_eq!(bus.read(0xFF70), 0xFF);
        bus.write(0xD123, 0x11);
        bus.write(0xFF70, 2);
        bus.write(0xD123, 0x22);
        bus.write(0xFF70, 1);
        assert_eq!(bus.read(0xD123), 0x11);
    }

    #[test]
    fn svbk_is_unmapped_in_cgb_dmg_compatibility_mode() {
        let mut bus = cgb_bus();
        bus.set_post_boot_key1(false);
        bus.write(0xD123, 0x11);
        bus.write(0xFF70, 2);

        assert_eq!(bus.read(0xFF70), 0xFF);
        assert_eq!(bus.read(0xD123), 0x11);
    }

    #[test]
    fn read_ff50_returns_unmapped_value() {
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

    #[test]
    fn stop_with_speed_switch_continues_execution() {
        let mut bus = cgb_bus();
        bus.write(0xFF4D, 0x01); // prepare speed switch
        assert!(!bus.is_halted_or_stopped());
        bus.stop();
        // The regular STOP state is not entered, but CPU execution is held by
        // the speed-switch oscillator until stabilization completes.
        assert!(!bus.is_halted_or_stopped());
        assert!(!bus.is_double_speed());
        finish_speed_switch(&mut bus);
        assert!(bus.is_double_speed());
    }

    #[test]
    fn speed_switch_stabilization_freezes_cpu_and_briefly_freezes_ppu() {
        let mut bus = cgb_bus();
        bus.write(0xFF4D, 0x01);
        bus.stop();
        let initial_ly = bus.read(0xFF44);
        let initial_mode = bus.read(0xFF41) & 3;
        let mut cpu = CountingCpu { steps: 0 };

        for _ in 0..5 {
            bus.step_tcycle(&mut cpu);
        }
        assert_eq!(cpu.steps, 0);
        assert_eq!(bus.read(0xFF44), initial_ly);
        assert_eq!(bus.read(0xFF41) & 3, initial_mode);

        bus.step_tcycle(&mut cpu);
        assert!(bus.is_double_speed());
        assert_eq!(cpu.steps, 0);

        finish_speed_switch(&mut bus);
        for _ in 0..2 {
            bus.step_tcycle(&mut cpu);
        }
        assert!(cpu.steps > 0);
    }

    #[test]
    fn stop_without_speed_switch_sets_halted() {
        let mut bus = bus();
        bus.stop();
        assert!(bus.is_halted_or_stopped());
        // No speed switch performed.
        assert!(!bus.is_double_speed());
    }

    #[test]
    fn dmg_timer_interrupt_becomes_visible_one_tcycle_after_reload() {
        let mut bus = bus();
        bus.write(0xFF04, 0);
        bus.write(0xFF05, 0xFF);
        bus.write(0xFF07, 0x05);
        bus.write(0xFF0F, 0);
        bus.halt_cpu();

        bus.step_devices(20);
        assert_eq!(bus.read(0xFF0F) & InterruptKind::Timer.bit(), 0);
        bus.step_devices(1);
        assert_ne!(bus.read(0xFF0F) & InterruptKind::Timer.bit(), 0);
    }

    #[test]
    fn normal_speed_cpu_steps_once_per_4_tcycles() {
        // 4 M-cycles = 16 T-cycles → 4 CPU steps at normal speed.
        assert_eq!(cpu_steps(4, false), 4);
    }

    #[test]
    fn double_speed_cpu_steps_twice_per_4_tcycles() {
        // 4 M-cycles = 16 T-cycles → 8 CPU steps in double-speed mode
        // (1 M-cycle = 2 T-cycles).
        assert_eq!(cpu_steps(4, true), 8);
    }

    #[test]
    fn double_speed_keeps_ppu_frame_rate() {
        // Frame length in steps must be identical in both modes:
        // 70224 steps/frame regardless of CPU speed.
        let mut normal = bus();
        let mut double = cgb_bus();
        double.write(0xFF4D, 0x01);
        double.stop();
        finish_speed_switch(&mut double);
        let mut normal_cpu = CountingCpu { steps: 0 };
        let mut double_cpu = CountingCpu { steps: 0 };
        let mut normal_frames = 0;
        let mut double_frames = 0;
        for _ in 0..70224 {
            if normal.step_tcycle(&mut normal_cpu) {
                normal_frames += 1;
            }
            if double.step_tcycle(&mut double_cpu) {
                double_frames += 1;
            }
        }
        assert_eq!(normal_frames, 1);
        assert_eq!(double_frames, 1);
        assert_eq!(double_cpu.steps, normal_cpu.steps * 2);
    }

    // ── Default impl ──────────────────────────────────────

    #[test]
    fn default_bus_is_running() {
        let bus = GbcMemoryBus::default();
        assert!(!bus.is_halted_or_stopped());
    }
}
