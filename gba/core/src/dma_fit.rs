//! Native replication of nba hw-test DMA/PPU timing measurements.
//!
//! Executes the exact ROM instruction sequences (128kb-boundary) or the
//! exact DMA programs (basic/exact-timing sampling) and asserts the
//! hardware constants embedded in the test sources. This gives a joint
//! fit target for DMA timing: 128kb-boundary (18 constants) plus the
//! HBlank/video burst rates derived from basic/exact-timing.

use crate::system::GbaSystem;

const ROM_128KB: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../roms/gba/nba-emu_hw-test/bus/128kb-boundary/128kb-boundary.gba"
);
// The measured functions are IWRAM_CODE: at boot the CRT copies them from
// ROM to IWRAM and they execute there (1-cycle fetches). Replicate that by
// copying the contiguous ROM span (code + adjacent literals, all
// PC-relative accesses preserved) to IWRAM and executing the copy.
const ROM_SPAN_START: usize = 0x142B0;
const ROM_SPAN_END: usize = 0x8014340 - 0x08000000; // include bx-r3 trampoline
const COPY_BASE: u32 = 0x03000000;
const TIME_FN: u32 = COPY_BASE + (0x08014314 - 0x080142B0);
// Function *pointers* (loaded into r0 then `bx`ed) need the THUMB bit set.
const LDM_FN: u32 = COPY_BASE + 1;
const DMA_INC_FN: u32 = COPY_BASE + (0x080142B4 - 0x080142B0) + 1;
const DMA_DEC_FN: u32 = COPY_BASE + (0x080142E4 - 0x080142B0) + 1;
// TRAP needs the THUMB bit: time() returns via `bx r1` and an even
// address would switch to ARM and run EWRAM zeros.
const TRAP: u32 = 0x02000001;
const VCOUNT: u32 = 0x04000006;
const DISPSTAT: u32 = 0x04000004;

/// Run the ROM's `time(fn, address)` (TM0 start, call, TM0 read) and
/// return the measured TM0CNT_L value.
fn run_time_fn(fn_addr: u32, arg: u32) -> u16 {
    let rom = std::fs::read(ROM_128KB).expect("test rom present");
    let mut system = GbaSystem::from_test_rom(rom).expect("valid test rom");
    // Console-like display state (MODE_0, no forced blank) so that the
    // OAM-mirror reads stall like in a running game.
    system.bus.write16(0x04000000, 1 << 8);
    // Suite-runner phase: all 18 subtests execute back-to-back (~1100 ticks,
    // under one scanline) with OAM reads un stalled, i.e. inside VBlank
    // (the only window fitting all of them). Replicate that phase.
    while system.bus.read16(0x04000006) != 200 {
        system.bus.tick();
    }
    for _ in 0..200 {
        system.bus.tick();
    }
    eprintln!(
        "pre-jump pc={:#x} vcount={}",
        system.cpu.registers().pc(),
        system.bus.read16(0x04000006)
    );
    {
        let rom = std::fs::read(ROM_128KB).expect("test rom present");
        let blob = &rom[ROM_SPAN_START..ROM_SPAN_END];
        for (i, &b) in blob.iter().enumerate() {
            system.bus.write8(COPY_BASE + i as u32, b);
        }
    }
    system.cpu.test_jump(&mut system.bus, TIME_FN, true);
    system.cpu.registers_mut().set_r(0, fn_addr);
    system.cpu.registers_mut().set_r(1, arg);
    system.cpu.registers_mut().set_r(14, TRAP);
    eprintln!("start pc={:#x}", system.cpu.registers().pc());
    for _ in 0..100000 {
        if system.cpu.registers().pc() == (TRAP & !1) + 4 {
            break;
        }
        system.step_tcycle();
    }
    assert_eq!(
        system.cpu.registers().pc(),
        (TRAP & !1) + 4,
        "fn {fn_addr:#010X} did not return"
    );
    system.cpu.registers().r(0) as u16
}

