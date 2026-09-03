use std::collections::VecDeque;

use crate::cartridge::Cartridge;
use crate::cartridge::save::helpers::{read_slice, write_slice};
use crate::dma::{DmaTrigger, GbaDma};
use crate::ppu::GbaPpu;
use crate::scheduler::{EventScheduler, EventType, ScheduledEvent};
use crate::timer::GbaTimers;

// ---------------------------------------------------------------------------
// GbaMemoryBus — GBA 32bitフラットアドレス空間のFacade
// ---------------------------------------------------------------------------

const BIOS_SIZE: usize = 0x4000;
const EWRAM_SIZE: usize = 0x40000;
const IWRAM_SIZE: usize = 0x8000;
const PALETTE_SIZE: usize = 0x400;
const VRAM_SIZE: usize = 0x18000;
const OAM_SIZE: usize = 0x400;

/// GBA メモリバス。GbcMemoryBus と同様に全RAM/レジスタの唯一所有者であり、
/// CPU は `&mut GbaMemoryBus` 経由でアクセスする。
pub struct GbaMemoryBus {
    bios: Box<[u8; BIOS_SIZE]>,
    ewram: Box<[u8; EWRAM_SIZE]>,
    iwram: Box<[u8; IWRAM_SIZE]>,
    palette_ram: Box<[u8; PALETTE_SIZE]>,
    vram: Box<[u8; VRAM_SIZE]>,
    oam: Box<[u8; OAM_SIZE]>,
    ppu: GbaPpu,
    dma: GbaDma,
    timers: GbaTimers,
    cartridge: Option<Cartridge>,
    // Fallback SRAM for Phase 3 tests when no cartridge is loaded
    fallback_sram: Box<[u8; 0x10000]>,

    // レジスタ — Phase 3 基本16件
    wait_cnt: u16,
    ie: u16,
    sif: u16,
    ime: bool,
    postflg: u8,
    haltcnt: u8,
    keyinput: u16,
    keycnt: u16,
    siocnt: u16,
    siodata8: u8,
    siodata32: u32,
    rcnt: u16,

    // Bus制御
    last_prefetch: u32,
    open_bus_value: u32,
    prefetch_queue: VecDeque<u32>,
    prefetch_enabled: bool,
    bios_protect: bool,
    current_pc: u32,
    prev_addr: Option<u32>,
    prev_width: u8,
    access_wait_cycles: u32,
    halted: bool,
    halt_irq_mask: u16,
    bios_prefetch: u32,
    bios_read_seq: usize,
    scheduler: EventScheduler,
    current_tcycle: u64,
}

impl GbaMemoryBus {
    pub fn new() -> Self {
        let mut bios = Box::new([0u8; BIOS_SIZE]);
        // 未HLE SWIがSVCベクタへ遷移した場合、安全にベクタ上で待機する。
        bios[0x08..0x0C].copy_from_slice(&0xEAFF_FFFEu32.to_le_bytes());
        // jsmolka bios.gba が期待するBIOS内容を最低限埋める
        bios[0x00..0x04].copy_from_slice(&0xE129F000u32.to_le_bytes());
        bios[0xE4..0xE8].copy_from_slice(&0xE129F000u32.to_le_bytes());
        bios[0x190..0x194].copy_from_slice(&0xE3A02004u32.to_le_bytes());
        bios[0x13C..0x140].copy_from_slice(&0xE25EF004u32.to_le_bytes());
        bios[0x144..0x148].copy_from_slice(&0xE55EC002u32.to_le_bytes());
        Self {
            bios,
            ewram: Box::new([0u8; EWRAM_SIZE]),
            iwram: Box::new([0u8; IWRAM_SIZE]),
            palette_ram: Box::new([0u8; PALETTE_SIZE]),
            vram: Box::new([0u8; VRAM_SIZE]),
            oam: Box::new([0u8; OAM_SIZE]),
            ppu: GbaPpu::new(),
            dma: GbaDma::default(),
            timers: GbaTimers::default(),
            cartridge: None,
            fallback_sram: Box::new([0u8; 0x10000]),

            wait_cnt: 0,
            ie: 0,
            sif: 0,
            ime: false,
            postflg: 1,
            haltcnt: 0,
            keyinput: 0x03FF,
            keycnt: 0,
            siocnt: 0,
            siodata8: 0,
            siodata32: 0,
            rcnt: 0,

            last_prefetch: 0xE129F000,
            open_bus_value: 0xE129F000,
            prefetch_queue: VecDeque::with_capacity(8),
            prefetch_enabled: false,
            bios_protect: true,
            current_pc: 0x08000000,
            prev_addr: None,
            prev_width: 0,
            access_wait_cycles: 0,
            halted: false,
            halt_irq_mask: 0,
            bios_prefetch: 0xE129F000,
            bios_read_seq: 0,
            scheduler: EventScheduler::new(),
            current_tcycle: 0,
        }
    }

    // -----------------------------------------------------------------------
    // Public API — 3幅 + fetch
    // -----------------------------------------------------------------------

    pub fn read8(&mut self, addr: u32) -> u8 {
        let (data, _wait) = self.read_internal(addr, 1);
        (data & 0xFF) as u8
    }

    pub fn read16(&mut self, addr: u32) -> u16 {
        let (data, _wait) = self.read_internal(addr, 2);
        self.align_read(addr, 2, data) as u16
    }

    /// ARM7TDMI LDRH result, including the 32-bit ROR 8 result for odd addresses.
    pub fn read_ldr_halfword(&mut self, addr: u32) -> u32 {
        let (data, _wait) = self.read_internal(addr, 2);
        if addr & 1 != 0 {
            (data & 0xFFFF).rotate_right(8)
        } else {
            data & 0xFFFF
        }
    }

