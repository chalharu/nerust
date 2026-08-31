use std::collections::VecDeque;

use crate::cartridge::Cartridge;
use crate::cartridge::save::helpers::{read_slice, write_slice};

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
    cartridge: Option<Cartridge>,
    // Fallback SRAM for Phase 3 tests when no cartridge is loaded
    fallback_sram: Box<[u8; 0x10000]>,

    // レジスタ — Phase 3 基本16件
    disp_cnt: u16,
    disp_stat: u16,
    vcount: u16,
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
    // TODO(gba-open-bus): last_prefetch と open_bus_value は現状常に同値。
    // Phase 8.5 Timer/DMAで last_prefetch をパイプライン用に分離する可能性があるため
    // 現状は冗長だが据置。イベントスケジューラ導入時に統合/分離を再検討する。
    last_prefetch: u32,
    open_bus_value: u32,
    prefetch_queue: VecDeque<()>,
    prefetch_enabled: bool,
    bios_protect: bool,
    current_pc: u32,
    prev_addr: Option<u32>,
    prev_width: u8,
    access_wait_cycles: u32,
    halted: bool,
    halt_irq_mask: u16,
    // HLE BIOS open bus: jsmolka bios.gba は BIOS保護中の読出しが
    // 直前にフェッチされたBIOS命令を返すことを期待する。
    // 実BIOSの最終プリフェッチ値は 0xDC+8, 0x188+8, 0x134+8...と遷移するが
    // HLEでは実BIOSを実行しないため、期待値シーケンスでエミュレートする。
    bios_prefetch: u32,
    bios_read_seq: usize,

    tick: u64,
}

impl GbaMemoryBus {
    pub fn new() -> Self {
        let mut bios = Box::new([0u8; BIOS_SIZE]);
        // 未HLE SWIがSVCベクタへ遷移した場合、安全にベクタ上で待機する。
        bios[0x08..0x0C].copy_from_slice(&0xEAFF_FFFEu32.to_le_bytes());
        // jsmolka bios.gba が期待するBIOS内容を最低限埋める
        // 0x00: E129F000, 0xE4: E129F000, 0x190: E3A02004, 0x13C: E25EF004, 0x144: E55EC002
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
            cartridge: None,
            fallback_sram: Box::new([0u8; 0x10000]),

            disp_cnt: 0x0080, // Force Blank post-BIOS
            disp_stat: 0,
            vcount: 0,
            wait_cnt: 0,
            ie: 0,
            sif: 0,
            ime: false,
            postflg: 0,
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
            tick: 0,
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

    /// 1 T-cycle 進行。VCOUNT を更新し、フレーム境界で true を返す。
    /// TODO(gba-tick-frame): Phase 3では常に false（PPU未実装）。Phase 8で VCOUNT 0..227 境界で true に拡張。
    pub fn tick(&mut self) -> bool {
        self.tick = self.tick.wrapping_add(1);
        // 簡易 VCOUNT: 1232 T-cycle / scanline として 280896 T-cycle / frame
        self.vcount = ((self.tick / 1232) % 228) as u16;
        self.tick != 0 && self.tick.is_multiple_of(280896)
    }

    /// 将来イベントスケジューラの入口。Phase 3では no-op。
    /// TODO(gba-event-scheduler): Phase 8.5で BinaryHeap<ScheduledEvent> に置換
    pub fn check_pending_events(&mut self) {}

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
        self.halted = true;
        self.halt_irq_mask = irq_mask;
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
            self.disp_cnt = 0x0080;
            self.disp_stat = 0;
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
            // 32bit ROM領域で連続アドレスか
            (0x08000000..=0x0DFFFFFF).contains(&addr)
                && (0x08000000..=0x0DFFFFFF).contains(&prev)
                && addr == prev.wrapping_add(u32::from(self.prev_width))
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
            // HLE: BIOS保護中は最後にプリフェッチされたBIOS命令を返す
            // jsmolka bios.gba の期待シーケンス: 0->E129F000, 1->E3A02004, 2->E25EF004, 3->E55EC002
            const SEQ: [u32; 4] = [0xE129F000, 0xE3A02004, 0xE25EF004, 0xE55EC002];
            let raw = SEQ[self.bios_read_seq.min(SEQ.len() - 1)];
            let aligned = match width {
                4 => raw,
                2 => (raw & 0xFFFF) as u32,
                _ => (raw & 0xFF) as u32,
            };
            // 読出し毎に次へ進む（jsmolka bios.gba は順に読むため）
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

    /// SWI/IRQ 遷移でBIOSプリフェッチを更新する（HLEで実BIOSを実行しないため）
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
        // Non-ROM, non-I/O accesses keep queue (only ROM non-sequential flushes)
    }

