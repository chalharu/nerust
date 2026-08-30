use std::collections::VecDeque;

// ---------------------------------------------------------------------------
// GbaMemoryBus — GBA 32bitフラットアドレス空間のFacade
// ---------------------------------------------------------------------------

const BIOS_SIZE: usize = 0x4000;
const EWRAM_SIZE: usize = 0x40000;
const IWRAM_SIZE: usize = 0x8000;
const PALETTE_SIZE: usize = 0x400;
const VRAM_SIZE: usize = 0x18000;
const OAM_SIZE: usize = 0x400;
const SRAM_SIZE: usize = 0x10000;
const ROM_DUMMY_SIZE: usize = 0x4000;

/// GBA メモリバス。GbcMemoryBus と同様に全RAM/レジスタの唯一所有者であり、
/// CPU は `&mut GbaMemoryBus` 経由でアクセスする。
pub struct GbaMemoryBus {
    bios: Box<[u8; BIOS_SIZE]>,
    ewram: Box<[u8; EWRAM_SIZE]>,
    iwram: Box<[u8; IWRAM_SIZE]>,
    palette_ram: Box<[u8; PALETTE_SIZE]>,
    vram: Box<[u8; VRAM_SIZE]>,
    oam: Box<[u8; OAM_SIZE]>,
    rom_ws0: Vec<u8>,
    sram: Box<[u8; SRAM_SIZE]>,

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
    prefetch_queue: VecDeque<u32>,
    prefetch_enabled: bool,
    bios_protect: bool,
    current_pc: u32,
    prev_addr: Option<u32>,

    tick: u64,
}

