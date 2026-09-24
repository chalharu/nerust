use crate::apu::GbaApu;
use crate::bios::hle_operation::{HleBiosBus, HleBiosOperation};
use crate::cartridge::Cartridge;
use crate::cartridge::save::helpers::{read_slice, repeat_byte, selected_write_byte, write_slice};
use crate::dma::{DmaTransfer, DmaTrigger, GbaDma};
use crate::ppu::{GbaPpu, HDRAW_CYCLES, PpuEvent};
use crate::sound_driver::SoundDriverBus;
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
    /// DMA open-bus latch + PC tag: every serviced DMA unit latches its
    /// data word into `dma_bus`; a CPU data read from unmapped space by
    /// the instruction immediately following the serviced unit observes
    /// the latch instead of the prefetch window. `dma_open_pc` tags the
    /// in-flight instruction using the bus `current_pc` convention
    /// (architectural PC, i.e. execute+4 Thumb / execute+8 ARM), so the
    /// read-side compare is a plain 2/4-cycle difference.
    dma_bus: u32,
    dma_open_pc: u32,
    /// Trigger-time fetch PC (per-cycle co-sim): latched when the DMA
    /// request fires (HBlank/VBlank event), while `dma_open_pc` tags the
    /// serviced unit 3 ticks later. The CPU advances between the two, so
    /// the open-bus gate must test both tags (see `dma_open_bus`).
    dma_trigger_pc: u32,
    /// Sticky DMA bus latch validity: set when a serviced DMA unit drives
    /// the shared latch, cleared by the next CPU mapped data access (which
    /// drives the bus with its own value). Opcode fetches and unmapped
    /// reads only sample the bus and never disturb it, so a DMA value
    /// survives ALU/branch/fetch instructions until real bus traffic lands
    /// (HW open-bus capacitance; mgba-suite DMA Prefetch pins the break).
    /// Reads from unmapped space observe the latch while valid (lane
    /// selected), the prefetch window otherwise. The adjacent-PC gate
    /// below stays as the single-instruction fast path.
    dma_bus_valid: bool,
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
    /// Last two CPU data addresses (non-opcode reads/writes), for
    /// back-to-back fetch-break coalescing (an unmapped open-bus load
    /// followed by an IWRAM store pre-pays once; Timing isolated accesses
    /// keep their own break).
    last_data_addr: Option<u32>,
    prev_data_addr: Option<u32>,
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
    /// redirect, modulo the fill duty below. Only ROM-bus occupancy
    /// advances it; CPU-internal/IO/RAM/DMA activity freezes it.
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
    /// Set when an IntrWait-family halt wakes: the real BIOS exit path
    /// runs after the wake ISR, so its last opcode (0xE3A02004, pinned
    /// by mgba-suite "BIOS load") replaces the HLE epilogue latch at
    /// the next IRQ return. Consumed there; cleared on halt entry.
    bios_wait_exit_armed: bool,
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

/// Phase 10 wire state: all RAM (as bytes), the interrupt pipeline, SIO/
/// UART registers, open-bus latches, prefetch/fill clocks, block-batch and
/// stream trackers, halt/stop/wake flags, tick-driven counters and every
/// device state. Excluded by design: `fallback_sram` (Phase 3 test shim),
/// the write-only `haltcnt` latch, and the `mgba-debug-log` feature state.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct GbaMemoryBusState {
    bios: serde_bytes::ByteBuf,
    ewram: serde_bytes::ByteBuf,
    iwram: serde_bytes::ByteBuf,
    palette_ram: serde_bytes::ByteBuf,
    vram: serde_bytes::ByteBuf,
    oam: serde_bytes::ByteBuf,
    wait_cnt: u16,
    ie: u16,
    sif: u16,
    ime: bool,
    pending_ie: u16,
    pending_ime: bool,
    pending_if: u16,
    pending_at: Option<u64>,
    line_write_assert: bool,
    irq_available: bool,
    avail_queue: Vec<(bool, u64)>,
    irq_line: bool,
    line_queue: Vec<(bool, u64)>,
    postflg: u8,
    mem_control: u32,
    keyinput: u16,
    keycnt: u16,
    siocnt: u16,
    siodata8: u16,
    siodata32: u32,
    rcnt: u16,
    joycnt: u16,
    sio_xfer_cycles: u32,
    sio_xfer_32: bool,
    sio_xfer_uart: bool,
    uart_tx: Vec<u8>,
    uart_rx: Vec<u8>,
    uart_err: bool,
    uart_prev_irqsrc: u8,
    last_prefetch: u32,
    prefetch_win: [u32; 2],
    prefetch_thumb: bool,
    cpu_bus: u32,
    dma_bus: u32,
    dma_bus_valid: bool,
    dma_open_pc: u32,
    dma_trigger_pc: u32,
    prefetch_enabled: bool,
    pf_start: u32,
    pf_end: u32,
    pf_valid: bool,
    pf_branch_drain: bool,
    fill_countdown: u32,
    last_prefetched_pc: u32,
    bios_prefetch: u32,
    data_sequential_override: bool,
    block_batching: bool,
    block_batch_any: bool,
    block_batch_words: u32,
    block_batch_erase_sum: i32,
    block_batch_is_load: bool,
    block_batch_fetch_width: u8,
    block_batch_has_rom: bool,
    block_batch_raw: bool,
    access_wait_cycles: i64,
    last_opcode_addr: Option<u32>,
    last_data_addr: Option<u32>,
    prev_data_addr: Option<u32>,
    prev_load_pc: Option<u32>,
    prev_load_is_stack: bool,
    current_pc: u32,
    prev_addr: Option<u32>,
    prev_width: u8,
    fetch_addr: Option<u32>,
    fetch_width: u8,
    halted: bool,
    halt_irq_mask: u16,
    stopped: bool,
    wake_clear_mask: u16,
    wake_latency: u32,
    woke_from_halt: bool,
    dma_stall_pending: u32,
    bios_wait_exit_armed: bool,
    pub(crate) current_tcycle: u64,
    timer0_raise_tick: u64,
    bios_protect: bool,
    video_armed: bool,
    video_countdown: u8,
    pending_hblank_irq: bool,
    eeprom_burst_open: bool,
    ppu: crate::ppu::GbaPpuState,
    dma: crate::dma::GbaDmaState,
    pub(crate) timers: crate::timer::GbaTimersState,
    apu: crate::apu::GbaApuState,
    cartridge: Option<crate::cartridge::CartridgeState>,
    hle_bios: Option<crate::bios::hle_operation::HleBiosOperation>,
}

