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
        // &mut include_bytes!("../../../sample_roms/sample1.nes")
        // &mut include_bytes!("../../../sample_roms/giko005.nes")
        // &mut include_bytes!("../../../sample_roms/giko008.nes")
        // &mut include_bytes!("../../../sample_roms/giko009.nes")
        // &mut include_bytes!("../../../sample_roms/giko010.nes")
        // &mut include_bytes!("../../../sample_roms/giko010b.nes")
        // &mut include_bytes!("../../../sample_roms/giko011.nes")
        // &mut include_bytes!("../../../sample_roms/giko012.nes")
        // &mut include_bytes!("../../../sample_roms/giko013.nes")
        // &mut include_bytes!("../../../sample_roms/giko014.nes")
        // &mut include_bytes!("../../../sample_roms/giko014b.nes")
        // &mut include_bytes!("../../../sample_roms/giko015.nes")
        // &mut include_bytes!("../../../sample_roms/giko016.nes")
        // &mut include_bytes!("../../../sample_roms/giko017.nes")
        // &mut include_bytes!("../../../sample_roms/giko018.nes")
        &mut include_bytes!("../../../sample_roms/cpu_flag_concurrency/test_cpu_flag_concurrency.nes")
        // &mut include_bytes!("../../../sample_roms/cpu_interrupts_v2/cpu_interrupts.nes")
        // &mut include_bytes!("../../../sample_roms/cpu_interrupts_v2/rom_singles/1-cli_latency.nes")
        // &mut include_bytes!("../../../sample_roms/blargg_apu_2005.07.30/03.irq_flag.nes")
        // &mut include_bytes!("../../../sample_roms/blargg_apu_2005.07.30/04.clock_jitter.nes")
        // &mut include_bytes!("../../../sample_roms/blargg_apu_2005.07.30/05.len_timing_mode0.nes")
        // &mut include_bytes!("../../../sample_roms/blargg_apu_2005.07.30/06.len_timing_mode1.nes")
        // &mut include_bytes!("../../../sample_roms/blargg_apu_2005.07.30/07.irq_flag_timing.nes")
        // &mut include_bytes!("../../../sample_roms/blargg_apu_2005.07.30/08.irq_timing.nes")
        // &mut include_bytes!("../../../sample_roms/blargg_apu_2005.07.30/09.reset_timing.nes")
        // &mut include_bytes!("../../../sample_roms/blargg_apu_2005.07.30/10.len_halt_timing.nes")
        // &mut include_bytes!("../../../sample_roms/blargg_apu_2005.07.30/11.len_reload_timing.nes")
        // &mut include_bytes!("../../../sample_roms/cpu_reset/ram_after_reset.nes")
        // &mut include_bytes!("../../../sample_roms/cpu_reset/registers.nes")
        // &mut include_bytes!("../../../sample_roms/full_palette/full_palette.nes")
        // &mut include_bytes!("../../../sample_roms/full_palette/full_palette_smooth.nes")
        // &mut include_bytes!("../../../sample_roms/full_palette/flowing_palette.nes")
        // &mut include_bytes!("../../../sample_roms/nmi_sync/demo_ntsc.nes")
        // &mut include_bytes!("../../../sample_roms/ntsc_torture.nes")
        // &mut include_bytes!("../../../sample_roms/sprite_hit_tests_2005.10.05/02.alignment.nes")
        // &mut include_bytes!("../../../sample_roms/sprite_overflow_tests/3.Timing.nes")
        // &mut include_bytes!("../../../sample_roms/sprite_overflow_tests/4.Obscure.nes")

        // &mut include_bytes!("../../../sample_roms/instr_test-v5/all_instrs.nes")
        // &mut include_bytes!("../../../sample_roms/nestest.nes")
        // &mut include_bytes!("../../../sample_roms/branch_timing_tests/1.Branch_Basics.nes")
        // &mut include_bytes!("../../../sample_roms/branch_timing_tests/2.Backward_Branch.nes")
        // &mut include_bytes!("../../../sample_roms/branch_timing_tests/3.Forward_Branch.nes")
        // &mut include_bytes!("../../../sample_roms/instr_timing/instr_timing.nes")
        // &mut include_bytes!("../../../sample_roms/instr_misc/instr_misc.nes")
        // &mut include_bytes!("../../../sample_roms/instr_misc/rom_singles/04-dummy_reads_apu.nes")
        // &mut include_bytes!("../../../sample_roms/instr_timing/rom_singles/1-instr_timing.nes")
        // &mut include_bytes!("../../../sample_roms/instr_timing/rom_singles/2-branch_timing.nes")
            .into_iter()
            .cloned(),
        44_100,
    ).unwrap();

    let gui = Gui::new(console);
    gui.run();
}