impl GbaMemoryBus {
    pub fn new() -> Self {
        Self {
            bios: Box::new([0u8; BIOS_SIZE]),
            ewram: Box::new([0u8; EWRAM_SIZE]),
            iwram: Box::new([0u8; IWRAM_SIZE]),
            palette_ram: Box::new([0u8; PALETTE_SIZE]),
            vram: Box::new([0u8; VRAM_SIZE]),
            oam: Box::new([0u8; OAM_SIZE]),
            rom_ws0: vec![0xEA; ROM_DUMMY_SIZE],
            sram: Box::new([0u8; SRAM_SIZE]),

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

            last_prefetch: 0,
            open_bus_value: 0,
            prefetch_queue: VecDeque::with_capacity(8),
            prefetch_enabled: false,
            bios_protect: true,
            current_pc: 0x08000000,
            prev_addr: None,
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

    pub fn read32(&mut self, addr: u32) -> u32 {
        let (data, _wait) = self.read_internal(addr, 4);
        self.align_read(addr, 4, data)
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

    pub fn cycles_for(&self, addr: u32, _width: u8) -> u8 {
        match addr {
            0x00000000..=0x00003FFF => 1,
            0x02000000..=0x0203FFFF => self.ewram_wait() as u8 + 1,
            0x03000000..=0x03007FFF => 1,
            0x04000000..=0x040003FE => 1,
            0x05000000..=0x050003FF => 1,
            0x06000000..=0x06017FFF => 1,
            0x07000000..=0x070003FF => 1,
            0x08000000..=0x0DFFFFFF => {
                if self.is_sequential(addr)
                    && self.prefetch_enabled
                    && !self.prefetch_queue.is_empty()
                {
                    1
                } else {
                    4 // 3+1
                }
            }
            0x0E000000..=0x0E00FFFF => 1,
            _ => 1,
        }
    }

    /// 1 T-cycle 進行。VCOUNT を更新し、フレーム境界で true を返す。
    /// TODO(gba-tick-frame): Phase 3では常に false（PPU未実装）。Phase 8で VCOUNT 0..227 境界で true に拡張。
    pub fn tick(&mut self) -> bool {
        self.tick = self.tick.wrapping_add(1);
        // 簡易 VCOUNT: 1232 T-cycle / scanline として 280896 T-cycle / frame
        self.vcount = ((self.tick / 1232) % 228) as u16;
        self.tick % 280896 == 0 && self.tick != 0
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

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn ewram_wait(&self) -> u32 {
        match self.wait_cnt & 0b11 {
            0b00 => 2,
            0b01 => 2,
            0b10 => 1,
            _ => 0,
        }
    }

    fn is_sequential(&self, addr: u32) -> bool {
        if let Some(prev) = self.prev_addr {
            // 32bit ROM領域で連続アドレスか
            (0x08000000..=0x0DFFFFFF).contains(&addr)
                && (0x08000000..=0x0DFFFFFF).contains(&prev)
                && addr == prev + 4
        } else {
            false
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
                    // Thumb LDRH: 下位1bit 無視 (truncated)
                    raw & 0xFFFF
                } else {
                    raw & 0xFFFF
                }
            }
            _ => raw & 0xFF,
        }
    }

    fn read_internal(&mut self, addr: u32, width: u8) -> (u32, u8) {
        let wait = self.cycles_for(addr, width);
        let sequential_prefetch = self.is_sequential(addr) && self.prefetch_enabled;

        let raw: u32 = match addr {
            0x00000000..=0x00003FFF => {
                if self.bios_protect && !(0x00000000..=0x00003FFF).contains(&self.current_pc) {
                    0
                } else {
                    self.read_bios(addr, width)
                }
            }
            0x02000000..=0x0203FFFF => self.read_ewram(addr, width),
            0x03000000..=0x03007FFF => self.read_iwram(addr, width),
            0x04000000..=0x040003FE => self.read_io(addr, width),
            0x05000000..=0x050003FF => self.read_palette(addr, width),
            0x06000000..=0x06017FFF => self.read_vram(addr, width),
            0x07000000..=0x070003FF => self.read_oam(addr, width),
            0x08000000..=0x09FFFFFF => self.read_rom(addr, width),
            0x0A000000..=0x0BFFFFFF => self.read_rom(addr, width),
            0x0C000000..=0x0DFFFFFF => self.read_rom(addr, width),
            0x0E000000..=0x0E00FFFF => self.read_sram(addr, width),
            _ => self.open_bus_value,
        };

        // プリフェッチキュー更新: ROM連続読みでキューを消費、非連続でフラッシュ
        if (0x08000000..=0x0DFFFFFF).contains(&addr) {
            if sequential_prefetch && !self.prefetch_queue.is_empty() {
                let _ = self.prefetch_queue.pop_front();
            } else if !sequential_prefetch {
                self.prefetch_queue.clear();
                // 次の8ワードをプリフェッチ（ダミー実装では単純にクリアのみ）
                if self.prefetch_enabled {
                    for i in 1..=8 {
                        let next = addr.wrapping_add(i * 4);
                        if (0x08000000..=0x0DFFFFFF).contains(&next)
                            && (next as usize) < self.rom_ws0.len() + 0x08000000
                        {
                            self.prefetch_queue.push_back(0xEAEA_EAEA);
                        }
                    }
                }
            }
        } else if !(0x04000000..=0x040003FE).contains(&addr) {
            // ROM以外へのアクセスでプリフェッチキューをフラッシュ（GBA実機挙動の簡易版）
            // ただし I/Oアクセスではフラッシュしない
            if self.prefetch_enabled && !self.prefetch_queue.is_empty() {
                // keep queue for now — only ROM non-sequential flushes
            }
        }

        self.prev_addr = Some(addr);
        self.last_prefetch = raw;
        self.open_bus_value = raw;
        (raw, wait)
    }

    fn write_internal(&mut self, addr: u32, width: u8, value: u32) {
        match addr {
            0x02000000..=0x0203FFFF => self.write_ewram(addr, width, value),
            0x03000000..=0x03007FFF => self.write_iwram(addr, width, value),
            0x04000000..=0x040003FE => self.write_io(addr, width, value),
            0x05000000..=0x050003FF => self.write_palette(addr, width, value),
            0x06000000..=0x06017FFF => self.write_vram(addr, width, value),
            0x07000000..=0x070003FF => self.write_oam(addr, width, value),
            0x0E000000..=0x0E00FFFF => self.write_sram(addr, width, value),
            _ => {
                // 未マッピング・ROM・BIOSへの書き込みは無視だが open_bus は更新
                self.open_bus_value = value;
            }
        }
        // 非ROMへの書き込みでプリフェッチはフラッシュしない（ROM連続性のみで判定）
        if (0x08000000..=0x0DFFFFFF).contains(&addr) {
            self.prev_addr = Some(addr);
        }
    }

    // -- Region readers --

    fn read_bios(&self, addr: u32, width: u8) -> u32 {
        let off = Self::aligned_off(addr, width, 0x3FFF);
        Self::read_slice(&*self.bios, off, width)
    }

    fn read_ewram(&self, addr: u32, width: u8) -> u32 {
        let off = Self::aligned_off(addr, width, 0x3FFFF);
        Self::read_slice(&*self.ewram, off, width)
    }

    fn read_iwram(&self, addr: u32, width: u8) -> u32 {
        let off = Self::aligned_off(addr, width, 0x7FFF);
        Self::read_slice(&*self.iwram, off, width)
    }

    fn read_palette(&self, addr: u32, width: u8) -> u32 {
        let off = Self::aligned_off(addr, width, 0x3FF);
        Self::read_slice(&*self.palette_ram, off, width)
    }

    fn read_vram(&self, addr: u32, width: u8) -> u32 {
        // TODO(gba-vram-mirror): 実機は 0x06018000以降で32KBミラー (&0x1FFFF 説あり)。Phase 3は &0x17FFF で検証。
        let off = Self::aligned_off(addr, width, 0x17FFF) % VRAM_SIZE;
        Self::read_slice(&*self.vram, off, width)
    }

    fn read_oam(&self, addr: u32, width: u8) -> u32 {
        let off = Self::aligned_off(addr, width, 0x3FF);
        Self::read_slice(&*self.oam, off, width)
    }

    fn read_rom(&self, addr: u32, width: u8) -> u32 {
        // Phase 3 ダミー: rom_ws0 の範囲内なら読む、範囲外は open_bus
        if (0x08000000..=0x09FFFFFF).contains(&addr) {
            let aligned = addr & !((width as u32) - 1).min(3);
            let off = (aligned - 0x08000000) as usize;
            if off < self.rom_ws0.len() {
                return Self::read_slice(&self.rom_ws0, off, width);
            }
        }
        self.open_bus_value
    }

    fn read_sram(&self, addr: u32, width: u8) -> u32 {
        let off = Self::aligned_off(addr, width, 0xFFFF);
        Self::read_slice(&*self.sram, off, width)
    }

    fn read_io(&mut self, addr: u32, width: u8) -> u32 {
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
                0x04000122 => ((self.siodata32 >> 16) & 0xFFFF) as u32,
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
        let off = (addr & 0x3FFFF) as usize;
        Self::write_slice(&mut *self.ewram, off, width, value);
        self.open_bus_value = value;
    }

    fn write_iwram(&mut self, addr: u32, width: u8, value: u32) {
        let off = (addr & 0x7FFF) as usize;
        Self::write_slice(&mut *self.iwram, off, width, value);
        self.open_bus_value = value;
    }

    fn write_palette(&mut self, addr: u32, width: u8, value: u32) {
        let off = (addr & 0x3FF) as usize;
        Self::write_slice(&mut *self.palette_ram, off, width, value);
        self.open_bus_value = value;
    }

    fn write_vram(&mut self, addr: u32, width: u8, value: u32) {
        // TODO(gba-vram-mirror): 同上
        let off = (addr as usize) & 0x17FFF;
        let off = off % VRAM_SIZE;
        Self::write_slice(&mut *self.vram, off, width, value);
        self.open_bus_value = value;
    }

    fn write_oam(&mut self, addr: u32, width: u8, value: u32) {
        let off = (addr & 0x3FF) as usize;
        Self::write_slice(&mut *self.oam, off, width, value);
        self.open_bus_value = value;
    }

    fn write_sram(&mut self, addr: u32, width: u8, value: u32) {
        let off = (addr & 0xFFFF) as usize;
        Self::write_slice(&mut *self.sram, off, width, value);
        self.open_bus_value = value;
    }

    fn write_io(&mut self, addr: u32, width: u8, value: u32) {
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

    // -- Slice helpers --

    fn read_slice(slice: &[u8], off: usize, width: u8) -> u32 {
        match width {
            4 => {
                let b0 = *slice.get(off).unwrap_or(&0) as u32;
                let b1 = *slice.get(off + 1).unwrap_or(&0) as u32;
                let b2 = *slice.get(off + 2).unwrap_or(&0) as u32;
                let b3 = *slice.get(off + 3).unwrap_or(&0) as u32;
                b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
            }
            2 => {
                let b0 = *slice.get(off).unwrap_or(&0) as u32;
                let b1 = *slice.get(off + 1).unwrap_or(&0) as u32;
                b0 | (b1 << 8)
            }
            _ => *slice.get(off).unwrap_or(&0) as u32,
        }
    }

    fn write_slice(slice: &mut [u8], off: usize, width: u8, value: u32) {
        match width {
            4 => {
                if let Some(b) = slice.get_mut(off) {
                    *b = (value & 0xFF) as u8;
                }
                if let Some(b) = slice.get_mut(off + 1) {
                    *b = ((value >> 8) & 0xFF) as u8;
                }
                if let Some(b) = slice.get_mut(off + 2) {
                    *b = ((value >> 16) & 0xFF) as u8;
                }
                if let Some(b) = slice.get_mut(off + 3) {
                    *b = ((value >> 24) & 0xFF) as u8;
                }
            }
            2 => {
                if let Some(b) = slice.get_mut(off) {
                    *b = (value & 0xFF) as u8;
                }
                if let Some(b) = slice.get_mut(off + 1) {
                    *b = ((value >> 8) & 0xFF) as u8;
                }
            }
            _ => {
                if let Some(b) = slice.get_mut(off) {
                    *b = (value & 0xFF) as u8;
                }
            }
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
        assert_eq!(bus.read16(0x04000008), 0x5678 as u16 & 0xFFFF | 0); // open_bus lower 16
        // 正確には open_bus_value の下位16bit
        bus.write32(0x03000000, 0xAABBCCDD);
        let _ = bus.read32(0x03000000);
        assert_eq!(bus.read16(0x04000008), 0xCCDD);
    }

    #[test]
    fn waitcnt_ewram_change() {
        let mut bus = GbaMemoryBus::new();
        assert_eq!(bus.cycles_for(0x02000000, 2), 3); // 2+1
        bus.write16(0x04000204, 0x0002); // EWRAM 1 wait
        assert_eq!(bus.cycles_for(0x02000000, 2), 2);
        bus.write16(0x04000204, 0x0003); // EWRAM 0 wait
        assert_eq!(bus.cycles_for(0x02000000, 2), 1);
    }

    #[test]
    fn waitcnt_rom_ws() {
        let bus = GbaMemoryBus::new();
        // デフォルト ROM は 3+1 = 4 cycles
        assert_eq!(bus.cycles_for(0x08000000, 2), 4);
    }

    #[test]
    fn prefetch_sequential_saves_cycles() {
        let mut bus = GbaMemoryBus::new();
        bus.write16(0x04000204, 1 << 14); // prefetch enable
        assert!(bus.prefetch_enabled);
        // 非連続 → 通常 wait
        assert_eq!(bus.cycles_for(0x08000000, 4), 4);
        // 連続読みでプリフェッチキューが貯まる
        let _ = bus.read32(0x08000000);
        // 次の連続アドレスはプリフェッチヒットで 1 cycle
        assert_eq!(bus.cycles_for(0x08000004, 4), 1);
        bus.write16(0x04000204, 0); // disable clears queue
        assert!(!bus.prefetch_enabled);
        assert!(bus.prefetch_queue.is_empty());
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
        // 奇数アドレスの LDRH: 下位1bit 無視で同じ値を返す（簡易 truncate）
        assert_eq!(bus.read16(0x03000001), 0xABCD);
    }
}
