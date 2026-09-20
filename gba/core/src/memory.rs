use crate::apu::GbaApu;
use crate::bios::HleBiosOperation;
use crate::cartridge::Cartridge;
use crate::cartridge::save::helpers::{read_slice, repeat_byte, selected_write_byte, write_slice};
use crate::dma::{DmaTrigger, GbaDma};
use crate::ppu::{GbaPpu, HDRAW_CYCLES};
use crate::timer::GbaTimers;

// ---------------------------------------------------------------------------
// GbaMemoryBus — GBA 32bitフラットアドレス空間のFacade
// ---------------------------------------------------------------------------

/// True for Thumb opcodes performing a data access (loads, stores,
/// push/pop, block transfers). Everything else (ALU, nop, address
/// math, branches) leaves the bus idle for a cycle.
fn thumb_next_is_datamover(next: u16) -> bool {
    match next >> 12 {
        0b0101 | 0b0110 | 0b0111 | 0b1000 | 0b1001 | 0b1100 => true,
        0b1011 => (0xB400..=0xB5FF).contains(&next) || (0xBC00..=0xBDFF).contains(&next),
        _ => false,
    }
}

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
    apu: GbaApu,
    cartridge: Option<Cartridge>,
    // Fallback SRAM for Phase 3 tests when no cartridge is loaded
    fallback_sram: Box<[u8; 0x10000]>,

    // レジスタ — Phase 3 基本16件
    wait_cnt: u16,
    ie: u16,
    sif: u16,
    ime: bool,
    /// Delayed interrupt pipeline: IE/IME/IF writes and IRQ raises apply
    /// to effective registers with staged latency; reads return effective values.
    pending_ie: u16,
    pending_ime: bool,
    pending_if: u16,
    pending_at: Option<u64>,
    /// An IE/IME register write armed the pending apply: the CPU line
    /// asserts one tick after apply for write-triggered asserts (the
    /// write is bus-synchronous), while device raises synchronize an
    /// extra tick. Pinned by nba-emu irq-delay (92/112/120) against
    /// mgba timer-irq and cancel-irq-ime (raise-triggered, +2).
    line_write_assert: bool,
    irq_available: bool,
    avail_queue: Vec<(bool, u64)>,
    irq_line: bool,
    line_queue: Vec<(bool, u64)>,
    postflg: u8,
    /// Write-only HALTCNT latch (GBATEK System Control). Reads return open
    /// bus; bit 7 selects Stop mode (wake mask IE&0x3080, SIO+Keypad+Pak).
    #[allow(dead_code)]
    haltcnt: u8,
    /// 4000800h Internal Memory Control (GBATEK System Control, R/W, init
    /// 0D000020h, mirrored each 64K). Stored only: the WRAM remap/wait bits
    /// have no observable effect implemented (no test coverage).
    mem_control: u32,
    keyinput: u16,
    keycnt: u16,
    siocnt: u16,
    siodata8: u16,
    siodata32: u32,
    rcnt: u16,
    joycnt: u16,
    /// Pending SIO Normal-mode transfer: T-cycles until completion (0 =
    /// idle) and whether it is 32-bit. Scheduled on the START edge;
    /// completion clears START, delivers pulled-high receive data and
    /// raises the serial IRQ when enabled.
    sio_xfer_cycles: u32,
    sio_xfer_32: bool,
    /// UART shift state: true while a UART frame is on the wire.
    sio_xfer_uart: bool,
    /// UART FIFOs (4-deep each when enabled, single unit otherwise).
    uart_tx: std::collections::VecDeque<u8>,
    uart_rx: std::collections::VecDeque<u8>,
    /// Latched UART error flag (cleared by SIOCNT read, GBATEK).
    uart_err: bool,
    /// Previous UART IRQ-source levels for edge detection.
    uart_prev_irqsrc: u8,

    // Bus制御
    last_prefetch: u32,
    /// Last two fetched opcodes ([older, newer]) for open-bus modeling
    /// (updated by opcode fetches only — data reads/writes never touch
    /// the bus latch, GBATEK "Unpredictable").
    prefetch_win: [u32; 2],
    /// Whether the newest fetch was Thumb (16-bit window composition).
    prefetch_thumb: bool,
    /// Shared CPU/DMA open-bus latch (power-on 0xFFFFFFFF): driven by DMA
    /// reads from accessible sources and by CPU OAM loads (full OAM word);
    /// returned (lane-selected, never updated) for CPU reads from
    /// IO-unmapped addresses and Thumb-mode DMA reads from unreadable
    /// sources (HW-pinned by the alyosha Bus suite).
    cpu_bus: u32,
    /// GamePak prefetch enable (WAITCNT bit 14). The enable gates only
    /// the prefetch erase (`prefetch_erase_delta`): opcode fetches always
    /// follow the fetch stream at S/N cost (the suite proves pure-fetch
    /// streams get no ride discount: nop P.. = 6 = S+S), and prefetch
    /// hides data/internal stalls via erases.
    prefetch_enabled: bool,
    /// Block-transfer continuation: LDM/STM/PUSH/POP words after the first
    /// follow bus order via `data_continuation_sequential`.
    data_sequential_override: bool,
    /// Block-transfer erase batching (2+ word LDM/STM/PUSH/POP): word 1
    /// erases like a single access, continuation words erase marginally,
    /// total floored at one N-fetch worth. Set by `begin_block_batch`.
    block_batching: bool,
    block_batch_any: bool,
    block_batch_words: u32,
    block_batch_erase_sum: i32,
    block_batch_is_load: bool,
    block_batch_fetch_width: u8,
    /// True when a ROM word appeared inside the batch (OAM-overflow LDM
    /// into ROM): HW (mgba-suite Timing OAM cells) applies NO prefetch
    /// erase at all then, so `end_block_batch` undoes the tracked erases.
    /// Pure non-ROM bursts keep word1 + marginals + floor. The remaining
    /// +1 on mixed bursts (S-words + S-speed parity) arrives via the fill
    /// clock (`fill_collision`): open-bus words advance it, the first ROM
    /// word collides on a finishing fill.
    block_batch_has_rom: bool,
    /// Raw bulk mode (HLE CpuSet/CpuFastSet loops): HW BIOS runs from
    /// BIOS ROM (flat waits, no GamePak-prefetch erase dynamics), so bulk
    /// words accrue raw `(wait-1)` with no erase at all. Set with
    /// `begin_block_batch_raw`; bypasses even the ROM-code gate.
    block_batch_raw: bool,
    /// Address of the most recently fetched opcode. Scopes the
    /// fetch-stream-break charge (`charge_fetch_stream_break`) to the
    /// owning code region (PC-tagged, like the CPU pipeline it models).
    /// fetch-stream-break charge (`charge_fetch_stream_break`) to the
    /// owning code region (PC-tagged, like the CPU pipeline it models).
    last_opcode_addr: Option<u32>,
    /// Execute PC of the last retired Thumb single word load (stack or
    /// ROM data, literals included), with its stack-data class tag.
    /// Readers validate adjacency (+2) or a 3-instruction window
    /// (pc-diff <= 6), so branches, mode switches, IRQ and DMA need no
    /// explicit clear (any of them breaks the distance).
    prev_load_pc: Option<u32>,
    prev_load_is_stack: bool,
    /// Tick of the last timer0 down-edge IF raise (missed-take entry
    /// compensation reads the take latency off it).
    timer0_raise_tick: u64,
    /// End address of the current prefetch run, capping overlap fills in
    /// `prefetch_erase_delta`.
    last_prefetched_pc: u32,
    bios_protect: bool,
    current_pc: u32,
    prev_addr: Option<u32>,
    prev_width: u8,
    /// Opcode-fetch N/S stream, independent of the data stream: data
    /// accesses never break code sequentiality and vice versa.
    fetch_addr: Option<u32>,
    fetch_width: u8,
    /// GamePak prefetch buffer window [pf_start, pf_end) of buffered
    /// bytes (8 halfwords). A ROM opcode fetch inside the window costs S
    /// (sliding the window as background refills keep up); outside costs
    /// N and refills ahead (capped at the next 128KiB boundary, where the
    /// prefetcher stops). HW-pinned by the alyosha prefetcher suite
    /// (branch-to-buffered costs S, boundary stops, full pauses).
    pf_start: u32,
    pf_end: u32,
    pf_valid: bool,
    /// A taken branch may consume already-buffered opcodes. Once they
    /// drain, the first fetch outside the window is non-sequential; the
    /// ordinary linear fetch stream resumes after that miss.
    pf_branch_drain: bool,
    /// Prefetch fill phase (bus-wait clock): ticks since the last fill
    /// redirect, modulo the fill duty below. A ROM data access issued
    /// while a half-word fill finishes costs one arbitration cycle
    /// (Mesen `GbaRomPrefetch::Reset` + NanoBoyAdvance `Bus::StopPrefetch`
    /// agree on the mechanism; the countdown values are pinned by the
    /// mgba-suite Timing cells). Sequential opcode fetches advance by
    /// exactly one duty (no-op); fetch misses and ROM data accesses
    /// redirect (reset). Only ROM-bus occupancy advances it: CPU internal
    /// cycles, IO/RAM accesses and whole DMA bursts freeze it (the bus
    /// grant blocks fills), except open-bus data reads which free a
    /// single ROM-bus slot.
    fill_countdown: u32,
    /// Signed prefetch-erase deltas; MUST stay signed until `take_*` at the
    /// instruction boundary (clamping at zero overshoots every Thumb P-cell).
    access_wait_cycles: i64,
    halted: bool,
    halt_irq_mask: u16,
    /// Stop mode latched (HALTCNT bit 7, GBATEK "System Control"): the CPU
    /// stays parked until a wake interrupt arrives. GBATEK SWI 03h stops the
    /// CPU, system clock, sound, video, SIO-shift clock, DMAs and timers, so
    /// the PPU/timers/DMA are frozen here too (see `tick`'s stopped branch);
    /// only the IRQ pipeline keeps running so a wake request can land.
    stopped: bool,
    /// IntrWait/VBlankIntrWait wake-clear mask (GBATEK: waited flags are
    /// reset in the BIOS RAM mirror upon wake). Plain Halt leaves this zero.
    wake_clear_mask: u16,
    /// IntrWait/VBlankIntrWait wake-exit latency (real-BIOS exit-path cost).
    /// The HLE returns inline, so without this the woken thread outruns the
    /// staging IRQ line; consumed as CPU-stall cycles by the step loop.
    wake_latency: u32,
    /// Set when a halted CPU wakes for IRQ; consumed by the next take.
    /// Wake takes keep the full entry (T4 gate): only the first take
    /// after a wake is special, and the flag never leaks past it.
    woke_from_halt: bool,
    /// Pending DMA prefetch-collision arbitration stall. Set when a
    /// DMA GamePak access collides with an in-flight prefetch fill
    /// (fill_collision fired); burned as a CPU-stall tick at the next
    /// step boundary so the arbitration cycle advances the bus clock.
    /// Set (not counted): at most one stall per burst is observable.
    dma_stall_pending: u32,
    bios_prefetch: u32,
    current_tcycle: u64,
    hle_bios: Option<HleBiosOperation>,
    video_armed: bool,
    video_countdown: u8,
    /// HBlank IRQ deferred one tick: raised at the next tick start, so only
    /// same-tick IF reads observe the lag; CPU entry timing is unchanged.
    pending_hblank_irq: bool,
    /// A DMA burst is currently feeding the EEPROM serial chip; closed when
    /// no DMA channel is active or pending (frame decoded at burst end).
    eeprom_burst_open: bool,
    /// Test-ROM log sink behind the `mgba-debug-log` cargo feature. No
    /// hardware counterpart exists: zero waits, no prefetch/N-S side effects.
    #[cfg(feature = "mgba-debug-log")]
    mgba_debug_enable: bool,
    #[cfg(feature = "mgba-debug-log")]
    mgba_debug_buf: [u8; 256],
    #[cfg(feature = "mgba-debug-log")]
    mgba_debug_logs: Vec<MgbaDebugLog>,
}

/// One committed suite debug-log line (the suite sources call it
/// `mgba_printf`).
/// Only exists with the `mgba-debug-log` cargo feature (test harness).
#[cfg(feature = "mgba-debug-log")]
#[derive(Debug, Clone)]
pub struct MgbaDebugLog {
    /// `MGBA_LOG_*` level from the flags write (`INFO = 3`, `DEBUG = 4`).
    pub level: u8,
    /// NUL-terminated string buffer content at commit time.
    pub text: String,
}

/// Snapshot of the PPU state relevant to display-controller contention.
/// Lets DMA cost computation query contention without borrowing the bus
/// while a DMA channel is mutably borrowed.
#[derive(Clone, Copy)]
struct DisplayStallSnapshot {
    forced_blank: bool,
    vcount: u16,
    cycle: u16,
    dispcnt: u16,
    bg_fetch_active: bool,
}

impl DisplayStallSnapshot {
    fn stall(self, addr: u32) -> u8 {
        if self.forced_blank {
            return 0;
        }
        if self.vcount >= 160 {
            return 0;
        }
        // BG/palette data is fetched during draw only. OAM stays busy
        // through HBlank too, unless H-Blank Interval Free idles it. The
        // HBlank free window opens at HDraw end (cycle 960, GBATEK LCD
        // Dimensions: 240 draw dots + 68 blank dots) — not at the HBlank
        // flag (1006), which only marks the flag/IRQ edge.
        let in_hblank = self.cycle >= HDRAW_CYCLES;
        // BG fetch clock (archive/ppu/mode3): contention only while the
        // fetcher runs and a BG is enabled (latched AND live).
        let in_fetch = (32..989).contains(&self.cycle) && self.bg_fetch_active;
        // Palette feeds every rendered pixel (backdrop included).
        let in_draw = self.cycle <= HDRAW_CYCLES;
        let oam_busy = !in_hblank || ((self.dispcnt & (1 << 5)) == 0);
        // OBJ texture fetch needs the OBJ layer live (hw-test ROM
        // ram-access DISPCNT-latch rule: fetch iff CURRENT enable, latch
        // disregarded). OAM evaluation itself stays ungated: it runs
        // during draw regardless (hw-test ROM burst-into-tears needs
        // its 3 OAM stalls with OBJ disabled, TIME 41 vs 38).
        let obj_fetch = oam_busy && (self.dispcnt & (1 << 12)) != 0;
        match addr {
            0x05000000..=0x05FFFFFF => u8::from(in_draw),
            0x06000000..=0x06FFFFFF => {
                let bitmap_mode = (self.dispcnt & 7) >= 3;
                let bg_limit = if bitmap_mode { 0x14000 } else { 0x10000 };
                // Classify by the same folded offset the data path uses:
                // 0x18000-0x1FFFF mirrors 0x10000-0x17FFF (OBJ bank), so a
                // raw-mask test would misfile mirror accesses as OBJ-busy.
                let off = addr & 0x1FFFF;
                let off = if off >= 0x18000 { off - 0x8000 } else { off };
                if off < bg_limit {
                    u8::from(in_fetch)
                } else {
                    u8::from(obj_fetch)
                }
            }
            0x07000000..=0x07FFFFFF => u8::from(oam_busy),
            _ => 0,
        }
    }
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
            apu: GbaApu::new(),
            cartridge: None,
            fallback_sram: Box::new([0u8; 0x10000]),

            wait_cnt: 0,
            ie: 0,
            sif: 0,
            ime: false,
            postflg: 1,
            haltcnt: 0,
            mem_control: 0x0D00_0020,
            keyinput: 0x03FF,
            keycnt: 0,
            siocnt: 0,
            siodata8: 0,
            siodata32: 0,
            rcnt: 0x8000,
            joycnt: 0,
            sio_xfer_cycles: 0,
            sio_xfer_32: false,
            sio_xfer_uart: false,
            uart_tx: std::collections::VecDeque::new(),
            uart_rx: std::collections::VecDeque::new(),
            uart_err: false,
            uart_prev_irqsrc: 0,

