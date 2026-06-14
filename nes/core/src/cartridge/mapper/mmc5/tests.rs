use super::Cartridge;
use super::{ChrBankSet, Mmc5};
use crate::cartridge_data_parts::CartridgeDataParts;
use crate::cartridge_rom::CartridgeData;
use crate::interrupt::{Interrupt, IrqSource};
use crate::mapper::Mapper;
use crate::ppu::Core as PpuCore;
use crate::ppu_memory_access::PpuReadAccess;
use crate::ppu_memory_access::{PpuBusAccess, PpuBusEvent};
use nerust_contract_core::mirror::MirrorMode;
use nerust_contract_core::rom::RomFormat;

fn test_data() -> CartridgeData {
    CartridgeData::new(CartridgeDataParts {
        format: RomFormat::Nes20,
        prog_rom: (0..0x20000).map(|i| (i / 0x2000) as u8).collect(),
        char_rom: (0..0x40000).map(|i| (i / 0x400) as u8).collect(),
        pram_length: 0,
        save_pram_length: 0,
        vram_length: 0,
        save_vram_length: 0,
        mapper_type: 5,
        mirror_mode: MirrorMode::Horizontal,
        has_battery: false,
        sub_mapper_type: 0,
        trainer: Vec::new(),
    })
    .expect("test cartridge data should be valid")
}

fn split_test_data() -> CartridgeData {
    CartridgeData::new(CartridgeDataParts {
        format: RomFormat::Nes20,
        prog_rom: vec![0; 0x20000],
        char_rom: (0..0x8000).map(|i| i as u8).collect(),
        pram_length: 0,
        save_pram_length: 0,
        vram_length: 0,
        save_vram_length: 0,
        mapper_type: 5,
        mirror_mode: MirrorMode::Horizontal,
        has_battery: false,
        sub_mapper_type: 0,
        trainer: Vec::new(),
    })
    .expect("split test cartridge data should be valid")
}

fn emit_ppu_read(mapper: &mut Mmc5, address: usize, tick: u64, interrupt: &mut Interrupt) {
    mapper.notify_ppu_bus_event(
        PpuBusEvent::AddressBusUpdate {
            address,
            ppu_tick: tick,
            from_cpu_register: false,
            access: PpuBusAccess::Read,
        },
        interrupt,
    );
}

#[test]
fn ppudata_uses_last_written_chr_bank_set_in_split_mode() {
    let mut mapper = Mmc5::new(test_data());
    Cartridge::initialize(&mut mapper);
    mapper.write_expansion(0x5101, 0x03, &mut Interrupt::new());
    mapper.notify_ppu_ctrl(0x20);
    mapper.notify_ppu_mask(0x18);

    mapper.write_expansion(0x5120, 0x02, &mut Interrupt::new());
    mapper.write_expansion(0x5128, 0x07, &mut Interrupt::new());

    assert_eq!(mapper.last_chr_bank_set, ChrBankSet::Background);
    assert_eq!(
        mapper
            .read_ppu_pattern(0x0000, PpuReadAccess::CpuData, &mut Interrupt::new())
            .data,
        0x07
    );
}

#[test]
fn extended_attributes_override_background_palette_and_chr_bank() {
    let mut mapper = Mmc5::new(test_data());
    Cartridge::initialize(&mut mapper);
    mapper.notify_ppu_mask(0x18);
    mapper.write_expansion(0x5104, 0x01, &mut Interrupt::new());
    mapper.write_expansion(0x5130, 0x01, &mut Interrupt::new());
    mapper.exram[0] = 0b10_000011;

    let mut ciram = vec![0; 0x800];
    let _ = mapper.read_ppu_nametable(0x2000, PpuReadAccess::BackgroundNameTable, &mut ciram);

    assert_eq!(
        mapper
            .read_ppu_nametable(0x23C0, PpuReadAccess::BackgroundAttribute, &mut ciram)
            .data,
        0xAA
    );
    assert_eq!(
        mapper
            .read_ppu_pattern(
                0x0000,
                PpuReadAccess::BackgroundPattern,
                &mut Interrupt::new()
            )
            .data,
        0x0C
    );
}