macro_rules! fit_case {
    ($name:ident, $fn:expr, $addr:expr, $expected:expr) => {
        #[test]
        fn $name() {
            assert_eq!(run_time_fn($fn, $addr), $expected);
        }
    };
}

fit_case!(ldm_07fffff8, LDM_FN, 0x07FFFFF8, 28);
fit_case!(ldm_080003f8, LDM_FN, 0x080003F8, 38);
fit_case!(ldm_0801fff0, LDM_FN, 0x0801FFF0, 38);
fit_case!(ldm_0801fff8, LDM_FN, 0x0801FFF8, 40);
fit_case!(ldm_0803fff0, LDM_FN, 0x0803FFF0, 38);
fit_case!(ldm_0803fff8, LDM_FN, 0x0803FFF8, 40);
fit_case!(dma_inc_07fffff8, DMA_INC_FN, 0x07FFFFF8, 57);
fit_case!(dma_inc_080003f8, DMA_INC_FN, 0x080003F8, 67);
fit_case!(dma_inc_0801fff0, DMA_INC_FN, 0x0801FFF0, 67);
fit_case!(dma_inc_0801fff8, DMA_INC_FN, 0x0801FFF8, 69);
fit_case!(dma_inc_0803fff0, DMA_INC_FN, 0x0803FFF0, 67);
fit_case!(dma_inc_0803fff8, DMA_INC_FN, 0x0803FFF8, 69);
fit_case!(dma_dec_07fffff8, DMA_DEC_FN, 0x07FFFFF8, 63);
fit_case!(dma_dec_080003f8, DMA_DEC_FN, 0x080003F8, 68);
fit_case!(dma_dec_0801fff0, DMA_DEC_FN, 0x0801FFF0, 70);
fit_case!(dma_dec_0801fff8, DMA_DEC_FN, 0x0801FFF8, 68);
fit_case!(dma_dec_0803fff0, DMA_DEC_FN, 0x0803FFF0, 70);
fit_case!(dma_dec_0803fff8, DMA_DEC_FN, 0x0803FFF8, 68);

/// Replicates basic-timing `test_hblank_and_vcount_bit`: HBlank DMA samples
/// DISPSTAT (LYC=65) into IWRAM from line 64; analysis finds flag edges.
#[test]
fn hblank_sample_flag_edges() {
    let mut bus = crate::memory::GbaMemoryBus::new();
    bus.write16(DISPSTAT, 65 << 8);
    while bus.read16(VCOUNT) != 64 {
        bus.tick();
    }
    bus.write32(0x040000B0, DISPSTAT); // DMA0SAD
    bus.write32(0x040000B4, 0x03000000); // DMA0DAD
    bus.write16(0x040000B8, 616 * 2); // count 1232
    bus.write16(0x040000BA, 0x8000 | (2 << 7) | (2 << 12)); // ENABLE | SRC_FIXED | HBLANK | 16-bit
    for _ in 0..6000 {
        bus.tick();
    }
    let samples: Vec<u16> = (0..616 * 2)
        .map(|i| bus.read16(0x03000000 + i * 2))
        .collect();
    let first_set = |bit: u16| {
        samples
            .iter()
            .position(|&s| s & bit != 0)
            .unwrap_or(usize::MAX)
    };
    let first_unset_after = |bit: u16, from: usize| {
        samples
            .iter()
            .enumerate()
            .skip(from + 1)
            .find(|&(_, &s)| s & bit == 0)
            .map(|(i, _)| i)
            .unwrap_or(usize::MAX)
    };
    let hbl_set = first_set(2);
    let hbl_unset = first_unset_after(2, hbl_set);
    let vcnt_set = first_set(4);
    let vcnt_unset = first_unset_after(4, vcnt_set);
    assert_eq!(
        (hbl_set, hbl_unset, vcnt_set, vcnt_unset),
        (0, 111, 111, 727)
    );
}