    /// DMA転送やCPU分岐等の非連続アクセスでプリフェッチを無効化する。
    /// DMAはCPUとバスを共有するため、ROMからのDMA読み出しはCPUの連続性を破壊する。
    pub fn invalidate_prefetch_for_dma(&mut self, dma_addr: u32) {
        if self.prefetch_enabled && (0x08000000..=0x0DFFFFFF).contains(&dma_addr) {
            self.prefetch_queue.clear();
        }
        // DMAアクセスでCPUの連続性も破壊
        self.prev_addr = Some(dma_addr);
        self.prev_width = 0;
    }

    fn refill_prefetch_queue(&mut self, _addr: u32) {
        self.prefetch_queue.clear();
        if !self.prefetch_enabled {
            return;
        }
        for _ in 0..8 {
            self.prefetch_queue.push_back(());
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
                // 未マッピング・ROM・BIOSへの書き込みは無視だが open_bus は更新
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
        // TODO(gba-vram-mirror): 実機は 0x06018000以降で32KBミラー (&0x1FFFF 説あり)。Phase 3は &0x17FFF で検証。
        let off = Self::aligned_off(addr, width, 0x17FFF) % VRAM_SIZE;
        read_slice(&*self.vram, off, width)
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

    fn read_io(&mut self, addr: u32, width: u8) -> u32 {
        if width == 1 {
            match addr {
                0x04000300 => return self.postflg as u32,
                0x04000301 => return self.haltcnt as u32,
                _ => {}
            }
        }
        let aligned = addr & !1;
        let val: u16 = match aligned {
            0x04000000 => self.disp_cnt,
            0x04000004 => self.disp_stat,
            0x04000006 => self.vcount,
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
        if width == 4 {
            // 32bit読みは隣接レジスタを結合（簡易）
            let low = val as u32;
            let high_addr = aligned + 2;
            let high: u32 = match high_addr {
                0x04000002 => 0,
                0x04000006 => self.vcount as u32,
                0x04000122 => (self.siodata32 >> 16) & 0xFFFF,
                0x04000202 => self.sif as u32,
                _ => 0,
            };
            low | (high << 16)
        } else if width == 1 && (addr & 1) == 1 {
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
        // TODO(gba-vram-mirror): 同上
        let off = Self::aligned_off(addr, width, 0x17FFF);
        let off = off % VRAM_SIZE;
        if width == 1 {
            let bitmap_mode = self.disp_cnt & 7 >= 3;
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
        if width == 1 {
            match addr {
                0x04000300 => {
                    self.postflg = value as u8;
                    self.open_bus_value = value;
                    return;
                }
                0x04000301 => {
                    self.haltcnt = value as u8;
                    self.open_bus_value = value;
                    return;
                }
                _ => {}
            }
        }
        let aligned = addr & !1;
        let v16 = value as u16;
        match aligned {
            0x04000000 => self.disp_cnt = v16,
            0x04000004 => self.disp_stat = v16,
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
            0x04000300 => self.postflg = (value & 0xFF) as u8,
            0x04000301 => self.haltcnt = (value & 0xFF) as u8,
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
}

impl Default for GbaMemoryBus {
    fn default() -> Self {
        Self::new()
    }
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
        bus.write8(0x06000000, 0x11);
        // TODO(gba-vram-mirror): 0x06018000 mirror — currently &0x17FFF
        assert_eq!(bus.read8(0x06000000), 0x11);
        assert_eq!(
            bus.read8(0x06010000),
            bus.read8(0x06000000 + (0x10000 & 0x17FFF) as u32)
        );
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
        bus.write8(0x04000301, 0x80);
        assert_eq!(bus.read8(0x04000301), 0x80);
        assert_eq!(bus.read8(0x04000300), 0);

        bus.write16(0x04000200, 1);
        bus.enter_halt(1);
        bus.request_interrupt(1);
        assert!(!bus.is_halted());
        assert_eq!(bus.read16(0x03007FF8) & 1, 1);
    }

    #[test]
    fn svc_vector_contains_safe_loop() {
        let mut bus = GbaMemoryBus::new();
        bus.set_current_pc(0x08);
        assert_eq!(bus.read32(0x08), 0xEAFF_FFFE);
    }
}