impl GbaMemoryBusState {
    pub(crate) fn validate(&self) -> Result<(), String> {
        for (name, bytes, len) in [
            ("bios", &self.bios, BIOS_SIZE),
            ("ewram", &self.ewram, EWRAM_SIZE),
            ("iwram", &self.iwram, IWRAM_SIZE),
            ("palette", &self.palette_ram, PALETTE_SIZE),
            ("vram", &self.vram, VRAM_SIZE),
            ("oam", &self.oam, OAM_SIZE),
        ] {
            if bytes.len() != len {
                return Err(format!("bus: {name} length wrong: {}", bytes.len()));
            }
        }
        // Both write paths mask WAITCNT with !(0x8000 | 0x2000).
        if self.wait_cnt & (0x8000 | 0x2000) != 0 {
            return Err(format!("bus: wait_cnt reserved bits: {:#X}", self.wait_cnt));
        }
        // KEYCNT writes mask with 0xC3FF.
        if self.keycnt & !0xC3FF != 0 {
            return Err(format!("bus: keycnt reserved bits: {:#X}", self.keycnt));
        }
        // UART FIFOs hold at most the 4-deep cap.
        if self.uart_tx.len() > 4 || self.uart_rx.len() > 4 {
            return Err(format!(
                "bus: uart fifo overflow: {}/{}",
                self.uart_tx.len(),
                self.uart_rx.len()
            ));
        }
        if self.sio_xfer_cycles > 0x10_0000 {
            return Err(format!(
                "bus: sio transfer too long: {}",
                self.sio_xfer_cycles
            ));
        }
        // Scheduled interrupt-pipeline events land within a few ticks.
        if self.avail_queue.len() > 32 || self.line_queue.len() > 32 {
            return Err("bus: irq queue too long".to_string());
        }
        if let Some(at) = self.pending_at
            && at > self.current_tcycle.saturating_add(16)
        {
            return Err("bus: pending interrupt apply too far ahead".to_string());
        }
        for (_, at) in self.avail_queue.iter().chain(self.line_queue.iter()) {
            if *at > self.current_tcycle.saturating_add(16) {
                return Err("bus: queued irq event too far ahead".to_string());
            }
        }
        if self.timer0_raise_tick > self.current_tcycle {
            return Err("bus: timer0 raise tick in the future".to_string());
        }
        // Open-bus PC tags are fresh while the DMA latch is valid (a CPU
        // mapped access clears it), so they must sit near the current PC.
        // Stale tags with a cleared latch carry no constraint.
        if self.dma_bus_valid {
            for (name, tag) in [
                ("dma_open_pc", self.dma_open_pc),
                ("dma_trigger_pc", self.dma_trigger_pc),
            ] {
                if tag.abs_diff(self.current_pc) > 64 {
                    return Err(format!("bus: {name} too far from current pc"));
                }
            }
        }
        if let Some(op) = &self.hle_bios {
            op.validate().map_err(|e| format!("bus: {e}"))?;
        }
        self.ppu.validate().map_err(|e| format!("bus: {e}"))?;
        self.dma.validate().map_err(|e| format!("bus: {e}"))?;
        self.timers.validate().map_err(|e| format!("bus: {e}"))?;
        self.apu.validate().map_err(|e| format!("bus: {e}"))?;
        Ok(())
    }
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

/// I/O read outcome for the region halves: lane-adjusted value, or a
/// direct early return (open bus / prefetch fallback).
enum IoRead {
    Value(u16),
    Direct(u32),
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
            dma_bus: 0,
            dma_open_pc: 0,
            dma_trigger_pc: 0,
            dma_bus_valid: false,
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
            last_data_addr: None,
            prev_data_addr: None,
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
            bios_wait_exit_armed: false,
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
        self.tick_sound_dma();
        let event = self.tick_video();
        let timer_irq = self.tick_timers();
        let mut interrupt_mask = event.interrupt_mask | timer_irq;
        // +1-tick HBlank IRQ (see `pending_hblank_irq`): the DISPSTAT flag
        // edge stays immediate, but the IF raise waits a tick. Stash the
        // HBlank bit for the next tick start instead of raising now.
        if interrupt_mask & (1 << 1) != 0 {
            interrupt_mask &= !(1 << 1);
            self.pending_hblank_irq = true;
        }
        self.tick_sio();
        let stall_snapshot = DisplayStallSnapshot {
            forced_blank: self.ppu.forced_blank(),
            vcount: self.ppu.vcount(),
            cycle: self.ppu.cycle(),
            dispcnt: self.ppu.dispcnt(),
            bg_fetch_active: self.ppu.bg_fetch_active(),
        };
        self.tick_dma(&stall_snapshot);
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

    /// Pending-DMA countdown plus the APU native-grid tick (BIOS sound
    /// driver voices mix into the buffer tail).
    fn tick_sound_dma(&mut self) {
        self.dma.tick_pending();
        if self.apu.tick() {
            crate::sound_driver::mix_driver_grid(self);
        }
    }
    /// Video phase: countdown, PPU step, and the HBlank/VBlank/line
    /// event DMA triggers plus video-capture arming.
    fn tick_video(&mut self) -> PpuEvent {
        if self.video_countdown > 0 {
            self.video_countdown -= 1;
            if self.video_countdown == 0 {
                self.dma.trigger_channel(3, DmaTrigger::Special);
            }
        }
        let event = self
            .ppu
            .step(&self.vram[..], &self.palette_ram[..], &self.oam[..]);
        if event.hblank_dma {
            // H-Blank DMA starts only on visible scanlines (vcount < 160).
            // During V-Blank the H-Blank flag and IRQ still toggle every
            // line, but no HBlank DMA request is generated.
            if self.ppu.vcount() < 160 {
                self.dma.trigger(DmaTrigger::HBlank);
                // Per-cycle co-sim: latch the trigger-time fetch PC next
                // to the serviced-unit tag (`dma_open_pc` below), so the
                // 3-tick startup advance stays observable for the
                // trigger/active joint work (see `dma_open_bus`).
                self.dma_trigger_pc = self.current_pc;
            }
        }
        if event.vblank_started {
            self.dma.trigger(DmaTrigger::VBlank);
            self.dma_trigger_pc = self.current_pc;
        }
        if event.line_started {
            self.tick_video_line();
        }
        event
    }