/// Replicates exact-timing `test_hblank_flag_set_unset`: video DMA3 samples
/// DISPSTAT (LYC=3) from frame start; analysis finds flag edges.
#[test]
fn video_sample_flag_edges() {
    let mut bus = crate::memory::GbaMemoryBus::new();
    bus.write16(DISPSTAT, 3 << 8);
    while bus.read16(VCOUNT) != 0 {
        bus.tick();
    }
    bus.write32(0x040000D4, DISPSTAT); // DMA3SAD
    bus.write32(0x040000D8, 0x03001000); // DMA3DAD
    bus.write16(0x040000DC, 616); // count 616
    bus.write16(0x040000DE, 0x8000 | (2 << 7) | (3 << 12)); // ENABLE | SRC_FIXED | SPECIAL | 16-bit
    for _ in 0..600000 {
        bus.tick();
        if bus.read16(0x040000DE) & 0x8000 == 0 {
            break;
        }
    }
    assert_eq!(bus.read16(0x040000DE) & 0x8000, 0);
    let samples: Vec<u16> = (0..616).map(|i| bus.read16(0x03001000 + i * 2)).collect();
    let hbl_set = samples
        .iter()
        .position(|&s| s & 2 != 0)
        .unwrap_or(usize::MAX);
    let hbl_unset = samples
        .iter()
        .enumerate()
        .skip(hbl_set + 1)
        .find(|&(_, &s)| s & 2 == 0)
        .map(|(i, _)| i)
        .unwrap_or(usize::MAX);
    let vcnt_set = samples
        .iter()
        .position(|&s| s & 4 != 0)
        .unwrap_or(usize::MAX);
    assert_eq!((hbl_set, hbl_unset, vcnt_set), (500, 613, 613));
}

/// Replicates exact-timing `test_hblank_irq`: video DMA3 samples IF with
/// the HBlank IRQ enabled; analysis finds the first HBLANK-bit sample.
/// The ROM spins clearing IF until the burst takes the bus, so clear IF
/// at line-2 start natively. Pins the video phase (countdown 3) together
/// with the +1 HBlank IRQ deferral: the flag-edge sample (index 500)
/// must still read clear, the next one set (HW 501).
#[test]
fn video_sample_if_edge() {
    use crate::memory::GbaMemoryBus;
    let mut bus = GbaMemoryBus::new();
    bus.write16(DISPSTAT, 1 << 4); // HBlank IRQ enable
    while bus.read16(VCOUNT) != 0 {
        bus.tick();
    }
    bus.write32(0x040000D4, 0x04000202); // DMA3SAD = IF
    bus.write32(0x040000D8, 0x03001000); // DMA3DAD
    bus.write16(0x040000DC, 616); // count 616
    bus.write16(0x040000DE, 0x8000 | (2 << 7) | (3 << 12)); // ENABLE | SRC_FIXED | SPECIAL | 16-bit
    // The video-arm latch (vcount==162) delays the first burst a frame;
    // the ROM spins clearing IF until the burst takes the bus, so clear
    // IF at the firing frame's line-2 start (stale line 162..1 HBlanks
    // would otherwise trip sample 0).
    while bus.read16(VCOUNT) != 162 {
        bus.tick();
    }
    while bus.read16(VCOUNT) != 2 {
        bus.tick();
    }
    bus.write16(0x04000202, 0xFFFF); // clear IF like the ROM spin loop
    for _ in 0..600000 {
        bus.tick();
        if bus.read16(0x040000DE) & 0x8000 == 0 {
            break;
        }
    }
    assert_eq!(bus.read16(0x040000DE) & 0x8000, 0);
    let samples: Vec<u16> = (0..616).map(|i| bus.read16(0x03001000 + i * 2)).collect();
    let irq_assert = samples
        .iter()
        .position(|&s| s & 2 != 0)
        .unwrap_or(usize::MAX);
    assert_eq!(irq_assert, 501);
}