            last_prefetch: 0xE129F000,
            prefetch_win: [0xE129F000, 0xE129F000],
            prefetch_thumb: false,
            cpu_bus: 0xFFFF_FFFF,
            prefetch_enabled: false,
            data_sequential_override: false,
            block_batching: false,
            block_batch_any: false,
            block_batch_words: 0,
            block_batch_erase_sum: 0,
            block_batch_is_load: true,
            block_batch_fetch_width: 4,
            block_batch_has_rom: false,
            block_batch_raw: false,
            last_opcode_addr: None,
            prev_load_pc: None,
            prev_load_is_stack: false,
            timer0_raise_tick: 0,
            last_prefetched_pc: 0,
            bios_protect: true,
            current_pc: 0x08000000,
            prev_addr: None,
            prev_width: 0,
            fetch_addr: None,
            fetch_width: 0,
            pf_start: 0,
            pf_end: 0,
            pf_valid: false,
            pf_branch_drain: false,
            fill_countdown: 4,
            access_wait_cycles: 0,
            halted: false,
            halt_irq_mask: 0,
            stopped: false,
            wake_clear_mask: 0,
            wake_latency: 0,
            woke_from_halt: false,
            dma_stall_pending: 0,
            pending_ie: 0,
            pending_ime: false,
            pending_if: 0,
            pending_at: None,
            line_write_assert: false,
            irq_available: false,
            avail_queue: Vec::new(),
            irq_line: false,
            line_queue: Vec::new(),
            bios_prefetch: 0xE129F000,
            current_tcycle: 0,
            hle_bios: None,
            video_armed: false,
            video_countdown: 0,
            pending_hblank_irq: false,
            eeprom_burst_open: false,
            #[cfg(feature = "mgba-debug-log")]
            mgba_debug_enable: false,
            #[cfg(feature = "mgba-debug-log")]
            mgba_debug_buf: [0; 256],
            #[cfg(feature = "mgba-debug-log")]
            mgba_debug_logs: Vec::new(),
        }
    }

    // -----------------------------------------------------------------------
    // Public API — 3幅 + fetch
    // -----------------------------------------------------------------------

    pub fn read8(&mut self, addr: u32) -> u8 {
        let (data, _wait) = self.read_internal(addr, 1, false);
        (data & 0xFF) as u8
    }

    /// RCNT top-mode bit selects the SIO block (00) over GPIO (10) and
    /// Joybus (11): only in SIO mode do the SIOCNT sub-mode and transfer
    /// engine apply.
    fn sio_block_selected(&self) -> bool {
        self.rcnt & 0xC000 == 0
    }

    /// SIO sub-mode from SIOCNT bits 12-13: 0 = Normal-8, 1 = Normal-32,
    /// 2 = Multiplayer, 3 = UART.
    fn sio_submode(siocnt: u16) -> u8 {
        ((siocnt >> 12) & 3) as u8
    }

    pub fn read16(&mut self, addr: u32) -> u16 {
        let (data, _wait) = self.read_internal(addr, 2, false);
        self.align_read(addr, 2, data) as u16
    }

    /// ARM7TDMI LDRH result, including the 32-bit ROR 8 result for odd addresses.
    pub fn read_ldr_halfword(&mut self, addr: u32) -> u32 {
        let (data, _wait) = self.read_internal(addr, 2, false);
        if addr & 1 != 0 {
            (data & 0xFFFF).rotate_right(8)
        } else {
            data & 0xFFFF
        }
    }

    pub fn read32(&mut self, addr: u32) -> u32 {
        let (data, _wait) = self.read_internal(addr, 4, false);
        self.align_read(addr, 4, data)
    }

    /// Aligned word transfer used by LDM, which ignores address bits 0-1 without ROR.
    pub fn read_aligned32(&mut self, addr: u32) -> u32 {
        self.read_internal(addr & !3, 4, false).0
    }

    pub fn write8(&mut self, addr: u32, value: u8) {
        self.write_internal(addr, 1, value as u32, false);
    }

    pub fn write16(&mut self, addr: u32, value: u16) {
        self.write_internal(addr, 2, value as u32, false);
    }

    pub fn write32(&mut self, addr: u32, value: u32) {
        self.write_internal(addr, 4, value, false);
    }

    pub fn write_hle_bios16(&mut self, addr: u32, value: u16) {
        self.write_internal(addr, 2, value as u32, true);
    }

    pub fn write_hle_bios8(&mut self, addr: u32, value: u8) {
        self.write_internal(addr, 1, value as u32, true);
    }

    pub fn write_hle_bios32(&mut self, addr: u32, value: u32) {
        self.write_internal(addr, 4, value, true);
    }

    pub(crate) fn start_hle_bios(&mut self, operation: HleBiosOperation) {
        debug_assert!(self.hle_bios.is_none());
        self.hle_bios = Some(operation);
    }

    pub(crate) fn hle_bios_active(&self) -> bool {
        self.hle_bios.is_some()
    }

    pub(crate) fn step_hle_bios(&mut self) -> u32 {
        self.take_access_wait_cycles();
        let mut operation = self.hle_bios.take().expect("active HLE BIOS operation");
        let step = operation.step(self);
        if !step.complete {
            self.hle_bios = Some(operation);
        }
        (step.cycles as i64 + self.take_access_wait_cycles()).max(0) as u32
    }

    pub fn fetch16(&mut self, addr: u32) -> u16 {
        let (data, _wait) = self.read_internal(addr, 2, true);
        self.align_read(addr, 2, data) as u16
    }

    pub fn fetch32(&mut self, addr: u32) -> u32 {
        let (data, _wait) = self.read_internal(addr, 4, true);
        self.align_read(addr, 4, data)
    }

    /// Wait cycles for a data access.
    pub fn cycles_for(&self, addr: u32, width: u8) -> u8 {
        self.cycles_for_access(addr, width, false)
    }

    /// Wait cycles for an opcode fetch (prefetch buffer + Disable Bug apply).
    pub fn opcode_cycles_for(&self, addr: u32, width: u8) -> u8 {
        self.cycles_for_access(addr, width, true)
    }

    fn cycles_for_access(&self, addr: u32, width: u8, is_opcode: bool) -> u8 {
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
            // I/O stays width-insensitive at 1 cycle: twelve HW-pinned
            // 128kb-boundary DMA fits measure three 32-bit setup stores
            // (SAD/DAD/CNT) at 1 cycle each. The 16-bit-bus theory would
            // charge 2 and overshoots every one of them by exactly +3.
            0x04000000..=0x040003FE => 1,
            0x05000000..=0x05FFFFFF => {
                // GBATEK bus widths: Palette 16bit=1, 32bit=2 (+display stall).
                (if width == 4 { 2 } else { 1 }) + self.display_stall(addr)
            }
            0x06000000..=0x06FFFFFF => {
                // GBATEK bus widths: VRAM 16bit=1, 32bit=2 (+display stall).
                (if width == 4 { 2 } else { 1 }) + self.display_stall(addr)
            }
            // OAM stays width-insensitive at 1 cycle: the HW-pinned
            // 128kb-boundary LDM from 0x07FFFFF8 measures 28 with two
            // 32-bit OAM loads at 1 cycle each (the sprite engine itself
            // fetches A01 32-bit, i.e. a 32-bit bus). Palette/VRAM keep
            // the 16-bit-bus 32bit=2 split.
            0x07000000..=0x07FFFFFF => 1 + self.display_stall(addr),
            0x08000000..=0x0DFFFFFF => {
                // Linear opcode fetches follow the fetch stream at S cost.
                // After a branch, already-buffered targets still cost S,
                // but the first fetch beyond that window costs N. Data
                // accesses are nonsequential (continuation words use the
                // sequential data path); the fetch-stream break is pre-paid
                // per instruction.
                let sequential = if is_opcode {
                    // Fetches issued while a DMA burst is pending (trigger
                    // stored, bus handover imminent) cost N: the arbitrated
                    // bus is non-sequential (HW-pinned by hw-test ROM
                    // force-nseq-access: post-trigger nops cost 1N).
                    // With prefetch off there is no buffer: plain address
                    // sequentiality decides (fetch_stream unit tests pin
                    // S for in-stream, N for jumps ahead).
                    !self.dma.has_pending()
                        && (if self.prefetch_enabled {
                            self.fetch_buffer_hit(addr)
                                || (!self.pf_branch_drain && self.is_fetch_sequential(addr))
                        } else {
                            self.is_fetch_sequential(addr)
                        })
                } else {
                    self.data_sequential_override
                };
                if is_opcode {
                    self.gamepak_rom_cycles(addr, width, sequential)
                } else {
                    // Data reads use N/S waitstates and never the buffer.
                    self.gamepak_rom_cycles(addr, width, sequential)
                }
            }
            0x0E000000..=0x0FFFFFFF => {
                // GBATEK WAITCNT: SRAM Wait Control selects 4/3/2/8
                // waitstates, and like every access the total is 1 clock
                // cycle PLUS waitstates. The SRAM bus is 8-bit and wide CPU
                // accesses move a single byte (GBATEK SRAM data semantics),
                // so there is no width multiplier.
                const SRAM_WAIT: [u8; 4] = [4, 3, 2, 8];
                SRAM_WAIT[(self.wait_cnt & 0b11) as usize] + 1
            }
            _ => 1,
        }
    }

    /// Extra wait while the LCD controller fetches the same video memory.
    /// +1 while actively drawing, 0 during blanks/forced blank.
    fn display_stall(&self, addr: u32) -> u8 {
        DisplayStallSnapshot {
            forced_blank: self.ppu.forced_blank(),
            vcount: self.ppu.vcount(),
            cycle: self.ppu.cycle(),
            dispcnt: self.ppu.dispcnt(),
            bg_fetch_active: self.ppu.bg_fetch_active(),
        }
        .stall(addr)
    }

    /// Advance the LCD controller by exactly one T-cycle.
    pub fn tick(&mut self) -> bool {
        self.current_tcycle = self.current_tcycle.wrapping_add(1);
        // Deferred HBlank IRQ first (+1 tick): same pipeline visibility as
        // a same-tick raise (processed below), so CPU entry is unchanged.
        if self.pending_hblank_irq {
            self.pending_hblank_irq = false;
            self.request_interrupt(1 << 1);
        }
        // Delayed interrupt pipeline first: yesterday's IE/IME/IF writes
        // and IRQ raises become effective before devices run this tick.
        self.process_irq_pipeline();
        if self.stopped {
            // GBATEK Stop: CPU, system clock, video, sound, DMA and timers
            // are frozen; only an interrupt request wakes the machine.
            // (Wake-source subset and IF-not-set are not modeled.)
            return false;
        }
        self.dma.tick_pending();
        if self.apu.tick() {
            crate::bios::sound_driver::mix_driver_grid(self);
        }
        if self.video_countdown > 0 {
            self.video_countdown -= 1;
            if self.video_countdown == 0 {
                self.dma.trigger_channel(3, DmaTrigger::Special);
            }
        }
        let event = self
            .ppu
            .step(&self.vram[..], &self.palette_ram[..], &self.oam[..]);
        if event.hblank_started {
            // GBATEK DISPSTAT: H-Blank conditions are generated once per
            // scanline, including the hidden scanlines during V-Blank — a
            // repeat HBlank channel fires on lines 0..227, not just <160.
            self.dma.trigger(DmaTrigger::HBlank);
        }
        if event.vblank_started {
            self.dma.trigger(DmaTrigger::VBlank);
        }
        if event.line_started {
            // DMA3 video-capture is latched at vcount==162 (a
            // stale still-running transfer is stopped) and fires 3 cycles
            // into each line of vcount in [2, 162).
            let vcount = self.ppu.vcount();
            if vcount == 162 {
                if self.video_armed {
                    self.dma.stop_video_transfer();
                }
                self.video_armed = self.dma.has_video_transfer();
            }
            if self.video_armed && (2..162).contains(&vcount) && self.dma.has_video_transfer() {
                // Video DMA request lands 3 cycles into the line; the first
                // unit's xI is absorbed pre-start, so the countdown alone
                // sets the phase.
                self.video_countdown = 3;
            }
        }
        let (timer_irq, timer_overflow) = {
            self.timers.set_current_cycle(self.current_tcycle);
            self.timers.step_full()
        };
        if timer_overflow != 0 {
            for i in 0..4 {
                if timer_overflow & (1 << i) != 0 {
                    // The overflowing timer clocks one sample byte out of each
                    // selecting FIFO; a FIFO at 14 bytes or fewer requests
                    // its Special DMA channel. Every overflow clocks the
                    // sample stream, whether or not the timer IRQ is
                    // enabled (the IRQ bit only raises IF).
                    if self.apu.soundcnt_x & 0x80 != 0 && i <= 1 {
                        // SOUNDCNT_H bits 8/9/12/13 are output routing, not
                        // a DMA gate (GBATEK SOUNDCNT_H); only the
                        // timer-select bits pick which overflow clocks
                        // each FIFO.
                        for (fifo_b, select_bit) in [(false, 10), (true, 14)] {
                            let timer = (self.apu.soundcnt_hi >> select_bit) & 1;
                            if timer as usize != i {
                                continue;
                            }
                            // The first overflow after the selecting timer's
                            // enable primes the sample pipeline without
                            // consuming (alyosha fifo_4: a preloaded FIFO
                            // must still hold 16 at the second overflow, so
                            // the third — not the second — fires the DMA).
                            // The level check still runs (an empty FIFO
                            // fires its DMA here).
                            if self.timers.overflows_since_enable(i) != 1 {
                                self.apu.drain_fifo(fifo_b);
                            }
                            // Post-drain <=14 bytes requests DMA (alyosha fifo
                            // t002b/fifo_3 pin fire-at-14: pre-pop <=12
                            // never fires there).
                            if self.apu.fifo_len(fifo_b) <= 14
                                && let Some(ch) = self.dma.sound_channel_for_fifo(fifo_b)
                            {
                                self.dma.trigger_channel(ch, DmaTrigger::Special);
                            }
                        }
                    }
                }
            }
        }
        let mut interrupt_mask = event.interrupt_mask | timer_irq;
        // +1-tick HBlank IRQ (see `pending_hblank_irq`): the DISPSTAT flag
        // edge stays immediate, but the IF raise waits a tick. Stash the
        // HBlank bit for the next tick start instead of raising now.
        if interrupt_mask & (1 << 1) != 0 {
            interrupt_mask &= !(1 << 1);
            self.pending_hblank_irq = true;
        }
        // SIO Normal-mode transfer completion (scheduled on the START
        // edge): clears START, delivers pulled-high receive data (no link
        // partner drives the lines low) and raises the serial IRQ when
        // enabled. Pinned by mgba-suite sio-timing (measured = transfer
        // cycles + a constant 121-cycle setup/exit path).
        if self.sio_xfer_cycles > 0 {
            self.sio_xfer_cycles -= 1;
            if self.sio_xfer_cycles == 0 {
                if self.sio_xfer_uart {
                    // UART frame done: the sent byte leaves, an idle-high
                    // byte arrives when receive is enabled (overrun sets
                    // the error flag), then chained frames kick.
                    self.sio_xfer_uart = false;
                    self.uart_tx.pop_front();
                    if self.siocnt & 0x0800 != 0 {
                        if self.uart_rx.len() >= self.uart_fifo_cap() {
                            self.uart_err = true;
                        } else {
                            self.uart_rx.push_back(0xFF);
                        }
                    }
                    self.uart_eval_irq();
                    self.kick_uart();
                } else {
                    self.siocnt &= !0x0080;
                    if self.sio_xfer_32 {
                        self.siodata32 = 0xFFFF_FFFF;
                    } else {
                        // No link partner: only the receive lane reads
                        // pulled-high; the send high byte is preserved
                        // (HW-pinned by serial_read_data).
                        self.siodata8 = (self.siodata8 & 0xFF00) | 0x00FF;
                    }
                    if self.siocnt & 0x4000 != 0 {
                        self.request_interrupt(1 << 7);
                    }
                }
            }
        }
        let stall_snapshot = DisplayStallSnapshot {
            forced_blank: self.ppu.forced_blank(),
            vcount: self.ppu.vcount(),
            cycle: self.ppu.cycle(),
            dispcnt: self.ppu.dispcnt(),
            bg_fetch_active: self.ppu.bg_fetch_active(),
        };
        if let Some(transfer) = self
            .dma
            .step(self.wait_cnt, &|addr| stall_snapshot.stall(addr))
        {
            if std::env::var("GBA_DTRACE").is_ok() {
                eprintln!(
                    "DMA{} t={} vc={} cyc={} src={:#010X} dst={:#010X} w={}",
                    transfer.channel,
                    self.current_tcycle,
                    stall_snapshot.vcount,
                    stall_snapshot.cycle,
                    transfer.data_source,
                    transfer.destination,
                    transfer.width
                );
            }
            let in_eeprom_range = |addr: u32| (0x0D000000..=0x0DFFFFFF).contains(&addr);
            // GBATEK Backup Media: only 16-bit DMA3 drives the EEPROM chip;
            // other channels/widths see the window as ROM/open bus.
            let use_eeprom = transfer.channel == 3
                && transfer.width == 2
                && self.is_eeprom()
                && (in_eeprom_range(transfer.data_source) || in_eeprom_range(transfer.destination));
            let readable_source = transfer.data_source >= 0x02000000;
            let value = if use_eeprom && in_eeprom_range(transfer.data_source) {
                // EEPROM DMA read: one response bit per 16-bit unit.
                let bit = self.next_eeprom_read_bit();
                if transfer.width == 4 {
                    bit | bit << 16
                } else {
                    bit
                }
            } else if readable_source {
                // 16-bit GamePak reads pre-increment (dest[i] =
                // mem16(src+2+2i)), firing only for primed bursts whose
                // read lands in ROM; single-unit and non-ROM reads are
                // unaffected. N/S timing follows the programmed counter.
                let src = transfer.data_source;
                let read_addr = if transfer.width == 2
                    && transfer.shift_primed
                    && !transfer.single_unit
                    && ((0x08000000..=0x0DFFFFFF).contains(&src)
                        || (0x08000000..=0x0DFFFFFF).contains(&src.wrapping_add(2)))
                {
                    src.wrapping_add(2)
                } else {
                    src
                };
                let value = self.read_dma_source(read_addr, transfer.width);
                // GamePak ROM reads collide with an in-flight prefetch
                // fill exactly like CPU ROM data (Mesen/NBA/ares agree):
                // advance the fill clock across the handover idle plus
                // earlier non-cartridge ticks this burst, then charge the
                // collision. One-shot per burst via the restart below.
                // The penalty stalls the bus (a real tick, like Mesen's
                // Step on Reset): acc charging would be discarded by the
                // resume instruction's expansion take. Idempotent set:
                // a both-ROM unit collides on read and write but stalls
                // once.
                if crate::dma::is_rom(read_addr) && self.pf_valid {
                    self.fill_advance(transfer.pre_read_idle);
                    if self.fill_collision() != 0 {
                        self.dma_stall_pending = 1;
                    }
                }
                self.dma
                    .update_latch(transfer.channel, transfer.width, value);
                // DMA reads from accessible sources also drive the shared
                // bus latch (HW-pinned by DMA_IWRAM_Bus: the ROM word read
                // by the priming DMA is what the racing DMA samples).
                // Inaccessible sources leave it alone (DMA_CPU_Bus needs
                // the power-on 0xFF to survive the priming DMA).
                let inaccessible = is_unreadable_io(read_addr)
                    || ((0x04000800..=0x04FFFFFF).contains(&read_addr)
                        && !is_mem_control(read_addr));
                if !inaccessible {
                    self.cpu_bus = if transfer.width == 4 {
                        value
                    } else {
                        let half = value & 0xFFFF;
                        half | (half << 16)
                    };
                }
                value
            } else if transfer.width == 2 && transfer.destination & 2 != 0 {
                transfer.latched_value >> 16
            } else {
                transfer.latched_value
            };
            if use_eeprom && in_eeprom_range(transfer.destination) {
                // EEPROM DMA write: each unit carries serial bit(s).
                self.feed_eeprom_write(transfer.width, value);
            } else {
                // GamePak ROM writes collide like reads (same one-shot).
                if crate::dma::is_rom(transfer.destination) && self.pf_valid {
                    self.fill_advance(transfer.pre_write_idle);
                    if self.fill_collision() != 0 {
                        self.dma_stall_pending = 1;
                    }
                }
                self.write_dma_value(
                    transfer.channel,
                    transfer.destination,
                    transfer.width,
                    value,
                );
            }
            // DMA owns the bus between CPU accesses: the CPU's next access
            // is non-sequential (GBATEK DMA owns the bus).
            self.prev_addr = None;
            self.prev_width = 0;
            self.fetch_addr = None;
            self.fetch_width = 0;
            // The ROM prefetch buffer survives DMA that never touches
            // GamePak ROM (ROM contents can't change under DMA, so the
            // buffered opcodes stay valid). Only ROM-touching DMA
            // restarts the buffer.
            if crate::dma::is_rom(transfer.data_source) || crate::dma::is_rom(transfer.destination)
            {
                self.pf_valid = false;
                self.pf_branch_drain = false;
            }
            // Completion IRQs are raised via take_completion_interrupts
            // below (one tick after the final write).
        }
        interrupt_mask |= self.dma.take_completion_interrupts();
        if interrupt_mask != 0 {
            self.request_interrupt(interrupt_mask);
        }
        // Close an EEPROM serial burst once no DMA is in flight: the
        // buffered frame is decoded (and 512B/8KB latched) at burst end.
        if self.eeprom_burst_open && !self.dma.is_active() && !self.dma.has_pending() {
            self.eeprom_burst_open = false;
            if let Some(cart) = self.cartridge.as_mut() {
                cart.eeprom_end_burst();
            }
        }
        event.frame_complete
    }

    pub fn dma_active(&self) -> bool {
        self.dma.is_active()
    }

    pub fn irq_pending(&self) -> bool {
        // Delayed CPU IRQ line (IME && IE&IF, ~3 ticks after
        // the request). Sampled by the CPU once per instruction.
        self.irq_line
    }

    /// Post-enable timer0 take deferral (the first take samples a boundary
    /// later than the line alone allows): while timer0's IF is up within a
    /// few ticks of its first overflow since the fresh enable, the take
    /// waits for a later boundary (IF stays raised, so nothing is lost).
    /// Gated on the first couple of overflows: an established-rate timer
    /// (many overflows already) takes immediately, as do stale takes from
    /// before the enable (no overflow since it yet). The delay is relative
    /// to the first overflow, not the enable: longer reloads overflow past
    /// any enable window. Starts with a fresh reload (atomic 32-bit
    /// enables, whose data phase lands the enable later) sample later
    /// than starts from an earlier split reload (the mgba timer suites
    /// pin 5, the cancel_ime race pins 3).
    pub fn defer_timer0_take(&self) -> bool {
        if self.irq_flags() & (1 << 3) == 0 {
            return false;
        }
        if self.timers.overflows_since_enable(0) > 2 {
            return false;
        }
        let window = if self.timers.last_enable_fresh_reload(0) {
            5
        } else {
            3
        };
        self.timers
            .last_ovf1_cycle(0)
            .is_some_and(|at| self.current_tcycle.wrapping_sub(at) < window)
    }

    /// Delayed interrupt pipeline: apply due pendings, then propagate
    /// availability (+1) and the CPU line (+2). Queued transitions are
    /// never cancelled, so transient edges stay observable.
    fn process_irq_pipeline(&mut self) {
        let now = self.current_tcycle;
        if self.pending_at.is_some_and(|at| at <= now) {
            self.pending_at = None;
            let line_fast = std::mem::replace(&mut self.line_write_assert, false);
            let ie = self.pending_ie;
            let ime = self.pending_ime;
            let sif = self.pending_if;
            if ie != self.ie || ime != self.ime || sif != self.sif {
                self.ie = ie;
                self.ime = ime;
                self.sif = sif;
                // GBATEK Stop wake sources are "as far as enabled in IE
                // register" (present tense): while stopped, track IE edits
                // instead of keeping enter_stop's entry-time snapshot.
                // (Plain Halt never sets `stopped`, so its IntrWait mask is
                // unaffected.)
                if self.stopped {
                    self.halt_irq_mask = ie & 0x3080;
                }
                // BIOS IRQ-flags mirror follows the effective IF.
                self.iwram[0x7FF8..0x7FFA].copy_from_slice(&sif.to_le_bytes());
                let avail = ie & sif != 0;
                let avail_cur = self
                    .avail_queue
                    .last()
                    .map(|(v, _)| *v)
                    .unwrap_or(self.irq_available);
                if avail != avail_cur {
                    self.avail_queue.push((avail, now + 1));
                }
                let line = ime && avail;
                let line_cur = self
                    .line_queue
                    .last()
                    .map(|(v, _)| *v)
                    .unwrap_or(self.irq_line);
                if line != line_cur {
                    let delay = if line && line_fast { 1 } else { 2 };
                    self.line_queue.push((line, now + delay));
                }
                // Halt wake on the effective IE/IF registers, evaluated at
                // apply time (sees final levels); CPU entry uses the
                // delayed line.
                if ie & sif & self.halt_irq_mask != 0 {
                    self.evaluate_halt_wake();
                }
            }
        }
        while self.avail_queue.first().is_some_and(|(_, at)| *at <= now) {
            let (avail, _) = self.avail_queue.remove(0);
            self.irq_available = avail;
        }
        while self.line_queue.first().is_some_and(|(_, at)| *at <= now) {
            let (line, _) = self.line_queue.remove(0);
            self.irq_line = line;
        }
    }

    pub fn frame_buffer(&self) -> &[u32] {
        self.ppu.frame_buffer()
    }

    pub fn ppu_bgcnt(&self, bg: usize) -> u16 {
        self.ppu.bgcnt(bg)
    }

    pub fn ppu_hofs(&self, bg: usize) -> u16 {
        self.ppu.hofs(bg)
    }

    pub fn timer_last_reload(&self, ch: usize) -> Option<u64> {
        self.timers.last_reload_cycle(ch)
    }

    pub fn timer_current_cycle(&self) -> u64 {
        self.timers.current_cycle()
    }

    pub fn set_keyinput(&mut self, value: u16) {
        self.keyinput = value | 0xFC00;
        self.check_keycnt();
    }

    /// GBATEK KEYCNT: with bit 14 set, a keypad condition (bit 15:
    /// 0 = any selected key pressed, 1 = all selected keys pressed;
    /// KEYINPUT bits are 0 when pressed) raises IF bit 12.
    fn check_keycnt(&mut self) {
        if self.keycnt & (1 << 14) == 0 {
            return;
        }
        let mask = self.keycnt & 0x3FF;
        if mask == 0 {
            return;
        }
        let pressed = !self.keyinput & 0x3FF;
        let hit = if self.keycnt & (1 << 15) != 0 {
            pressed & mask == mask
        } else {
            pressed & mask != 0
        };
        if hit {
            self.request_interrupt(1 << 12);
        }
    }

    pub fn set_current_pc(&mut self, pc: u32) {
        self.current_pc = pc;
    }

    pub fn is_bios_addr(&self, addr: u32) -> bool {
        (0x00000000..=0x00003FFF).contains(&addr)
    }

    pub fn enter_halt(&mut self, irq_mask: u16) {
        self.halt_irq_mask = irq_mask;
        // Halt entry samples the not-yet-applied level: pending IE/IF is
        // exactly what the 1-tick pipeline will apply, so a just-raised IF
        // prevents halting while a just-written ack is honored. Unconditional
        // entry was tried and regressed nba_haltcnt (5 -> 4 vs 6 expected);
        // the wake path samples the same level one tick later (applied).
        self.halted =
            !(self.irq_available && self.pending_ie & self.pending_if & self.halt_irq_mask != 0);
    }

    /// SWI Stop / HALTCNT-stop: park the CPU with clocks down.
    pub fn enter_stop(&mut self) {
        self.stopped = true;
        // GBATEK HALTCNT Stop: only keypad, GamePak and serial interrupts
        // wake the machine (timers/DMA/video cannot).
        self.enter_halt(self.ie & 0x3080);
    }

    pub fn is_halted(&self) -> bool {
        self.halted
    }

    /// Take a pending IntrWait wake-exit latency (see `wake_latency`).
    pub fn take_wake_latency(&mut self) -> u32 {
        std::mem::take(&mut self.wake_latency)
    }

    /// Take pending DMA fill-collision arbitration stalls (see
    /// `dma_stall_pending`). Burned as CPU-stall ticks like wake
    /// latency, so the bus arbitration cycle is measured.
    pub fn take_dma_stall(&mut self) -> u32 {
        std::mem::take(&mut self.dma_stall_pending)
    }

    pub fn request_interrupt(&mut self, mask: u16) {
        // Raise: OR into the pending IF level; applied (with the
        // BIOS RAM mirror) 1 tick later by process_irq_pipeline. Halt wake
        // is evaluated when availability propagates, not here.
        let mask = mask & 0x3FFF;
        // Stamp down-edge timer0 raises: the missed-take entry rule
        // reads the take latency off the answered raise.
        if mask & (1 << 3) != 0 && self.pending_if & (1 << 3) == 0 {
            self.timer0_raise_tick = self.current_tcycle;
        }
        // T6: the 2nd timer0 overflow raising IF from down applies 2
        // ticks later (slow-row take#2 runs 2 early otherwise: ovf#2 at
        // count==2 with a fresh atomic ps0 enable pins +2 on exactly
        // this edge). First raisings keep the fast pipeline (T3 window
        // and live cells pin it), 3rd+ raisings are return-anchored or
        // absorbed (T5 covers their entry), and non-timer0 sources never
        // match. Mechanism open (see the timers box note).
        if mask & (1 << 3) != 0
            && self.pending_if & (1 << 3) == 0
            && self.timers.overflows_since_enable(0) == 2
            && self.timers.last_enable_fresh_reload(0)
            && self.timers.prescaler_bits(0) == 0
        {
            self.pending_if |= mask;
            self.pending_at = Some(self.current_tcycle + 3);
            return;
        }
        self.pending_if |= mask;
        self.pending_at = Some(self.current_tcycle + 1);
    }

    /// Wake a halted/stopped CPU once delayed availability arrives.
    fn evaluate_halt_wake(&mut self) {
        if self.ie & self.sif & self.halt_irq_mask != 0 {
            if self.halted {
                // IntrWait wake: reset the waited flags in the BIOS RAM
                // mirror (GBATEK IntrWait/VBlankIntrWait).
                let clear = std::mem::take(&mut self.wake_clear_mask);
                if clear != 0 {
                    let kept =
                        u16::from_le_bytes([self.iwram[0x7FF8], self.iwram[0x7FF9]]) & !clear;
                    self.iwram[0x7FF8..0x7FFA].copy_from_slice(&kept.to_le_bytes());
                }
                // A live-line wake pays the exit latency and stalls past
                // line-rise so the wake dispatch runs before the thread
                // resumes; an IME=0 wake stays free. IntrWait burns 48,
                // plain Halt 32 (timer-source wakes cost 10: the timer
                // IRQ line rises without the serial/DMA/video wake path;
                // pinned by alyosha halt_pc t001 and mgba sio-timing).
                let src = self.ie & self.sif & self.halt_irq_mask;
                let timer_only = src & 0x0078 != 0 && src & !0x0078 == 0;
                self.wake_latency = if clear != 0 {
                    48
                } else if self.ime {
                    if timer_only { 10 } else { 32 }
                } else {
                    0
                };
                self.woke_from_halt = true;
            }
            self.halted = false;
            self.stopped = false;
        }
    }

    /// Raw pending interrupt flags (IE-independent), for IntrWait's r0=0 check.
    /// GBATEK: return immediately when an OLD flag is already set — that is
    /// the raw IF level, not the +1-tick-delayed effective `sif`. A flag
    /// raised just before the SWI (still in `pending_if`) must count.
    pub fn irq_flags(&self) -> u16 {
        self.pending_if
    }

    /// T7 gate: missed-one timer0 takes (third overflow answered second:
    /// the second overflow arrived while masked and was discarded, so
    /// exactly one ack is on record) complete take+entry at a constant
    /// raise+25: the entry absorbs the sampling latency. Storm fast-row
    /// take#2 pins 20/22 on latencies 5/3; clean takes keep T4/T5 and
    /// non-timer0 takes never match (timer0 IF required). Returns the
    /// prologue when the class matches. Mechanism open (see the timers
    /// box note).
    pub fn catchup_timer0_entry(&self) -> Option<u32> {
        if self.irq_flags() & (1 << 3) == 0
            || self.timers.overflows_since_enable(0) != 3
            || self.timers.timer0_acks_since_enable() != 1
            || !self.timers.last_enable_fresh_reload(0)
            || self.timers.prescaler_bits(0) != 0
        {
            return None;
        }
        let latency = self.current_tcycle.saturating_sub(self.timer0_raise_tick);
        Some(25u32.saturating_sub(latency.min(25) as u32))
    }

    /// T8 gate: late-sampled clean take#4s (fourth overflow answered
    /// fourth: every overflow answered, three acks on record) sampled 4+
    /// after the raise complete take+entry at raise+25 like T7: the entry
    /// absorbs the sampling latency. Storm slow-row take#4 pins 21 on
    /// latency 4; earlier-sampled (latency 3) takes keep T5, missed takes
    /// keep T7, and non-timer0 takes never match (timer0 IF required).
    /// Returns the prologue when the class matches. Mechanism open (see
    /// the timers box note).
    pub fn late_timer0_entry(&self) -> Option<u32> {
        if self.irq_flags() & (1 << 3) == 0
            || self.timers.overflows_since_enable(0) != 4
            || self.timers.timer0_acks_since_enable() != 3
            || !self.timers.last_enable_fresh_reload(0)
            || self.timers.prescaler_bits(0) != 0
        {
            return None;
        }
        let latency = self.current_tcycle.saturating_sub(self.timer0_raise_tick);
        if latency < 4 {
            return None;
        }
        Some(25u32.saturating_sub(latency.min(25) as u32))
    }

    /// T5 gate: established timer0 re-takes (third overflow onward from
    /// a fresh atomic prescaler-0 enable) enter 1 more expensive. Storm
    /// take#2+ phase pins +1 on exactly this class (2i values ran
    /// systematic -1 with the T4-only model); first takes keep T4/full
    /// entry, and non-timer0 takes never match (timer0 IF required).
    /// Mechanism open (see the timers box note).
    pub fn retook_timer0_entry(&self) -> bool {
        self.irq_flags() & (1 << 3) != 0
            && self.timers.overflows_since_enable(0) >= 3
            && self.timers.last_enable_fresh_reload(0)
            && self.timers.prescaler_bits(0) == 0
    }

    /// T4 gate: boundary takes of a freshly atomically-enabled
    /// prescaler-0 timer0 enter 3 cheaper (mgba timer-irq frozen + storm
    /// 1i/2i pin -3 on exactly this class). Every condition is
    /// ablation-proven load-bearing: dropping the IF gate discounts
    /// unrelated takes; dropping the overflow count or the atomic-enable
    /// gate breaks nba irq-delay (split enables keep 23); dropping the
    /// prescaler gate breaks slow-timer storm rows; wake takes (alyosha
    /// halt_pc_4) keep the full 23. Consumes the wake flag on every take
    /// so it never leaks past the first post-wake take. Mechanism open
    /// (see the timers box note); never fit the entry to these cells
    /// beyond this gate.
    pub fn discount_timer0_entry(&mut self) -> bool {
        let woke = std::mem::take(&mut self.woke_from_halt);
        !woke
            && self.irq_flags() & (1 << 3) != 0
            && self.timers.overflows_since_enable(0) <= 2
            && self.timers.last_enable_fresh_reload(0)
            && self.timers.prescaler_bits(0) == 0
    }

    /// Arm the IntrWait wake-clear mask (cleared on next halt wake).
    pub fn set_wake_clear_mask(&mut self, mask: u16) {
        self.wake_clear_mask = mask & 0x1FFF;
    }

    pub fn reset_io_groups(&mut self, flags: u8) {
        // GBATEK RegisterRamReset bug: LSBs of SIODATA32 are always
        // destroyed, even when bit 5 (SIO) is clear.
        self.siodata32 &= 0xFFFF0000;
        if flags & 0x20 != 0 {
            // RegisterRamReset SIO: SIOCNT=0, RCNT=RCNT_INITIAL(0x8000),
            // SIOMLT_SEND=0, JOYCNT=0, JOY_RECV=0, JOY_TRANS=0 (the JOY
            // data regs read 0 here regardless, so only the latch and
            // transfer state need clearing).
            self.siocnt = 0;
            self.rcnt = 0x8000;
            self.siodata32 = 0;
            self.siodata8 = 0;
            self.joycnt = 0;
            self.sio_xfer_cycles = 0;
            self.sio_xfer_32 = false;
            self.sio_xfer_uart = false;
            self.uart_tx.clear();
            self.uart_rx.clear();
            self.uart_err = false;
            self.uart_prev_irqsrc = 0;
        }
        if flags & 0x40 != 0 {
            self.apu.reset_sound();
        }
        if flags & 0x80 != 0 {
            // Timers are NOT cleared: HW RegisterRamReset proves the timer
            // runs through the reset.
            self.ppu.reset();
            self.dma.reset();
            self.video_armed = false;
            self.video_countdown = 0;
            self.ie = 0;
            self.sif = 0;
            self.ime = false;
            // Keep the delayed pipeline in sync with the direct clear.
            self.pending_ie = 0;
            self.pending_ime = false;
            self.pending_if = 0;
            self.pending_at = None;
            self.line_write_assert = false;
            self.irq_available = false;
            self.avail_queue.clear();
            self.irq_line = false;
            self.line_queue.clear();
            self.wait_cnt = 0;
            self.keycnt = 0;
            self.postflg = 0;
            self.haltcnt = 0;
            self.prefetch_enabled = false;
            self.prev_load_pc = None;
            self.prev_load_is_stack = false;
            self.last_prefetched_pc = 0;
            self.halted = false;
            self.halt_irq_mask = 0;
            self.wake_clear_mask = 0;
        }
    }

    /// 16-bit GamePak lane merge into the shared bus latch (odd
    /// addresses drive the high lane, even use the A1 lane). GamePak
    /// only; other regions never drive the latch here.
    pub(crate) fn merge_rom_half(&mut self, addr: u32, half: u32) {
        if !(0x08000000..=0x0CFFFFFF).contains(&addr) {
            return;
        }
        let half = half & 0xFFFF;
        if addr & 1 == 1 || addr & 2 == 2 {
            self.cpu_bus = (self.cpu_bus & 0xFFFF) | (half << 16);
        } else {
            self.cpu_bus = (self.cpu_bus & 0xFFFF_0000) | half;
        }
    }

    pub fn take_access_wait_cycles(&mut self) -> i64 {
        std::mem::take(&mut self.access_wait_cycles)
    }

    /// Prefetch erase: with prefetch on, a non-ROM data/internal stall fills
    /// the prefetch unit, converting N into S and erasing subsequent S waits.
    /// Returns the delta INSTEAD of the normal `(wait - 1)` contribution.
    pub(crate) fn prefetch_erase_delta(&mut self, addr: u32, wait_our: u32, is_load: bool) -> i32 {
        if !self.prefetch_enabled || addr >= 0x08000000 {
            return 0;
        }
        // Bus-wait space: loads add +2, stores +1 over the raw table.
        // Our raw already carries +1 base, so loads add one more.
        let base_adj = if is_load { 1 } else { 0 };
        let wait_stall = wait_our as i32 + base_adj;
        self.prefetch_stall_erased(wait_stall) - wait_stall
    }

    /// Multiply-tick erase: the m-tick array fills prefetch P-ON.
    /// `tick_wait` is WAIT+m in bus-wait space (MUL 0+m,
    /// MLA/SMULL/UMULL 1+m, SMLAL/UMLAL 2+m); the handler base already
    /// carries the un-erased ticks, so only the delta lands on the bus.
    /// P-ON-gated like the data erase (P-OFF the stall is identity).
    pub(crate) fn erase_for_multiply(&mut self, tick_wait: u32, fetch_width: u8) {
        if !self.prefetch_enabled {
            return;
        }
        let delta = self.prefetch_stall_erased(tick_wait as i32) - tick_wait as i32;
        // MUL ticks are CPU-internal, so the erase benefit caps at one
        // N-fetch worth; scoped to GamePak-ROM code.
        let delta = match self.last_opcode_addr {
            Some(pc @ 0x08000000..=0x0DFFFFFF) => {
                let n_cap = u32::from(self.gamepak_rom_cycles(pc, fetch_width, false)) - 1;
                delta.max(-(n_cap as i32))
            }
            _ => delta,
        };
        self.access_wait_cycles += i64::from(delta);
    }

    /// Prefetch-stall core: erase a wait (bus-wait space) via prefetch
    /// fills, returning the erased wait (possibly larger when the fill
    /// itself costs, possibly negative when it overlaps).
    fn prefetch_stall_erased(&mut self, wait_stall: i32) -> i32 {
        let mut wait = wait_stall;
        // Code-region S16/N16 in bus-wait space (our totals minus base 1).
        let (n16_our, s16_our) = self.code_wait16();
        let s = s16_our as i32 - 1;
        let n = n16_our as i32 - 1;
        // Overlap cap with the previous prefetch run.
        let dist = self.last_prefetched_pc.wrapping_sub(self.current_pc);
        let prev = if dist < 16 { (dist >> 1) as i32 } else { 0 };
        let max_loads = 8 - prev;
        // Halfword slots the stall can fill.
        let mut stall = s + 1;
        let mut loads = 1;
        while stall < wait && loads < max_loads {
            stall += s;
            loads += 1;
        }
        self.last_prefetched_pc = self
            .current_pc
            .wrapping_add(2 * (loads + prev - 1).max(0) as u32);
        if stall > wait {
            wait = stall;
        }
        // This access used to have an N: convert to S; the filled slots
        // erase subsequent S waits (possibly driving this term negative).
        wait -= n - s;
        wait -= stall;
        wait
    }

    /// S16/N16 wait totals (our +1 convention) of the owning code region,
    /// for the prefetch erase. ROM uses the WAITCNT shifts; other regions
    /// carry no N/S split (N == S == the flat access cost).
    fn code_wait16(&self) -> (u32, u32) {
        match self.last_opcode_addr {
            Some(pc @ 0x08000000..=0x0DFFFFFF) => {
                let n = self.gamepak_rom_cycles(pc, 2, false);
                let s = self.gamepak_rom_cycles(pc, 2, true);
                (u32::from(n), u32::from(s))
            }
            Some(pc) => {
                let c = u32::from(self.cycles_for(pc, 2));
                (c, c)
            }
            None => (1, 1),
        }
    }

    /// Fill duty (bus-wait clock): the sequential access cost of the
    /// owning code region in its fetch width. A sequential fetch lasts
    /// exactly one duty, so buffer hits are phase no-ops by construction.
    fn fill_duty(&self) -> u32 {
        let width = if self.prefetch_thumb { 2 } else { 4 };
        match self.last_opcode_addr {
            Some(pc @ 0x08000000..=0x0DFFFFFF) => {
                u32::from(self.gamepak_rom_cycles(pc, width, true)).max(2)
            }
            _ => {
                if self.prefetch_thumb {
                    2
                } else {
                    4
                }
            }
        }
    }

    /// Redirect the fill clock (fetch miss, ROM data access): the
    /// prefetcher restarts its fill cadence.
    fn fill_reset(&mut self) {
        self.fill_countdown = self.fill_duty();
    }

    /// Advance the fill clock by a ROM-bus occupancy (open-bus data
    /// reads free a single slot). Wraps into `[1, duty]`.
    fn fill_advance(&mut self, duration: u32) {
        let duty = self.fill_duty();
        let current = self.fill_countdown.clamp(1, duty);
        self.fill_countdown = (current + duty - 1 - (duration % duty)) % duty + 1;
    }

    /// ROM data collision check: a ROM access issued while a
    /// half-word fill finishes (countdown 1, or the ARM half-duty
    /// midpoint) costs one arbitration cycle, then redirects the clock.
    /// Scoped to prefetch-on ROM code; HLE bulk loops see flat waits
    /// with no prefetch dynamics. Returns the 0/1 penalty.
    fn fill_collision(&mut self) -> i64 {
        if !self.prefetch_enabled {
            return 0;
        }
        if !matches!(self.last_opcode_addr, Some(0x08000000..=0x0DFFFFFF)) {
            return 0;
        }
        if self.block_batch_raw {
            return 0;
        }
        let duty = self.fill_duty();
        let mut countdown = self.fill_countdown;
        if countdown < 1 || countdown > duty {
            countdown = duty;
        }
        let fire = countdown == 1 || (!self.prefetch_thumb && countdown == (duty >> 1) + 1);
        self.fill_countdown = duty;
        i64::from(fire)
    }

    /// True for open-bus data reads (OAM mirror/unmapped space): the ROM
    /// bus idles for one slot while the access completes elsewhere.
    /// OAM-proper, RAM, IO and BIOS reads hold the bus grant instead.
    fn fill_open_read(addr: u32) -> bool {
        (0x0700_0400..=0x07FF_FFFF).contains(&addr)
            || (0x0000_4000..=0x01FF_FFFF).contains(&addr)
            || addr >= 0x1000_0000
    }

    /// HLE SWI entry residual: the inline HLE skips the HW exception entry,
    /// whose cost differs by a tiny code-region constant. IWRAM is
    /// deliberately unadjusted; no code context yields 0.
    pub(crate) fn swi_region_adjust(&self) -> i32 {
        match self.last_opcode_addr {
            Some(0x08000000..=0x09FFFFFF) => {
                if (self.wait_cnt >> 2) & 3 == 0 {
                    1
                } else {
                    0
                }
            }
            Some(0x02000000..=0x02FFFFFF) => -1,
            _ => 0,
        }
    }

    /// True when the calling code runs from IWRAM (FastSet EWRAM-source
    /// bulk residual below; see `swi_region_adjust`).
    pub(crate) fn swi_caller_is_iwram(&self) -> bool {
        matches!(self.last_opcode_addr, Some(0x03000000..=0x03FFFFFF))
    }

    /// HLE charge self-calibration: waits accumulated so far (the HLE
    /// body reads this before/after its bus accesses and subtracts the
    /// actual incurred waits from its displayed cycle count, so the total
    /// stays correct under any bus-wait model).
    pub fn accumulated_wait_cycles(&self) -> i64 {
        self.access_wait_cycles
    }

    pub fn set_cartridge(&mut self, cart: Cartridge) {
        self.cartridge = Some(cart);
    }

    pub fn cartridge(&self) -> Option<&Cartridge> {
        self.cartridge.as_ref()
    }

    pub fn cartridge_mut(&mut self) -> Option<&mut Cartridge> {
        self.cartridge.as_mut()
    }

    pub(crate) fn apu_mut(&mut self) -> &mut GbaApu {
        &mut self.apu
    }

    pub(crate) fn apu(&self) -> &GbaApu {
        &self.apu
    }

    pub fn take_cartridge(&mut self) -> Option<Cartridge> {
        self.cartridge.take()
    }

    pub fn bios_checksum(&self) -> u32 {
        // GBATEK GetBiosChecksum: the real 16K BIOS sums to $BAAE187F (GBA /
        // GBA SP; $BAAE1880 on NDS/3DS-in-GBA-mode). The HLE BIOS image
        // carries no ROM bytes, so report the hardware value directly
        // instead of summing the (zero) placeholder. Pinned by the
        // PeterLemon BIOSCHECKSUM ROM's in-ROM `cmp r0,$BAAE187F` check.
        0xBAAE_187F
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn is_fetch_sequential(&self, addr: u32) -> bool {
        Self::seq_in_rom(self.fetch_addr, self.fetch_width, addr)
    }

    /// Pure buffer-hit check (no mutation): ROM addr inside [pf_start,
    /// pf_end). Used by the S/N decision; mutation happens separately in
    /// `fetch_buffer_update`, so read-only cost queries see the same hit.
    fn fetch_buffer_hit(&self, addr: u32) -> bool {
        (0x08000000..=0x0DFFFFFF).contains(&addr)
            && self.pf_valid
            && self.pf_start <= addr
            && addr < self.pf_end
    }

    /// Slide/refill the buffer after a ROM opcode fetch of `width` bytes.
    /// Hits consume (drain); misses refill ahead. Background refills from
    /// idle cycles arrive via `fetch_buffer_idle` (non-ROM data accesses).
    fn fetch_buffer_update(&mut self, addr: u32, width: u8) {
        if self.fetch_buffer_hit(addr) {
            self.pf_start = addr.wrapping_add(u32::from(width));
        } else {
            let new_start = addr.wrapping_add(u32::from(width));
            let boundary = (addr & !0x1FFFF).wrapping_add(0x20000);
            self.pf_start = new_start;
            self.pf_end = new_start.wrapping_add(16).min(boundary);
            self.pf_valid = true;
            self.pf_branch_drain = false;
            // A missed fetch redirects the prefetcher: restart the fill
            // cadence (sequential hits last exactly one duty, so they
            // need no phase update).
            self.fill_reset();
        }
    }

    /// Background refill during idle (non-ROM-bus) data accesses:
    /// any idle refills fully (sweep trial).
    fn fetch_buffer_idle(&mut self, _wait: u8) {
        if self.prefetch_enabled && self.pf_valid {
            self.pf_end = self.pf_start.wrapping_add(16);
        }
    }

    /// IRQ-entry refill credit: the skipped BIOS prologue leaves the ROM
    /// bus free, so the first handler word is already prefetched. Refund
    /// its N-S when it evolved N (cold tags), keeping the natural window
    /// tail for the rest of the handler stream. ROM + prefetch only.
    pub(crate) fn credit_entry_refill(&mut self, target: u32) {
        if self.prefetch_enabled
            && (0x08000000..=0x0DFFFFFF).contains(&target)
            && !self.fetch_buffer_hit(target)
        {
            let n = self.gamepak_rom_cycles(target, 4, false);
            let s = self.gamepak_rom_cycles(target, 4, true);
            self.access_wait_cycles -= i64::from(n.saturating_sub(s));
        }
    }

    /// Pre-refill the window ahead of a mode-switching branch target
    /// (the branch execution gives the prefetcher time to fill, so the
    /// target fetch hits; HW-pinned by branch_thumb_arm_4: ARM->Thumb
    /// bx lands on a buffered target). Regular branches preserve.
    pub fn refill_prefetch_for_switch(&mut self, target: u32) {
        if self.prefetch_enabled && (0x08000000..=0x0DFFFFFF).contains(&target) {
            let boundary = (target & !0x1FFFF).wrapping_add(0x20000);
            self.pf_start = target;
            self.pf_end = target.wrapping_add(16).min(boundary);
            self.pf_valid = true;
            self.pf_branch_drain = false;
        }
    }

    /// Bus-order contiguity for block-transfer continuation words (LDM/STM
    /// words 2+): sequential to the previous bus access of any kind,
    /// including the 128KB-boundary N-force and region changes (hw-test
    /// 128kb-boundary LDM pins: boundary-crossing words stay N). Single
    /// data accesses are always N; only block loops query this.
    pub(crate) fn data_continuation_sequential(&self, addr: u32) -> bool {
        Self::seq_in_rom(self.prev_addr, self.prev_width, addr)
    }

    fn seq_in_rom(prev: Option<u32>, prev_w: u8, addr: u32) -> bool {
        if let Some(prev) = prev {
            // 32bit ROM領域で連続アドレスか、かつ128KB境界を跨がない
            (0x08000000..=0x0DFFFFFF).contains(&addr)
                && (0x08000000..=0x0DFFFFFF).contains(&prev)
                && addr == prev.wrapping_add(u32::from(prev_w))
                && (addr & !0x1FFFF) == (prev & !0x1FFFF)
        } else {
            false
        }
    }

    fn gamepak_rom_cycles(&self, addr: u32, width: u8, sequential: bool) -> u8 {
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
        if sequential {
            second * if width == 4 { 2 } else { 1 } + if width == 4 { 2 } else { 1 }
        } else if width == 4 {
            first + second + 2
        } else {
            // GBATEK WAITCNT: "the actual access time is 1 clock cycle PLUS
            // the number of waitstates" — sub-32-bit totals carry the +1
            // base (N16=5/S16=3 at WS0 defaults).
            first + 1
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

    /// Open-bus 32-bit value (CPU path): the prefetch window, combined by
    /// execute-region in Thumb mode with address-lane selection below
    /// (validated by nba_dma_latch BUS LATCH 0x46C046C0).
    /// DMA never interleaves here — the CPU stalls while DMA owns the bus,
    /// and HLE bulk copies are CPU-side accesses.
    fn open_bus32(&self) -> u32 {
        let [p0, p1] = self.prefetch_win;
        if !self.prefetch_thumb {
            return p1;
        }
        match self.current_pc >> 24 {
            0x00 | 0x06 => (p1 << 16) | (p0 & 0xFFFF),
            0x03 => {
                if self.current_pc & 2 != 0 {
                    (p1 << 16) | (p0 & 0xFFFF)
                } else {
                    p1 | ((p0 & 0xFFFF) << 16)
                }
            }
            _ => p1 | (p1 << 16),
        }
    }

    /// Lane-selected open-bus reads (shift by address lane).
    fn open_bus8(&self, addr: u32) -> u32 {
        (self.open_bus32() >> ((addr & 3) * 8)) & 0xFF
    }

    fn open_bus16(&self, addr: u32) -> u32 {
        (self.open_bus32() >> ((addr & 2) * 8)) & 0xFFFF
    }

    fn read_mapped(&mut self, addr: u32, width: u8) -> u32 {
        // GBATEK Backup Media / EEPROM: on EEPROM cartridges the 0D window
        // is the serial chip, not ROM (CPU loads see the chip state);
        // loads see the chip state); plain ROMs mirror WS2 here.
        match addr {
            0x00000000..=0x00003FFF => self.read_bios_guarded(addr, width),
            0x02000000..=0x02FFFFFF => self.read_ewram(addr, width),
            0x03000000..=0x03FFFFFF => self.read_iwram(addr, width),
            0x04000000..=0x040003FE => self.read_io(addr, width),
            a if is_mem_control(a) => self.read_io(addr, width),
            0x05000000..=0x05FFFFFF => self.read_palette(addr, width),
            0x06000000..=0x06FFFFFF => self.read_vram(addr, width),
            0x07000000..=0x07FFFFFF => {
                // OAM loads drive the whole word onto the shared bus
                // (HW-pinned by DMA_OAM_Bus: concurrent ldrh exposes the
                // full OAM word to a racing DMA).
                let word = self.read_oam(addr & !3, 4);
                self.cpu_bus = word;
                self.read_oam(addr, width)
            }
            0x08000000..=0x0CFFFFFF => self.read_rom(addr, width),
            0x0D000000..=0x0DFFFFFF => {
                // On EEPROM cartridges the 0D window is the serial chip, not
                // ROM; idle drives 1. Plain ROMs mirror WS2 here.
                if self.is_eeprom() {
                    let bit = self.cartridge.as_ref().is_none_or(|c| c.eeprom_peek_bit());
                    u32::from(bit)
                } else {
                    self.read_rom(addr, width)
                }
            }
            0x0E000000..=0x0FFFFFFF => self.read_sram(addr, width),
            // Unmapped IO-block and beyond (mem_control mirrors are
            // handled above): 32-bit CPU reads see the shared bus latch
            // (HW-pinned by DMA_CPU_Bus_Interaction: power-on 0xFF survives
            // untouched by CPU setup traffic); sub-32-bit reads see the
            // prefetch window (HW-pinned by mgba_suite io_read INVALID
            // 100C: ldrh returns the 0xDEAD literal prefetched after).
            0x04000800..=0x04FFFFFF => {
                if width == 4 {
                    self.cpu_bus
                } else if width == 2 {
                    self.open_bus16(addr)
                } else {
                    self.open_bus8(addr)
                }
            }
            // Unmapped: prefetch-latch open bus, lane-selected by width.
            _ => match width {
                4 => self.open_bus32(),
                2 => self.open_bus16(addr),
                _ => self.open_bus8(addr),
            },
        }
    }

    fn read_bios_guarded(&mut self, addr: u32, width: u8) -> u32 {
        if self.bios_protect && !(0x00000000..=0x00003FFF).contains(&self.current_pc) {
            // A protected read returns the latched last BIOS-fetched opcode,
            // same value on repeats; refreshed when BIOS execution is left.
            let raw = self.bios_prefetch;
            let aligned = match width {
                4 => raw,
                2 => raw & 0xFFFF,
                _ => raw & 0xFF,
            };
            let _ = addr;
            aligned
        } else {
            self.read_bios(addr, width)
        }
    }

    /// Latch a new BIOS prefetch value (HLE synthesis of the BIOS
    /// region-leave update).
    pub fn set_bios_prefetch(&mut self, value: u32) {
        self.bios_prefetch = value;
        self.last_prefetch = value;
        self.prefetch_win = [value, value];
        // BIOS entry/exit synthesis is ARM code.
        self.prefetch_thumb = false;
    }

    /// Reset fetch/data stream tracking on a CPU jump (branch taken, IRQ
    /// entry): the next access is non-sequential. DMA completion is not a
    /// jump and never calls this.
    pub fn invalidate_prefetch_for_dma(&mut self, _dma_addr: u32) {
        self.prev_addr = None;
        self.prev_width = 0;
        self.fetch_addr = None;
        self.fetch_width = 0;
        self.pf_valid = false;
        self.pf_branch_drain = false;
        self.last_prefetched_pc = 0;
        self.data_sequential_override = false;
    }

    /// Branch/PC-write invalidate: address streams reset (next data/fetch
    /// is N by address), but the prefetch buffer window survives — a
    /// branch into buffered addresses costs S (HW-pinned by the alyosha
    /// prefetcher branch suite). DMA/IRQ keep the full invalidate above.
    pub fn invalidate_prefetch_for_branch(&mut self) {
        self.prev_addr = None;
        self.prev_width = 0;
        self.fetch_addr = None;
        self.fetch_width = 0;
        self.pf_branch_drain = self.pf_valid;
        self.last_prefetched_pc = 0;
        self.data_sequential_override = false;
    }

    fn read_internal(&mut self, addr: u32, width: u8, is_opcode: bool) -> (u32, u8) {
        // Test-ROM log sink (only with the `mgba-debug-log` feature; see
        // field docs). Without the feature these addresses fall through
        // to plain open bus below.
        #[cfg(feature = "mgba-debug-log")]
        if !is_opcode && (0x04FFF600..=0x04FFF7FF).contains(&addr) {
            let raw = self.read_mgba_debug(addr, width);
            return (raw, 0);
        }
        // GamePak prefetch buffer: ROM opcode fetches slide/check the
        // window when prefetch is enabled (data/DMA never touch it here;
        // DMA completion and invalidate paths reset validity below).
        // With prefetch off there is no buffer (plain address stream).
        let wait = self.cycles_for_access(addr, width, is_opcode);
        if is_opcode
            && self.prefetch_enabled
            && (0x08000000..=0x0DFFFFFF).contains(&addr)
        {
            self.fetch_buffer_update(addr, width);
        } else if !is_opcode && !(0x08000000..=0x0DFFFFFF).contains(&addr) {
            // Non-ROM data accesses free the ROM bus: background refill.
            self.fetch_buffer_idle(wait);
        }
        if is_opcode {
            self.last_opcode_addr = Some(addr);
        }
        // Prefetch erase replaces the normal contribution for non-ROM
        // data (may go negative: overlapped fills); opcodes keep (wait-1).
        let mut contrib = u32::from(wait.saturating_sub(1)) as i32;
        if !is_opcode {
            if let Some(batch_delta) = self.batch_word_delta(addr, wait) {
                contrib += batch_delta;
            } else {
                contrib += self.prefetch_erase_delta(addr, u32::from(wait), true);
            }
        }
        // Fill-collision arbitration: a ROM-bus data access issued while
        // a half-word fill finishes costs one cycle; open-bus reads free
        // a single ROM-bus slot for the fill clock.
        if !is_opcode && (0x08000000..=0x0DFFFFFF).contains(&addr) {
            contrib += self.fill_collision() as i32;
        } else if !is_opcode && Self::fill_open_read(addr) {
            self.fill_advance(1);
        }
        self.access_wait_cycles += i64::from(contrib);
        let raw = self.read_mapped(addr, width);
        // prev_* tracks the last bus access of ANY kind (GBATEK N/S bus
        // order); fetch_* tracks the opcode stream for the prefetch-ON
        // fetch path above.
        self.prev_addr = Some(addr);
        self.prev_width = width;
        if is_opcode {
            self.fetch_addr = Some(addr);
            self.fetch_width = width;
            // Opcode fetches slide the prefetch window (data accesses and
            // stores leave the bus latch alone, GBATEK "Unpredictable").
            self.prefetch_win = [self.prefetch_win[1], raw];
            self.prefetch_thumb = width == 2;
            self.last_prefetch = raw;
        }
        (raw, wait)
    }

    /// Fetch-stream-break charge: a CPU data access breaks the fetch stream,
    /// so the next fetch costs N instead of S. Pre-paid once per load/store
    /// instruction as N32-S32 of the owning code region.
    pub(crate) fn charge_fetch_stream_break(&mut self) {
        if let Some(pc) = self.last_opcode_addr
            && (0x08000000..=0x0DFFFFFF).contains(&pc)
        {
            let n = self.gamepak_rom_cycles(pc, 4, false);
            let s = self.gamepak_rom_cycles(pc, 4, true);
            self.access_wait_cycles += i64::from(n.saturating_sub(s));
        }
    }

    /// S-fast GamePak fetch-stream gate for the single-load charges
    /// below: code in ROM with the prefetcher on and sequential 16-bit
    /// fetches at minimum wait.
    fn single_load_gate(&self) -> bool {
        if !self.prefetch_enabled {
            return false;
        }
        match self.last_opcode_addr {
            Some(pc @ 0x08000000..=0x0DFFFFFF) => self.gamepak_rom_cycles(pc, 2, true) == 2,
            _ => false,
        }
    }

    /// Thumb single word-load retire (literals included): a stack-data
    /// (non-ROM) load leaves the S-fast prefetch stream one fetch short
    /// when a word load retired within the last three instructions and
    /// the already fetched next opcode is bus-busy. A ROM-data load owes
    /// one more only when directly following a stack-data load. Stale
    /// markers self-invalidate via execute-PC distance.
    pub(crate) fn note_thumb_single_load(&mut self, regs_pc: u32, data_addr: u32) {
        let exec = regs_pc.wrapping_sub(4);
        let load_adj = self
            .prev_load_pc
            .is_some_and(|pc| pc.wrapping_add(2) == exec);
        let was_stack = load_adj && self.prev_load_is_stack;
        let in_window = self
            .prev_load_pc
            .is_some_and(|pc| exec.wrapping_sub(pc) <= 6);
        let stack = !(0x08000000..=0x0DFFFFFF).contains(&data_addr);
        self.prev_load_pc = None;
        self.prev_load_is_stack = false;
        if !self.single_load_gate() {
            return;
        }
        self.prev_load_pc = Some(exec);
        self.prev_load_is_stack = stack;
        if !stack {
            if was_stack {
                self.access_wait_cycles += 1;
            }
            return;
        }
        if !in_window {
            return;
        }
        if !thumb_next_is_datamover((self.prefetch_win[0] & 0xFFFF) as u16) {
            return;
        }
        self.access_wait_cycles += 1;
    }

    /// Mark block-transfer continuation words (LDM/STM/PUSH/POP after the
    /// first) sequential per bus order (first-N plus contiguity, including
    /// the 128KB N-force). The loop must reset to false when done.
    pub(crate) fn set_data_sequential(&mut self, sequential: bool) {
        self.data_sequential_override = sequential;
    }

    /// Open a block-transfer erase batch (call once per LDM/STM/PUSH/POP
    /// instruction around the word loop). `is_load` selects the
    /// GBALoad (+2) / GBAStore (+1) word convention; `fetch_width` (4 for
    /// ARM, 2 for Thumb) selects the erase-floor N.
    pub(crate) fn begin_block_batch(&mut self, is_load: bool, fetch_width: u8) {
        self.block_batching = true;
        self.block_batch_any = false;
        self.block_batch_words = 0;
        self.block_batch_erase_sum = 0;
        self.block_batch_is_load = is_load;
        self.block_batch_fetch_width = fetch_width;
        self.block_batch_has_rom = false;
        self.block_batch_raw = false;
    }

    /// Raw bulk batch (HLE loops): words accrue `(wait-1)` with no erase,
    /// regardless of code region (HW BIOS sees flat waits).
    pub(crate) fn begin_raw_batch(&mut self) {
        self.block_batching = true;
        self.block_batch_any = false;
        self.block_batch_words = 0;
        self.block_batch_erase_sum = 0;
        self.block_batch_has_rom = false;
        self.block_batch_raw = true;
    }

    /// Close the batch: undo all tracked erases when a ROM word appeared
    /// (mixed ROM-overflow bursts apply no prefetch erase at all), else
    /// floor the instruction's total erase benefit at one N-fetch worth
    /// (ROM code only). No-op when nothing batched.
    pub(crate) fn end_block_batch(&mut self) {
        self.block_batching = false;
        if !self.block_batch_any {
            return;
        }
        if self.block_batch_has_rom {
            self.access_wait_cycles -= i64::from(self.block_batch_erase_sum);
            return;
        }
        if let Some(pc @ 0x08000000..=0x0DFFFFFF) = self.last_opcode_addr {
            let n_cap =
                u32::from(self.gamepak_rom_cycles(pc, self.block_batch_fetch_width, false)) - 1;
            let floor = -(n_cap as i32);
            if self.block_batch_erase_sum < floor {
                self.access_wait_cycles += i64::from(floor - self.block_batch_erase_sum);
            }
        }
    }

    /// Batched-word erase routing: returns the erase-delta for this word, or
    /// `None` for the normal per-word path. The caller still adds `(wait-1)`.
    fn batch_word_delta(&mut self, addr: u32, wait: u8) -> Option<i32> {
        if !self.block_batching || !self.prefetch_enabled {
            return None;
        }
        if addr >= 0x08000000 {
            // ROM word inside the batch (OAM-overflow LDM): flag mixed
            // burst (end undoes all erases); the word itself uses the
            // normal N/S path with no erase.
            self.block_batch_has_rom = true;
            return None;
        }
        // Raw bulk (HLE): no erase at all, any code region.
        if self.block_batch_raw {
            return Some(0);
        }
        if !matches!(self.last_opcode_addr, Some(0x08000000..=0x0DFFFFFF)) {
            return None;
        }
        let region = u32::from(wait.saturating_sub(1)) as i32;
        // Single-access conventions: GBALoad region+2, GBAStore region+1.
        let single_input = region + if self.block_batch_is_load { 2 } else { 1 };
        let delta = if self.block_batch_words == 0 {
            // Word 1: full single-access stall (updates lastPrefetchedPc).
            self.prefetch_stall_erased(single_input) - single_input
        } else {
            // Continuation words: marginal `min(-1, S - single_input)`;
            // no lastPrefetchedPc update (word 1's fills persist).
            let (_, s16_our) = self.code_wait16();
            let s = s16_our as i32 - 1;
            (s - single_input).min(-1)
        };
        self.block_batch_words += 1;
        self.block_batch_any = true;
        self.block_batch_erase_sum += delta;
        Some(delta)
    }

    fn write_internal(&mut self, addr: u32, width: u8, value: u32, bios: bool) {
        // Test-ROM log sink (only with the `mgba-debug-log` feature; see
        // field docs). Without the feature these addresses fall through
        // to the open-bus default below.
        #[cfg(feature = "mgba-debug-log")]
        if (0x04FFF600..=0x04FFF7FF).contains(&addr) {
            self.write_mgba_debug(addr, width, value);
            self.prev_addr = Some(addr);
            self.prev_width = width;
            return;
        }
        // GBATEK Backup Media / EEPROM: CPU stores to 0D000000h are open bus;
        // only DMA bursts reach the serial chip (handled in the tick loop).
        let wait = self.cycles_for(addr, width);
        // Stores erase like loads (same stall, +1 base convention).
        let mut contrib = u32::from(wait.saturating_sub(1)) as i32;
        if let Some(batch_delta) = self.batch_word_delta(addr, wait) {
            contrib += batch_delta;
        } else {
            contrib += self.prefetch_erase_delta(addr, u32::from(wait), false);
        }
        // ROM stores collide with in-flight fills like ROM loads do.
        if (0x08000000..=0x0DFFFFFF).contains(&addr) {
            contrib += self.fill_collision() as i32;
        }
        self.access_wait_cycles += i64::from(contrib);
        if !(0x08000000..=0x0DFFFFFF).contains(&addr) {
            // Non-ROM stores free the ROM bus: background refill.
            self.fetch_buffer_idle(wait);
        }
        match addr {
            0x02000000..=0x02FFFFFF => self.write_ewram(addr, width, value),
            0x03000000..=0x03FFFFFF => self.write_iwram(addr, width, value),
            0x04000000..=0x040003FE => self.write_io(addr, width, value, bios),
            a if is_mem_control(a) => self.write_io(addr, width, value, bios),
            0x05000000..=0x05FFFFFF => self.write_palette(addr, width, value),
            0x06000000..=0x06FFFFFF => self.write_vram(addr, width, value),
            0x07000000..=0x07FFFFFF => self.write_oam(addr, width, value),
            0x0E000000..=0x0FFFFFFF => self.write_sram(addr, width, value),
            _ => {
                // Attached GPIO registers overlay ROM (see read_rom).
                if let Some(cart) = self.cartridge.as_mut()
                    && cart.gpio.write(addr, width, value)
                {
                                self.prev_addr = Some(addr);
                    self.prev_width = width;
                    return;
                }
                    }
        }
        self.prev_addr = Some(addr);
        self.prev_width = width;
    }

    // -- Region readers --

    /// Drain committed test-ROM debug-log lines (oldest first). Only
    /// exists with the `mgba-debug-log` cargo feature (test harness).
    #[cfg(feature = "mgba-debug-log")]
    pub fn drain_mgba_debug_logs(&mut self) -> Vec<MgbaDebugLog> {
        std::mem::take(&mut self.mgba_debug_logs)
    }

    /// Whether the guest enabled the test-ROM log sink. Only exists
    /// with the `mgba-debug-log` cargo feature (test harness).
    #[cfg(feature = "mgba-debug-log")]
    pub fn mgba_debug_enabled(&self) -> bool {
        self.mgba_debug_enable
    }

    /// Test-ROM log-sink write (suite log protocol): `strncpy` into the
    /// string buffer, commit on a flags write with bit 8 set, enable on `0xC0DE` (`mgba_close`
    /// writes 0). Byte-granular so any store width works. Only compiled
    /// with the `mgba-debug-log` cargo feature (test harness).
    #[cfg(feature = "mgba-debug-log")]
    fn write_mgba_debug(&mut self, addr: u32, width: u8, value: u32) {
        match addr {
            0x04FFF600..=0x04FFF6FF => {
                let bytes = value.to_le_bytes();
                for (i, byte) in bytes.iter().enumerate().take(usize::from(width)) {
                    let off = (addr as usize).wrapping_sub(0x4FFF600) + i;
                    if off < self.mgba_debug_buf.len() {
                        self.mgba_debug_buf[off] = *byte;
                    }
                }
            }
            0x04FFF700 => {
                if width >= 2 && value & 0x100 != 0 {
                    let len = self
                        .mgba_debug_buf
                        .iter()
                        .position(|&b| b == 0)
                        .unwrap_or(self.mgba_debug_buf.len());
                    let text = String::from_utf8_lossy(&self.mgba_debug_buf[..len]).into_owned();
                    self.mgba_debug_logs.push(MgbaDebugLog {
                        level: (value & 7) as u8,
                        text,
                    });
                    self.mgba_debug_buf = [0; 256];
                }
            }
            0x04FFF780 if width >= 2 => {
                self.mgba_debug_enable = match (value & 0xFFFF) as u16 {
                    0xC0DE => true,
                    0 => false,
                    _ => self.mgba_debug_enable,
                };
            }
            _ => {}
        }
    }

    /// Test-ROM log-sink read: buffer bytes, or `0x1DEA` from the
    /// enable register while enabled (handshake); disabled reads fall
    /// through to open bus (prior behavior). Only compiled with the
    /// `mgba-debug-log` cargo feature (test harness).
    #[cfg(feature = "mgba-debug-log")]
    fn read_mgba_debug(&mut self, addr: u32, width: u8) -> u32 {
        match addr {
            0x04FFF600..=0x04FFF6FF => {
                let off = (addr as usize).wrapping_sub(0x4FFF600);
                let mut bytes = [0u8; 4];
                for (i, byte) in bytes.iter_mut().enumerate().take(usize::from(width)) {
                    if off + i < self.mgba_debug_buf.len() {
                        *byte = self.mgba_debug_buf[off + i];
                    }
                }
                u32::from_le_bytes(bytes)
            }
            0x04FFF780 => {
                if self.mgba_debug_enable {
                    0x1DEA
                } else {
                    match width {
                        4 => self.open_bus32(),
                        2 => self.open_bus16(addr),
                        _ => self.open_bus8(addr),
                    }
                }
            }
            _ => match width {
                4 => self.open_bus32(),
                2 => self.open_bus16(addr),
                _ => self.open_bus8(addr),
            },
        }
    }

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
            // Attached GPIO registers overlay ROM (GBATEK cartridge GPIO).
            if let Some(value) = cart.gpio.read(addr, width) {
                return value;
            }
            return cart.read_rom(addr, width);
        }
        // No cartridge: prefetch-latch open bus, lane-selected by width.
        match width {
            4 => self.open_bus32(),
            2 => self.open_bus16(addr),
            _ => self.open_bus8(addr),
        }
    }

    fn read_sram(&self, addr: u32, width: u8) -> u32 {
        // GBATEK backup detection order starts with an SRAM probe: on
        // EEPROM carts there is no 0E window, so CPU reads see open bus
        // (only the DMA3 serial protocol reaches the chip).
        if self.is_eeprom() {
            return match width {
                4 => self.open_bus32(),
                2 => self.open_bus16(addr),
                _ => self.open_bus8(addr),
            };
        }
        if let Some(cart) = &self.cartridge {
            return cart.read_sram(addr, width);
        }
        // No cartridge: no backup chip answers, so the bus floats high
        // (0xFF, not stored-byte echo).
        repeat_byte(0xFF, width)
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

    /// Feed EEPROM serial bits for a DMA write burst to 0D000000h.
    /// Each 16-bit unit carries one bit (GBATEK). Wider units are
    /// undefined on hardware; only bit 0 is consumed.
    fn feed_eeprom_write(&mut self, width: u8, value: u32) {
        if let Some(cart) = self.cartridge.as_mut() {
            cart.eeprom_write_bit(value & 1 != 0);
            let _ = width;
            self.eeprom_burst_open = true;
        }
    }

    /// Pop one EEPROM response bit for a DMA read from 0D000000h.
    fn next_eeprom_read_bit(&mut self) -> u32 {
        if let Some(cart) = self.cartridge.as_mut() {
            u32::from(cart.eeprom_read_bit())
        } else {
            1
        }
    }

    fn read_io(&mut self, addr: u32, width: u8) -> u32 {
        self.timers.set_current_cycle(self.current_tcycle);
        // 4000800h Internal Memory Control, mirrored each 64K.
        if is_mem_control(addr) {
            let shift = (addr & 3) * 8;
            return (self.mem_control >> shift) & (u32::MAX >> (8 * (4 - u32::from(width))));
        }
        if width == 1 && (0x04000100..=0x0400010D).contains(&addr) {
            return self.timers.read8(addr).unwrap_or(0) as u32;
        }
        if width == 4 {
            return self.read_io(addr, 2) | (self.read_io(addr + 2, 2) << 16);
        }
        if width == 1 {
            match addr {
                0x04000300 => return self.postflg as u32,
                // HALTCNT is write-only (GBATEK System Control): reads see
                // open bus, never the latch.
                0x04000301 => return self.open_bus8(addr),
                _ => {}
            }
        }
        let aligned = addr & !1;
        let val: u16 = match aligned {
            0x04000000..=0x04000006 | 0x04000008..=0x0400000E | 0x04000048..=0x04000052 => {
                match self.ppu.read_register(aligned) {
                    Some(v) => v,
                    None => {
                        // Write-only register (MOSAIC, BLDY, HOFS, affine, WINH/V, ...)
                        return self.open_bus16(aligned);
                    }
                }
            }
            0x040000B0..=0x040000DE => {
                // SAD/DAD are write-only (reads see open bus); CNT_L reads
                // back 0, CNT_H reads the latched control.
                if matches!(aligned, 0x040000BA | 0x040000C6 | 0x040000D2 | 0x040000DE) {
                    self.dma.read(aligned).unwrap_or(0)
                } else if matches!(aligned, 0x040000B8 | 0x040000C4 | 0x040000D0 | 0x040000DC) {
                    0
                } else {
                    self.open_bus16(aligned) as u16
                }
            }
            0x04000100..=0x0400010E => {
                if std::env::var("GBA_TTRACE").is_ok() && aligned == 0x04000100 {
                    eprintln!("T tmread @{}", self.current_tcycle);
                }
                self.timers.read(aligned).unwrap_or(0)
            }
            0x04000060 => self.apu.sound1cnt_lo,
            0x04000062 => self.apu.sound1cnt_hi,
            0x04000064 => self.apu.sound1cnt_x,
            0x04000068 => self.apu.sound2cnt_lo,
            0x0400006C => self.apu.sound2cnt_hi,
            0x04000070 => self.apu.sound3cnt_lo,
            0x04000072 => self.apu.sound3cnt_hi,
            0x04000074 => self.apu.sound3cnt_x,
            0x04000078 => self.apu.sound4cnt_lo,
            0x0400007C => self.apu.sound4cnt_hi,
            0x04000080 => self.apu.soundcnt_lo,
            0x04000082 => self.apu.soundcnt_hi,
            0x04000084 => self.apu.soundcnt_x_read(),
            0x04000088 => self.apu.soundbias,
            0x04000090..=0x0400009E => self.apu.wave_read(aligned),
            // FIFO_A/B (A0/A4) are write-only; reads return open bus.
            // UART SIOCNT: bits 4-6 are live status (send-full,
            // receive-empty, error); the error latch clears on read.
            0x04000128 => {
                if self.sio_block_selected() && Self::sio_submode(self.siocnt) == 3 {
                    let out = (self.siocnt & !0x0070) | (u16::from(self.uart_irqsrc()) << 4);
                    self.uart_err = false;
                    self.uart_prev_irqsrc = self.uart_irqsrc();
                    out
                } else {
                    self.siocnt
                }
            }
            // 0x12A is SIOMLT_SEND (multi), SIODATA8 (normal-8/32) or a
            // plain latch (GPIO/Joybus); in UART mode it addresses the
            // FIFOs — reads pop receive bytes (0 when empty, suite table).
            0x0400012A => {
                if self.sio_block_selected() && Self::sio_submode(self.siocnt) == 3 {
                    let byte = self.uart_rx.pop_front().unwrap_or(0) as u16;
                    // Draining to empty raises the receive flag (IRQ edge).
                    self.uart_eval_irq();
                    byte
                } else {
                    self.siodata8
                }
            }
            // 0x120/0x122 latch only in Normal-32 SIO mode (send data);
            // every other mode reads the idle receive path as 0.
            0x04000120 => {
                if self.sio_block_selected() && Self::sio_submode(self.siocnt) == 1 {
                    (self.siodata32 & 0xFFFF) as u16
                } else {
                    0
                }
            }
            0x04000122 => {
                if self.sio_block_selected() && Self::sio_submode(self.siocnt) == 1 {
                    ((self.siodata32 >> 16) & 0xFFFF) as u16
                } else {
                    0
                }
            }
            // SIOMULTI2/3 (and SIOMULTI0/1 outside Normal-32) are receive
            // registers: 0 with no transfer (suite table; writes ignored).
            0x04000124 | 0x04000126 => 0,
            0x04000130 => self.keyinput,
            0x04000132 => self.keycnt,
            0x04000134 => self.read_rcnt(),
            0x04000140 => self.joycnt,
            // JOY_RECV/TRANS read 0 with no link transfer (suite table);
            // JOYSTAT has no status source yet either (0x15A reads 0).
            0x04000150 | 0x04000152 | 0x04000154 | 0x04000156 => 0,
            // JOYSTAT (0x158) has no status source with no link transfer.
            0x04000158 => 0,
            0x04000200 => self.ie,
            0x04000202 => self.sif,
            0x04000204 => self.wait_cnt,
            0x04000208 => self.ime as u16,
            0x04000300 => (self.postflg as u16) | (self.open_bus32() & 0xFF00) as u16,
            // Unused I/O reads return 0, NOT open bus (the mgba-suite
            // io-read table is the HW capture: 0x20A reads 0 too, as
            // the empty high half of the 1-bit IME register).
            0x04000066 | 0x0400006A | 0x0400006E | 0x04000076 | 0x0400007A | 0x0400007E
            | 0x04000086 | 0x0400008A | 0x04000136 | 0x04000142 | 0x0400015A | 0x04000206
            | 0x0400020A | 0x04000302 => 0,
            _ => {
                // Unimplemented/write-only registers return the recently
                // prefetched opcode, not the last written value (GBATEK
                // "Unpredictable Things": open bus tracks the prefetch,
                // with a zero-mix rule for partially-readable ports that
                // is not modeled here).
                return self.last_prefetch & 0xFFFF;
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
    }

    fn write_iwram(&mut self, addr: u32, width: u8, value: u32) {
        let off = Self::aligned_off(addr, width, 0x7FFF);
        write_slice(&mut *self.iwram, off, width, value);
    }

    fn write_palette(&mut self, addr: u32, width: u8, value: u32) {
        let off = Self::aligned_off(addr, width, 0x3FF);
        if width == 1 {
            let aligned = off & !1;
            write_slice(&mut *self.palette_ram, aligned, 2, (value & 0xFF) * 0x0101);
        } else {
            write_slice(&mut *self.palette_ram, off, width, value);
        }
    }

    fn write_vram(&mut self, addr: u32, width: u8, value: u32) {
        let Some(off) = self.vram_offset(addr, width) else {
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
    }

    fn write_oam(&mut self, addr: u32, width: u8, value: u32) {
        let off = Self::aligned_off(addr, width, 0x3FF);
        if width != 1 {
            write_slice(&mut *self.oam, off, width, value);
        }
    }

    fn write_sram(&mut self, addr: u32, width: u8, value: u32) {
        // No 0E window on EEPROM carts (see read_sram): stores go nowhere.
        if self.is_eeprom() {
                return;
        }
        if let Some(cart) = &mut self.cartridge {
            cart.write_sram(addr, width, value);
        } else {
            // No cartridge: one byte lands on the addressed lane
            // (GBATEK LSB of ROR).
            let off = (addr & 0xFFFF) as usize;
            self.fallback_sram[off] = selected_write_byte(addr, width, value);
        }
    }

    /// SIOCNT write with per-sub-mode R/W maps. Unreadable bits never
    /// persist; the suite HW table is the authority on R/W bits.
    fn write_siocnt(&mut self, v: u16) {
        // Bit 15 is always 0 (GBATEK: "Not used (Read only, always 0)").
        let mut value = v & 0x7FFF;
        let old_fifo_en = self.siocnt & 0x0100 != 0;
        let sub = Self::sio_submode(value);
        let sio_block = self.sio_block_selected();
        match sub {
            2 if sio_block => {
                // Multiplayer: Slave/Ready/ID/Error are read-only. A solo
                // master is the parent waiting for children that never
                // answer, so START never completes (mgba-suite sio-timing
                // Multi/* cells pin timedOut=true on hardware; other
                // emulators complete solo transfers, but the HW suite
                // rules here).
                // Slaves never start.
                value &= 0xFF83;
                value |= 0x0004;
                value &= !0x0030;
                value |= self.siocnt & 0x00FC;
                value |= 0x0008;
            }
            3 if sio_block => {
                // UART SCCNT_L (GBATEK): bits 4-6 are RO status composed
                // at read time; bit 7 is the R/W data length (NOT start);
                // error/send-full never persist from the written value.
                value &= !0x0070;
            }
            _ => {
                // Normal-8/32, and GPIO/Joybus (SIOCNT keeps its
                // sub-format there): bits 4-6 always read 0 (GBATEK
                // "Not used"), bits 8-11 are R/W latches, and SI floats
                // high with no link partner (SI floats high).
                value &= !0x8070;
                value |= 0x0004;
            }
        }
        let started = value & 0x0080 != 0 && self.siocnt & 0x0080 == 0;
        self.siocnt = value;
        // Normal-mode transfer (no partner): 8/32 bits at 256KHz
        // (64 T-cycles/bit) or 2MHz (8 T-cycles/bit).
        if started && sio_block && (sub == 0 || sub == 1) {
            let bit: u32 = if value & 0x0002 != 0 { 8 } else { 64 };
            self.sio_xfer_32 = sub == 1;
            self.sio_xfer_uart = false;
            self.sio_xfer_cycles = (if sub == 1 { 32 } else { 8 }) * bit;
        }
        if sub == 3 && sio_block {
            // FIFO content resets when FIFO is disabled (GBATEK).
            if old_fifo_en && self.siocnt & 0x0100 == 0 {
                self.uart_tx.clear();
                self.uart_rx.clear();
                self.uart_eval_irq();
            }
            self.kick_uart();
        }
    }

    /// UART FIFO depth (4 with enable, single unit without).
    fn uart_fifo_cap(&self) -> usize {
        if self.siocnt & 0x0100 != 0 { 4 } else { 1 }
    }

    /// UART IRQ-source levels {send_full, recv_empty, err}.
    fn uart_irqsrc(&self) -> u8 {
        let cap = self.uart_fifo_cap();
        (u8::from(self.uart_tx.len() >= cap))
            | (u8::from(self.uart_rx.is_empty()) << 1)
            | (u8::from(self.uart_err) << 2)
    }

    /// UART IRQ on rising source edges (GBATEK bit 14: IRQ when any of
    /// bits 4/5/6 become set).
    fn uart_eval_irq(&mut self) {
        let now = self.uart_irqsrc();
        if self.siocnt & 0x4000 != 0 && (now & !self.uart_prev_irqsrc) != 0 {
            self.request_interrupt(1 << 7);
        }
        self.uart_prev_irqsrc = now;
    }

    /// Start a UART frame when send is enabled, data waits and the wire is
    /// idle. CTS set blocks (SC floats high with no peer); blind senders
    /// transmit as soon as Send Enable is set (GBATEK).
    fn kick_uart(&mut self) {
        if self.sio_xfer_cycles != 0
            || !self.sio_block_selected()
            || Self::sio_submode(self.siocnt) != 3
            || self.siocnt & 0x0400 == 0
            || self.uart_tx.is_empty()
            || self.siocnt & 0x0004 != 0
        {
            return;
        }
        const BIT_CYCLES: [u32; 4] = [1748, 437, 291, 146];
        let baud = (self.siocnt & 3) as usize;
        let data_bits = if self.siocnt & 0x0080 != 0 { 8 } else { 7 };
        let parity = if self.siocnt & 0x0200 != 0 { 1 } else { 0 };
        self.sio_xfer_cycles = (1 + data_bits + parity + 1) * BIT_CYCLES[baud];
        self.sio_xfer_uart = true;
    }

    /// RCNT write latch (mask `0xC1FF` + per-mode maps, suite-pinned).
    /// Data pins {0-3} are live state composed at read time; only the
    /// direction/control bits {4-8} latch here.
    fn write_rcnt(&mut self, v: u16) {
        let value = v & 0xC1FF;
        self.rcnt = match value & 0xC000 {
            // GPIO: full {0-8} latch.
            0x8000 => value,
            // Joybus: {2-8} latch; SC/SD read 0.
            0xC000 => (value & 0xC1FC) | 0xC000,
            // SIO: {4-8} latch; data pins are forced at read.
            _ => value & 0xC1F0,
        };
    }

    /// RCNT read: latched {4-8,14-15} plus live data pins. With no link
    /// partner the pins float high in Multi/UART mode, while Normal mode
    /// shows SC+SI high (suite HW table: M/U 0xF, N8/N32 0x5).
    fn read_rcnt(&self) -> u16 {
        let mut value = self.rcnt;
        if self.sio_block_selected() {
            let sub = Self::sio_submode(self.siocnt);
            let data = if sub == 0 || sub == 1 { 0x0005 } else { 0x000F };
            value = (value & !0x000F) | data;
        }
        value
    }

    fn write_io(&mut self, addr: u32, width: u8, value: u32, bios: bool) {
        self.timers.set_current_cycle(self.current_tcycle);
        // 4000800h Internal Memory Control, mirrored each 64K. Only the
        // documented bits are stored (0-3, 5, 24-31); sub-word writes merge
        // lanes. Remap/wait effects are not modeled.
        if is_mem_control(addr) {
            let shift = (addr & 3) * 8;
            let mask = (u32::MAX >> (8 * (4 - u32::from(width)))) << shift;
            self.mem_control = (self.mem_control & !mask) | ((value << shift) & mask & 0xFF00_002F);
                return;
        }
        if width == 4 && self.timers.write32(addr, value) {
            return;
        }
        if width > 1 && addr == 0x04000300 {
            // POSTFLG/HALTCNT are BIOS-gated (confirmed by hw-test ROM
            // haltcnt): CPU writes from outside the BIOS are ignored;
            // HLE BIOS and DMA writes act. POSTFLG is set-only; HALTCNT bit
            // 7 = 0 halts, bit 7 = 1 stops (CPU parked until IRQ in both).
            let gated = bios || self.current_pc <= 0x3FFF;
            if gated {
                self.postflg |= (value & 1) as u8;
            }
            self.haltcnt = (value >> 8) as u8;
            if gated {
                if value & 0x8000 == 0 {
                    self.enter_halt(self.ie);
                } else {
                    // Direct Stop uses the same restricted wake mask as SWI
                    // Stop (keypad/GamePak/serial only): timers, DMA and
                    // video are paused in Stop mode and cannot wake it.
                    self.enter_stop();
                }
            }
                return;
        }
        if width == 4 {
            self.write_io(addr, 2, value & 0xFFFF, bios);
            self.write_io(addr + 2, 2, value >> 16, bios);
            return;
        }
        if width == 1 {
            match addr {
                0x04000300 => {
                    let gated = bios || self.current_pc <= 0x3FFF;
                    if gated {
                        self.postflg |= (value & 1) as u8;
                    }
                                return;
                }
                0x04000301 => {
                    let gated = bios || self.current_pc <= 0x3FFF;
                    self.haltcnt = value as u8;
                    if gated {
                        if value & 0x80 == 0 {
                            self.enter_halt(self.ie);
                        } else {
                            // Same restricted wake mask as SWI Stop (see above).
                            self.enter_stop();
                        }
                    }
                                return;
                }
                _ => {}
            }
        }
        let aligned = addr & !1;
        // The I/O bus is 16-bit: a sub-word store must preserve the
        // untouched lane instead of zeroing it.
        let v16 = if width == 1 {
            let shift = (addr & 1) * 8;
            let lane = (value & 0xFF) << shift;
            let cur = if (0x040000B0..=0x040000DE).contains(&aligned) {
                u32::from(self.dma.read(aligned).unwrap_or(0))
            } else {
                self.read_io(aligned, 2)
            };
            ((cur & !(0xFF << shift)) | lane) as u16
        } else {
            value as u16
        };
        match aligned {
            0x04000000..=0x04000054 if aligned != 0x04000006 => {
                let irq = self.ppu.write_register(aligned, v16);
                if irq != 0 {
                    self.request_interrupt(irq);
                }
            }
            0x040000B0..=0x040000DE => {
                if std::env::var("GBA_TTRACE").is_ok()
                    && matches!(aligned, 0x040000BA | 0x040000C6 | 0x040000D2 | 0x040000DE)
                    && v16 & 0x8000 != 0
                {
                    eprintln!(
                        "T dmaen ch{} @{}",
                        (aligned - 0x040000B0) / 12,
                        self.current_tcycle
                    );
                }
                self.dma.write(aligned, v16);
                // Immediate CNT_H arming with prefetch on starts one tick
                // sooner (pending 4->3): prefetch overlaps the enabling
                // bus cycle, so short P-ON triggers still park before the
                // next CPU step (mgba-suite Timing Thumb P.. race: without
                // the retime those cells read 3 instead of 7/11/37).
                // P-OFF, event triggers, and hw-test pins keep pending=4
                // (uniform 3 was tried: start-delay reads 19, not 20).
                if self.prefetch_enabled
                    && matches!(aligned, 0x040000BA | 0x040000C6 | 0x040000D2 | 0x040000DE)
                    && v16 & 0x8000 != 0
                    && (v16 >> 12) & 3 == 0
                {
                    let channel = ((aligned - 0x040000B0) / 12) as usize;
                    self.dma.retime_pending(channel, 3);
                }
                // DMA GamePak fill-collision arbitration: a DMA access
                // that touches GamePak ROM collides with an in-flight
                // prefetch fill exactly like CPU ROM data (Mesen Reset,
                // NanoBoyAdvance StopPrefetch and ares wait==1 reset all
                // agree on the check). The fill clock advances across
                // the bus-handover idle tick plus earlier non-cartridge
                // ticks this burst (GBATEK prefetch fills while the bus
                // is free; the CPU-owned pending window freezes it), and
                // the one-shot check fires at the first GamePak access
                // (the per-unit restart below keeps later accesses from
                // re-firing). The penalty stalls the bus a real tick
                // (Mesen's Step on Reset), not an acc charge: acc would
                // be discarded by the resume instruction's expansion
                // take, and max-subsumption would hide it inside
                // in-flight remainders (both effects HW-pinned by the
                // mgba-suite Timing Thumb/ARM ROM-DMA cells).
                // DMA CNT_H commit writes no longer break the CPU fetch
                // stream. GBATEK's "STR to DMA CNT forces NSEQ" describes
                // the DMA unit's own first access (modeled via is_first),
                // not CPU opcode fetches: breaking the stream here cost +2
                // on DMA_Mode_Change (HW-pinned S-continuation, also passing
                // on mgba) with no HW pin supporting the break. SAD/DAD/
                // CNT_L setup writes never touched the stream either.
            }
            0x04000100..=0x0400010E => {
                if std::env::var("GBA_TTRACE").is_ok() && aligned == 0x04000102 && v16 & 0x80 != 0 {
                    eprintln!("T start @{}", self.current_tcycle);
                }
                self.timers.write(aligned, v16);
            }
            // 0x04000006 VCOUNT は RO
            // APU readable-bit masks are applied at write time (GBATEK
            // R/W maps): unreadable bits never persist, so reads return
            // the stored value.
            // mgba-suite io-read pins write-0xFFFF -> each mask.
            0x04000060 => self.apu.write_sound1cnt_lo(v16),
            0x04000062 => self.apu.write_sound1cnt_hi(v16),
            0x04000064 => self.apu.write_sound1cnt_x(v16),
            0x04000068 => self.apu.write_sound2cnt_lo(v16),
            0x0400006C => self.apu.write_sound2cnt_hi(v16),
            0x04000070 => self.apu.write_sound3cnt_lo(v16),
            0x04000072 => self.apu.write_sound3cnt_hi(v16),
            0x04000074 => self.apu.write_sound3cnt_x(v16),
            0x04000078 => self.apu.write_sound4cnt_lo(v16),
            0x0400007C => self.apu.write_sound4cnt_hi(v16),
            0x04000080 => self.apu.soundcnt_lo = v16 & 0xFF77,
            0x04000082 => self.apu.write_soundcnt_hi(v16),
            0x04000084 => self.apu.write_soundcnt_x(v16),
            0x04000088 => self.apu.soundbias = v16 & 0xC3FE,
            0x04000090..=0x0400009E => self.apu.wave_write(aligned, v16),
            // FIFO_A/B are write-only streaming buffers (GBATEK Sound FIFO):
            // each access appends its bytes; 32-bit writes split above into
            // two halfword pushes in LSB-first order, matching DMA bursts.
            0x040000A0 | 0x040000A2 => self.apu.push_fifo(false, value, width),
            0x040000A4 | 0x040000A6 => self.apu.push_fifo(true, value, width),
            0x04000128 => self.write_siocnt(v16),
            // 0x12A latches except in UART mode, where only the low byte
            // reaches the send FIFO (GBATEK: upper 8 bits unused).
            0x0400012A => {
                if self.sio_block_selected() && Self::sio_submode(self.siocnt) == 3 {
                    if self.uart_tx.len() < self.uart_fifo_cap() {
                        self.uart_tx.push_back((v16 & 0xFF) as u8);
                    }
                    self.uart_eval_irq();
                    self.kick_uart();
                } else {
                    self.siodata8 = v16;
                }
            }
            0x04000120 => {
                if self.sio_block_selected() && Self::sio_submode(self.siocnt) == 1 {
                    if width == 4 {
                        self.siodata32 = value;
                    } else {
                        self.siodata32 = (self.siodata32 & 0xFFFF0000) | (v16 as u32);
                    }
                }
            }
            0x04000122 => {
                if self.sio_block_selected() && Self::sio_submode(self.siocnt) == 1 {
                    self.siodata32 = (self.siodata32 & 0x0000FFFF) | ((v16 as u32) << 16);
                }
            }
            // Receive-register writes land nowhere readable.
            0x04000124 | 0x04000126 => {}
            // 0x04000130 KEYINPUT は RO
            0x04000132 => {
                self.keycnt = v16 & 0xC3FF;
                self.check_keycnt();
            }
            0x04000134 => self.write_rcnt(v16),
            // JOYCNT (all modes): only the reset bit persists from the
            // reset bit persists from the new value; old flag bits clear
            // where the new value selects them. From reset this turns a
            // 0xFFFF write into 0x0040 (suite table).
            0x04000140 => {
                self.joycnt = (v16 & 0x0040) | (self.joycnt & !(v16 & 7) & !0x0040);
            }
            // JOY_RECV/TRANS/JOYSTAT writes have no observable effect
            // with no link transfer (suite table reads all zero).
            0x04000150 | 0x04000152 | 0x04000154 | 0x04000156 | 0x04000158 => {}
            0x04000200 => {
                // Delayed: merges into the pending level,
                // applied 1 tick later; reads still return the effective IE.
                // (32-bit stores are split into halfword writes before this
                // match, so IE+IF / IF+WAITCNT pairs land in order.)
                self.pending_ie = v16 & 0x3FFF;
                self.pending_at = Some(self.current_tcycle + 1);
                self.line_write_assert = true;
                        return;
            }
            0x04000202 => {
                // IF acknowledge: only written 1-bits clear.
                // A byte store acks its lane only, not the merged halfword.
                let bits = if width == 1 {
                    ((value & 0xFF) << ((addr & 1) * 8)) as u16
                } else {
                    (value & 0xFFFF) as u16
                };
                if bits & (1 << 3) != 0 {
                    self.timers.note_timer0_ack();
                }
                // GBATEK Interrupt Request Flags are write-1-clear: the
                // clear lands on the register (and its BIOS RAM mirror)
                // with the bus write. Only the CPU line still propagates
                // delayed (nIRQ synchronizer plus the pending pipeline);
                // a same-tick ack-then-raise dips the line on HW too,
                // since the nIRQ level follows the IF register.
                self.sif &= !bits;
                self.pending_if &= !bits;
                self.iwram[0x7FF8..0x7FFA].copy_from_slice(&self.sif.to_le_bytes());
                let line_now = self.ime && (self.ie & self.sif != 0);
                let line_cur = self
                    .line_queue
                    .last()
                    .map(|(v, _)| *v)
                    .unwrap_or(self.irq_line);
                if line_now != line_cur {
                    self.line_queue.push((line_now, self.current_tcycle + 2));
                }
                self.pending_at = Some(self.current_tcycle + 1);
                        return;
            }
            0x04000204 => {
                // Bit 15 (GamePak type) and bit 13 are read-only/unused.
                self.wait_cnt = v16 & !(0x8000 | 0x2000);
                self.prefetch_enabled = (v16 & (1 << 14)) != 0;
            }
            0x04000208 => {
                // Delayed like IE.
                self.pending_ime = (v16 & 1) != 0;
                self.pending_at = Some(self.current_tcycle + 1);
                self.line_write_assert = true;
                        return;
            }
            _ => {
                // 未実装レジスタへの書き込みは open_bus のみ更新
                        return;
            }
        }
        // 32bit書き込みで2レジスタ跨ぎの場合、上位側も反映されるが簡易実装では上記で十分
        let _ = width;
    }

    #[inline]
    fn aligned_off(addr: u32, width: u8, mask: u32) -> usize {
        match width {
            4 => ((addr & !3) & mask) as usize,
            2 => ((addr & !1) & mask) as usize,
            _ => (addr & mask) as usize,
        }
    }

    /// VRAM is 96 KiB at 06000000-06017FFF; 06018000-0601FFFF mirrors
    /// 06010000-06017FFF (`offset - 0x8000`). In bitmap BG modes (3-5) the
    /// 06018000-0601BFFF window reads as 0 (bad access) in bitmap modes
    /// (0x14000 boundary for modes 3-5, else mirror).
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

    fn write_dma_value(&mut self, channel: usize, address: u32, width: u8, value: u32) {
        match address {
            0x02000000..=0x02FFFFFF => self.write_ewram(address, width, value),
            0x03000000..=0x03FFFFFF => self.write_iwram(address, width, value),
            0x04000000..=0x040003FE => self.write_io(address, width, value, true),
            0x05000000..=0x05FFFFFF => self.write_palette(address, width, value),
            0x06000000..=0x06FFFFFF => self.write_vram(address, width, value),
            0x07000000..=0x07FFFFFF => self.write_oam(address, width, value),
            // GamePak SRAM is CPU-only for DMA0-2 (stores go nowhere).
            // DMA3 programs Flash/plain SRAM alike; EEPROM carts have no
            // SRAM window, so DMA3 stores there still drop.
            0x0E000000..=0x0FFFFFFF => {
                if channel == 3
                    && let Some(cart) = self.cartridge.as_mut()
                    && !matches!(
                        cart.save_type(),
                        crate::cartridge::save::SaveType::Eeprom512
                            | crate::cartridge::save::SaveType::Eeprom8k
                    )
                {
                    cart.write_sram(address, width, value);
                }
            }
            // Stores to unmapped memory vanish (the bus latch is
            // fetch-driven; stores never publish).
            _ => {}
        }
    }

    fn read_dma_source(&mut self, address: u32, width: u8) -> u32 {
        // Register-gap I/O always reads the prefetch window, in either
        // mode (nba_dma_latch BUS LATCH 0x46C046C0, whose Thumb NOP sled
        // the window replicates).
        if is_unreadable_io(address) {
            // nba_dma_latch BUS LATCH cell pins this: DMA from unreadable
            // I/O reads regular open bus (the prefetched instruction), not
            // the stale DMA latch. A stale-latch model contradicts the HW
            // ROM, so the prefetch window rules.
            return if width == 4 {
                self.open_bus32()
            } else {
                self.open_bus16(address)
            };
        }
        // Unmapped IO block past the register file (alyosha DMA#2 source
        // 0x04001000): ARM samples prefetch (Unused_location_update_bus),
        // Thumb samples the shared latch (DMA_IWRAM_Bus/DMA_OAM_Bus).
        if (0x04000800..=0x04FFFFFF).contains(&address) && !is_mem_control(address) {
            if self.prefetch_thumb {
                return match width {
                    4 => self.cpu_bus,
                    _ => (self.cpu_bus >> ((address & 2) * 8)) & 0xFFFF,
                };
            }
            return if width == 4 {
                self.open_bus32()
            } else {
                self.open_bus16(address)
            };
        }
        // GamePak SRAM has no DMA restriction on the read path in practice
        // (jsmolka save/none t002 requires the 0E-0F mirror to read back;
        // only DMA *stores* are dropped, as before).
        self.read_mapped(address, width)
    }
}

impl Default for GbaMemoryBus {
    fn default() -> Self {
        Self::new()
    }
}

/// 4000800h Internal Memory Control mirror test (GBATEK: repeated each 64K
/// across 0x04000000-0x04FFFFFF).
fn is_mem_control(address: u32) -> bool {
    address & 0xFF00_0000 == 0x0400_0000 && matches!(address & 0xFFFF, 0x0800..=0x0803)
}

fn is_unreadable_io(address: u32) -> bool {
    matches!(
        address & !1,
        0x04000010..=0x04000046
            | 0x0400004C
            | 0x04000054
            | 0x04000060..=0x040000FE
            | 0x04000110..=0x0400011E
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// S-fast prefetch stream, ROM code, next opcodes bus-busy. Fields
    /// are set directly (same module): fetches would work too, but the
    /// no-cartridge open bus never yields data-mover patterns.
    fn busy_stream_bus() -> GbaMemoryBus {
        let mut bus = GbaMemoryBus::new();
        bus.write16(0x04000204, 0x4010);
        bus.last_opcode_addr = Some(0x08012002);
        bus.prefetch_win = [0x9000, 0x9001];
        bus.take_access_wait_cycles();
        bus
    }

    #[test]
    fn thumb_single_load_owe_needs_window_and_busy_next() {
        let mut bus = busy_stream_bus();
        // First stack load: no window yet, latches only.
        bus.note_thumb_single_load(0x08012008, 0x03000000);
        assert_eq!(bus.take_access_wait_cycles(), 0);
        // Adjacent stack load inside the window, busy next: owes one.
        bus.note_thumb_single_load(0x0801200A, 0x03000000);
        assert_eq!(bus.take_access_wait_cycles(), 1);
        // ROM-data load directly after a stack load: owes one more.
        bus.note_thumb_single_load(0x0801200C, 0x08000000);
        assert_eq!(bus.take_access_wait_cycles(), 1);
        // ROM-data load without a stack predecessor: no owe.
        bus.note_thumb_single_load(0x0801200E, 0x08000000);
        assert_eq!(bus.take_access_wait_cycles(), 0);
    }

    #[test]
    fn thumb_single_load_skips_on_idle_next_or_cold_window() {
        let mut bus = busy_stream_bus();
        bus.note_thumb_single_load(0x08012008, 0x03000000);
        assert_eq!(bus.take_access_wait_cycles(), 0);
        // Idle next opcode absorbs the owe (bus-idle peek window).
        bus.prefetch_win = [0x0000, 0x0001];
        bus.take_access_wait_cycles();
        bus.note_thumb_single_load(0x0801200A, 0x03000000);
        assert_eq!(bus.take_access_wait_cycles(), 0);
        // Window closed (marker older than three instructions): no owe.
        let mut bus = busy_stream_bus();
        bus.note_thumb_single_load(0x08012008, 0x03000000);
        assert_eq!(bus.take_access_wait_cycles(), 0);
        bus.note_thumb_single_load(0x08012010, 0x03000000);
        assert_eq!(bus.take_access_wait_cycles(), 0);
    }

    #[test]
    fn thumb_single_load_gated_off_without_prefetch_stream() {
        let mut bus = GbaMemoryBus::new();
        bus.fetch16(0x08012000);
        bus.fetch16(0x08012002);
        bus.take_access_wait_cycles();
        bus.note_thumb_single_load(0x08012008, 0x03000000);
        assert_eq!(bus.take_access_wait_cycles(), 0);
    }

    #[cfg(feature = "mgba-debug-log")]
    #[test]
    fn mgba_debug_handshake_and_log_commit() {
        // mgba-emu/suite `mgba_open` handshake + `mgba_printf` commit.
        let mut bus = GbaMemoryBus::new();
        assert!(!bus.mgba_debug_enabled());
        bus.write16(0x04FFF780, 0xC0DE);
        assert!(bus.mgba_debug_enabled());
        assert_eq!(bus.read16(0x04FFF780), 0x1DEA);
        for (i, &b) in b"PASS: x".iter().enumerate() {
            bus.write8(0x04FFF600 + i as u32, b);
        }
        bus.write8(0x04FFF600 + 7, 0);
        bus.write16(0x04FFF700, 4 | 0x100);
        let logs = bus.drain_mgba_debug_logs();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].level, 4);
        assert_eq!(logs[0].text, "PASS: x");
        assert!(bus.drain_mgba_debug_logs().is_empty());
        bus.write16(0x04FFF780, 0);
        assert!(!bus.mgba_debug_enabled());
    }

    #[test]
    fn read_wram_bounds() {
        let mut bus = GbaMemoryBus::new();
        bus.write8(0x02000000, 0xAB);
        bus.write8(0x0203FFFF, 0xCD);
        assert_eq!(bus.read8(0x02000000), 0xAB);
        assert_eq!(bus.read8(0x0203FFFF), 0xCD);
    }

    #[test]
    fn waitcnt_type_flag_and_reserved_bits_are_read_only() {
        let mut bus = GbaMemoryBus::new();
        bus.write16(0x04000204, 0xFFFF);
        // Bit 15 (GamePak type, GBATEK read-only) and bit 13 (unused)
        // never stick; everything else does.
        assert_eq!(bus.read16(0x04000204), 0x5FFF);
    }

    #[test]
    fn interrupt_enable_covers_gamepak_irq_bit() {
        let mut bus = GbaMemoryBus::new();
        bus.write16(0x04000200, 0xFFFF);
        // IE writes apply 1 tick later (delayed interrupt pipeline).
        bus.tick();
        assert_eq!(bus.read16(0x04000200) & 0x3FFF, 0x3FFF);
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
        // No cartridge: no backup chip answers, the bus floats high
        // (0xFF, not stored-byte echo).
        bus.write8(0x0E000000, 0x42);
        bus.write8(0x0E00FFFF, 0x99);
        assert_eq!(bus.read8(0x0E000000), 0xFF);
        assert_eq!(bus.read8(0x0E00FFFF), 0xFF);
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
        bus.set_current_pc(0x02000000);
        bus.write32(0x02000000, 0xDEADBEEF);
        let _ = bus.fetch32(0x02000000);
        // Unmapped reads see the fetch latch (ARM: newest opcode whole).
        assert_eq!(bus.read32(0x04000400), 0xDEADBEEF);
        assert_eq!(bus.read16(0x04000402), 0xDEAD);
        // Stores never publish to the bus latch.
        bus.write32(0x02000004, 0x12345678);
        let _ = bus.read32(0x02000004);
        assert_eq!(bus.read32(0x04000400), 0xDEADBEEF);
    }

    #[test]
    fn write_only_reg_returns_open_bus() {
        let mut bus = GbaMemoryBus::new();
        bus.set_current_pc(0x02000000);
        bus.write32(0x02000000, 0x12345678);
        let _ = bus.fetch32(0x02000000);
        // BG0CNT (0x04000008) is R/W, so it returns the register value (default 0).
        assert_eq!(bus.read16(0x04000008), 0);
        bus.write16(0x04000008, 0x1234);
        assert_eq!(bus.read16(0x04000008), 0x1234);
        // Write-only MOSAIC (0x0400004C) sees the fetch latch, not the
        // last stored value.
        assert_eq!(bus.read16(0x0400004C), 0x5678);
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
        // GBATEK WAITCNT totals (1 base + waits): N16=5, N32=8 at defaults.
        assert_eq!(bus.cycles_for(0x08000000, 2), 5);
        assert_eq!(bus.cycles_for(0x08000000, 4), 8);
    }

    #[test]
    fn prefetch_sequential_saves_cycles() {
        let mut bus = GbaMemoryBus::new();
        bus.write16(0x04000204, 1 << 14); // prefetch enable
        assert!(bus.prefetch_enabled);
        // 非連続 → 通常 wait
        assert_eq!(bus.opcode_cycles_for(0x08000000, 4), 8);
        // 連続fetchはSコスト (pure-fetchにride割引なし。
        // suite nop P.. = 6 が証拠)。キューは撤去済み。
        let _ = bus.fetch32(0x08000000);
        assert_eq!(bus.opcode_cycles_for(0x08000004, 4), 6);
        // データリードはバッファに乗らず常にN:
        // N32 = 8 at WS0.
        assert_eq!(bus.cycles_for(0x08000004, 4), 8);
        bus.write16(0x04000204, 0);
        assert!(!bus.prefetch_enabled);
    }

    #[test]
    fn linear_prefetch_stream_stays_sequential_past_window() {
        let mut bus = GbaMemoryBus::new();
        bus.write16(0x04000204, 1 << 14);
        let _ = bus.fetch32(0x08000000);

        for address in (0x08000004..0x08000040).step_by(4) {
            assert_eq!(bus.opcode_cycles_for(address, 4), 6);
            let _ = bus.fetch32(address);
        }
    }

    #[test]
    fn branch_consumes_buffer_before_nonsequential_refill() {
        let mut bus = GbaMemoryBus::new();
        bus.write16(0x04000204, 1 << 14);
        let _ = bus.fetch32(0x08000000);
        bus.invalidate_prefetch_for_branch();

        for address in [0x08000004, 0x08000008, 0x0800000C, 0x08000010] {
            assert_eq!(bus.opcode_cycles_for(address, 4), 6);
            let _ = bus.fetch32(address);
        }
        assert_eq!(bus.opcode_cycles_for(0x08000014, 4), 8);
    }

    #[test]
    fn mode_switch_prefill_starts_active_linear_stream() {
        let mut bus = GbaMemoryBus::new();
        bus.write16(0x04000204, 1 << 14);
        let _ = bus.fetch32(0x08000000);
        bus.invalidate_prefetch_for_branch();
        bus.refill_prefetch_for_switch(0x08000100);

        for address in (0x08000100..0x08000120).step_by(4) {
            assert_eq!(bus.opcode_cycles_for(address, 4), 6);
            let _ = bus.fetch32(address);
        }
    }

    #[test]
    fn pipeline_flush_makes_next_gamepak_access_nonsequential() {
        let mut bus = GbaMemoryBus::new();
        bus.read32(0x08000000);
        bus.invalidate_prefetch_for_dma(0x08000004);

        assert_eq!(bus.cycles_for(0x08000004, 4), 8);
    }

    #[test]
    fn dma_invalidates_prefetch_tracking() {
        // DMA owns the bus: CPU fetch/data stream tracking resets, so the
        // next access is non-sequential however contiguous it looks.
        let mut bus = GbaMemoryBus::new();
        bus.write16(0x04000204, 1 << 14); // prefetch enable
        let _ = bus.fetch32(0x08000000);
        assert_eq!(bus.opcode_cycles_for(0x08000004, 4), 6);
        bus.invalidate_prefetch_for_dma(0x08000004);
        assert_eq!(bus.opcode_cycles_for(0x08000004, 4), 8);
        bus.invalidate_prefetch_for_dma(0x04000000);
        // Non-ROM target: same tracking reset.
        assert_eq!(bus.opcode_cycles_for(0x08000008, 4), 8);
    }

    #[test]
    fn dma_pending_n_ifies_fetches() {
        // Bus arbitration: fetches issued while an immediate DMA burst is
        // pending (trigger stored, handover imminent) cost N even when
        // fetch-stream-contiguous (HW-pinned by hw-test ROM force-nseq-access:
        // post-trigger nops cost 1N, TIME 88).
        let mut bus = GbaMemoryBus::new();
        let _ = bus.fetch32(0x08000000);
        assert_eq!(bus.opcode_cycles_for(0x08000004, 4), 6);
        bus.write32(0x040000C4, 0x80000001); // DMA1CNT: ENABLE|16|IMM|1
        assert!(bus.dma_active() || bus.dma.has_pending());
        assert_eq!(bus.opcode_cycles_for(0x08000008, 4), 8);
    }

    #[test]
    fn fetch_stream_survives_data_reads() {
        // N/S model: data reads advance neither the fetch stream
        // nor its sequentiality. Linear fetches stay S (3 total at WS0);
        // a fetch jumping ahead of the stream costs N (5 total).
        let mut bus = GbaMemoryBus::new();
        assert!(!bus.prefetch_enabled);
        // Plain sequential fetches: S timing (WS0: 3 cycles/halfword total).
        let _ = bus.fetch16(0x08000000);
        assert_eq!(bus.opcode_cycles_for(0x08000002, 2), 3);
        // A data read leaves the stream alone: the next in-stream fetch
        // is still S ...
        let _ = bus.read16(0x08000004);
        let _ = bus.fetch16(0x08000002);
        assert_eq!(bus.opcode_cycles_for(0x08000004, 2), 3);
        // ... while a fetch jumping ahead of the stream is N.
        assert_eq!(bus.opcode_cycles_for(0x08000008, 2), 5);
    }

    #[test]
    fn prefetch_on_data_access_keeps_fetch_stream() {
        // N/S model: a data access to another area never breaks code
        // sequentiality — the next ROM fetch costs S (3 total at WS0),
        // never N. Prefetch hides the stall via erases, not via ride
        // discounts (pure-fetch streams pay full S: suite nop P.. = 6).
        let mut bus = GbaMemoryBus::new();
        bus.write16(0x04000204, 1 << 14); // prefetch enable
        let _ = bus.fetch16(0x08000000);
        let _ = bus.fetch16(0x08000002);
        // Data access to I/O: must not poison the ROM fetch stream.
        let _ = bus.read16(0x04000000);
        // Next ROM fetch still sequential: S cost, not 1N.
        assert_eq!(bus.opcode_cycles_for(0x08000004, 2), 3);
    }

    #[test]
    fn prefetch_off_data_access_breaks_bus_sequence() {
        // N/S follows the fetch stream (mgba-suite Timing truth), so the
        // same I/O access leaves the next ROM fetch sequential (1S = 3
        // total at WS0). This is the suite `nop` cell (HW 6 = S+S fetch
        // + 1 internal).
        let mut bus = GbaMemoryBus::new();
        assert!(!bus.prefetch_enabled);
        let _ = bus.fetch16(0x08000000);
        let _ = bus.fetch16(0x08000002);
        let _ = bus.read16(0x04000000);
        assert_eq!(bus.opcode_cycles_for(0x08000004, 2), 3);
    }

    #[test]
    fn fetch_stream_discontinuity_is_nonsequential() {
        // A fetch discontinuous with the fetch stream costs N (5 total at
        // WS0), even with no data access involved: IWRAM-resident code
        // fetching ROM directly (real flow reaches this only via a branch,
        // which refills N the same way).
        let mut bus = GbaMemoryBus::new();
        assert!(!bus.prefetch_enabled);
        let _ = bus.fetch16(0x03000000);
        let _ = bus.read16(0x08000002);
        assert_eq!(bus.opcode_cycles_for(0x08000004, 2), 5);
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
    fn display_stall_inserts_wait_during_draw() {
        let mut bus = GbaMemoryBus::new();
        // Reset state keeps forced blank set (DISPCNT=0x0080): no contention.
        assert_eq!(bus.cycles_for(0x06000000, 2), 1);
        // Mode 0, BG0 on: the enable propagates through the 3-stage
        // DISPCNT latch (shifts at +40 cycles/line, shared per-line
        // reference), then BG-VRAM stalls during the fetch window and
        // palette during pixel output.
        bus.write16(0x04000000, 0x0100);
        for _ in 0..4000 {
            bus.tick();
        }
        assert_eq!(bus.cycles_for(0x06000000, 2), 2);
        assert_eq!(bus.cycles_for(0x05000000, 2), 2);
        assert_eq!(bus.cycles_for(0x07000000, 2), 2);
        // HBlank: BG/palette idle, OAM still busy (H-Blank Interval Free off).
        while bus.ppu.cycle() < 1006 {
            bus.tick();
        }
        assert_eq!(bus.cycles_for(0x06000000, 2), 1);
        assert_eq!(bus.cycles_for(0x05000000, 2), 1);
        assert_eq!(bus.cycles_for(0x07000000, 2), 2);
        // Forced blank idles the controller: no stalls anywhere.
        bus.write16(0x04000000, 1 << 7);
        assert_eq!(bus.cycles_for(0x06000000, 2), 1);
        assert_eq!(bus.cycles_for(0x07000000, 2), 1);
    }

    #[test]
    fn internal_memory_control_mirror() {
        // GBATEK System Control: R/W, init 0D000020h, mirrored each 64K.
        let mut bus = GbaMemoryBus::new();
        assert_eq!(bus.read32(0x04000800), 0x0D00_0020);
        assert_eq!(bus.read32(0x04100800), 0x0D00_0020);
        bus.write32(0x04200800, 0xFFFF_FFFF);
        // Only documented bits stick (0-3, 5, 24-31).
        assert_eq!(bus.read32(0x04000800), 0xFF00_002F);
        assert_eq!(bus.read8(0x04000801), 0x00);
    }

    #[test]
    fn write_if_clears() {
        let mut bus = GbaMemoryBus::new();
        bus.request_interrupt(0x0003);
        bus.tick();
        assert_eq!(bus.sif, 0x0003);
        bus.write16(0x04000202, 0x0001);
        bus.tick();
        assert_eq!(bus.sif, 0x0002);
        bus.write16(0x04000202, 0x0002);
        bus.tick();
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
    fn fifo_writes_append_bytes() {
        // GBATEK Sound FIFO: writes append to the 32-byte buffer;
        // reads are open bus (not wave RAM). FIFO writes land only
        // while the sound master enable is on (HW-observed: an empty
        // FIFO stays empty with the master off).
        let mut bus = GbaMemoryBus::new();
        bus.write32(0x040000A0, 0x04030201);
        assert!(bus.apu.fifo_a.is_empty());
        bus.write16(0x04000084, 0x0080);
        bus.write32(0x040000A0, 0x04030201);
        assert_eq!(bus.apu.fifo_a.len(), 4);
        assert_eq!(
            bus.apu.fifo_a.iter().copied().collect::<Vec<_>>(),
            vec![0x01, 0x02, 0x03, 0x04]
        );
        bus.write16(0x040000A4, 0x0B0A);
        assert_eq!(
            bus.apu.fifo_b.iter().copied().collect::<Vec<_>>(),
            vec![0x0A, 0x0B]
        );
        bus.write16(0x04000090, 0x1234);
        // Default NR30 selects bank 0 for playback, so the CPU sees bank 1.
        assert_eq!(bus.apu.wave_ram[16], 0x34);
        assert_eq!(bus.read16(0x04000090), 0x1234);
        // Selecting bank 1 flips the CPU window to bank 0.
        bus.write16(0x04000070, 1 << 6);
        bus.write16(0x04000090, 0x5678);
        assert_eq!(bus.apu.wave_ram[0], 0x78);
        assert_eq!(bus.read16(0x04000090), 0x5678);
    }

    #[test]
    fn keycnt_raises_keypad_interrupt() {
        // GBATEK KEYCNT: enable + OR over button A; pressing A sets IF bit 12.
        let mut bus = GbaMemoryBus::new();
        bus.write16(0x04000132, (1 << 14) | (1 << 0));
        bus.set_keyinput(0x03FF); // nothing pressed
        assert_eq!(bus.sif & (1 << 12), 0);
        bus.set_keyinput(0x03FE); // A pressed (bit 0 = 0)
        // Keypad raises apply 1 tick later (delayed interrupt pipeline).
        bus.tick();
        assert_ne!(bus.sif & (1 << 12), 0);
    }

    #[test]
    fn eeprom_dma_bitstream_roundtrip() {
        use crate::cartridge::Cartridge;
        use crate::cartridge::header::finalize_test_gba_rom;
        // EEPROM-detected cart: CPU access must not consume stream bits.
        let mut rom = vec![0u8; 0x1000];
        finalize_test_gba_rom(&mut rom);
        rom[0x200..0x20A].copy_from_slice(b"EEPROM_V12");
        let mut bus = GbaMemoryBus::new();
        bus.set_cartridge(Cartridge::new(rom).unwrap());
        // CPU loads from the EEPROM window see the chip state: idle
        // drives 1 (Ready), never open bus.
        bus.write32(0x02000000, 0x12345678);
        let _ = bus.read32(0x02000000);
        bus.write8(0x0D000000, 0x42);
        assert_eq!(bus.read8(0x0D000000), 1);
        // DMA write burst: 8K frame (start, write-op, 14-bit addr 0, data, stop).
        let data = [0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02, 0x03, 0x04];
        let mut bits = vec![true, false];
        bits.extend_from_slice(&[false; 14]);
        for byte in data {
            for i in (0..8).rev() {
                bits.push((byte >> i) & 1 != 0);
            }
        }
        bits.push(false);
        assert_eq!(bits.len(), 81);
        // Program the source in IWRAM, then DMA it to 0D000000h.
        for (i, bit) in bits.iter().enumerate() {
            bus.write16(0x03000000 + (i as u32) * 2, u16::from(*bit));
        }
        bus.write32(0x040000D4, 0x03000000); // DMA3 SAD
        bus.write32(0x040000D8, 0x0D000000); // DMA3 DAD
        bus.write16(0x040000DC, bits.len() as u16); // count
        bus.write16(0x040000DE, 0x8000); // enable, 16-bit, immediate
        for _ in 0..100000 {
            bus.tick();
            if !bus.dma_active() && !bus.dma.has_pending() {
                break;
            }
        }
        for _ in 0..10 {
            bus.tick();
        }
        let cart = bus.cartridge().unwrap();
        assert_eq!(&cart.ram_data().unwrap()[0..8], &data);
        // DMA read back: request (start, read-op, addr 0) then 68-unit read.
        let mut req = vec![true, true];
        req.extend_from_slice(&[false; 14]);
        for (i, bit) in req.iter().enumerate() {
            bus.write16(0x03001000 + (i as u32) * 2, u16::from(*bit));
        }
        bus.write32(0x040000D4, 0x03001000);
        bus.write32(0x040000D8, 0x0D000000);
        bus.write16(0x040000DC, req.len() as u16);
        bus.write16(0x040000DE, 0x8000);
        for _ in 0..100000 {
            bus.tick();
            if !bus.dma_active() && !bus.dma.has_pending() {
                break;
            }
        }
        for _ in 0..10 {
            bus.tick();
        }
        bus.write32(0x040000D4, 0x0D000000);
        bus.write32(0x040000D8, 0x03002000);
        bus.write16(0x040000DC, 68);
        bus.write16(0x040000DE, 0x8000);
        for _ in 0..100000 {
            bus.tick();
            if !bus.dma_active() && !bus.dma.has_pending() {
                break;
            }
        }
        let mut got = [0u8; 8];
        for i in 0..64 {
            let bit = bus.read16(0x03002000 + 8 + (i as u32) * 2) & 1;
            if bit != 0 {
                got[i / 8] |= 1 << (7 - (i % 8));
            }
        }
        assert_eq!(got, data);
    }

    #[test]
    fn eeprom_cart_has_no_cpu_sram_window() {
        use crate::cartridge::Cartridge;
        use crate::cartridge::header::finalize_test_gba_rom;
        // GBATEK backup detection: an SRAM probe on an EEPROM cart must
        // fail (open bus), not read back a phantom direct window.
        let mut rom = vec![0u8; 0x1000];
        finalize_test_gba_rom(&mut rom);
        rom[0x200..0x20A].copy_from_slice(b"EEPROM_V12");
        let mut bus = GbaMemoryBus::new();
        bus.set_cartridge(Cartridge::new(rom).unwrap());
        bus.write32(0x02000000, 0x12345678);
        bus.write16(0x0E000000, 0xBEEF);
        // Point the fetch latch at EWRAM, then prove 0E stored nothing
        // (stores never publish to the bus latch).
        let _ = bus.fetch32(0x02000000);
        assert_eq!(bus.read16(0x0E000000), 0x5678);
        bus.write32(0x02000004, 0xAABBCCDD);
        let _ = bus.read32(0x02000004);
        assert_eq!(bus.read16(0x0E000000), 0x5678);
    }

    #[test]
    fn gpio_overlay_attaches_on_control_write() {
        use crate::cartridge::Cartridge;
        use crate::cartridge::header::finalize_test_gba_rom;
        // Plain ROM data shows through until the first GPIO enable.
        let mut rom = vec![0u8; 0x1000];
        finalize_test_gba_rom(&mut rom);
        rom[0xC4] = 0x12;
        rom[0xC5] = 0x34;
        let mut bus = GbaMemoryBus::new();
        bus.set_cartridge(Cartridge::new(rom).unwrap());
        assert_eq!(bus.read16(0x080000C4), 0x3412);
        bus.write16(0x080000C8, 1);
        bus.write16(0x080000C6, 0b1010);
        bus.write16(0x080000C4, 0b1111);
        assert_eq!(bus.read16(0x080000C4), 0b1010);
    }

    #[test]
    fn hblank_dma_fires_on_all_lines_including_vblank() {
        // GBATEK DISPSTAT: H-Blank conditions are generated once per
        // scanline, including hidden V-Blank scanlines — one full frame of
        // a repeat HBlank channel transfers 228 units, not 160.
        let mut bus = GbaMemoryBus::new();
        for i in 0..256u16 {
            bus.write16(0x03000000 + u32::from(i) * 2, 0xABCD);
        }
        bus.write32(0x040000B0, 0x03000000); // DMA0SAD
        bus.write32(0x040000B4, 0x03001000); // DMA0DAD
        bus.write16(0x040000B8, 1); // count 1
        // ENABLE | REPEAT | HBLANK | 16-bit (DMA0CNT_H)
        bus.write16(0x040000BA, 0x8000 | (1 << 9) | (2 << 12));
        let mut frames = 0;
        for _ in 0..300000 {
            if bus.tick() {
                frames += 1;
                break;
            }
        }
        assert_eq!(frames, 1);
        let written = (0..256u16)
            .filter(|&i| bus.read16(0x03001000 + u32::from(i) * 2) != 0)
            .count();
        assert_eq!(written, 228);
    }

    #[test]
    fn video_dma_runs_after_line_162_latch() {
        // DMA3 video-capture: latched at vcount==162, runs on lines [2,162)
        // of the next frame. Must not transfer before the latch.
        let mut bus = GbaMemoryBus::new();
        bus.write16(0x03000000, 0x1111);
        bus.write16(0x03000002, 0x2222);
        bus.write16(0x03000004, 0x3333);
        bus.write16(0x03000006, 0x4444);
        bus.write32(0x040000D4, 0x03000000); // DMA3SAD
        bus.write32(0x040000D8, 0x03001000); // DMA3DAD
        bus.write16(0x040000DC, 4); // count 4
        // ENABLE | SPECIAL | 16-bit
        bus.write16(0x040000DE, 0x8000 | (3 << 12));
        for _ in 0..100000 {
            bus.tick();
        }
        // Still frame 0, before the line-162 latch: nothing transferred.
        assert_eq!(bus.read16(0x03001000), 0);
        // Run until the transfer completes (must happen, capped).
        let mut done = false;
        for _ in 0..500000 {
            bus.tick();
            if bus.read16(0x040000DE) & 0x8000 == 0 {
                done = true;
                break;
            }
        }
        assert!(done, "video DMA never completed");
        assert_eq!(bus.read16(0x03001000), 0x1111);
        assert_eq!(bus.read16(0x03001002), 0x2222);
        assert_eq!(bus.read16(0x03001004), 0x3333);
        assert_eq!(bus.read16(0x03001006), 0x4444);
        // Completed early in the next frame (lines 2..3), not mid-frame 0.
        assert!(bus.read16(0x04000006) < 10);
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
        // Stop mode (bit 7 set) latches without halting (unmodeled).
        bus.write8(0x04000301, 0x80);
        assert!(!bus.is_halted());
        // HALTCNT is write-only: reads see the fetch latch (lane 1),
        // never the written value or later data traffic.
        bus.write32(0x02000000, 0x12345678);
        let _ = bus.fetch32(0x02000000);
        assert_eq!(bus.read8(0x04000301), 0x56);
        bus.write8(0x02000000, 0x12);
        let _ = bus.read8(0x02000000);
        assert_eq!(bus.read8(0x04000301), 0x56);
        assert_eq!(bus.read8(0x04000300), 1);

        bus.write16(0x04000200, 1);
        bus.enter_halt(1);
        bus.request_interrupt(1);
        // Wake propagates through the delayed pipeline (apply +1,
        // availability +1).
        bus.tick();
        bus.tick();
        assert!(!bus.is_halted());
        assert_eq!(bus.read16(0x03007FF8) & 1, 1);
    }

    #[test]
    fn cpu_haltcnt_writes_are_bios_gated() {
        // HW behavior (hw-test ROM haltcnt): CPU writes from outside the
        // BIOS are ignored; HLE BIOS and DMA writes act.
        let mut bus = GbaMemoryBus::new();
        bus.set_current_pc(0x03000000);
        bus.write16(0x04000300, 0x0001);
        assert!(!bus.is_halted());
        bus.write8(0x04000301, 0x00);
        assert!(!bus.is_halted());
        // POSTFLG likewise ignores non-BIOS writes (stays at reset 1).
        bus.write16(0x04000300, 0x0000);
        assert_eq!(bus.read8(0x04000300), 1);

        // BIOS-context (HLE) writes halt and set POSTFLG.
        let mut bus = GbaMemoryBus::new();
        bus.write_hle_bios16(0x04000300, 0x0001);
        assert!(bus.is_halted());

        let mut bus = GbaMemoryBus::new();
        bus.set_current_pc(0x00000000);
        bus.write8(0x04000301, 0x00);
        assert!(bus.is_halted());
    }

    #[test]
    fn halt_wakes_once_irq_availability_propagates() {
        let mut bus = GbaMemoryBus::new();
        bus.write16(0x04000200, 1);
        bus.request_interrupt(1);
        bus.enter_halt(1);
        // Halt parks first (availability propagates with delay)...
        assert!(bus.is_halted());
        // ...then the pending IRQ wakes it once availability arrives.
        bus.tick();
        bus.tick();
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
        // Memory is sampled after the 3 CPU-visible startup cycles
        // (3-cycle start latency plus the enabling bus cycle; hw-test
        // ROM start-delay pins the first read one tick later).
        // The channel remains active for the transfer cycles after this.
        for _ in 0..3 {
            bus.tick();
            assert_eq!(bus.read32(0x02000000), 0);
        }
        bus.tick();
        assert_eq!(bus.read32(0x02000000), 0xDEADBEEF);
        while bus.dma_active() {
            bus.tick();
        }
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
        // The overflow's IF propagates 1 tick after the request.
        bus.tick();
        assert_ne!(bus.read16(0x04000202) & (1 << 3), 0);
        assert_eq!(bus.read16(0x04000104), 1);
    }

    #[test]
    fn uart_transfers_idle_high_bytes() {
        let mut bus = GbaMemoryBus::new();
        bus.write16(0x04000134, 0);
        // UART, 115200 baud, 8-bit, FIFO + send/recv enable.
        bus.write16(0x04000128, 0x3D83);
        // Idle status: send not full, receive empty, no error.
        assert_eq!(bus.read16(0x04000128) & 0x0070, 0x0020);
        bus.write16(0x0400012A, 0x42);
        // 10-bit frame at 146 T-cycles/bit.
        for _ in 0..2000 {
            bus.tick();
        }
        assert_eq!(bus.read16(0x0400012A), 0xFF);
        // Drained receive FIFO reads empty again.
        assert_eq!(bus.read16(0x0400012A), 0);
    }

    #[test]
    fn multiplayer_start_never_completes_solo() {
        let mut bus = GbaMemoryBus::new();
        bus.write16(0x04000134, 0);
        // Multi, 115200 baud, START as parent-would: the solo master
        // waits for children forever (suite timeout cells).
        bus.write16(0x04000128, 0x2080);
        for _ in 0..200_000 {
            bus.tick();
        }
        assert_ne!(bus.read16(0x04000128) & 0x0080, 0);
    }
}