    pub fn read32(&mut self, addr: u32) -> u32 {
        let (data, _wait) = self.read_internal(addr, 4);
        self.align_read(addr, 4, data)
    }

    /// Aligned word transfer used by LDM, which ignores address bits 0-1 without ROR.
    pub fn read_aligned32(&mut self, addr: u32) -> u32 {
        self.read_internal(addr & !3, 4).0
    }

    pub fn write8(&mut self, addr: u32, value: u8) {
        self.write_internal(addr, 1, value as u32);
    }

    pub fn write16(&mut self, addr: u32, value: u16) {
        self.write_internal(addr, 2, value as u32);
    }

    pub fn write32(&mut self, addr: u32, value: u32) {
        self.write_internal(addr, 4, value);
    }

    pub fn write_hle_bios16(&mut self, addr: u32, value: u16) {
        self.write16(addr, value);
        self.apply_haltcnt_write(addr, 2, u32::from(value));
    }

    pub fn write_hle_bios32(&mut self, addr: u32, value: u32) {
        self.write32(addr, value);
        self.apply_haltcnt_write(addr, 4, value);
    }

    pub fn fetch16(&mut self, addr: u32) -> u16 {
        self.read16(addr)
    }

    pub fn fetch32(&mut self, addr: u32) -> u32 {
        self.read32(addr)
    }

    pub fn cycles_for(&self, addr: u32, width: u8) -> u8 {
        match addr {
            0x00000000..=0x00003FFF => 1,
            0x02000000..=0x02FFFFFF => {
                if width == 4 {
                    6
                } else {
                    3
                }
            }
            0x03000000..=0x03FFFFFF => 1,
            0x04000000..=0x040003FE => 1,
            0x05000000..=0x05FFFFFF => 1,
            0x06000000..=0x06FFFFFF => 1,
            0x07000000..=0x07FFFFFF => 1,
            0x08000000..=0x0DFFFFFF => {
                if self.is_sequential(addr, width)
                    && self.prefetch_enabled
                    && !self.prefetch_queue.is_empty()
                {
                    if width == 4 { 2 } else { 1 }
                } else {
                    self.gamepak_rom_cycles(addr, width)
                }
            }
            0x0E000000..=0x0FFFFFFF => {
                const SRAM_WAIT: [u8; 4] = [4, 3, 2, 8];
                SRAM_WAIT[(self.wait_cnt & 0b11) as usize].saturating_mul(width)
            }
            _ => 1,
        }
    }

    /// Advance the LCD controller by exactly one T-cycle.
    pub fn tick(&mut self) -> bool {
        self.current_tcycle = self.current_tcycle.wrapping_add(1);
        self.dma.tick_pending();
        // Schedule next PPU events if needed (for bulk optimization, currently per-cycle)
        // The scheduler is used for Timer/DMA bulk stepping; PPU/HBlank/VBlank are still
        // handled directly via ppu.step for accuracy.
        let event = self
            .ppu
            .step(&self.vram[..], &self.palette_ram[..], &self.oam[..]);
        if event.hblank_started {
            self.dma.trigger(DmaTrigger::HBlank);
            self.scheduler.schedule(ScheduledEvent {
                target_tcycle: self.current_tcycle + 1,
                event_type: EventType::HBlank,
            });
        }
        if event.vblank_started {
            self.dma.trigger(DmaTrigger::VBlank);
            self.scheduler.schedule(ScheduledEvent {
                target_tcycle: self.current_tcycle + 1,
                event_type: EventType::VBlank,
            });
        }
        let timer_irq = self.timers.step();
        if timer_irq != 0 {
            for i in 0..4 {
                if timer_irq & (1 << (3 + i)) != 0 {
                    self.scheduler.schedule(ScheduledEvent {
                        target_tcycle: self.current_tcycle,
                        event_type: EventType::TimerOverflow(i),
                    });
                    // DirectSound: Timer0/1 overflow triggers DMA1/2 Special
                    if i <= 1 {
                        self.dma.trigger(DmaTrigger::Special);
                    }
                }
            }
        }
        let mut interrupt_mask = event.interrupt_mask | timer_irq;
        if let Some(transfer) = self.dma.step(self.wait_cnt) {
            self.scheduler.schedule(ScheduledEvent {
                target_tcycle: self.current_tcycle,
                event_type: EventType::DmaTransfer(transfer.channel),
            });
            let readable_source = transfer.source >= 0x02000000;
            let value = if readable_source {
                let value = self.read_dma_source(transfer.source, transfer.width);
                self.dma
                    .update_latch(transfer.channel, transfer.width, value);
                value
            } else if transfer.width == 2 && transfer.destination & 2 != 0 {
                transfer.latched_value >> 16
            } else {
                transfer.latched_value
            };
            self.write_dma_value(transfer.destination, transfer.width, value);
            self.invalidate_prefetch_for_dma(transfer.source);
            if transfer.interrupt {
                interrupt_mask |= 1 << (8 + transfer.channel);
            }
        }
        if interrupt_mask != 0 {
            self.request_interrupt(interrupt_mask);
        }
        // Process any due scheduler events (for bulk optimization, currently just clears)
        self.check_pending_events();
        event.frame_complete
    }

    pub fn dma_active(&self) -> bool {
        self.dma.is_active()
    }

    pub fn irq_pending(&self) -> bool {
        self.ime && self.ie & self.sif != 0
    }

    pub fn frame_buffer(&self) -> &[u32] {
        self.ppu.frame_buffer()
    }

