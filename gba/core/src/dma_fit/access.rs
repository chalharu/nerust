use super::edges::wait_vcount;
use super::PARK_PC;
use crate::system::GbaSystem;

/// Native replication of nba ram-access-timing single-access probes:
/// TM0 START, delay NOPs, LDRH [address], TM0 STOP (emit.c verbatim,
/// address literal only). Returns the stopped TM0 (access cost +
/// constant prologue). Same park/vector/sync/arm machinery as the IRQ
/// sweeps; entry runs the handler, IE=0 signals completion.
const ACC_BASE: u32 = 0x03002800;

fn emit_access(bus: &mut crate::memory::GbaMemoryBus, delay: u16, address: u32) {
    let mut a = ACC_BASE;
    let mut w = |v: u32| {
        bus.write32(a, v);
        a += 4;
    };
    w(0xE3A00301); // MOV R0, #0x04000000
    w(0xE2800C01); // ADD R0, #0x100 (= TM0CNT)
    w(0xE3A01080); // MOV R1, #0x80
    w(0xE1C010B2); // STRH R1, [R0, #2] (TM0 START)
    for _ in 0..delay {
        w(0xE320F000); // NOP
    }
    w(0xE59F2014); // LDR R2, [PC, #20] (= address)
    w(0xE1D220B0); // LDRH R2, [R2]
    w(0xE3A01000); // MOV R1, #0
    w(0xE1C010B2); // STRH R1, [R0, #2] (TM0 STOP)
    w(0xE2800C01); // ADD R0, #0x100 (= IE)
    w(0xE5801000); // STR R1, [R0] (IE=0: done)
    w(0xE12FFF1E); // BX LR
    w(address);
}

/// Cost (TM0 minus delay) of one access probe: 8 prologue + 0/1 stall.
fn access_cost(dispcnt: u16, address: u32, delay: u16) -> u16 {
    access_sample(dispcnt, 0x0000, address, delay) - delay
}

#[test]
fn access_scan_vram_bg() {
    // VRAM BG-bank fetch window (BG0 on): stall only while the fetcher
    // runs. Rise 223 / fall 1180 in delay space (width 957 = fetch clock
    // 32..989); line-10 cyc 0 sits at delay 191 (see OAMfree/PAL below).
    assert_eq!(access_cost(0x0100, 0x06000000, 222), 8);
    assert_eq!(access_cost(0x0100, 0x06000000, 223), 9);
    assert_eq!(access_cost(0x0100, 0x06000000, 1179), 9);
    assert_eq!(access_cost(0x0100, 0x06000000, 1180), 8);
}

#[test]
fn access_flat_gates() {
    // OBJ-bank needs live OBJ enable (Fix A): on = stalled flat,
    // off = clean flat. BG-bank needs BG enables. OAM without free-bit
    // stays busy through HBlank (flat stalled).
    for d in [0, 600, 1200] {
        assert_eq!(access_cost(0x1000, 0x06010000, d), 9); // OBJ on
        assert_eq!(access_cost(0x0100, 0x06010000, d), 8); // OBJ off
        assert_eq!(access_cost(0x0000, 0x06000000, d), 8); // BG off
        assert_eq!(access_cost(0x0100, 0x07000000, d), 9); // OAM busy
    }
}

#[test]
fn access_oam_free_window() {
    // OAM with HBlank-free: busy only in draw (cyc < 960). Rise 191,
    // fall 1151 (width exactly 960).
    assert_eq!(access_cost(0x0100 | 0x0020, 0x07000000, 190), 8);
    assert_eq!(access_cost(0x0100 | 0x0020, 0x07000000, 191), 9);
    assert_eq!(access_cost(0x0100 | 0x0020, 0x07000000, 1150), 9);
    assert_eq!(access_cost(0x0100 | 0x0020, 0x07000000, 1151), 8);
}

#[test]
fn access_palette_window() {
    // Palette feeds every rendered pixel (cyc <= 960, one wider than
    // OAM's strict <960). Rise 191, fall 1152 (width 961).
    assert_eq!(access_cost(0x0100, 0x05000000, 190), 8);
    assert_eq!(access_cost(0x0100, 0x05000000, 191), 9);
    assert_eq!(access_cost(0x0100, 0x05000000, 1151), 9);
    assert_eq!(access_cost(0x0100, 0x05000000, 1152), 8);
}

fn access_sample(dispcnt: u16, dispstat: u16, address: u32, delay: u16) -> u16 {
    let mut system = GbaSystem::new();
    {
        let regs = system.cpu.registers_mut();
        regs.set_cpsr(regs.cpsr() & !(1 << 7));
        regs.set_cpsr_t(true);
        regs.set_pc(PARK_PC);
        regs.set_sp(0x03007E00);
    }
    system.bus.write16(PARK_PC, 0xE7FE);
    emit_access(&mut system.bus, delay, address);
    system.bus.write32(0x03007FFC, ACC_BASE);
    system.bus.write16(0x04000000, dispcnt);
    wait_vcount(&mut system.bus, 8);
    wait_vcount(&mut system.bus, 9);
    system.bus.write16(0x04000200, 0x0002);
    system.bus.write16(0x04000202, 0xFFFF);
    system.bus.write16(0x04000208, 0x0001);
    system.bus.write16(0x04000004, dispstat | 0x0010);
    system.bus.take_access_wait_cycles();
    for _ in 0..600000 {
        system.step_tcycle();
        if system.bus.read16(0x04000200) == 0 {
            break;
        }
    }
    assert_eq!(
        system.bus.read16(0x04000200),
        0,
        "access handler did not run"
    );
    system.bus.read16(0x04000100)
}