#[test]
fn fill_mode_supplies_nametable_and_attribute_bytes() {
    let mut mapper = Mmc5::new(test_data());
    Cartridge::initialize(&mut mapper);
    Mapper::write_expansion(&mut mapper, 0x5105, 0xFF, &mut Interrupt::new());
    Mapper::write_expansion(&mut mapper, 0x5106, 0x25, &mut Interrupt::new());
    Mapper::write_expansion(&mut mapper, 0x5107, 0x03, &mut Interrupt::new());
    let mut ciram = vec![0; 0x800];

    assert_eq!(
        mapper
            .read_ppu_nametable(0x2000, PpuReadAccess::CpuData, &mut ciram)
            .data,
        0x25
    );
    assert_eq!(
        mapper
            .read_ppu_nametable(0x23C0, PpuReadAccess::CpuData, &mut ciram)
            .data,
        0xFF
    );
}

#[test]
fn exram_can_be_executed_from_expansion_space() {
    let mut mapper = Mmc5::new(test_data());
    Cartridge::initialize(&mut mapper);
    mapper.write_expansion(0x5104, 0x02, &mut Interrupt::new());
    mapper.write_expansion(0x5C00, 0xA9, &mut Interrupt::new());
    mapper.write_expansion(0x5C01, 0x5A, &mut Interrupt::new());

    assert_eq!(mapper.read_expansion(0x5C00).data, 0xA9);
    assert_eq!(mapper.read_expansion(0x5C01).data, 0x5A);
}

#[test]
fn mirrored_ppuctrl_writes_do_not_toggle_sprite_size() {
    let mut mapper = Mmc5::new(test_data());
    Cartridge::initialize(&mut mapper);
    let mut ppu = PpuCore::new();
    let mut interrupt = Interrupt::new();

    ppu.write_register(0x2008, 0x20, &mut mapper, &mut interrupt);
    assert!(!mapper.sprite_size_16);

    ppu.write_register(0x2000, 0x20, &mut mapper, &mut interrupt);
    assert!(mapper.sprite_size_16);
}

#[test]
fn exram_mode_zero_rejects_cpu_writes() {
    let mut mapper = Mmc5::new(test_data());
    Cartridge::initialize(&mut mapper);
    mapper.write_expansion(0x5104, 0x00, &mut Interrupt::new());
    mapper.write_expansion(0x5C00, 0xA9, &mut Interrupt::new());

    assert_eq!(mapper.read_expansion(0x5C00).mask, 0);

    mapper.write_expansion(0x5104, 0x02, &mut Interrupt::new());
    assert_eq!(mapper.read_expansion(0x5C00).data, 0x00);
}

#[test]
fn exram_mode_zero_only_accepts_cpu_writes_while_in_frame() {
    let mut mapper = Mmc5::new(test_data());
    Cartridge::initialize(&mut mapper);
    mapper.write_expansion(0x5104, 0x00, &mut Interrupt::new());
    mapper.notify_ppu_mask(0x18);
    mapper.write_expansion(0x5C00, 0x12, &mut Interrupt::new());

    mapper.write_expansion(0x5104, 0x02, &mut Interrupt::new());
    assert_eq!(mapper.read_expansion(0x5C00).data, 0x00);

    mapper.write_expansion(0x5104, 0x00, &mut Interrupt::new());
    mapper.in_frame = true;
    mapper.write_expansion(0x5C00, 0x34, &mut Interrupt::new());
    mapper.write_expansion(0x5104, 0x02, &mut Interrupt::new());
    assert_eq!(mapper.read_expansion(0x5C00).data, 0x34);
}