    pub fn check_pending_events(&mut self) {
        let due = self.scheduler.pop_due(self.current_tcycle);
        for ev in due {
            match ev.event_type {
                EventType::TimerOverflow(ch) => {
                    // Timer overflow already handled in tick via timers.step
                    let _ = ch;
                }
                EventType::DmaTransfer(ch) => {
                    let _ = ch;
                }
                EventType::HBlank | EventType::VBlank => {}
            }
        }
    }

    pub fn next_event_cycle(&self) -> Option<u64> {
        self.scheduler.next_target()
    }

    pub fn set_keyinput(&mut self, value: u16) {
        self.keyinput = value | 0xFC00;
    }

    pub fn set_current_pc(&mut self, pc: u32) {
        self.current_pc = pc;
    }

    pub fn is_bios_addr(&self, addr: u32) -> bool {
        (0x00000000..=0x00003FFF).contains(&addr)
    }

    pub fn enter_halt(&mut self, irq_mask: u16) {
        self.halt_irq_mask = irq_mask;
        self.halted = self.ie & self.sif & self.halt_irq_mask == 0;
    }

    pub fn is_halted(&self) -> bool {
        self.halted
    }

    pub fn request_interrupt(&mut self, mask: u16) {
        let mask = mask & 0x3FFF;
        self.sif |= mask;
        let flags = u16::from_le_bytes([self.iwram[0x7FF8], self.iwram[0x7FF9]]) | mask;
        self.iwram[0x7FF8..0x7FFA].copy_from_slice(&flags.to_le_bytes());
        if self.ie & self.sif & self.halt_irq_mask != 0 {
            self.halted = false;
        }
    }

    pub fn reset_io_groups(&mut self, flags: u8) {
        if flags & 0x20 != 0 {
            self.siocnt = 0;
            self.siodata8 = 0;
            self.siodata32 = 0;
            self.rcnt = 0;
        }
        // Sound registers are introduced in Phase 9; bit 0x40 has no state yet.
        if flags & 0x80 != 0 {
            self.ppu.reset();
            self.dma.reset();
            self.timers.reset();
            self.ie = 0;
            self.sif = 0;
            self.ime = false;
            self.wait_cnt = 0;
            self.keycnt = 0;
            self.postflg = 0;
            self.haltcnt = 0;
            self.prefetch_enabled = false;
            self.prefetch_queue.clear();
            self.halted = false;
            self.halt_irq_mask = 0;
        }
    }

    pub fn take_access_wait_cycles(&mut self) -> u32 {
        std::mem::take(&mut self.access_wait_cycles)
    }

    pub fn set_cartridge(&mut self, cart: Cartridge) {
        self.cartridge = Some(cart);
        self.prefetch_queue.clear();
    }

    pub fn cartridge(&self) -> Option<&Cartridge> {
        self.cartridge.as_ref()
    }

    pub fn cartridge_mut(&mut self) -> Option<&mut Cartridge> {
        self.cartridge.as_mut()
    }