/// Native replication of nba status-irq-dma sweeps (Phase 1: DISPSTAT /
/// VCOUNT flag edges; IF/DMA need the libgba ack, see Phase 2 note).
///
/// The ROM's emit functions generate exact ARM bytes in IWRAM (delay
/// NOPs + LDR/LDRH/LDR/STRH + IE=0 + BX LR); this generates the identical
/// bytes and installs them DIRECTLY as the IRQ vector (no libgba
/// IntrMain dispatch -- the joint +50 uniform offset below absorbs it:
/// HBLANK=0 194/144, HBLANK=1 1200/1151, VMATCH 194/145,
/// VBLANK=1 196/144, VBLANK=0 195/144, VCOUNT 196/144; spread +-2 is
/// sync/poll granularity on both sides). The CPU is parked on a Thumb
/// b-loop; HBlank entry runs the emit fn and returns via the HLE
/// trampoline. Each pin brackets its edge (2 probes, ~2 frames) so the
/// suite stays fast while locking flag/edge/entry behavior.
const EMIT_BASE: u32 = 0x03002000;
const EMIT_RESULT: u32 = 0x03001000;
const PARK_PC: u32 = 0x03000000;

/// Generate the status-irq-dma delay/read handler (emit.c verbatim).
fn emit_delay_read(bus: &mut crate::memory::GbaMemoryBus, delay: u16, address: u32) {
    let mut a = EMIT_BASE;
    let mut w = |v: u32| {
        bus.write32(a, v);
        a += 4;
    };
    for _ in 0..delay {
        w(0xE320F000); // NOP
    }
    w(0xE59F201C); // LDR R2, [PC, #28] (= address)
    w(0xE1D220B0); // LDRH R2, [R2]
    w(0xE59F1018); // LDR R1, [PC, #24] (= result)
    w(0xE1C120B0); // STRH R2, [R1]
    w(0xE3A00301); // MOV R0, #0x04000000
    w(0xE2800C02); // ADD R0, #0x200 (= 0x04000200, IE)
    w(0xE3A01000); // MOV R1, #0
    w(0xE5801000); // STR R1, [R0] (IE=0: handler done)
    w(0xE12FFF1E); // BX LR
    w(address);
    w(EMIT_RESULT);
}

/// One __test_cycle probe: sync to (a,b), arm HBlank IRQ with a
/// delay-nop emit handler sampling `reg`, run to handler completion,
/// return the sampled halfword.
fn irq_sample(line_a: u16, line_b: u16, dispstat: u16, reg: u32, delay: u16) -> u16 {
    let mut system = GbaSystem::new();
    {
        let regs = system.cpu.registers_mut();
        regs.set_cpsr(regs.cpsr() & !(1 << 7));
        regs.set_cpsr_t(true);
        regs.set_pc(PARK_PC);
        regs.set_sp(0x03007E00);
    }
    system.bus.write16(PARK_PC, 0xE7FE); // Thumb b .
    emit_delay_read(&mut system.bus, delay, reg);
    system.bus.write32(0x03007FFC, EMIT_BASE); // vector direct (ARM)
    for _ in 0..600000 {
        if system.bus.read16(0x04000006) == line_a {
            break;
        }
        system.step_tcycle();
    }
    for _ in 0..600000 {
        if system.bus.read16(0x04000006) == line_b {
            break;
        }
        system.step_tcycle();
    }
    system.bus.write16(0x04000200, 0x0002); // IE = HBlank
    system.bus.write16(0x04000202, 0xFFFF); // IF clear
    system.bus.write16(0x04000208, 0x0001); // IME = 1
    system.bus.write16(0x04000004, dispstat | 0x0010); // DISPSTAT |= HBL
    system.bus.take_access_wait_cycles(); // drain out-of-band residue
    for _ in 0..600000 {
        system.step_tcycle();
        if system.bus.read16(0x04000200) == 0 {
            break;
        }
    }
    assert_eq!(system.bus.read16(0x04000200), 0, "handler did not run");
    system.bus.read16(EMIT_RESULT)
}

#[test]
fn irq_hblank_clear_edge() {
    // HBLANK=1->0 on line 227 end (HW 144, +50 dispatch offset).
    assert_ne!(irq_sample(226, 227, 0x0000, DISPSTAT, 193) & 2, 0);
    assert_eq!(irq_sample(226, 227, 0x0000, DISPSTAT, 194) & 2, 0);
}