#[test]
fn scanline_irq_pending_latches_until_acknowledged() {
    let mut mapper = Mmc5::new(test_data());
    Cartridge::initialize(&mut mapper);
    let mut interrupt = Interrupt::new();
    mapper.write_expansion(0x5203, 0x01, &mut interrupt);

    for tick in 0..3 {
        emit_ppu_read(&mut mapper, 0x2000, tick, &mut interrupt);
    }
    emit_ppu_read(&mut mapper, 0x23C0, 3, &mut interrupt);

    for tick in 4..7 {
        emit_ppu_read(&mut mapper, 0x2000, tick, &mut interrupt);
    }
    emit_ppu_read(&mut mapper, 0x23C0, 7, &mut interrupt);

    assert_eq!(mapper.read_expansion(0x5204).data, 0xC0);
    assert!(!interrupt.get_irq(IrqSource::EXTERNAL));

    mapper.write_expansion(0x5204, 0x80, &mut interrupt);
    assert!(interrupt.get_irq(IrqSource::EXTERNAL));

    let status = mapper.read_expansion(0x5204).data;
    assert_eq!(status, 0xC0);
    Cartridge::notify_cpu_read(&mut mapper, 0x5204, status, &mut interrupt);

    assert_eq!(mapper.read_expansion(0x5204).data, 0x40);
    assert!(!interrupt.get_irq(IrqSource::EXTERNAL));
}

#[test]
fn in_frame_flag_clears_after_three_idle_cpu_cycles() {
    let mut mapper = Mmc5::new(test_data());
    Cartridge::initialize(&mut mapper);
    let mut interrupt = Interrupt::new();

    for tick in 0..3 {
        emit_ppu_read(&mut mapper, 0x2000, tick, &mut interrupt);
    }
    emit_ppu_read(&mut mapper, 0x23C0, 3, &mut interrupt);
    assert_eq!(mapper.read_expansion(0x5204).data, 0x40);

    // Complete the CPU cycle that observed the last PPU read, then wait three idle cycles.
    mapper.step(&mut interrupt);
    for _ in 0..3 {
        mapper.step(&mut interrupt);
    }

    assert_eq!(mapper.read_expansion(0x5204).data, 0x00);
}

#[test]
fn vertical_split_uses_exram_tile_attribute_and_chr_bank() {
    let mut mapper = Mmc5::new(split_test_data());
    Cartridge::initialize(&mut mapper);
    mapper.notify_ppu_mask(0x18);
    mapper.write_expansion(0x5104, 0x00, &mut Interrupt::new());
    mapper.write_expansion(0x5200, 0x80 | 0x02, &mut Interrupt::new());
    mapper.write_expansion(0x5201, 0x09, &mut Interrupt::new());
    mapper.write_expansion(0x5202, 0x02, &mut Interrupt::new());
    mapper.in_frame = true;
    mapper.scanline_counter = 1;
    mapper.exram[32] = 0x3C;
    mapper.exram[0x03C0] = 0b10;

    let mut ciram = vec![0; 0x800];
    assert_eq!(
        mapper
            .read_ppu_nametable(0x2000, PpuReadAccess::BackgroundNameTable, &mut ciram)
            .data,
        0x3C
    );
    assert_eq!(
        mapper
            .read_ppu_nametable(0x23C0, PpuReadAccess::BackgroundAttribute, &mut ciram)
            .data,
        0xAA
    );
    assert_eq!(
        mapper
            .read_ppu_pattern(
                0x1007,
                PpuReadAccess::BackgroundPattern,
                &mut Interrupt::new()
            )
            .data,
        0x02
    );
}

#[test]
fn vertical_split_threshold_wraps_prefetch_columns_to_next_scanline_left_edge() {
    let mut mapper = Mmc5::new(split_test_data());
    Cartridge::initialize(&mut mapper);
    mapper.notify_ppu_mask(0x18);
    mapper.write_expansion(0x5104, 0x00, &mut Interrupt::new());
    mapper.write_expansion(0x5200, 0x80 | 0x02, &mut Interrupt::new());
    mapper.in_frame = true;
    mapper.scanline_counter = 10;

    assert_eq!(mapper.split_tile_context_for_fetch(0).unwrap().column, 0);
    assert_eq!(mapper.split_tile_context_for_fetch(1).unwrap().column, 1);
    assert_eq!(mapper.split_tile_context_for_fetch(32).unwrap().column, 0);
    assert_eq!(mapper.split_tile_context_for_fetch(33).unwrap().column, 1);

    mapper.write_expansion(0x5200, 0x80 | 0x40 | 0x1F, &mut Interrupt::new());
    assert_eq!(mapper.split_tile_context_for_fetch(31).unwrap().column, 31);
    assert!(mapper.split_tile_context_for_fetch(32).is_none());
    assert!(mapper.split_tile_context_for_fetch(33).is_none());
}

