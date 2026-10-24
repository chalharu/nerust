use super::super::*;

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