#[test]
fn irq_hblank_set_edge() {
    // HBLANK=0->1 at line-0 HBlank (HW 1151, +49).
    assert_eq!(irq_sample(226, 227, 0x0000, DISPSTAT, 1199) & 2, 0);
    assert_ne!(irq_sample(226, 227, 0x0000, DISPSTAT, 1200) & 2, 0);
}

#[test]
fn irq_vmatch_edge() {
    // VCNT flag 0->1, LYC=0, line 227->0 (HW 145, +49).
    assert_eq!(irq_sample(226, 227, 0x0000, DISPSTAT, 193) & 4, 0);
    assert_ne!(irq_sample(226, 227, 0x0000, DISPSTAT, 194) & 4, 0);
}

#[test]
fn irq_vblank_set_edge() {
    // VBlank flag 0->1, line 159->160 (HW 144, +52).
    assert_eq!(irq_sample(158, 159, 0x0000, DISPSTAT, 195) & 1, 0);
    assert_ne!(irq_sample(158, 159, 0x0000, DISPSTAT, 196) & 1, 0);
}

#[test]
fn irq_vblank_clear_edge() {
    // VBlank flag 1->0, line 226->227 (HW 144, +51).
    assert_ne!(irq_sample(225, 226, 0x0000, DISPSTAT, 194) & 1, 0);
    assert_eq!(irq_sample(225, 226, 0x0000, DISPSTAT, 195) & 1, 0);
}

#[test]
fn irq_vcount_inc_edge() {
    // VCOUNT 0->1, line 0 end (HW 144, +52).
    assert_eq!(irq_sample(227, 0, 0x0000, VCOUNT, 195), 0);
    assert_eq!(irq_sample(227, 0, 0x0000, VCOUNT, 196), 1);
}

/// Native replication of status-irq-dma test_dma (HBlank/VBlank/Video
/// DMA absolute TM0 phases; HW 1137/1362/22069). Hand-assembled ARM
/// equivalent of __test_dma_hblank_irq_handler: same operations in the
/// same order (IE=0, TM0CNT=0, TM0CNT_H=START, 3x SAD/DAD/CNT programs,
/// 3x enable-spins, done flag, BX LR). A two-pass builder resolves the
/// PC-relative literals exactly. Joint offset absorbs code-shape +
/// IntrMain dispatch like Phase 1 (predict +50 all three).
const DMAH_BASE: u32 = 0x03002400;
const DMAH_HB: u32 = 0x03001000;
const DMAH_VB: u32 = 0x03001004;
const DMAH_VD: u32 = 0x03001008;
const DMAH_DONE: u32 = 0x0300100C;

