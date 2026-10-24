use super::edges::{dma0_burst, wait_vcount};

/// Native replication of nba ram-access-timing DISPCNT-latch probes:
/// 128-halfword VRAM DMA0 bursts at lines 2,3,4,5 with enable/blank
/// flips per the ROM scripts. The tuple pattern reads out fetch gating.
/// DISPCNT patch applied at a scanline boundary: set `bits` when `clear`
/// is false, clear `bits` when true. `line2`/`line5` ops form such a
/// pair, so they are grouped to keep the probe signature readable.
#[derive(Clone, Copy)]
struct DispPatch {
    bits: u16,
    clear: bool,
}

struct LatchConfig {
    init_disp: u16,
    hofs7: bool,
    oam_setup: bool,
    sad_dad: u32,
    line2: DispPatch,
    line5: DispPatch,
}

fn apply_patch(disp: u16, patch: DispPatch) -> u16 {
    if patch.clear {
        disp & !patch.bits
    } else {
        disp | patch.bits
    }
}

fn latch_run(cfg: LatchConfig) -> [u16; 4] {
    use crate::memory::GbaMemoryBus;
    let mut bus = GbaMemoryBus::new();
    bus.write16(0x04000000, cfg.init_disp);
    if cfg.hofs7 {
        for r in [0x04000010, 0x04000014, 0x04000018, 0x0400001C] {
            bus.write16(r, 7);
        }
    }
    if cfg.oam_setup {
        for i in 0..128u32 {
            bus.write16(0x07000000 + i * 8, 0); // attr0: enable
            bus.write16(0x07000000 + i * 8 + 2, 0xC000); // attr1: 64x64
        }
    }
    bus.write32(0x040000B0, cfg.sad_dad);
    bus.write32(0x040000B4, cfg.sad_dad);
    // while(VCOUNT==2){} then while(VCOUNT!=2){} from boot (vc=0):
    // first loop exits at once, second waits for line 2.
    wait_vcount(&mut bus, 2);
    let d = bus.read16(0x04000000);
    bus.write16(0x04000000, apply_patch(d, cfg.line2));
    let r0 = dma0_burst(&mut bus);
    wait_vcount(&mut bus, 3);
    let r1 = dma0_burst(&mut bus);
    wait_vcount(&mut bus, 4);
    let r2 = dma0_burst(&mut bus);
    wait_vcount(&mut bus, 5);
    let d = bus.read16(0x04000000);
    bus.write16(0x04000000, apply_patch(d, cfg.line5));
    let r3 = dma0_burst(&mut bus);
    [r0, r1, r2, r3]
}

#[test]
fn latch_bg_fetch() {
    // BG fetch iff latched AND current BG enable (ROM doc rule): line 2
    // (enables just turned on, latch still off) is fast, lines 3-4 fetch,
    // line 5 (enables off) is fast.
    assert_eq!(
        latch_run(LatchConfig {
            init_disp: 0x0100,
            hofs7: true,
            oam_setup: false,
            sad_dad: 0x06000000,
            line2: DispPatch {
                bits: 0x0F00,
                clear: false,
            },
            line5: DispPatch {
                bits: 0x0F00,
                clear: true,
            },
        }),
        [260, 490, 490, 260]
    );
}

#[test]
fn latch_bg_fetch_blank() {
    // BG fetch iff latched-blank-clear AND current-blank-clear: blank
    // samples at line turnover (not the 3-stage enable latch), so fetch
    // resumes the line after the blank write: [fast, slow, slow, fast].
    assert_eq!(
        latch_run(LatchConfig {
            init_disp: 0x0100 | 0x0080 | 0x0F00,
            hofs7: false,
            oam_setup: false,
            sad_dad: 0x06000000,
            line2: DispPatch {
                bits: 0x0080,
                clear: true,
            },
            line5: DispPatch {
                bits: 0x0080,
                clear: false,
            },
        }),
        [260, 490, 490, 260]
    );
}

#[test]
fn latch_obj_fetch() {
    // OBJ fetch iff CURRENT OBJ enable (latched disregarded): lines 2-4
    // fetch, line 5 (disabled at line start) does not.
    assert_eq!(
        latch_run(LatchConfig {
            init_disp: 0x0100,
            hofs7: false,
            oam_setup: true,
            sad_dad: 0x06010000,
            line2: DispPatch {
                bits: 0x1000,
                clear: false,
            },
            line5: DispPatch {
                bits: 0x1000,
                clear: true,
            },
        }),
        [516, 516, 516, 260]
    );
}

#[test]
fn latch_obj_fetch_blank() {
    // OBJ fetch vs forced blank: line 2 (blank just cleared, sampled on)
    // does not fetch; lines 3-4 (blank clear, OBJ on) fetch; line 5
    // (blank on) does not. Same gates as above, jointly.
    assert_eq!(
        latch_run(LatchConfig {
            init_disp: 0x0100 | 0x0080 | 0x1000,
            hofs7: false,
            oam_setup: true,
            sad_dad: 0x06010000,
            line2: DispPatch {
                bits: 0x0080,
                clear: true,
            },
            line5: DispPatch {
                bits: 0x0080,
                clear: false,
            },
        }),
        [260, 516, 516, 260]
    );
}
