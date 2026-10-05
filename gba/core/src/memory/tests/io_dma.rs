use super::super::*;

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
fn keyinput_zero_upper_bits() {
    // GBATEK 4000130h: bits 10-15 are unused and read 0 on hardware.
    let mut bus = GbaMemoryBus::new();
    bus.set_keyinput(0x0000);
    assert_eq!(bus.read16(0x04000130) & 0xFC00, 0x0000);
    bus.set_keyinput(0x03FF);
    assert_eq!(bus.read16(0x04000130), 0x03FF);
    bus.set_keyinput(0xFFFF);
    assert_eq!(bus.read16(0x04000130), 0x03FF);
}

#[test]
fn keyinput_b_only_reads_exact() {
    // Pokemon Emerald's evolution cancel requires heldKeys == B_BUTTON
    // exactly: with only B held, the 16-bit KEYINPUT read must be
    // precisely 0x03FD (no forced upper bits).
    let mut bus = GbaMemoryBus::new();
    bus.set_keyinput(0x03FD); // B pressed (bit 1 = 0), rest released
    assert_eq!(bus.read16(0x04000130), 0x03FD);
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
fn eeprom_dma_read_tolerates_trailing_request_bit() {
    use crate::cartridge::Cartridge;
    use crate::cartridge::header::finalize_test_gba_rom;
    // Minish Cap sends its 16-bit 8KB read request as 17 DMA units; the
    // 17th is uninitialized stack. The request must still decode end to
    // end (dropped requests read back stale 0xFF and flag healthy files
    // corrupt on fresh boot).
    let mut rom = vec![0u8; 0x1000];
    finalize_test_gba_rom(&mut rom);
    rom[0x200..0x20A].copy_from_slice(b"EEPROM_V12");
    let mut bus = GbaMemoryBus::new();
    bus.set_cartridge(Cartridge::new(rom).unwrap());
    // Store one block via an exact 81-bit write frame first.
    let data = [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0];
    let mut bits = vec![true, false];
    bits.extend_from_slice(&[false; 14]);
    for byte in data {
        for i in (0..8).rev() {
            bits.push((byte >> i) & 1 != 0);
        }
    }
    bits.push(false);
    for (i, bit) in bits.iter().enumerate() {
        bus.write16(0x03000000 + (i as u32) * 2, u16::from(*bit));
    }
    bus.write32(0x040000D4, 0x03000000);
    bus.write32(0x040000D8, 0x0D000000);
    bus.write16(0x040000DC, bits.len() as u16);
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
    // Read request with trailing 1 bit (17 units), then the 68-unit read.
    let mut req = vec![true, true];
    req.extend_from_slice(&[false; 14]);
    req.push(true);
    assert_eq!(req.len(), 17);
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
        if bus.tick().0 {
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