    /// Line-start phase: DMA3 video-capture latch/fire bookkeeping.
    fn tick_video_line(&mut self) {
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
    /// Timer phase: step the timers and clock the sound FIFOs from
    /// timer overflows. Returns the timer IRQ mask.
    fn tick_timers(&mut self) -> u16 {
        let (timer_irq, timer_overflow) = {
            self.timers.set_current_cycle(self.current_tcycle);
            self.timers.step_full()
        };
        if timer_overflow != 0 {
            for i in 0..4 {
                if timer_overflow & (1 << i) != 0 {
                    self.tick_fifo_overflow(i);
                }
            }
        }
        timer_irq
    }

    /// One overflowing timer clocks one sample byte out of each
    /// selecting FIFO; a FIFO at 14 bytes or fewer requests its
    /// Special DMA channel. Every overflow clocks the sample stream,
    /// whether or not the timer IRQ is enabled (the IRQ bit only
    /// raises IF).
    fn tick_fifo_overflow(&mut self, i: usize) {
        if self.apu.soundcnt_x & 0x80 == 0 || i > 1 {
            return;
        }
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
    /// SIO phase: Normal-mode transfer completion (scheduled on the
    /// START edge) clears START, delivers pulled-high receive data
    /// (no link partner drives the lines low) and raises the serial
    /// IRQ when enabled. Pinned by mgba-suite sio-timing (measured =
    /// transfer cycles + a constant 121-cycle setup/exit path).
    fn tick_sio(&mut self) {
        if self.sio_xfer_cycles == 0 {
            return;
        }
        self.sio_xfer_cycles -= 1;
        if self.sio_xfer_cycles != 0 {
            return;
        }
        if self.sio_xfer_uart {
            self.tick_uart_done();
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

    /// UART frame done: the sent byte leaves, an idle-high byte
    /// arrives when receive is enabled (overrun sets the error
    /// flag), then chained frames kick.
    fn tick_uart_done(&mut self) {
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
    }
    /// DMA phase: run one DMA step and service the transfer (EEPROM
    /// serial bits, GamePak prefetch collisions, bus-latch driving,
    /// destination write, bus-ownership reset).
    fn tick_dma(&mut self, stall_snapshot: &DisplayStallSnapshot) {
        let Some(transfer) = self
            .dma
            .step(self.wait_cnt, &|addr| stall_snapshot.stall(addr))
        else {
            return;
        };
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
        // Open-bus PC tag: snapshot the in-flight instruction's bus PC
        // (`current_pc` is already the architectural PC, so the
        // read-side compare below is a plain 2/4 difference).
        self.dma_open_pc = self.current_pc;
        // GBATEK Backup Media: only 16-bit DMA3 drives the EEPROM chip;
        // other channels/widths see the window as ROM/open bus.
        let dst = transfer.destination;
        let use_eeprom = transfer.channel == 3
            && transfer.width == 2
            && self.is_eeprom()
            && (dma_in_eeprom_range(transfer.data_source) || dma_in_eeprom_range(dst));
        let value = self.dma_transfer_value(&transfer, use_eeprom);
        self.dma_transfer_write(&transfer, value, use_eeprom);
        // DMA owns the bus between CPU accesses: the CPU's next access
        // is non-sequential (GBATEK DMA owns the bus).
        self.prev_addr = None;
        self.prev_width = 0;
        self.prev_data_addr = None;
        self.last_data_addr = None;
        self.fetch_addr = None;
        self.fetch_width = 0;
        // The ROM prefetch buffer survives DMA that never touches
        // GamePak ROM (ROM contents can't change under DMA, so the
        // buffered opcodes stay valid). Only ROM-touching DMA
        // restarts the buffer.
        if crate::dma::is_rom(transfer.data_source) || crate::dma::is_rom(dst) {
            self.pf_valid = false;
            self.pf_branch_drain = false;
        }
        // Completion IRQs are raised via take_completion_interrupts
        // in `tick` (one tick after the final write).
    }
    /// Resolve this transfer's data value: EEPROM serial bits, a bus
    /// read from an accessible source, or the latched value. GamePak
    /// reads also advance the prefetch fill and drive the shared latch.
    fn dma_transfer_value(&mut self, transfer: &DmaTransfer, use_eeprom: bool) -> u32 {
        if use_eeprom && dma_in_eeprom_range(transfer.data_source) {
            // EEPROM DMA read: one response bit per 16-bit unit.
            let bit = self.next_eeprom_read_bit();
            if transfer.width == 4 {
                bit | bit << 16
            } else {
                bit
            }
        } else if transfer.data_source >= 0x02000000 {
            self.dma_bus_read_value(transfer)
        } else if transfer.width == 2 && transfer.destination & 2 != 0 {
            transfer.latched_value >> 16
        } else {
            transfer.latched_value
        }
    }

    /// Bus read for an accessible source, with GamePak prefetch
    /// collision and shared-latch driving.
    fn dma_bus_read_value(&mut self, transfer: &DmaTransfer) -> u32 {
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
        // GamePak ROM reads collide with the in-flight prefetch
        // fill: one real-tick stall on the first GamePak access
        // per burst.
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
        if dma_read_drives_latch(read_addr) {
            let latch = if transfer.width == 4 {
                value
            } else {
                let half = value & 0xFFFF;
                half | (half << 16)
            };
            self.cpu_bus = latch;
            self.dma_bus = latch;
            // The serviced unit drives the shared bus: the sticky
            // latch goes valid until CPU mapped traffic re-drives it.
            self.dma_bus_valid = true;
        }
        value
    }
    /// Write this transfer's value: EEPROM serial bits or the
    /// destination, with GamePak prefetch collision like reads.
    fn dma_transfer_write(&mut self, transfer: &DmaTransfer, value: u32, use_eeprom: bool) {
        if use_eeprom && dma_in_eeprom_range(transfer.destination) {
            // EEPROM DMA write: each unit carries serial bit(s).
            self.feed_eeprom_write(transfer.width, value);
            return;
        }
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

    pub fn dma_active(&self) -> bool {
        self.dma.is_active()
    }

    pub fn irq_pending(&self) -> bool {
        // Delayed CPU IRQ line (IME && IE&IF, ~3 ticks after
        // the request). Sampled by the CPU once per instruction.
        self.irq_line
    }

    /// Post-enable timer0 take deferral: the first take waits for a later
    /// boundary (IF stays raised, so nothing is lost).
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
        // A fresh wait starts with no exit restore pending (see
        // `bios_wait_exit_armed`); the IntrWait wake below re-arms it.
        self.bios_wait_exit_armed = false;
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
        // match.
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
                // Wake resumes through the BIOS exit branch (halt loop
                // exit / IntrWait tail return): like any taken branch, it
                // restarts the fetch stream (next fetch N), while the
                // prefetch buffer window itself survives for buffered
                // targets. IWRAM-flat code observes nothing (N == S);
                // ROM code pays one N-S on resume (Break T0 phase).
                self.invalidate_prefetch_for_branch();
                // IntrWait-family wake: the real BIOS exit path runs after
                // the wake ISR, leaving 0xE3A02004 latched (mgba-suite
                // "BIOS load"). Latch it now (covers the no-ISR IME=0
                // wake) and arm the IRQ-return restore below.
                if clear != 0 {
                    self.bios_prefetch = 0xE3A02004;
                    self.bios_wait_exit_armed = true;
                }
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

    /// T7 gate: missed-one takes complete take+entry at raise+25
    /// (the entry absorbs the sampling latency).
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

    /// T8 gate: late-sampled clean take#4s complete take+entry at raise+25
    /// (the entry absorbs the sampling latency).
    pub fn late_timer0_entry(
        &self,
        entry_opcode: u32,
        entry_next: u32,
        thumb: bool,
    ) -> Option<u32> {
        if self.irq_flags() & (1 << 3) == 0
            || self.timers.overflows_since_enable(0) != 4
            || self.timers.timer0_acks_since_enable() != 3
            || !self.timers.last_enable_fresh_reload(0)
            || self.timers.prescaler_bits(0) != 0
        {
            return None;
        }
        let latency = self.current_tcycle.saturating_sub(self.timer0_raise_tick);
        if latency >= 4 {
            return Some(25u32.saturating_sub(latency.min(25) as u32));
        }
        if latency == 3
            && (Self::is_branch_opcode(entry_opcode, thumb)
                || (Self::is_simple_opcode(entry_opcode, thumb)
                    && Self::is_simple_opcode(entry_next, thumb)))
        {
            return Some(25u32.saturating_sub(latency.min(25) as u32));
        }
        None
    }

    /// T12 gate: prescaled take#3s at latency 3 outside tight pipes enter 2
    /// more expensive (the longer middle handler re-quantizes take#4 sampling).
    pub fn resampled_timer0_entry(&self, entry_opcode: u32, entry_next: u32, thumb: bool) -> bool {
        !thumb
            && self.irq_flags() & (1 << 3) != 0
            && self.timers.overflows_since_enable(0) == 3
            && self.timers.timer0_acks_since_enable() == 2
            && self.timers.last_enable_fresh_reload(0)
            && self.timers.prescaler_bits(0) != 0
            && self.current_tcycle.saturating_sub(self.timer0_raise_tick) == 3
            && ((!(entry_opcode & 0x0C000000 == 0x04000000)
                && !(Self::is_simple_opcode(entry_opcode, false)
                    && entry_next & 0x0C000000 == 0x04000000))
                || (Self::is_simple_opcode(entry_opcode, false)
                    && entry_next & 0x0C000000 == 0x04000000
                    && self.timers.take1_latency() == Some(5)))
    }

    /// T10 gate: clean take#3s before a transfer enter 24 more expensive
    /// (the pending transfer shifts the poll grid one loose iteration).
    pub fn brokenpipe_timer0_entry(&self, entry_opcode: u32, entry_next: u32, thumb: bool) -> bool {
        !thumb
            && self.irq_flags() & (1 << 3) != 0
            && self.timers.overflows_since_enable(0) == 3
            && self.timers.timer0_acks_since_enable() == 2
            && self.timers.last_enable_fresh_reload(0)
            && self.timers.prescaler_bits(0) == 0
            && Self::is_simple_opcode(entry_opcode, false)
            && (entry_next & 0x0C000000 == 0x04000000)
    }

    /// T11 gate: clean full-pipe take#3s after a latency-5 take#1 enter one
    /// loose iteration more expensive (take#2 keeps T5; only take#3 lengthens).
    pub fn history_timer0_entry(&self, entry_opcode: u32, entry_next: u32, thumb: bool) -> bool {
        !thumb
            && self.irq_flags() & (1 << 3) != 0
            && self.timers.overflows_since_enable(0) == 3
            && self.timers.timer0_acks_since_enable() == 2
            && self.timers.last_enable_fresh_reload(0)
            && self.timers.prescaler_bits(0) == 0
            && self.timers.take1_latency() == Some(5)
            && Self::is_simple_opcode(entry_opcode, false)
            && Self::is_simple_opcode(entry_next, false)
    }

    /// Record the run's first timer0 take latency (take#1 grid proxy for
    /// the T11 gate). Call on every non-deferred timer0 take; the gate
    /// reads only the latest first-take.
    pub fn record_take1_latency(&mut self) {
        if self.irq_flags() & (1 << 3) != 0
            && self.timers.overflows_since_enable(0) == 1
            && self.timers.timer0_acks_since_enable() == 0
        {
            let latency = self.current_tcycle.saturating_sub(self.timer0_raise_tick);
            self.timers.record_take1_latency(latency);
        }
    }

    /// Whether an in-flight opcode is single-cycle ALU (an undisturbed
    /// pipe of these drains cleanly on a take). Reads opcode class only,
    /// never addresses.
    fn is_simple_opcode(opcode: u32, thumb: bool) -> bool {
        if thumb {
            let op = (opcode & 0xFFFF) as u16;
            // Shift-imm, add/sub reg+imm, mov/cmp/add/sub imm, ALU ops,
            // and Hi-reg mov/cmp/add (not BX).
            (op & 0xE000 == 0x0000)
                || (op & 0xF800 == 0x1800)
                || (op & 0xE000 == 0x2000)
                || (op & 0xFC00 == 0x4000)
                || (op & 0xFF00 == 0x4400)
                || (op & 0xFF00 == 0x4500)
                || (op & 0xFF00 == 0x4600)
        } else {
            // Data processing except multiply and status moves.
            (opcode & 0x0C000000 == 0x00000000)
                && (opcode & 0x0FC000F0 != 0x00000090)
                && (opcode & 0x0FBF0000 != 0x010F0000)
                && (opcode & 0x0FBFFFF0 != 0x0129F000)
                && (opcode & 0x0FBFFFF0 != 0x012BF000)
        }
    }

    /// Whether an in-flight opcode is a branch (take-entry overlap: a
    /// take interrupting a branch shares the pipe flush with the vector
    /// fetch). Reads opcode class only, never addresses.
    fn is_branch_opcode(opcode: u32, thumb: bool) -> bool {
        if thumb {
            let op = (opcode & 0xFFFF) as u16;
            // Conditional branch, unconditional branch, or BX.
            (op & 0xF000 == 0xD000) || (op & 0xF800 == 0xE000) || (op & 0xFF87 == 0x4700)
        } else {
            // B/BL (any condition) or BX.
            (opcode & 0x0E000000 == 0x0A000000) || (opcode & 0x0FFFFFF0 == 0x012FFF10)
        }
    }

    /// T5 gate: established timer0 re-takes (third overflow onward from
    /// a fresh atomic prescaler-0 enable) enter 1 more expensive. Storm
    /// take#2+ phase pins +1 on exactly this class (2i values ran
    /// systematic -1 with the T4-only model); first takes keep T4/full
    /// entry, and non-timer0 takes never match (timer0 IF required).
    pub fn retook_timer0_entry(&self) -> bool {
        self.irq_flags() & (1 << 3) != 0
            && self.timers.overflows_since_enable(0) >= 3
            && self.timers.last_enable_fresh_reload(0)
            && self.timers.prescaler_bits(0) == 0
    }

    /// T4 gate: fresh atomic-enable ps0 takes enter 3 cheaper; every
    /// condition is load-bearing (ablation-pinned). `woke` is the
    /// halt-wake state taken once per IRQ entry by `service_irq`
    /// (a woken take never discounts).
    pub fn discount_timer0_entry(&self, woke: bool) -> bool {
        !woke
            && self.irq_flags() & (1 << 3) != 0
            && self.timers.overflows_since_enable(0) <= 2
            && self.timers.last_enable_fresh_reload(0)
            && self.timers.prescaler_bits(0) == 0
    }

    /// Take the halt-wake edge for one IRQ entry (per-cycle co-sim: at
    /// most one entry observes it; the flag never leaks past the take).
    pub fn take_woke_from_halt(&mut self) -> bool {
        std::mem::take(&mut self.woke_from_halt)
    }

    /// HLE IRQ return skips the real-BIOS restore sequence; fitted epilogue cost.
    /// Recalibrate against the timers and timing suites if this changes.
    const HLE_IRQ_EPILOGUE_CYCLES: u32 = 7;

    /// Skipped BIOS vector+prologue cycle count (region-independent): vector
    /// fetch pair, branch refill, push6, mov, adr, ldr-pc, exception entry
    /// internals, and the base cycles no HLE instruction absorbs. Anchored to
    /// the HW-pinned IWRAM-handler entry total; region dependence now comes
    /// from the real entry bus part, not a source-region term.
    /// (`bios_irq_prologue_cycles` below recomputes this total instruction
    /// by instruction; keep the two in sync.)
    const HLE_IRQ_PROLOGUE_CYCLES: u32 = 23;
    /// Anchor alias for the as-code prologue residual (see above).
    const HLE_IRQ_PROLOGUE_ANCHOR: u32 = Self::HLE_IRQ_PROLOGUE_CYCLES;

    /// HLE-as-code BIOS IRQ prologue: the real BIOS vector + 0x128
    /// handler modeled instruction by instruction with live bus costs
    /// (fetches, STMFD stack stores, IF/IE + IntrMain-address loads),
    /// plus the non-bus exception microcode residual. Anchored so the
    /// IWRAM-handler total equals HLE_IRQ_PROLOGUE_ANCHOR (23); region
    /// dependence arrives via the live `cycles_for` terms. `woke` is
    /// accepted for the T4-gate call shape but carries no discount here:
    /// the IRQ line trails the wake by 2 ticks, so the woken thread has
    /// refilled the pipe before entry (no flush to skip).
    pub fn bios_irq_prologue_cycles(&self, _woke: bool) -> u32 {
        // Exception microcode (CPSR save, mode switch, LR): 3 internal.
        let mut total = 3u32;
        // Vector fetch at 0x18 + branch refill at 0x128 (BIOS ROM).
        total += u32::from(self.cycles_for(0x00000018, 4));
        total += 2 * u32::from(self.cycles_for(0x00000128, 4));
        // STMFD SP_irq!, {r0-r3,r12,lr}: 6 IWRAM stores.
        total += 6 * u32::from(self.cycles_for(0x03007FA0, 4));
        // MOV + ADD (no bus): 2 internal.
        total += 2;
        // IF/IE I/O loads + IntrMain-address IWRAM load.
        total += u32::from(self.cycles_for(0x04000202, 2));
        total += u32::from(self.cycles_for(0x04000200, 2));
        total += u32::from(self.cycles_for(0x03007FFC, 4));
        // Residual (pipeline-flush overlap, arbitration, anchored fit):
        // calibrated so the IWRAM-handler total is HLE_IRQ_PROLOGUE_ANCHOR
        // (3 + 1 + 2 + 6 + 2 + 1 + 1 + 1 = 17 live, + residual).
        total += Self::HLE_IRQ_PROLOGUE_ANCHOR - 17;
        total
    }

    /// HLE-as-code BIOS IRQ epilogue: the restore sequence (LDMFD +
    /// CPSR restore + return branch) with live bus costs. Anchored to
    /// `HLE_IRQ_EPILOGUE_CYCLES` (7) for all paths today; the wake/halt
    /// distinction (if any) must be re-pinned against the timer suites
    /// before varying it.
    pub fn bios_irq_epilogue_cycles(&self) -> u32 {
        let loads = 6 * u32::from(self.cycles_for(0x03007FA0, 4));
        let congestion: u32 = Self::HLE_IRQ_EPILOGUE_CYCLES;
        // Neutral today: the anchored total wins; the live bus term only
        // documents the as-code shape (IWRAM restores) for the follow-up.
        let _ = loads;
        congestion
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
            self.prev_data_addr = None;
            self.last_data_addr = None;
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

    pub(crate) fn export_state(&self) -> Result<GbaMemoryBusState, String> {
        Ok(GbaMemoryBusState {
            bios: serde_bytes::ByteBuf::from(self.bios.to_vec()),
            ewram: serde_bytes::ByteBuf::from(self.ewram.to_vec()),
            iwram: serde_bytes::ByteBuf::from(self.iwram.to_vec()),
            palette_ram: serde_bytes::ByteBuf::from(self.palette_ram.to_vec()),
            vram: serde_bytes::ByteBuf::from(self.vram.to_vec()),
            oam: serde_bytes::ByteBuf::from(self.oam.to_vec()),
            wait_cnt: self.wait_cnt,
            ie: self.ie,
            sif: self.sif,
            ime: self.ime,
            pending_ie: self.pending_ie,
            pending_ime: self.pending_ime,
            pending_if: self.pending_if,
            pending_at: self.pending_at,
            line_write_assert: self.line_write_assert,
            irq_available: self.irq_available,
            avail_queue: self.avail_queue.clone(),
            irq_line: self.irq_line,
            line_queue: self.line_queue.clone(),
            postflg: self.postflg,
            mem_control: self.mem_control,
            keyinput: self.keyinput,
            keycnt: self.keycnt,
            siocnt: self.siocnt,
            siodata8: self.siodata8,
            siodata32: self.siodata32,
            rcnt: self.rcnt,
            joycnt: self.joycnt,
            sio_xfer_cycles: self.sio_xfer_cycles,
            sio_xfer_32: self.sio_xfer_32,
            sio_xfer_uart: self.sio_xfer_uart,
            uart_tx: self.uart_tx.iter().copied().collect(),
            uart_rx: self.uart_rx.iter().copied().collect(),
            uart_err: self.uart_err,
            uart_prev_irqsrc: self.uart_prev_irqsrc,
            last_prefetch: self.last_prefetch,
            prefetch_win: self.prefetch_win,
            prefetch_thumb: self.prefetch_thumb,
            cpu_bus: self.cpu_bus,
            dma_bus: self.dma_bus,
            dma_bus_valid: self.dma_bus_valid,
            dma_open_pc: self.dma_open_pc,
            dma_trigger_pc: self.dma_trigger_pc,
            prefetch_enabled: self.prefetch_enabled,
            pf_start: self.pf_start,
            pf_end: self.pf_end,
            pf_valid: self.pf_valid,
            pf_branch_drain: self.pf_branch_drain,
            fill_countdown: self.fill_countdown,
            last_prefetched_pc: self.last_prefetched_pc,
            bios_prefetch: self.bios_prefetch,
            data_sequential_override: self.data_sequential_override,
            block_batching: self.block_batching,
            block_batch_any: self.block_batch_any,
            block_batch_words: self.block_batch_words,
            block_batch_erase_sum: self.block_batch_erase_sum,
            block_batch_is_load: self.block_batch_is_load,
            block_batch_fetch_width: self.block_batch_fetch_width,
            block_batch_has_rom: self.block_batch_has_rom,
            block_batch_raw: self.block_batch_raw,
            access_wait_cycles: self.access_wait_cycles,
            last_opcode_addr: self.last_opcode_addr,
            last_data_addr: self.last_data_addr,
            prev_data_addr: self.prev_data_addr,
            prev_load_pc: self.prev_load_pc,
            prev_load_is_stack: self.prev_load_is_stack,
            current_pc: self.current_pc,
            prev_addr: self.prev_addr,
            prev_width: self.prev_width,
            fetch_addr: self.fetch_addr,
            fetch_width: self.fetch_width,
            halted: self.halted,
            halt_irq_mask: self.halt_irq_mask,
            stopped: self.stopped,
            wake_clear_mask: self.wake_clear_mask,
            wake_latency: self.wake_latency,
            woke_from_halt: self.woke_from_halt,
            dma_stall_pending: self.dma_stall_pending,
            bios_wait_exit_armed: self.bios_wait_exit_armed,
            current_tcycle: self.current_tcycle,
            timer0_raise_tick: self.timer0_raise_tick,
            bios_protect: self.bios_protect,
            video_armed: self.video_armed,
            video_countdown: self.video_countdown,
            pending_hblank_irq: self.pending_hblank_irq,
            eeprom_burst_open: self.eeprom_burst_open,
            ppu: self.ppu.export_state(),
            dma: self.dma.export_state(),
            timers: self.timers.export_state(),
            apu: self.apu.export_state().map_err(|e| format!("bus: {e}"))?,
            cartridge: self.cartridge.as_ref().map(|cart| cart.export_state()),
            hle_bios: self.hle_bios,
        })
    }

    pub(crate) fn import_state(&mut self, state: GbaMemoryBusState) -> Result<(), String> {
        state.validate()?;
        self.bios.copy_from_slice(&state.bios);
        self.ewram.copy_from_slice(&state.ewram);
        self.iwram.copy_from_slice(&state.iwram);
        self.palette_ram.copy_from_slice(&state.palette_ram);
        self.vram.copy_from_slice(&state.vram);
        self.oam.copy_from_slice(&state.oam);
        self.wait_cnt = state.wait_cnt;
        self.ie = state.ie;
        self.sif = state.sif;
        self.ime = state.ime;
        self.pending_ie = state.pending_ie;
        self.pending_ime = state.pending_ime;
        self.pending_if = state.pending_if;
        self.pending_at = state.pending_at;
        self.line_write_assert = state.line_write_assert;
        self.irq_available = state.irq_available;
        self.avail_queue = state.avail_queue;
        self.irq_line = state.irq_line;
        self.line_queue = state.line_queue;
        self.postflg = state.postflg;
        self.mem_control = state.mem_control;
        self.keyinput = state.keyinput;
        self.keycnt = state.keycnt;
        self.siocnt = state.siocnt;
        self.siodata8 = state.siodata8;
        self.siodata32 = state.siodata32;
        self.rcnt = state.rcnt;
        self.joycnt = state.joycnt;
        self.sio_xfer_cycles = state.sio_xfer_cycles;
        self.sio_xfer_32 = state.sio_xfer_32;
        self.sio_xfer_uart = state.sio_xfer_uart;
        self.uart_tx = state.uart_tx.into_iter().collect();
        self.uart_rx = state.uart_rx.into_iter().collect();
        self.uart_err = state.uart_err;
        self.uart_prev_irqsrc = state.uart_prev_irqsrc;
        self.last_prefetch = state.last_prefetch;
        self.prefetch_win = state.prefetch_win;
        self.prefetch_thumb = state.prefetch_thumb;
        self.cpu_bus = state.cpu_bus;
        self.dma_bus = state.dma_bus;
        self.dma_bus_valid = state.dma_bus_valid;
        self.dma_open_pc = state.dma_open_pc;
        self.dma_trigger_pc = state.dma_trigger_pc;
        self.prefetch_enabled = state.prefetch_enabled;
        self.pf_start = state.pf_start;
        self.pf_end = state.pf_end;
        self.pf_valid = state.pf_valid;
        self.pf_branch_drain = state.pf_branch_drain;
        self.fill_countdown = state.fill_countdown;
        self.last_prefetched_pc = state.last_prefetched_pc;
        self.bios_prefetch = state.bios_prefetch;
        self.data_sequential_override = state.data_sequential_override;
        self.block_batching = state.block_batching;
        self.block_batch_any = state.block_batch_any;
        self.block_batch_words = state.block_batch_words;
        self.block_batch_erase_sum = state.block_batch_erase_sum;
        self.block_batch_is_load = state.block_batch_is_load;
        self.block_batch_fetch_width = state.block_batch_fetch_width;
        self.block_batch_has_rom = state.block_batch_has_rom;
        self.block_batch_raw = state.block_batch_raw;
        self.access_wait_cycles = state.access_wait_cycles;
        self.last_opcode_addr = state.last_opcode_addr;
        self.last_data_addr = state.last_data_addr;
        self.prev_data_addr = state.prev_data_addr;
        self.prev_load_pc = state.prev_load_pc;
        self.prev_load_is_stack = state.prev_load_is_stack;
        self.current_pc = state.current_pc;
        self.prev_addr = state.prev_addr;
        self.prev_width = state.prev_width;
        self.fetch_addr = state.fetch_addr;
        self.fetch_width = state.fetch_width;
        self.halted = state.halted;
        self.halt_irq_mask = state.halt_irq_mask;
        self.stopped = state.stopped;
        self.wake_clear_mask = state.wake_clear_mask;
        self.wake_latency = state.wake_latency;
        self.woke_from_halt = state.woke_from_halt;
        self.dma_stall_pending = state.dma_stall_pending;
        self.bios_wait_exit_armed = state.bios_wait_exit_armed;
        self.current_tcycle = state.current_tcycle;
        self.timer0_raise_tick = state.timer0_raise_tick;
        self.bios_protect = state.bios_protect;
        self.video_armed = state.video_armed;
        self.video_countdown = state.video_countdown;
        self.pending_hblank_irq = state.pending_hblank_irq;
        self.eeprom_burst_open = state.eeprom_burst_open;
        self.ppu.import_state(state.ppu)?;
        self.dma.import_state(state.dma)?;
        self.timers.import_state(state.timers)?;
        self.apu
            .import_state(state.apu)
            .map_err(|e| format!("bus: {e}"))?;
        match (state.cartridge, self.cartridge.as_mut()) {
            (Some(cart_state), Some(cart)) => cart.import_state(&cart_state)?,
            (None, None) => {}
            _ => return Err("bus: cartridge presence mismatch".to_string()),
        }
        self.hle_bios = state.hle_bios;
        Ok(())
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

    /// Open-bus gate: true when the executing instruction directly
    /// follows the instruction that was in flight at the last serviced DMA
    /// unit (`current_pc` is the architectural PC on both sides).
    /// (Per-cycle co-sim keeps the trigger-time tag in `dma_trigger_pc`
    /// for phase analysis; the read gate stays on the active tag until
    /// the trigger/active joint is re-pinned — OR-ing it moved Break
    /// +1 iter the wrong way in the first measurement.)
    fn dma_open_bus(&self) -> bool {
        let step = if self.prefetch_thumb { 2 } else { 4 };
        let _ = self.dma_trigger_pc;
        self.current_pc.wrapping_sub(self.dma_open_pc) == step
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
            // Unmapped: prefetch-latch open bus, lane-selected by width —
            // except while the sticky DMA latch is valid (a serviced DMA
            // unit drove the shared bus and no CPU mapped traffic has
            // re-driven it since), or for the instruction immediately
            // following a serviced DMA unit (the PC tag matches the next
            // instruction; sub-word reads shift by address lane exactly
            // like the prefetch path).
            _ => {
                if self.dma_open_bus() || self.dma_bus_valid {
                    match width {
                        4 => self.dma_bus,
                        2 => (self.dma_bus >> ((addr & 2) * 8)) & 0xFFFF,
                        _ => (self.dma_bus >> ((addr & 3) * 8)) & 0xFF,
                    }
                } else {
                    match width {
                        4 => self.open_bus32(),
                        2 => self.open_bus16(addr),
                        _ => self.open_bus8(addr),
                    }
                }
            }
        }
    }

    fn read_bios_guarded(&mut self, addr: u32, width: u8) -> u32 {
        if self.bios_protect && !(0x00000000..=0x00003FFF).contains(&self.current_pc) {
            // A protected read returns the latched last BIOS-fetched opcode,
            // same value on repeats; refreshed when BIOS execution is left.
            // Sub-word reads select their lane of the 32-bit latch
            // (mgba-suite "BIOS load" pins byte@1 = 0x20 of 0xE3A02004;
            // halfword readers apply their own ROR on the low half).
            let raw = self.bios_prefetch;
            let aligned = match width {
                4 => raw,
                2 => raw & 0xFFFF,
                _ => (raw >> ((addr & 3) * 8)) & 0xFF,
            };
            let _ = addr;
            aligned
        } else {
            self.read_bios(addr, width)
        }
    }

    /// Consume a pending IntrWait-exit latch restore (see
    /// `bios_wait_exit_armed`): true when the next IRQ return must latch
    /// the BIOS exit opcode instead of the HLE epilogue opcode.
    pub fn take_bios_wait_exit(&mut self) -> bool {
        std::mem::take(&mut self.bios_wait_exit_armed)
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
        self.prev_data_addr = None;
        self.last_data_addr = None;
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
        self.prev_data_addr = None;
        self.last_data_addr = None;
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
        if is_opcode && self.prefetch_enabled && (0x08000000..=0x0DFFFFFF).contains(&addr) {
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
        // A mapped CPU data access re-drives the shared bus: the sticky
        // DMA latch stops being valid (fetches and unmapped reads only
        // sample it).
        if !is_opcode && self.cpu_data_drives_bus(addr) {
            self.dma_bus_valid = false;
        }
        // prev_* tracks the last bus access of ANY kind (GBATEK N/S bus
        // order); fetch_* tracks the opcode stream for the prefetch-ON
        // fetch path above.
        self.prev_addr = Some(addr);
        self.prev_width = width;
        if !is_opcode {
            self.prev_data_addr = self.last_data_addr;
            self.last_data_addr = Some(addr);
        }
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
    /// instruction as N32-S32 of the owning code region. High-unmapped
    /// open-bus reads (>=0x10000000, misc_edge Break's 0x10000000 loop)
    /// idle the bus for a fill slot and pre-pay nothing; OAM-mirror
    /// open-bus blocks (Timing OAM cells pin their break) and all mapped
    /// data (including IWRAM, pinned by Timing LDR cells) still break.
    /// Callers pass the data address.
    pub(crate) fn charge_fetch_stream_break(&mut self, data_addr: u32) {
        if let Some(pc) = self.last_opcode_addr
            && (0x08000000..=0x0DFFFFFF).contains(&pc)
            && data_addr < 0x1000_0000
            // Back-to-back unmapped-load + store (Break's ldmia
            // 0x10000000 + str IWRAM) pre-pays once: the load already
            // idled the bus, the store's break is absorbed. Isolated
            // Timing accesses (prev mapped) keep their break.
            && !matches!(self.prev_data_addr, Some(prev) if prev >= 0x1000_0000)
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
        // Mapped CPU stores drive the bus (see `cpu_data_drives_bus`):
        // unmapped-vanishing stores never publish.
        if self.cpu_data_drives_bus(addr) {
            self.dma_bus_valid = false;
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
        self.prev_data_addr = self.last_data_addr;
        self.last_data_addr = Some(addr);
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
        // Region halves (Direct = unmapped/write-only open-bus return,
        // skipping the lane adjust below like the legacy early return).
        let out = if aligned < 0x04000100 {
            self.read_io_low(aligned)
        } else {
            self.read_io_high(aligned)
        };
        let val: u16 = match out {
            IoRead::Value(v) => v,
            IoRead::Direct(d) => return d,
        };
        if width == 1 && (addr & 1) == 1 {
            ((val >> 8) & 0xFF) as u32
        } else {
            val as u32
        }
    }
    /// Video/APU half of I/O reads.
    fn read_io_low(&mut self, aligned: u32) -> IoRead {
        let val = match aligned {
            0x04000000..=0x04000006 | 0x04000008..=0x0400000E | 0x04000048..=0x04000052 => {
                match self.read_io_ppu(aligned) {
                    Some(v) => v,
                    // Write-only register (MOSAIC, BLDY, HOFS, affine, WINH/V, ...)
                    None => return IoRead::Direct(self.open_bus16(aligned)),
                }
            }
            0x040000B0..=0x040000DE => self.read_io_dma(aligned),
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
            // Unused APU holes read 0 (mgba-suite io-read table).
            0x04000066 | 0x0400006A | 0x0400006E | 0x04000076 | 0x0400007A | 0x0400007E
            | 0x04000086 | 0x0400008A => 0,
            _ => return IoRead::Direct(self.last_prefetch & 0xFFFF),
        };
        IoRead::Value(val)
    }

    /// PPU display registers; None for write-only registers.
    fn read_io_ppu(&mut self, aligned: u32) -> Option<u16> {
        self.ppu.read_register(aligned)
    }

    /// DMA registers: SAD/DAD are write-only (reads see open bus);
    /// CNT_L reads back 0, CNT_H reads the latched control.
    fn read_io_dma(&self, aligned: u32) -> u16 {
        if matches!(aligned, 0x040000BA | 0x040000C6 | 0x040000D2 | 0x040000DE) {
            self.dma.read(aligned).unwrap_or(0)
        } else if matches!(aligned, 0x040000B8 | 0x040000C4 | 0x040000D0 | 0x040000DC) {
            0
        } else {
            self.open_bus16(aligned) as u16
        }
    }
    /// Timer/SIO/key/joy/IRQ half of I/O reads.
    fn read_io_high(&mut self, aligned: u32) -> IoRead {
        let val = match aligned {
            0x04000100..=0x0400010E => {
                if std::env::var("GBA_TTRACE").is_ok() && aligned == 0x04000100 {
                    eprintln!("T tmread @{}", self.current_tcycle);
                }
                self.timers.read(aligned).unwrap_or(0)
            }
            0x04000120 | 0x04000122 | 0x04000128 | 0x0400012A => self.read_io_sio(aligned),
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
            0x04000136 | 0x04000142 | 0x0400015A | 0x04000206 | 0x0400020A | 0x04000302 => 0,
            _ => {
                // Unimplemented/write-only registers return the recently
                // prefetched opcode, not the last written value (GBATEK
                // "Unpredictable Things": open bus tracks the prefetch,
                // with a zero-mix rule for partially-readable ports that
                // is not modeled here).
                return IoRead::Direct(self.last_prefetch & 0xFFFF);
            }
        };
        IoRead::Value(val)
    }

    /// SIO block reads (0x120-0x12A).
    fn read_io_sio(&mut self, aligned: u32) -> u16 {
        match aligned {
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
            0x04000122 if self.sio_block_selected() && Self::sio_submode(self.siocnt) == 1 => {
                ((self.siodata32 >> 16) & 0xFFFF) as u16
            }
            // Only the four SIO registers above reach here (the
            // dispatcher routes 0x124/0x126 to 0 directly).
            _ => 0,
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
        if self.write_syscnt(addr, width, value, bios) {
            return;
        }
        if width == 4 {
            self.write_io(addr, 2, value & 0xFFFF, bios);
            self.write_io(addr + 2, 2, value >> 16, bios);
            return;
        }
        let aligned = addr & !1;
        // The I/O bus is 16-bit: a sub-word store must preserve the
        // untouched lane instead of zeroing it.
        let v16 = self.write_merge_lane(addr, aligned, width, value);
        self.write_io_reg(addr, aligned, width, value, v16);
        // 32bit書き込みで2レジスタ跨ぎの場合、上位側も反映されるが簡易実装では上記で十分
        let _ = width;
    }

    /// POSTFLG/HALTCNT system-control writes. True when handled.
    /// POSTFLG/HALTCNT are BIOS-gated (confirmed by hw-test ROM
    /// haltcnt): CPU writes from outside the BIOS are ignored;
    /// HLE BIOS and DMA writes act. POSTFLG is set-only; HALTCNT bit
    /// 7 = 0 halts, bit 7 = 1 stops (CPU parked until IRQ in both).
    fn write_syscnt(&mut self, addr: u32, width: u8, value: u32, bios: bool) -> bool {
        if width > 1 && addr == 0x04000300 {
            self.write_haltcnt_word(value, bios);
            return true;
        }
        if width != 1 {
            return false;
        }
        match addr {
            0x04000300 => {
                let gated = bios || self.current_pc <= 0x3FFF;
                if gated {
                    self.postflg |= (value & 1) as u8;
                }
                true
            }
            0x04000301 => {
                self.write_haltcnt_byte(value, bios);
                true
            }
            _ => false,
        }
    }

    /// Word-size HALTCNT block (see `write_syscnt` for the gating).
    fn write_haltcnt_word(&mut self, value: u32, bios: bool) {
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
    }

    /// Byte-size HALTCNT block (see `write_syscnt` for the gating).
    fn write_haltcnt_byte(&mut self, value: u32, bios: bool) {
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
    }

    /// Merge a sub-word store lane over the current halfword.
    fn write_merge_lane(&mut self, addr: u32, aligned: u32, width: u8, value: u32) -> u16 {
        if width != 1 {
            return value as u16;
        }
        let shift = (addr & 1) * 8;
        let lane = (value & 0xFF) << shift;
        let cur = if (0x040000B0..=0x040000DE).contains(&aligned) {
            u32::from(self.dma.read(aligned).unwrap_or(0))
        } else {
            self.read_io(aligned, 2)
        };
        ((cur & !(0xFF << shift)) | lane) as u16
    }

    /// Dispatch a merged halfword write to the region halves.
    fn write_io_reg(&mut self, addr: u32, aligned: u32, width: u8, value: u32, v16: u16) {
        if aligned < 0x040000B0 {
            self.write_io_low(aligned, width, value, v16);
        } else {
            self.write_io_high(addr, aligned, width, value, v16);
        }
    }

    /// Video/APU half of I/O writes.
    fn write_io_low(&mut self, aligned: u32, width: u8, value: u32, v16: u16) {
        match aligned {
            0x04000000..=0x04000054 if aligned != 0x04000006 => {
                let irq = self.ppu.write_register(aligned, v16);
                if irq != 0 {
                    self.request_interrupt(irq);
                }
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
            // Lower-half gaps (VCOUNT, unused holes) ignore writes.
            _ => {}
        }
    }

    /// DMA/timer/SIO/key/joy/IRQ half of I/O writes.
    fn write_io_high(&mut self, addr: u32, aligned: u32, width: u8, value: u32, v16: u16) {
        match aligned {
            0x040000B0..=0x040000DE => self.write_io_dma(aligned, v16),
            0x04000100..=0x0400010E => {
                if std::env::var("GBA_TTRACE").is_ok() && aligned == 0x04000102 && v16 & 0x80 != 0 {
                    eprintln!("T start @{}", self.current_tcycle);
                }
                self.timers.write(aligned, v16);
            }
            0x04000128 => self.write_siocnt(v16),
            // 0x12A latches except in UART mode, where only the low byte
            // reaches the send FIFO (GBATEK: upper 8 bits unused).
            0x0400012A | 0x04000120 | 0x04000122 => self.write_io_sio(aligned, width, value, v16),
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
            }
            0x04000202 => self.write_if_ack(addr, width, value),
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
            }
            _ => {
                // 未実装レジスタへの書き込みは open_bus のみ更新
            }
        }
    }

    /// DMA control writes plus Immediate CNT_H arming retime.
    fn write_io_dma(&mut self, aligned: u32, v16: u16) {
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
        // DMA GamePak fill-collision arbitration: ROM-touching DMA
        // accesses stall one real tick on the first GamePak access
        // per burst (fill clock spans handover idle + bus ticks).
    }

    /// SIO data writes (0x12A/0x120/0x122).
    fn write_io_sio(&mut self, aligned: u32, width: u8, value: u32, v16: u16) {
        match aligned {
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
            // Only 0x122 remains (the dispatcher covers 12A/120/122).
            _ => {
                if self.sio_block_selected() && Self::sio_submode(self.siocnt) == 1 {
                    self.siodata32 = (self.siodata32 & 0x0000FFFF) | ((v16 as u32) << 16);
                }
            }
        }
    }

    /// IF acknowledge: only written 1-bits clear.
    /// A byte store acks its lane only, not the merged halfword.
    fn write_if_ack(&mut self, addr: u32, width: u8, value: u32) {
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

    /// True when a CPU data access to `addr` drives the external data bus
    /// with its own value, re-driving the shared open-bus latch (so the
    /// sticky DMA latch stops being valid): real RAM/ROM (cartridge
    /// present), I/O registers and backup chips. Unmapped gaps, floating
    /// windows (cartridge-less ROM/SRAM, the EEPROM SRAM window) and the
    /// test debug sink only sample the bus and leave the latch alone.
    /// Opcode fetches never consult this (the prefetch unit buffers them).
    fn cpu_data_drives_bus(&self, addr: u32) -> bool {
        match addr {
            0x00000000..=0x00003FFF => true,
            0x02000000..=0x02FFFFFF => true,
            0x03000000..=0x03FFFFFF => true,
            0x04000000..=0x040003FE => true,
            a if is_mem_control(a) => true,
            0x05000000..=0x05FFFFFF => true,
            0x06000000..=0x06FFFFFF => true,
            0x07000000..=0x07FFFFFF => true,
            0x08000000..=0x0CFFFFFF => self.cartridge.is_some(),
            0x0D000000..=0x0DFFFFFF => true,
            0x0E000000..=0x0FFFFFFF => self.cartridge.as_ref().is_some_and(|c| {
                !matches!(
                    c.save_type(),
                    crate::cartridge::save::SaveType::Eeprom512
                        | crate::cartridge::save::SaveType::Eeprom8k
                )
            }),
            _ => false,
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

/// `HleBiosBus` lives in `bios` and `SoundDriverBus` is a crate-top leaf:
/// both only name these traits, so the file dependency runs
/// `memory -> bios::hle_operation`, `memory -> sound_driver` and never back
/// (see those modules).
impl HleBiosBus for GbaMemoryBus {
    fn read8(&mut self, addr: u32) -> u8 {
        GbaMemoryBus::read8(self, addr)
    }

    fn read16(&mut self, addr: u32) -> u16 {
        GbaMemoryBus::read16(self, addr)
    }

    fn read32(&mut self, addr: u32) -> u32 {
        GbaMemoryBus::read32(self, addr)
    }

    fn write_hle_bios16(&mut self, addr: u32, value: u16) {
        GbaMemoryBus::write_hle_bios16(self, addr, value);
    }

    fn write_hle_bios32(&mut self, addr: u32, value: u32) {
        GbaMemoryBus::write_hle_bios32(self, addr, value);
    }
}

impl SoundDriverBus for GbaMemoryBus {
    fn read8(&mut self, addr: u32) -> u8 {
        GbaMemoryBus::read8(self, addr)
    }

    fn read16(&mut self, addr: u32) -> u16 {
        GbaMemoryBus::read16(self, addr)
    }

    fn read32(&mut self, addr: u32) -> u32 {
        GbaMemoryBus::read32(self, addr)
    }

    fn write_hle_bios8(&mut self, addr: u32, value: u8) {
        GbaMemoryBus::write_hle_bios8(self, addr, value);
    }

    fn apu(&self) -> &GbaApu {
        GbaMemoryBus::apu(self)
    }

    fn apu_mut(&mut self) -> &mut GbaApu {
        GbaMemoryBus::apu_mut(self)
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

/// GBATEK Backup Media window for the EEPROM serial chip.
fn dma_in_eeprom_range(addr: u32) -> bool {
    (0x0D000000..=0x0DFFFFFF).contains(&addr)
}

/// A DMA read drives the shared bus latch unless its source is
/// inaccessible (unreadable I/O, or non-mem-control high I/O).
fn dma_read_drives_latch(read_addr: u32) -> bool {
    !(is_unreadable_io(read_addr)
        || ((0x04000800..=0x04FFFFFF).contains(&read_addr) && !is_mem_control(read_addr)))
}

#[cfg(test)]
mod tests;