fn build_dma_handler() -> Vec<u32> {
    // Pass 1: code words (LDRs carry literal indices in low 12 bits
    // temporarily) + deduped literal pool.
    let mut code: Vec<u32> = Vec::new();
    let mut lits: Vec<u32> = Vec::new();
    macro_rules! lit {
        ($v:expr) => {{
            let v: u32 = $v;
            match lits.iter().position(|&x| x == v) {
                Some(i) => i,
                None => {
                    lits.push(v);
                    lits.len() - 1
                }
            }
        }};
    }
    macro_rules! ldr {
        ($rd:expr, $v:expr) => {{
            let li = lit!($v);
            code.push(0xE59F0000 | (($rd) << 12) | ((li as u32) & 0xFFF));
        }};
    }
    // IE = 0.
    ldr!(0, 0x04000200);
    code.push(0xE3A01000); // MOV R1, #0
    code.push(0xE5801000); // STR R1, [R0]
    // TM0CNT = 0; TM0CNT_H = TIMER_START.
    ldr!(0, 0x04000100);
    code.push(0xE5801000); // STR R1, [R0] (R1 still 0)
    code.push(0xE3A01080); // MOV R1, #0x80
    code.push(0xE1C010B2); // STRH R1, [R0, #2]
    // Program one DMA channel (SAD/DAD/CNT) then spin on its CNT_H.
    // (sad, dad, dst, cnt, cntval).
    for (sad, dad, dst, cnt, cntval) in [
        (
            0x040000B0u32,
            0x040000B4u32,
            DMAH_HB,
            0x040000B8u32,
            0xA0000001u32,
        ),
        (
            0x040000BCu32,
            0x040000C0u32,
            DMAH_VB,
            0x040000C4u32,
            0x90000001u32,
        ),
        (
            0x040000D4u32,
            0x040000D8u32,
            DMAH_VD,
            0x040000DCu32,
            0xB0000001u32,
        ),
    ] {
        ldr!(0, sad);
        ldr!(1, 0x04000100); // src = TM0CNT
        code.push(0xE5801000); // STR R1, [R0]
        ldr!(0, dad);
        ldr!(1, dst);
        code.push(0xE5801000); // STR R1, [R0]
        ldr!(0, cnt);
        ldr!(1, cntval);
        code.push(0xE5801000); // STR R1, [R0]
    }
    for cnt_h in [0x040000BAu32, 0x040000C6u32, 0x040000DEu32] {
        ldr!(0, cnt_h);
        let top = code.len();
        // LDRH (not LDR): libgba polls CNT as vu16. A word LDR at CNT_H
        // is unaligned and RORs in the write-only SAD half (open bus),
        // whose stale bit15 keeps a TST #0x8000 spinning forever.
        code.push(0xE1D010B0); // LDRH R1, [R0]
        code.push(0xE3110080 | (12 << 8)); // TST R1, #0x8000
        code.push(0x1AFFFFFB); // BNE top
        let _ = top;
    }
    // done flag + return.
    ldr!(0, DMAH_DONE);
    code.push(0xE3A01001); // MOV R1, #1
    code.push(0xE5801000); // STR R1, [R0]
    code.push(0xE12FFF1E); // BX LR
    // Pass 2: lay literals after code; patch LDR offsets.
    // LDR at byte p (from DMAH_BASE) has PC = base+p+8; literal li sits
    // at base + code.len()*4 + li*4.
    let base = code.len() * 4;
    for i in 0..code.len() {
        if code[i] & 0xFFFF0000 == 0xE59F0000 {
            let li = (code[i] & 0xFFF) as usize;
            let off = (base + li * 4) as i32 - (i * 4 + 8) as i32;
            assert!(off >= 0 && off < 4096, "literal out of range");
            code[i] = (code[i] & 0xFFFFF000) | off as u32;
        }
    }
    code.extend_from_slice(&lits);
    code
}

/// Run one test_dma probe: sync to line 158, arm the HBlank IRQ with the
/// DMA handler, wait for completion, return (hb, vb, vd, done-tick).
fn dma_probe() -> (u16, u16, u16) {
    let mut system = GbaSystem::new();
    {
        let regs = system.cpu.registers_mut();
        regs.set_cpsr(regs.cpsr() & !(1 << 7));
        regs.set_cpsr_t(true);
        regs.set_pc(PARK_PC);
        regs.set_sp(0x03007E00);
    }
    system.bus.write16(PARK_PC, 0xE7FE);
    let blob = build_dma_handler();
    for (i, w) in blob.iter().enumerate() {
        system.bus.write32(DMAH_BASE + i as u32 * 4, *w);
    }
    for (i, w) in blob.iter().enumerate() {
        assert_eq!(
            system.bus.read32(DMAH_BASE + i as u32 * 4),
            *w,
            "iwram w32 mismatch"
        );
    }
    system.bus.write32(0x03007FFC, DMAH_BASE);
    for _ in 0..600000 {
        if system.bus.read16(0x04000006) == 157 {
            break;
        }
        system.step_tcycle();
    }
    for _ in 0..600000 {
        if system.bus.read16(0x04000006) == 158 {
            break;
        }
        system.step_tcycle();
    }
    system.bus.write16(0x04000200, 0x0002);
    system.bus.write16(0x04000202, 0xFFFF);
    system.bus.write16(0x04000208, 0x0001);
    system.bus.write16(0x04000004, 0x0010);
    system.bus.take_access_wait_cycles();
    for _ in 0..1200000 {
        system.step_tcycle();
        if system.bus.read16(DMAH_DONE) != 0 {
            break;
        }
    }
    assert_eq!(system.bus.read16(DMAH_DONE), 1, "dma handler did not run");
    (
        system.bus.read16(DMAH_HB),
        system.bus.read16(DMAH_VB),
        system.bus.read16(DMAH_VD),
    )
}

