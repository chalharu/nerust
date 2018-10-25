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
            "../../../sample_roms/",
            // "sample1.nes",
            // "giko005.nes",
            // "giko008.nes",
            // "giko009.nes",
            // "giko010.nes",
            // "giko010b.nes",
            // "giko011.nes",
            // "giko012.nes",
            // "giko013.nes",
            // "giko014.nes",
            // "giko014b.nes",
            // "giko015.nes",
            // "giko016.nes",
            // "giko017.nes",
            // "giko018.nes",
            // "cpu_flag_concurrency/test_cpu_flag_concurrency.nes",
            // "cpu_interrupts_v2/cpu_interrupts.nes",
            // "cpu_interrupts_v2/rom_singles/1-cli_latency.nes",
            // "blargg_apu_2005.07.30/03.irq_flag.nes",
            // "blargg_apu_2005.07.30/04.clock_jitter.nes",
            // "blargg_apu_2005.07.30/05.len_timing_mode0.nes",
            // "blargg_apu_2005.07.30/06.len_timing_mode1.nes",
            // "blargg_apu_2005.07.30/07.irq_flag_timing.nes",
            // "blargg_apu_2005.07.30/08.irq_timing.nes",
            // "ram_retain/ram_retain.nes",
            // "oamtest3/oam3.nes",
            "allpads.nes",
            // "bntest/bntest_aorom.nes",
            // "bntest/bntest_h.nes",
            // "bntest/bntest_v.nes",
            // "240pee-0.15/240pee-bnrom.nes",
            // "240pee-0.15/240pee.nes",
            // "bntest/bntest_h.nes",
            // "bntest/bntest_v.nes",
            // "color_test.nes",
            // "tvpassfail/tv.nes",
            // "blargg_apu_2005.07.30/09.reset_timing.nes",
            // "blargg_apu_2005.07.30/10.len_halt_timing.nes",
            // "blargg_apu_2005.07.30/11.len_reload_timing.nes",
            // "cpu_reset/ram_after_reset.nes",
            // "cpu_reset/registers.nes",
            // "full_palette/full_palette.nes",
            // "full_palette/full_palette_smooth.nes",
            // "full_palette/flowing_palette.nes",
            // "nmi_sync/demo_ntsc.nes",
            // "ntsc_torture.nes",
            // "sprite_hit_tests_2005.10.05/02.alignment.nes",
            // "sprite_hit_tests_2005.10.05/09.timing_basics.nes",
            // "sprite_hit_tests_2005.10.05/11.edge_timing.nes"
            // "sprite_overflow_tests/3.Timing.nes",
            // "sprite_overflow_tests/4.Obscure.nes",
            // "sprite_overflow_tests/5.Emulator.nes",
            // "instr_test-v5/all_instrs.nes",
            // "nestest.nes",
            // "branch_timing_tests/1.Branch_Basics.nes",
            // "branch_timing_tests/2.Backward_Branch.nes",
            // "branch_timing_tests/3.Forward_Branch.nes",
            // "instr_timing/instr_timing.nes",
            // "instr_misc/instr_misc.nes",
            // "instr_misc/rom_singles/04-dummy_reads_apu.nes",
            // "instr_timing/rom_singles/1-instr_timing.nes",
            // "instr_timing/rom_singles/2-branch_timing.nes",
        )).into_iter()
        .cloned(),
        44_100,
    ).unwrap();

    let gui = Gui::new(console);
    gui.run();
}
