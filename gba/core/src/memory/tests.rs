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
fn hblank_dma_fires_on_vdraw_lines_only() {
    // H-Blank DMA starts only on visible scanlines (vcount < 160):
    // during V-Blank the H-Blank flag and IRQ still toggle, but no
    // DMA request is generated — one full frame of a repeat HBlank
    // channel transfers 160 units, not 228.
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
    assert_eq!(written, 160);
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