#[test]
fn dma_handler_layout_selfcheck() {
    // Verify two-pass literal patching: every LDR resolves into the pool.
    let blob = build_dma_handler();
    // Find pool start: first word after BX LR (0xE12FFF1E).
    let bx = blob.iter().position(|&w| w == 0xE12FFF1E).unwrap();
    let pool_at = bx + 1;
    for (i, &w) in blob[..pool_at].iter().enumerate() {
        if w & 0xFFFF0000 == 0xE59F0000 {
            let off = w & 0xFFF;
            let target = (i * 4 + 8 + off as usize) / 4;
            assert!(
                target >= pool_at && target < blob.len(),
                "LDR@{i} escapes pool"
            );
        }
    }
    // Spots: first three words program IE=0; handler ends with BX LR.
    assert_eq!(blob[1], 0xE3A01000);
    assert_eq!(blob[2], 0xE5801000);
    // Every LDR must target the exact literal the builder assigned
    // (registers, SAD/DAD/CNT addresses and values, spin CNT_Hs, DONE).
    let mut want = vec![
        0x04000200u32,
        0x04000100,
        0x040000B0,
        0x04000100,
        0x040000B4,
        DMAH_HB,
        0x040000B8,
        0xA0000001,
        0x040000BC,
        0x04000100,
        0x040000C0,
        DMAH_VB,
        0x040000C4,
        0x90000001,
        0x040000D4,
        0x04000100,
        0x040000D8,
        DMAH_VD,
        0x040000DC,
        0xB0000001,
        0x040000BA,
        0x040000C6,
        0x040000DE,
        DMAH_DONE,
    ];
    let mut got = Vec::new();
    for (i, &w) in blob[..pool_at].iter().enumerate() {
        if w & 0xFFFF0000 == 0xE59F0000 {
            let off = w & 0xFFF;
            let target = (i * 4 + 8 + off as usize) / 4;
            got.push(blob[target]);
        }
    }
    assert_eq!(got, want, "literal routing");
}

#[test]
fn dma_hblank_phase() {
    // HBLANK DMA TM0 (HW 1137, +55 dispatch/shape offset).
    assert_eq!(dma_probe().0, 1192);
}

#[test]
fn dma_vblank_phase() {
    // VBLANK DMA TM0 (HW 1362, +56).
    assert_eq!(dma_probe().1, 1418);
}

#[test]
fn dma_video_phase() {
    // VIDEO DMA TM0 (HW 22069, +56).
    assert_eq!(dma_probe().2, 22125);
}

/// Native replication of nba ram-access-timing DISPCNT-latch probes
/// (bus-only: VCOUNT sync + TM0 + immediate DMA0, no IRQ needed).
/// Each probe returns the TM0 ticks for a 128-halfword VRAM DMA burst;
/// the 4-tuple pattern reads out the fetch-gating rule (documented in
/// the ROM comments from HW observation). Absolute values carry a fixed
/// setup offset vs the ROM (back-to-back driver writes); only the
/// fast/slow pattern is asserted.
fn dma0_burst(bus: &mut crate::memory::GbaMemoryBus) -> u16 {
    bus.write32(0x04000100, 0);
    bus.write16(0x04000102, 0x0080); // TM0CNT_H = START
    bus.write16(0x040000B8, 128);
    bus.write16(0x040000BA, 0x8140); // ENABLE|16|SRC_FIXED|DST_FIXED
    for _ in 0..60000 {
        bus.tick();
        if bus.read16(0x040000BA) & 0x8000 == 0 {
            break;
        }
    }
    assert_eq!(bus.read16(0x040000BA) & 0x8000, 0);
    bus.read16(0x04000100)
}