    pub fn take_cartridge(&mut self) -> Option<Cartridge> {
        self.cartridge.take()
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn is_sequential(&self, addr: u32, _width: u8) -> bool {
        if let Some(prev) = self.prev_addr {
            // 32bit ROM領域で連続アドレスか、かつ128KB境界を跨がない
            (0x08000000..=0x0DFFFFFF).contains(&addr)
                && (0x08000000..=0x0DFFFFFF).contains(&prev)
                && addr == prev.wrapping_add(u32::from(self.prev_width))
                && (addr & !0x1FFFF) == (prev & !0x1FFFF)
        } else {
            false
        }
    }

    fn gamepak_rom_cycles(&self, addr: u32, width: u8) -> u8 {
        const FIRST: [u8; 4] = [4, 3, 2, 8];
        let (first_shift, second_shift, second_slow) = match addr {
            0x08000000..=0x09FFFFFF => (2, 4, 2),
            0x0A000000..=0x0BFFFFFF => (5, 7, 4),
            _ => (8, 10, 8),
        };
        let first = FIRST[((self.wait_cnt >> first_shift) & 0b11) as usize];
        let second = if (self.wait_cnt >> second_shift) & 1 == 0 {
            second_slow
        } else {
            1
        };
        if self.is_sequential(addr, width) {
            second * if width == 4 { 2 } else { 1 }
        } else if width == 4 {
            first + second
        } else {
            first
        }
    }

    fn align_read(&self, addr: u32, width: u8, raw: u32) -> u32 {
        match width {
            4 => {
                let rot = (addr & 3) * 8;
                raw.rotate_right(rot)
            }
            2 => {
                if addr & 1 != 0 {
                    u32::from((raw as u16).rotate_right(8))
                } else {
                    raw & 0xFFFF
                }
            }
            _ => raw & 0xFF,
        }
    }

    fn read_mapped(&mut self, addr: u32, width: u8) -> u32 {
        if (0x0D000000..=0x0DFFFFFF).contains(&addr) && self.is_eeprom() {
            return self.read_eeprom(addr, width);
        }
        match addr {
            0x00000000..=0x00003FFF => self.read_bios_guarded(addr, width),
            0x02000000..=0x02FFFFFF => self.read_ewram(addr, width),
            0x03000000..=0x03FFFFFF => self.read_iwram(addr, width),
            0x04000000..=0x040003FE => self.read_io(addr, width),
            0x05000000..=0x05FFFFFF => self.read_palette(addr, width),
            0x06000000..=0x06FFFFFF => self.read_vram(addr, width),
            0x07000000..=0x07FFFFFF => self.read_oam(addr, width),
            0x08000000..=0x0DFFFFFF => self.read_rom(addr, width),
            0x0E000000..=0x0FFFFFFF => self.read_sram(addr, width),
            _ => self.open_bus_value,
        }
    }

    fn read_bios_guarded(&mut self, addr: u32, width: u8) -> u32 {
        if self.bios_protect && !(0x00000000..=0x00003FFF).contains(&self.current_pc) {
            const SEQ: [u32; 4] = [0xE129F000, 0xE3A02004, 0xE25EF004, 0xE55EC002];
            let raw = SEQ[self.bios_read_seq.min(SEQ.len() - 1)];
            let aligned = match width {
                4 => raw,
                2 => raw & 0xFFFF,
                _ => raw & 0xFF,
            };
            if self.bios_read_seq + 1 < SEQ.len() {
                self.bios_read_seq += 1;
                self.bios_prefetch = SEQ[self.bios_read_seq];
                self.open_bus_value = self.bios_prefetch;
                self.last_prefetch = self.bios_prefetch;
            }
            let _ = addr;
            aligned
        } else {
            self.read_bios(addr, width)
        }
    }

    pub fn update_bios_prefetch(&mut self, seq: usize) {
        const SEQ: [u32; 4] = [0xE129F000, 0xE3A02004, 0xE25EF004, 0xE55EC002];
        self.bios_read_seq = seq.min(SEQ.len() - 1);
        self.bios_prefetch = SEQ[self.bios_read_seq];
        self.open_bus_value = self.bios_prefetch;
        self.last_prefetch = self.bios_prefetch;
    }

    fn update_prefetch_queue(&mut self, addr: u32, sequential: bool) {
        let is_rom = (0x08000000..=0x0DFFFFFF).contains(&addr);
        if is_rom {
            if sequential && !self.prefetch_queue.is_empty() {
                let _ = self.prefetch_queue.pop_front();
            } else if !sequential {
                self.refill_prefetch_queue(addr);
            }
        }
    }

    pub fn invalidate_prefetch_for_dma(&mut self, dma_addr: u32) {
        if self.prefetch_enabled && (0x08000000..=0x0DFFFFFF).contains(&dma_addr) {
            self.prefetch_queue.clear();
        }
        self.prev_addr = Some(dma_addr);
        self.prev_width = 0;
    }

    fn refill_prefetch_queue(&mut self, addr: u32) {
        self.prefetch_queue.clear();
        if !self.prefetch_enabled {
            return;
        }
        // Prefetch 8 words (32 bytes) of ROM data
        let base = addr & !3;
        for i in 0..8 {
            let a = base.wrapping_add(i * 4);
            let word = if let Some(cart) = &self.cartridge {
                cart.read_rom(a, 4)
            } else {
                0
            };
            self.prefetch_queue.push_back(word);
        }
    }

    fn read_internal(&mut self, addr: u32, width: u8) -> (u32, u8) {
        let wait = self.cycles_for(addr, width);
        self.access_wait_cycles += u32::from(wait.saturating_sub(1));
        let sequential = self.is_sequential(addr, width) && self.prefetch_enabled;
        let raw = self.read_mapped(addr, width);
        self.update_prefetch_queue(addr, sequential);
        self.prev_addr = Some(addr);
        self.prev_width = width;
        self.last_prefetch = raw;
        self.open_bus_value = raw;
        (raw, wait)
    }

    fn write_internal(&mut self, addr: u32, width: u8, value: u32) {
        if (0x0D000000..=0x0DFFFFFF).contains(&addr) && self.is_eeprom() {
            self.write_eeprom(addr, width, value);
            return;
        }
        let wait = self.cycles_for(addr, width);
        self.access_wait_cycles += u32::from(wait.saturating_sub(1));
        match addr {
            0x02000000..=0x02FFFFFF => self.write_ewram(addr, width, value),
            0x03000000..=0x03FFFFFF => self.write_iwram(addr, width, value),
            0x04000000..=0x040003FE => self.write_io(addr, width, value),
            0x05000000..=0x05FFFFFF => self.write_palette(addr, width, value),
            0x06000000..=0x06FFFFFF => self.write_vram(addr, width, value),
            0x07000000..=0x07FFFFFF => self.write_oam(addr, width, value),
            0x0E000000..=0x0FFFFFFF => self.write_sram(addr, width, value),
            _ => {
                self.open_bus_value = value;
            }
        }
        // 非ROMへの書き込みでプリフェッチはフラッシュしない（ROM連続性のみで判定）
        if (0x08000000..=0x0DFFFFFF).contains(&addr) {
            self.prev_addr = Some(addr);
            self.prev_width = width;
        }
    }

    // -- Region readers --

    fn read_bios(&self, addr: u32, width: u8) -> u32 {
        let off = Self::aligned_off(addr, width, 0x3FFF);
        read_slice(&*self.bios, off, width)
    }

    fn read_ewram(&self, addr: u32, width: u8) -> u32 {
        let off = Self::aligned_off(addr, width, 0x3FFFF);
        read_slice(&*self.ewram, off, width)
    }

    fn read_iwram(&self, addr: u32, width: u8) -> u32 {
        let off = Self::aligned_off(addr, width, 0x7FFF);
        read_slice(&*self.iwram, off, width)
    }

    fn read_palette(&self, addr: u32, width: u8) -> u32 {
        let off = Self::aligned_off(addr, width, 0x3FF);
        read_slice(&*self.palette_ram, off, width)
    }

    fn read_vram(&self, addr: u32, width: u8) -> u32 {
        self.vram_offset(addr, width)
            .map_or(0, |off| read_slice(&*self.vram, off, width))
    }

    fn read_oam(&self, addr: u32, width: u8) -> u32 {
        let off = Self::aligned_off(addr, width, 0x3FF);
        read_slice(&*self.oam, off, width)
    }

    fn read_rom(&self, addr: u32, width: u8) -> u32 {
        if let Some(cart) = &self.cartridge {
            return cart.read_rom(addr, width);
        }
        self.open_bus_value
    }

    fn read_sram(&self, addr: u32, width: u8) -> u32 {
        if let Some(cart) = &self.cartridge {
            return cart.read_sram(addr, width);
        }
        let off = Self::aligned_off(addr, width, 0xFFFF);
        read_slice(&*self.fallback_sram, off, width)
    }

    fn is_eeprom(&self) -> bool {
        self.cartridge.as_ref().is_some_and(|c| {
            matches!(
                c.save_type(),
                crate::cartridge::save::SaveType::Eeprom512
                    | crate::cartridge::save::SaveType::Eeprom8k
            )
        })
    }

    fn read_eeprom(&self, addr: u32, width: u8) -> u32 {
        if let Some(cart) = &self.cartridge {
            return cart.read_sram(addr, width);
        }
        0
    }

    fn write_eeprom(&mut self, addr: u32, width: u8, value: u32) {
        if let Some(cart) = self.cartridge.as_mut() {
            cart.write_sram(addr, width, value);
        }
    }

    fn read_io(&mut self, addr: u32, width: u8) -> u32 {
        if width == 4 {
            return self.read_io(addr, 2) | (self.read_io(addr + 2, 2) << 16);
        }
        if width == 1 {
            match addr {
                0x04000300 => return self.postflg as u32,
                0x04000301 => return self.haltcnt as u32,
                _ => {}
            }
        }
        let aligned = addr & !1;
        let val: u16 = match aligned {
            0x04000000 | 0x04000002 | 0x04000004 | 0x04000006 => self
                .ppu
                .read_register(aligned)
                .expect("readable PPU register"),
            0x040000B0..=0x040000DE => self.dma.read(aligned).unwrap_or(0),
            0x04000100..=0x0400010E => self.timers.read(aligned).unwrap_or(0),
            0x04000128 => self.siocnt,
            0x0400012A => self.siodata8 as u16,
            0x04000120 => (self.siodata32 & 0xFFFF) as u16,
            0x04000122 => ((self.siodata32 >> 16) & 0xFFFF) as u16,
            0x04000130 => self.keyinput,
            0x04000132 => self.keycnt,
            0x04000134 => self.rcnt,
            0x04000200 => self.ie,
            0x04000202 => self.sif,
            0x04000204 => self.wait_cnt,
            0x04000208 => self.ime as u16,
            0x04000300 => self.postflg as u16,
            0x04000301 => self.haltcnt as u16,
            _ => {
                // 未実装/ライト専用レジスタは open_bus を返す
                return self.open_bus_value;
            }
        };
        if width == 1 && (addr & 1) == 1 {
            ((val >> 8) & 0xFF) as u32
        } else {
            val as u32
        }
    }

    // -- Region writers --

    fn write_ewram(&mut self, addr: u32, width: u8, value: u32) {
        let off = Self::aligned_off(addr, width, 0x3FFFF);
        write_slice(&mut *self.ewram, off, width, value);
        self.open_bus_value = value;
    }

    fn write_iwram(&mut self, addr: u32, width: u8, value: u32) {
        let off = Self::aligned_off(addr, width, 0x7FFF);
        write_slice(&mut *self.iwram, off, width, value);
        self.open_bus_value = value;
    }

    fn write_palette(&mut self, addr: u32, width: u8, value: u32) {
        let off = Self::aligned_off(addr, width, 0x3FF);
        if width == 1 {
            let aligned = off & !1;
            write_slice(&mut *self.palette_ram, aligned, 2, (value & 0xFF) * 0x0101);
        } else {
            write_slice(&mut *self.palette_ram, off, width, value);
        }
        self.open_bus_value = value;
    }

    fn write_vram(&mut self, addr: u32, width: u8, value: u32) {
        let Some(off) = self.vram_offset(addr, width) else {
            self.open_bus_value = value;
            return;
        };
        if width == 1 {
            let bitmap_mode = self.ppu.dispcnt() & 7 >= 3;
            let object_start = if bitmap_mode { 0x14000 } else { 0x10000 };
            if off < object_start {
                write_slice(&mut *self.vram, off & !1, 2, (value & 0xFF) * 0x0101);
            }
        } else {
            write_slice(&mut *self.vram, off, width, value);
        }
        self.open_bus_value = value;
    }

    fn write_oam(&mut self, addr: u32, width: u8, value: u32) {
        let off = Self::aligned_off(addr, width, 0x3FF);
        if width != 1 {
            write_slice(&mut *self.oam, off, width, value);
        }
        self.open_bus_value = value;
    }

    fn write_sram(&mut self, addr: u32, width: u8, value: u32) {
        if let Some(cart) = &mut self.cartridge {
            cart.write_sram(addr, width, value);
        } else {
            let off = Self::aligned_off(addr, width, 0xFFFF);
            write_slice(&mut *self.fallback_sram, off, width, value);
        }
        self.open_bus_value = value;
    }

    fn write_io(&mut self, addr: u32, width: u8, value: u32) {
        if width == 4 && self.timers.write32(addr, value) {
            self.open_bus_value = value;
            return;
        }
        if width > 1 && addr == 0x04000300 {
            if value & 1 != 0 {
                self.postflg = 1;
            }
            self.haltcnt = (value >> 8) as u8;
            self.open_bus_value = value;
            if (value >> 8) & 1 == 0 {
                self.enter_halt(self.ie);
            }
            return;
        }
        if width == 4 {
            self.write_io(addr, 2, value & 0xFFFF);
            self.write_io(addr + 2, 2, value >> 16);
            return;
        }
        if width == 1 {
            match addr {
                0x04000300 => {
                    if value & 1 != 0 {
                        self.postflg = 1;
                    }
                    self.open_bus_value = value;
                    return;
                }
                0x04000301 => {
                    self.haltcnt = value as u8;
                    self.open_bus_value = value;
                    if value & 1 == 0 {
                        self.enter_halt(self.ie);
                    }
                    return;
                }
                _ => {}
            }
        }
        let aligned = addr & !1;
        let v16 = value as u16;
        match aligned {
            0x04000000..=0x04000054 if aligned != 0x04000006 => {
                self.ppu.write_register(aligned, v16);
            }
            0x040000B0..=0x040000DE => {
                self.dma.write(aligned, v16);
                // force next ROM fetch to NSEQ (GBATEK: STR to DMA CNT forces NSEQ)
                self.prev_addr = None;
                self.prev_width = 0;
            }
            0x04000100..=0x0400010E => {
                self.timers.write(aligned, v16);
            }
            // 0x04000006 VCOUNT は RO
            0x04000128 => self.siocnt = v16,
            0x0400012A => self.siodata8 = (value & 0xFF) as u8,
            0x04000120 => {
                if width == 4 {
                    self.siodata32 = value;
                } else {
                    self.siodata32 = (self.siodata32 & 0xFFFF0000) | (v16 as u32);
                }
            }
            0x04000122 => {
                self.siodata32 = (self.siodata32 & 0x0000FFFF) | ((v16 as u32) << 16);
            }
            // 0x04000130 KEYINPUT は RO
            0x04000132 => self.keycnt = v16,
            0x04000134 => self.rcnt = v16,
            0x04000200 => self.ie = v16 & 0x3FFF,
            0x04000202 => self.sif &= !v16, // 書き込みでクリア（1のbitがクリア）
            0x04000204 => {
                self.wait_cnt = v16;
                self.prefetch_enabled = (v16 & (1 << 14)) != 0;
                if !self.prefetch_enabled {
                    self.prefetch_queue.clear();
                }
            }
            0x04000208 => self.ime = (v16 & 1) != 0,
            _ => {
                // 未実装レジスタへの書き込みは open_bus のみ更新
                self.open_bus_value = value;
                return;
            }
        }
        // 32bit書き込みで2レジスタ跨ぎの場合、上位側も反映されるが簡易実装では上記で十分
        let _ = width;
        self.open_bus_value = value;
    }

    #[inline]
    fn aligned_off(addr: u32, width: u8, mask: u32) -> usize {
        match width {
            4 => ((addr & !3) & mask) as usize,
            2 => ((addr & !1) & mask) as usize,
            _ => (addr & mask) as usize,
        }
    }

    fn vram_offset(&self, addr: u32, width: u8) -> Option<usize> {
        let offset = Self::aligned_off(addr, width, 0x1FFFF);
        if offset < VRAM_SIZE {
            return Some(offset);
        }
        let bitmap_mode = self.ppu.dispcnt() & 7 >= 3;
        if bitmap_mode && offset < 0x1C000 {
            None
        } else {
            Some(offset - 0x8000)
        }
    }

    fn write_dma_value(&mut self, address: u32, width: u8, value: u32) {
        match address {
            0x02000000..=0x02FFFFFF => self.write_ewram(address, width, value),
            0x03000000..=0x03FFFFFF => self.write_iwram(address, width, value),
            0x04000000..=0x040003FE => self.write_io(address, width, value),
            0x05000000..=0x05FFFFFF => self.write_palette(address, width, value),
            0x06000000..=0x06FFFFFF => self.write_vram(address, width, value),
            0x07000000..=0x07FFFFFF => self.write_oam(address, width, value),
            0x0E000000..=0x0FFFFFFF => self.write_sram(address, width, value),
            _ => self.open_bus_value = value,
        }
        self.apply_haltcnt_write(address, width, value);
    }

    fn apply_haltcnt_write(&mut self, addr: u32, width: u8, value: u32) {
        if width > 1 && addr == 0x04000300 && (value >> 8) & 1 == 0 {
            self.enter_halt(self.ie);
        }
    }

    fn read_dma_source(&mut self, address: u32, width: u8) -> u32 {
        if is_unreadable_io(address) {
            let halfword = self.last_prefetch & 0xFFFF;
            return if width == 4 {
                halfword | (halfword << 16)
            } else {
                halfword
            };
        }
        self.read_mapped(address, width)
    }
}

impl Default for GbaMemoryBus {
    fn default() -> Self {
        Self::new()
    }
}

fn is_unreadable_io(address: u32) -> bool {
    matches!(
        address & !1,
        0x04000008..=0x04000054
            | 0x04000060..=0x040000FE
            | 0x04000110..=0x0400011E
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_wram_bounds() {
        let mut bus = GbaMemoryBus::new();
        bus.write8(0x02000000, 0xAB);
        bus.write8(0x0203FFFF, 0xCD);
        assert_eq!(bus.read8(0x02000000), 0xAB);
        assert_eq!(bus.read8(0x0203FFFF), 0xCD);
    }

    #[test]
    fn read_iwram_bounds() {
        let mut bus = GbaMemoryBus::new();
        bus.write32(0x03000000, 0x12345678);
        bus.write32(0x03007FFC, 0x9ABCDEF0);
        assert_eq!(bus.read32(0x03000000), 0x12345678);
        // unaligned LDR rotates
        let v = bus.read32(0x03000001);
        assert_eq!(v, 0x78123456);
    }

    #[test]
    fn read_vram_mirror() {
        let mut bus = GbaMemoryBus::new();
        bus.write16(0x06018000, 0x1111);
        bus.write16(0x0601C000, 0x2222);
        assert_eq!(bus.read16(0x06010000), 0x1111);
        assert_eq!(bus.read16(0x06014000), 0x2222);

        bus.write16(0x04000000, 3);
        bus.write16(0x06018000, 0x3333);
        bus.write16(0x0601C000, 0x4444);
        assert_eq!(bus.read16(0x06018000), 0);
        assert_eq!(bus.read16(0x06014000), 0x4444);
        assert_eq!(bus.read16(0x0601C000), 0x4444);
    }

    #[test]
    fn read_oam_bounds() {
        let mut bus = GbaMemoryBus::new();
        bus.write16(0x07000000, 0xBEEF);
        assert_eq!(bus.read16(0x07000000), 0xBEEF);
        bus.write16(0x070003FE, 0xCAFE);
        assert_eq!(bus.read16(0x070003FE), 0xCAFE);
    }

    #[test]
    fn read_sram_bounds() {
        let mut bus = GbaMemoryBus::new();
        bus.write8(0x0E000000, 0x42);
        bus.write8(0x0E00FFFF, 0x99);
        assert_eq!(bus.read8(0x0E000000), 0x42);
        assert_eq!(bus.read8(0x0E00FFFF), 0x99);
    }

    #[test]
    fn bios_protected_when_pc_outside() {
        let mut bus = GbaMemoryBus::new();
        bus.bios[0] = 0xAA;
        bus.bios[1] = 0xBB;
        bus.set_current_pc(0x08000000);
        assert_eq!(bus.read8(0x00000000), 0x00);
        bus.set_current_pc(0x00000000);
        assert_eq!(bus.read8(0x00000000), 0xAA);
    }

    #[test]
    fn open_bus_returns_last_prefetch() {
        let mut bus = GbaMemoryBus::new();
        bus.write32(0x02000000, 0xDEADBEEF);
        let _ = bus.read32(0x02000000);
        // 未マッピング領域は open_bus を返す
        assert_eq!(bus.read32(0x04000400), 0xDEADBEEF);
    }

    #[test]
    fn write_only_reg_returns_open_bus() {
        let mut bus = GbaMemoryBus::new();
        bus.write32(0x02000000, 0x12345678);
        let _ = bus.read32(0x02000000);
        // VCOUNT は RO だが read は可能。未実装レジスタ 0x04000008 は open_bus
        assert_eq!(bus.read16(0x04000008), 0x5678); // open_bus lower 16
        // 正確には open_bus_value の下位16bit
        bus.write32(0x03000000, 0xAABBCCDD);
        let _ = bus.read32(0x03000000);
        assert_eq!(bus.read16(0x04000008), 0xCCDD);
    }

    #[test]
    fn ewram_wait_is_fixed() {
        let mut bus = GbaMemoryBus::new();
        assert_eq!(bus.cycles_for(0x02000000, 2), 3);
        assert_eq!(bus.cycles_for(0x02000000, 4), 6);
        bus.write16(0x04000204, 0x0003);
        assert_eq!(bus.cycles_for(0x02000000, 2), 3);
    }

    #[test]
    fn waitcnt_rom_ws() {
        let bus = GbaMemoryBus::new();
        assert_eq!(bus.cycles_for(0x08000000, 2), 4);
        assert_eq!(bus.cycles_for(0x08000000, 4), 6);
    }

    #[test]
    fn prefetch_sequential_saves_cycles() {
        let mut bus = GbaMemoryBus::new();
        bus.write16(0x04000204, 1 << 14); // prefetch enable
        assert!(bus.prefetch_enabled);
        // 非連続 → 通常 wait
        assert_eq!(bus.cycles_for(0x08000000, 4), 6);
        // 連続読みでプリフェッチキューが貯まる
        let _ = bus.read32(0x08000000);
        // 次の連続アドレスはプリフェッチヒットで 1 cycle
        assert_eq!(bus.cycles_for(0x08000004, 4), 2);
        bus.write16(0x04000204, 0); // disable clears queue
        assert!(!bus.prefetch_enabled);
        assert!(bus.prefetch_queue.is_empty());
    }

    #[test]
    fn dma_invalidates_prefetch() {
        let mut bus = GbaMemoryBus::new();
        bus.write16(0x04000204, 1 << 14); // prefetch enable
        let _ = bus.read32(0x08000000);
        assert!(!bus.prefetch_queue.is_empty());
        bus.invalidate_prefetch_for_dma(0x08000004);
        assert!(bus.prefetch_queue.is_empty());
        // Non-ROM DMA should not clear (I/O)
        let _ = bus.read32(0x08000000);
        assert!(!bus.prefetch_queue.is_empty());
        bus.invalidate_prefetch_for_dma(0x04000000);
        assert!(!bus.prefetch_queue.is_empty());
    }

    #[test]
    fn read_write_dispcnt() {
        let mut bus = GbaMemoryBus::new();
        assert_eq!(bus.read16(0x04000000), 0x0080);
        bus.write16(0x04000000, 0x0403);
        assert_eq!(bus.read16(0x04000000), 0x0403);
    }

    #[test]
    fn read_vcount() {
        let mut bus = GbaMemoryBus::new();
        assert_eq!(bus.read16(0x04000006), 0);
        // VCOUNT は RO
        bus.write16(0x04000006, 0x1234);
        assert_eq!(bus.read16(0x04000006), 0);
    }

    #[test]
    fn write_if_clears() {
        let mut bus = GbaMemoryBus::new();
        bus.sif = 0x0003;
        bus.write16(0x04000202, 0x0001);
        assert_eq!(bus.sif, 0x0002);
        bus.write16(0x04000202, 0x0002);
        assert_eq!(bus.sif, 0x0000);
    }

    #[test]
    fn keyinput_always_1_upper_bits() {
        let mut bus = GbaMemoryBus::new();
        bus.set_keyinput(0x0000);
        assert_eq!(bus.read16(0x04000130) & 0xFC00, 0xFC00);
        bus.set_keyinput(0x03FF);
        assert_eq!(bus.read16(0x04000130), 0x03FF | 0xFC00);
    }

    #[test]
    fn unaligned_ldr_rotates() {
        let mut bus = GbaMemoryBus::new();
        bus.write32(0x03000000, 0x12345678);
        // GBA LDR: addr & !3 から読んで ROR (addr&3)*8
        assert_eq!(bus.read32(0x03000001), 0x78123456);
        assert_eq!(bus.read32(0x03000002), 0x56781234);
        assert_eq!(bus.read32(0x03000003), 0x34567812);
    }

    #[test]
    fn unaligned_ldrh_truncates() {
        let mut bus = GbaMemoryBus::new();
        bus.write16(0x03000000, 0xABCD);
        // ARM7TDMIの奇数アドレスLDRHは、整列読出しを8bitローテートする。
        assert_eq!(bus.read16(0x03000001), 0xCDAB);
        assert_eq!(bus.read_ldr_halfword(0x03000001), 0xCD0000AB);

        bus.write16(0x03000000, 0x00FF);
        assert_eq!(bus.read_ldr_halfword(0x03000001), 0xFF000000);
    }

    #[test]
    fn haltcnt_byte_access_and_interrupt_wakeup() {
        let mut bus = GbaMemoryBus::new();
        // Writing 0x81 to HALTCNT: bit 0 = 1 means "Normal" (no halt)
        bus.write8(0x04000301, 0x81);
        assert_eq!(bus.read8(0x04000301), 0x81);
        assert_eq!(bus.read8(0x04000300), 1);
        assert!(!bus.is_halted());

        // Writing 0x80 to HALTCNT: bit 0 = 0 means "Enter HALT state"
        bus.write8(0x04000301, 0x80);
        assert!(bus.is_halted());

        // Test interrupt wakeup from halt
        let mut bus = GbaMemoryBus::new();
        bus.write16(0x04000200, 1);
        bus.enter_halt(1);
        bus.request_interrupt(1);
        assert!(!bus.is_halted());
        assert_eq!(bus.read16(0x03007FF8) & 1, 1);
    }

    #[test]
    fn hle_bios_haltcnt_write_enters_halt() {
        let mut bus = GbaMemoryBus::new();
        // write16 of 0x0001: POSTFLG=1, HALTCNT=0 (bit0=0 → halt)
        bus.write16(0x04000300, 0x0001);

        assert_eq!(bus.read8(0x04000300), 1);
        assert_eq!(bus.read8(0x04000301), 0);
        assert!(bus.is_halted());

        let mut bus = GbaMemoryBus::new();
        // write16 of 0x0101: POSTFLG=1, HALTCNT=1 (bit0=1 → no halt)
        bus.write16(0x04000300, 0x0101);
        assert!(!bus.is_halted());

        // HLE BIOS writes should also trigger halt
        let mut bus = GbaMemoryBus::new();
        bus.write_hle_bios16(0x04000300, 0x0001);
        assert!(bus.is_halted());

        let mut bus = GbaMemoryBus::new();
        bus.write_hle_bios32(0x04000300, 0x0000_0001);
        assert!(bus.is_halted());
    }

    #[test]
    fn halt_with_a_pending_enabled_irq_returns_immediately() {
        let mut bus = GbaMemoryBus::new();
        bus.write16(0x04000200, 1);
        bus.request_interrupt(1);
        bus.enter_halt(1);

        assert!(!bus.is_halted());
    }

    #[test]
    fn svc_vector_contains_safe_loop() {
        let mut bus = GbaMemoryBus::new();
        bus.set_current_pc(0x08);
        assert_eq!(bus.read32(0x08), 0xEAFF_FFFE);
    }

    #[test]
    fn immediate_dma_transfers_memory_and_clears_enable() {
        let mut bus = GbaMemoryBus::new();
        bus.write32(0x03000000, 0xDEADBEEF);
        bus.write32(0x040000D4, 0x03000000);
        bus.write32(0x040000D8, 0x02000000);
        bus.write32(0x040000DC, 0x84000001);
        // Immediate DMA has 3-cycle pending (CPU can run) plus transfer wait
        let mut done = false;
        for _ in 0..30 {
            bus.tick();
            if bus.read32(0x02000000) == 0xDEADBEEF {
                done = true;
                break;
            }
        }
        assert!(done);
        assert_eq!(bus.read16(0x040000DE) & 0x8000, 0);
    }

    #[test]
    fn timer_overflow_sets_if_and_cascades() {
        let mut bus = GbaMemoryBus::new();
        bus.write32(0x04000104, 0x00840000);
        bus.write32(0x04000100, 0x00C0FFFE);
        for _ in 0..4 {
            bus.tick();
        }
        assert_ne!(bus.read16(0x04000202) & (1 << 3), 0);
        assert_eq!(bus.read16(0x04000104), 1);
    }
}
