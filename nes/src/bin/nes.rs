// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

extern crate nes;
extern crate simple_logger;

use nes::gui::Gui;
use nes::nes::Console;

fn main() {
    // log initialize
    simple_logger::init().unwrap();

    let console = Console::new(
        &mut include_bytes!(concat!(
            "../../../roms/",
            // "samples/sample1.nes",
            // "giko005.nes",
            // "giko008.nes",
            // "giko009.nes",
            // "giko010.nes",
            // "samples/gikones/giko010b.nes",
            // "samples/gikones/giko011.nes",
            // "samples/gikones/giko012.nes",
            // "samples/gikones/giko013.nes",
            // "samples/gikones/giko014.nes",
            // "samples/gikones/giko014b.nes",
            // "samples/gikones/giko015.nes",
            // "samples/gikones/giko016.nes",
            // "samples/gikones/giko017.nes",
            // "samples/gikones/giko018.nes",
            // "cpu/cpu_flag_concurrency/test_cpu_flag_concurrency.nes",
            // "cpu/cpu_interrupts_v2/cpu_interrupts.nes",
            // "cpu/cpu_interrupts_v2/rom_singles/1-cli_latency.nes",
            // "cpu/cpu_interrupts_v2/rom_singles/2-nmi_and_brk.nes",
            // "cpu/cpu_interrupts_v2/rom_singles/3-nmi_and_irq.nes",
            // "cpu/cpu_interrupts_v2/rom_singles/4-irq_and_dma.nes",
            // "cpu/cpu_interrupts_v2/rom_singles/5-branch_delays_irq.nes",
            // "ram_retain/ram_retain.nes",
            // "ppu/oamtest3/oam3.nes",
            // "allpads.nes",
            // "apu/apu_mixer/noise.nes",
            // "apu/apu_phase_reset/apu_phase_reset.nes",
            // "mapper/bntest/bntest_aorom.nes",
            // "bntest/bntest_h.nes",
            // "bntest/bntest_v.nes",
            // "240pee-0.15/240pee-bnrom.nes",
            // "mapper/240pee-0.15/240pee.nes",
            // "bntest/bntest_h.nes",
            // "bntest/bntest_v.nes",
            // "color_test.nes",
            // "tvpassfail/tv.nes",
            // "apu/test_apu_timers/dmc_pitch.nes",
            // "apu/blargg_apu_2005.07.30/09.reset_timing.nes",
            // "ppu/ppu_open_bus/ppu_open_bus.nes",
            // "cpu_reset/ram_after_reset.nes",
            // "cpu_reset/registers.nes",
            // "full_palette/full_palette.nes",
            // "full_palette/full_palette_smooth.nes",
            // "full_palette/flowing_palette.nes",
            // "nmi_sync/demo_ntsc.nes",
            // "ntsc_torture.nes",
            // "instr_test-v5/all_instrs.nes",
            // "apu/test_apu_env/test_apu_env.nes",
            // "ppu/ppu_read_buffer/test_ppu_read_buffer.nes",
            // "cpu/cpu_exec_space/test_cpu_exec_space_ppuio.nes",
            // "tests/coredump-v1.3.nes",
            // "ppu/ppu_sprite_hit/ppu_sprite_hit.nes",
            // "ppu/ppu_sprite_overflow/ppu_sprite_overflow.nes",
            // "ppu/ppu_sprite_hit/rom_singles/01-basics.nes", 60, 0x42CFACAA9A15D013
            // "ppu/ppu_sprite_hit/rom_singles/02-alignment.nes", 85, 0xB1B5C43737FB16EF
            // "ppu/ppu_sprite_hit/rom_singles/03-corners.nes", 55. 0x700017AB5E656ECD
            // "ppu/ppu_sprite_hit/rom_singles/04-flip.nes", 40, 0x549F8BF6A80774B1
            // "ppu/ppu_sprite_hit/rom_singles/05-left_clip.nes", 52, 0xCB232878F232040A
            // "ppu/ppu_sprite_hit/rom_singles/06-right_edge.nes", 45, 0x89567E17B702EED4
            "ppu/ppu_sprite_hit/rom_singles/07-screen_bottom.nes",
            // "ppu/ppu_sprite_hit/rom_singles/08-double_height.nes", 40, 0xCCFC308B39369365
            // "ppu/ppu_sprite_hit/rom_singles/09-timing.nes",
            // "ppu/ppu_sprite_hit/rom_singles/10-timing_order.nes", 90, 0x65F210E6178421E0
        )).into_iter()
        .cloned(),
        44_100,
    ).unwrap();

    let gui = Gui::new(console);
    gui.run();
}