fn wait_vcount(bus: &mut crate::memory::GbaMemoryBus, v: u16) {
    for _ in 0..600000 {
        if bus.read16(0x04000006) == v {
            break;
        }
        bus.tick();
    }
}

/// Native replication of nba ram-access-timing DISPCNT-latch probes:
/// 128-halfword VRAM DMA0 bursts at lines 2,3,4,5 with enable/blank
/// flips per the ROM scripts. The tuple pattern reads out fetch gating.
fn latch_run(
    init_disp: u16,
    hofs7: bool,
    oam_setup: bool,
    sad_dad: u32,
    line2_op: u16,
    line2_clear: bool,
    line5_op: u16,
    line5_clear: bool,
) -> [u16; 4] {
    use crate::memory::GbaMemoryBus;
    let mut bus = GbaMemoryBus::new();
    bus.write16(0x04000000, init_disp);
    if hofs7 {
        for r in [0x04000010, 0x04000014, 0x04000018, 0x0400001C] {
            bus.write16(r, 7);
        }
    }
    if oam_setup {
        for i in 0..128u32 {
            bus.write16(0x07000000 + i * 8, 0); // attr0: enable
            bus.write16(0x07000000 + i * 8 + 2, 0xC000); // attr1: 64x64
        }
    }
    bus.write32(0x040000B0, sad_dad);
    bus.write32(0x040000B4, sad_dad);
    // while(VCOUNT==2){} then while(VCOUNT!=2){} from boot (vc=0):
    // first loop exits at once, second waits for line 2.
    wait_vcount(&mut bus, 2);
    let mut d = bus.read16(0x04000000);
    if line2_clear {
        d &= !line2_op;
    } else {
        d |= line2_op;
    }
    bus.write16(0x04000000, d);
    let r0 = dma0_burst(&mut bus);
    wait_vcount(&mut bus, 3);
    let r1 = dma0_burst(&mut bus);
    wait_vcount(&mut bus, 4);
    let r2 = dma0_burst(&mut bus);
    wait_vcount(&mut bus, 5);
    let mut d = bus.read16(0x04000000);
    if line5_clear {
        d &= !line5_op;
    } else {
        d |= line5_op;
    }
    bus.write16(0x04000000, d);
    let r3 = dma0_burst(&mut bus);
    [r0, r1, r2, r3]
}

#[test]
fn latch_bg_fetch() {
    // BG fetch iff latched AND current BG enable (ROM doc rule): line 2
    // (enables just turned on, latch still off) is fast, lines 3-4 fetch,
    // line 5 (enables off) is fast.
    assert_eq!(
        latch_run(0x0100, true, false, 0x06000000, 0x0F00, false, 0x0F00, true),
        [260, 490, 490, 260]
    );
}

#[test]
fn latch_bg_fetch_blank() {
    // BG fetch iff latched-blank-clear AND current-blank-clear: blank
    // samples at line turnover (not the 3-stage enable latch), so fetch
    // resumes the line after the blank write: [fast, slow, slow, fast].
    assert_eq!(
        latch_run(
            0x0100 | 0x0080 | 0x0F00,
            false,
            false,
            0x06000000,
            0x0080,
            true,
            0x0080,
            false
        ),
        [260, 490, 490, 260]
    );
}

#[test]
fn latch_obj_fetch() {
    // OBJ fetch iff CURRENT OBJ enable (latched disregarded): lines 2-4
    // fetch, line 5 (disabled at line start) does not.
    assert_eq!(
        latch_run(0x0100, false, true, 0x06010000, 0x1000, false, 0x1000, true),
        [516, 516, 516, 260]
    );
}

#[test]
fn latch_obj_fetch_blank() {
    // OBJ fetch vs forced blank: line 2 (blank just cleared, sampled on)
    // does not fetch; lines 3-4 (blank clear, OBJ on) fetch; line 5
    // (blank on) does not. Same gates as above, jointly.
    assert_eq!(
        latch_run(
            0x0100 | 0x0080 | 0x1000,
            false,
            true,
            0x06010000,
            0x0080,
            true,
            0x0080,
            false
        ),
        [260, 516, 516, 260]
    );
}
