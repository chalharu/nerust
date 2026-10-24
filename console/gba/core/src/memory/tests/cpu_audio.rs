use super::super::*;

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
fn halt_wake_latency_by_source() {
    // Halted raster ISRs must land inside their scanline (mgba-suite
    // Layer toggle 2: the HBlank ISR's DISPCNT write has to precede the
    // line-end latch shift). Video-source wakes (VBlank/HBlank/VCount)
    // burn 5; timer-source wakes 10; everything else keeps 32 (mgba
    // sio-timing pin).
    for (bit, expected) in [(1u16 << 1, 5), (1u16 << 3, 10), (1u16 << 7, 32)] {
        let mut bus = GbaMemoryBus::new();
        bus.write16(0x04000200, bit);
        bus.write16(0x04000208, 1);
        bus.tick(); // apply IE/IME
        bus.enter_halt(0x3FFF);
        assert!(bus.is_halted());
        bus.request_interrupt(bit);
        bus.tick();
        bus.tick();
        assert!(!bus.is_halted());
        assert_eq!(bus.take_wake_latency(), expected, "irq bit {bit:#06x}");
    }
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
fn dma_delay_burn_fold_reports_exact_bus_advance() {
    // Save states were unloadable on DMA-heavy games (Myst): the burn
    // fold advanced `current_tcycle` once at tick entry plus `k` more
    // while reporting only `k`, so the bus clock ran ahead of the
    // system tick and state validation rejected the snapshot.
    // Reported advances must equal clock movement.
    let mut bus = GbaMemoryBus::new();
    // 256-word immediate DMA from ROM: wait-state idle gaps force
    // multi-tick delay burns between units.
    bus.write32(0x040000D4, 0x08000000);
    bus.write32(0x040000D8, 0x02000000);
    bus.write32(0x040000DC, 0x84000100);
    let start = bus.current_tcycle;
    let mut credited = 0u64;
    let mut folds = 0u32;
    for _ in 0..1_000_000 {
        let (_, n) = bus.tick();
        credited += n;
        folds += u32::from(n > 1);
        if bus.read16(0x040000DE) & 0x8000 == 0 {
            break;
        }
    }
    assert_eq!(
        bus.read16(0x040000DE) & 0x8000,
        0,
        "DMA transfer must complete"
    );
    assert!(folds > 0, "test needs burn folds to guard the accounting");
    assert_eq!(bus.current_tcycle - start, credited);
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

#[test]
fn bus_state_rejects_accumulator_overflow_magnitudes() {
    let bus = GbaMemoryBus::new();
    // A fresh bus exports a valid state.
    bus.export_state().unwrap().validate().unwrap();
    // Rest accumulators drain per instruction/burst; unbounded restored
    // values would overflow the plain arithmetic on the step path.
    for mutate in [
        |s: &mut GbaMemoryBusState| s.access_wait_cycles = 1 << 41,
        |s: &mut GbaMemoryBusState| s.access_wait_cycles = -(1 << 41),
        |s: &mut GbaMemoryBusState| s.block_batch_erase_sum = 1 << 21,
        |s: &mut GbaMemoryBusState| s.block_batch_words = 0x1_0000,
        |s: &mut GbaMemoryBusState| s.dma_stall_pending = 2,
    ] {
        let mut state = bus.export_state().unwrap();
        mutate(&mut state);
        assert!(state.validate().is_err());
    }
}

/// Batching equivalence at bus level: `quiet_cycles` + `advance_idle` +
/// boundary `tick` must reproduce per-cycle `tick` bit-exactly across
/// timer/DMA/video/sound/SIO/IRQ traffic and mid-line PPU writes
/// (scanline segments), including halt/stop spans. Deterministic
/// xorshift (fixed seed): not flaky.
#[test]
fn batch_matches_per_cycle_on_seeded_io_programs() {
    // (address, width) pool: timers, DMA control, sound incl. wave RAM
    // and FIFOs, PPU regs, palette/VRAM/OAM windows, system/IRQ/SIO.
    const WRITES: [(u32, u8); 31] = [
        (0x04000100, 2),
        (0x04000102, 2),
        (0x04000104, 2),
        (0x04000106, 2),
        (0x040000BA, 2),
        (0x040000C6, 2),
        (0x04000060, 2),
        (0x04000064, 2),
        (0x04000070, 2),
        (0x04000080, 2),
        (0x04000084, 2),
        (0x04000088, 2),
        (0x04000090, 4),
        (0x04000098, 4),
        (0x040000A0, 4),
        (0x04000000, 2),
        (0x04000004, 2),
        (0x04000008, 2),
        (0x04000010, 2),
        (0x04000028, 4),
        (0x04000040, 2),
        (0x04000044, 2),
        (0x04000048, 2),
        (0x0400004C, 2),
        (0x04000050, 2),
        (0x05000100, 2),
        (0x06001000, 4),
        (0x07000100, 4),
        (0x04000200, 2),
        (0x04000208, 2),
        (0x04000128, 2),
    ];
    let mut rng = 0x2545F4914F6CDD1Du64;
    let mut next = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    for _ in 0..24 {
        let mut a = GbaMemoryBus::new();
        let mut b = GbaMemoryBus::new();
        for _ in 0..6 {
            let (addr, width) = WRITES[(next() % WRITES.len() as u64) as usize];
            let value = (next() & 0xFFFF_FFFF) as u32;
            match width {
                1 => {
                    a.write8(addr, value as u8);
                    b.write8(addr, value as u8);
                }
                4 => {
                    a.write32(addr & !3, value);
                    b.write32(addr & !3, value);
                }
                _ => {
                    a.write16(addr & !1, value as u16);
                    b.write16(addr & !1, value as u16);
                }
            }
        }
        let total = 300 + next() % 900;
        let mut cycle = 0u64;
        while cycle < total {
            if next() % 5 == 0 {
                let (addr, width) = WRITES[(next() % WRITES.len() as u64) as usize];
                let value = (next() & 0xFFFF_FFFF) as u32;
                match width {
                    1 => {
                        a.write8(addr, value as u8);
                        b.write8(addr, value as u8);
                    }
                    4 => {
                        a.write32(addr & !3, value);
                        b.write32(addr & !3, value);
                    }
                    _ => {
                        a.write16(addr & !1, value as u16);
                        b.write16(addr & !1, value as u16);
                    }
                }
            }
            if next() % 53 == 0 {
                // Timer counter + SIO status reads must match too
                // (receive-pop and flag behavior included).
                assert_eq!(a.read16(0x04000100), b.read16(0x04000100));
                assert_eq!(a.read16(0x04000128), b.read16(0x04000128));
            }
            let horizon = b.quiet_cycles().min(total - cycle);
            if horizon == 0 {
                // Lockstep single ticks (either side may fold DMA delay
                // burns; same state folds identically — the tuples must
                // match exactly, and cycle accounting follows the advance).
                let (end_b, n_b) = b.tick();
                let (end_a, n_a) = a.tick();
                assert_eq!((end_b, n_b), (end_a, n_a), "divergence at cycle {cycle}");
                cycle += n_a;
            } else {
                // Reference: the span must be event-free. `a` may fold
                // DMA burns inside the span; accumulate its advance.
                let mut span = 0u64;
                while span < horizon {
                    let (end_a, n_a) = a.tick();
                    assert!(!end_a, "event inside batched span at cycle {cycle}");
                    span += n_a;
                    cycle += n_a;
                }
                assert_eq!(span, horizon, "fold overshoot at cycle {cycle}");
                b.advance_idle(horizon);
            }
        }
        let bytes_a = rmp_serde::to_vec_named(&a.export_state().unwrap()).unwrap();
        let bytes_b = rmp_serde::to_vec_named(&b.export_state().unwrap()).unwrap();
        if bytes_a != bytes_b {
            // `timers.current_cycle` is call-scoped scratch: every read
            // path (`tick_timers`, `read_io`, `write_io`) refreshes it
            // first, so trailing staleness from different set points is
            // unobservable (saves happen at frame ends, post-boundary).
            // Normalize before comparing.
            a.timers.set_current_cycle(total);
            b.timers.set_current_cycle(total);
            let bytes_a = rmp_serde::to_vec_named(&a.export_state().unwrap()).unwrap();
            let bytes_b = rmp_serde::to_vec_named(&b.export_state().unwrap()).unwrap();
            assert_eq!(bytes_a, bytes_b, "final state diverged");
        }
    }
}

/// M4A (Emerald's music driver) programs CGB voices with byte stores:
/// `CgbSound` writes NR13, NR14, NR12, NR14|0x80 as separate bytes, and
/// rewrites NR12 alone on every envelope step. The I/O bus merges each
/// byte lane against the staged CNT latch, so the latch must keep the
/// full halfword — dropping frequency low bytes detuned every M4A CGB
/// note to its high bits (se_select's 0x7D6 ~3121Hz collapsed to 0x700
/// = 512Hz: the menu confirm SE lost its rise), and dropping NR11
/// length/duty reset them mid-note.
fn cgb_solo_bus() -> GbaMemoryBus {
    let mut bus = GbaMemoryBus::new();
    bus.write16(0x04000084, 0x0080); // master enable
    bus.write16(0x04000080, 0xFFFF); // all PSG to L+R
    bus.write16(0x04000082, 0x0003); // PSG gain
    bus
}

#[test]
fn cgb_byte_writes_preserve_frequency_low_byte() {
    use crate::apu::fft_test::{GRID_RATE_HZ, capture_grid_mono, dominant_frequency};
    let mut bus = cgb_solo_bus();
    bus.write8(0x04000060, 0x00); // NR10: sweep off
    bus.write8(0x04000062, 0x80); // NR11: duty 2, length hold
    bus.write8(0x04000063, 0xF0); // NR12: vol 15, frozen
    bus.write8(0x04000064, 0xD6); // NR13: freq low of 0x7D6
    bus.write8(0x04000065, 0x07); // NR14: freq high, length off
    bus.write8(0x04000065, 0x87); // NR14 + trigger
    let samples = capture_grid_mono(&mut bus.apu, 4096);
    let dominant = dominant_frequency(&samples, GRID_RATE_HZ);
    assert!(
        (dominant - 3121.0).abs() < 60.0,
        "byte-written 0x7D6 should sound ~3121Hz, got {dominant}"
    );
}

#[test]
fn cgb_envelope_byte_write_preserves_duty_and_length() {
    let mut bus = cgb_solo_bus();
    bus.write8(0x04000060, 0x00); // NR10: sweep off
    bus.write8(0x04000062, 0x80 | 60); // NR11: duty 2, length 4 ticks
    bus.write8(0x04000063, 0xF0); // NR12: vol 15, frozen
    bus.write8(0x04000064, 0xD6); // NR13: freq low
    bus.write8(0x04000065, 0x47 | 0x80); // NR14: high + length gate + trigger
    assert_eq!(bus.apu.duty1_for_test(), 2);
    // Mid-note envelope step (NR12 alone, as M4A's sustain re-fire does):
    // duty must stay 2 and the length countdown must continue.
    for _ in 0..5000 {
        bus.apu.tick();
    }
    bus.write8(0x04000063, 0xF0);
    assert_eq!(bus.apu.duty1_for_test(), 2);
    // Length 4 ticks at 256Hz (~15.6ms = ~262k T-cycles): the voice must
    // be gone well before 300k ticks. A length reset to 64 would keep it
    // alive for ~4.2M ticks instead.
    for _ in 0..300_000 {
        bus.apu.tick();
    }
    assert_eq!(
        bus.apu.soundcnt_x_read() & 1,
        0,
        "gated note must expire on its latched length"
    );
}

#[test]
fn se_select_note_sequence_ascends() {
    use crate::apu::fft_test::{GRID_RATE_HZ, dominant_frequency};
    // Emerald's menu confirm SE: note 94 on the sweep-up square voice
    // (sweep 0x77 = pace 7/inc/shift 7, duty 2), then note 103 on the
    // plain square voice. Real M4A CGB freq values (MidiKeyToCgbFreq):
    // 94 -> 0x7B9 (~1846Hz), 103 -> 0x7D6 (~3121Hz).
    let mut bus = cgb_solo_bus();
    bus.write8(0x04000060, 0x77); // NR10: sweep up (voice 87)
    bus.write8(0x04000062, 0x80); // NR11: duty 2, hold
    bus.write8(0x04000063, 0xF0); // NR12: vol 15
    bus.write8(0x04000064, 0xB9); // NR13: 0x7B9 low
    bus.write8(0x04000065, 0x07); // NR14: high
    bus.write8(0x04000065, 0x87); // NR14 + trigger
    while bus.apu.grid_buffer().len() < 4096 {
        bus.apu.tick();
    }
    // Second note (voice 88: sweep off), retriggered over the first.
    bus.write8(0x04000060, 0x00);
    bus.write8(0x04000062, 0x80);
    bus.write8(0x04000063, 0xF0);
    bus.write8(0x04000064, 0xD6);
    bus.write8(0x04000065, 0x07);
    bus.write8(0x04000065, 0x87);
    while bus.apu.grid_buffer().len() < 8192 {
        bus.apu.tick();
    }
    let early: Vec<f32> = bus.apu.grid_buffer()[0..2048]
        .iter()
        .map(|sample| sample.0)
        .collect();
    let late: Vec<f32> = bus.apu.grid_buffer()[6144..8192]
        .iter()
        .map(|sample| sample.0)
        .collect();
    let early_pitch = dominant_frequency(&early, GRID_RATE_HZ);
    let late_pitch = dominant_frequency(&late, GRID_RATE_HZ);
    assert!(
        late_pitch - early_pitch > 400.0,
        "se_select should rise 94 -> 103: early={early_pitch}, late={late_pitch}"
    );
}