#[test]
fn vertical_split_scroll_uses_attribute_rows_before_wrapping_to_top() {
    let mut mapper = Mmc5::new(split_test_data());
    Cartridge::initialize(&mut mapper);
    mapper.notify_ppu_mask(0x18);
    mapper.write_expansion(0x5104, 0x00, &mut Interrupt::new());
    mapper.write_expansion(0x5200, 0x80 | 0x01, &mut Interrupt::new());
    mapper.write_expansion(0x5201, 248, &mut Interrupt::new());
    mapper.in_frame = true;
    mapper.exram[0x03E0] = 0x5A;
    mapper.exram[0x0000] = 0x24;

    mapper.scanline_counter = 0;
    let attribute_tile = mapper.split_tile_context_for_fetch(0).unwrap();
    assert!(attribute_tile.uses_attribute_tiles);
    assert_eq!(attribute_tile.coarse_y, 31);
    assert_eq!(mapper.split_nametable_byte(attribute_tile), 0x5A);

    mapper.scanline_counter = 8;
    let wrapped_tile = mapper.split_tile_context_for_fetch(0).unwrap();
    assert!(!wrapped_tile.uses_attribute_tiles);
    assert_eq!(wrapped_tile.coarse_y, 0);
    assert_eq!(mapper.split_nametable_byte(wrapped_tile), 0x24);
}

#[test]
fn multiplier_returns_low_and_high_product_bytes() {
    let mut mapper = Mmc5::new(test_data());
    Cartridge::initialize(&mut mapper);
    mapper.write_expansion(0x5205, 13, &mut Interrupt::new());
    mapper.write_expansion(0x5206, 17, &mut Interrupt::new());

    assert_eq!(mapper.read_expansion(0x5205).data, 221);
    assert_eq!(mapper.read_expansion(0x5206).data, 0);
}

#[test]
fn pcm_irq_and_audio_output_follow_register_protocol() {
    let mut mapper = Mmc5::new(test_data());
    Cartridge::initialize(&mut mapper);
    let mut interrupt = Interrupt::new();

    mapper.write_expansion(0x5010, 0x80, &mut interrupt);
    mapper.write_expansion(0x5011, 0x00, &mut interrupt);
    assert!(interrupt.get_irq(IrqSource::EXTERNAL));
    assert_eq!(mapper.read_expansion(0x5010).data, 0x80);

    Cartridge::notify_cpu_read(&mut mapper, 0x5010, 0x80, &mut interrupt);
    assert!(!interrupt.get_irq(IrqSource::EXTERNAL));

    mapper.write_expansion(0x5011, 0x40, &mut interrupt);
    assert!(mapper.expansion_audio_output() > 0.0);
}

#[test]
fn pcm_control_read_does_not_echo_mode_bit() {
    let mut mapper = Mmc5::new(test_data());
    Cartridge::initialize(&mut mapper);
    let mut interrupt = Interrupt::new();

    mapper.write_expansion(0x5010, 0x81, &mut interrupt);
    assert_eq!(mapper.read_expansion(0x5010).data, 0x00);
}

#[test]
fn mmc5a_timer_counts_cpu_cycles_and_acknowledges_on_read() {
    let mut mapper = Mmc5::new(test_data());
    Cartridge::initialize(&mut mapper);
    let mut interrupt = Interrupt::new();

    mapper.write_expansion(0x520A, 0x00, &mut interrupt);
    mapper.write_expansion(0x5209, 0x03, &mut interrupt);
    assert_eq!(mapper.read_expansion(0x5209).data, 0x00);

    mapper.step(&mut interrupt);
    mapper.step(&mut interrupt);
    assert!(!interrupt.get_irq(IrqSource::EXTERNAL));

    mapper.step(&mut interrupt);
    assert!(interrupt.get_irq(IrqSource::EXTERNAL));
    let status = mapper.read_expansion(0x5209).data;
    assert_eq!(status, 0x80);
    Cartridge::notify_cpu_read(&mut mapper, 0x5209, status, &mut interrupt);
    assert!(!interrupt.get_irq(IrqSource::EXTERNAL));
    assert_eq!(mapper.read_expansion(0x5209).data, 0x00);
}

#[test]
fn step_cpu_cycles_matches_repeated_step_for_timer_audio_and_idle_state() {
    fn configure(mapper: &mut Mmc5, interrupt: &mut Interrupt) {
        Cartridge::initialize(mapper);
        mapper.write_expansion(0x5000, 0x3F, interrupt);
        mapper.write_expansion(0x5002, 0x08, interrupt);
        mapper.write_expansion(0x5003, 0xF8, interrupt);
        mapper.write_expansion(0x5004, 0x3F, interrupt);
        mapper.write_expansion(0x5006, 0x10, interrupt);
        mapper.write_expansion(0x5007, 0xF8, interrupt);
        mapper.write_expansion(0x5015, 0x03, interrupt);
        mapper.write_expansion(0x5207, 0x03, interrupt);
        mapper.write_expansion(0x5800, 0x00, interrupt);
        mapper.write_expansion(0x520A, 0x00, interrupt);
        mapper.write_expansion(0x5209, 0x40, interrupt);
        mapper.in_frame = true;
        mapper.ppu_read_seen_this_cpu_cycle = true;
    }

    let mut exact = Mmc5::new(test_data());
    let mut batched = Mmc5::new(test_data());
    let mut exact_interrupt = Interrupt::new();
    let mut batched_interrupt = Interrupt::new();
    configure(&mut exact, &mut exact_interrupt);
    configure(&mut batched, &mut batched_interrupt);

    for _ in 0..64 {
        exact.step(&mut exact_interrupt);
    }
    batched.step_cpu_cycles(64, &mut batched_interrupt);

    assert_eq!(
        Cartridge::export_runtime_state(&batched)
            .expect("batched state should export")
            .extra_body,
        Cartridge::export_runtime_state(&exact)
            .expect("exact state should export")
            .extra_body
    );
    assert_eq!(
        batched_interrupt.irq_flag.bits(),
        exact_interrupt.irq_flag.bits()
    );
}

#[test]
fn hardware_timer_reports_next_cpu_event_distance() {
    let mut mapper = Mmc5::new(test_data());
    Cartridge::initialize(&mut mapper);
    let mut interrupt = Interrupt::new();

    mapper.write_expansion(0x520A, 0x00, &mut interrupt);
    mapper.write_expansion(0x5209, 0x03, &mut interrupt);

    assert_eq!(mapper.cycles_until_next_cpu_event(), 3);
}

#[test]
fn mmc5a_port_status_reflects_configured_output_levels() {
    let mut mapper = Mmc5::new(test_data());
    Cartridge::initialize(&mut mapper);

    mapper.write_expansion(0x5207, 0x00, &mut Interrupt::new());
    mapper.write_expansion(0x5208, 0xC0, &mut Interrupt::new());

    assert_eq!(mapper.read_expansion(0x5208).data, 0xC0);
}

#[test]
fn mmc5a_port_function_bits_idle_high_and_pulse_low_on_access() {
    let mut mapper = Mmc5::new(test_data());
    Cartridge::initialize(&mut mapper);
    let mut interrupt = Interrupt::new();

    mapper.write_expansion(0x5207, 0x03, &mut interrupt);
    assert_eq!(mapper.read_expansion(0x5208).data, 0xC0);

    mapper.write_expansion(0x5800, 0x00, &mut interrupt);
    assert!(!mapper.sl3_pin_level());
    mapper.step(&mut interrupt);
    assert!(mapper.sl3_pin_level());

    Cartridge::notify_cpu_read(&mut mapper, 0x5800, 0x00, &mut interrupt);
    assert!(!mapper.cl3_pin_level());
    mapper.step(&mut interrupt);
    assert!(mapper.cl3_pin_level());
}

#[test]
fn mmc5a_unknown_range_reads_as_open_bus() {
    let mapper = Mmc5::new(test_data());
    assert_eq!(mapper.read_expansion(0x5800).mask, 0);
}
