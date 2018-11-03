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
            // "cpu/cpu_flag_concurrency/test_cpu_flag_concurrency.nes",
            "cpu/cpu_interrupts_v2/cpu_interrupts.nes",
            // "cpu_interrupts_v2/rom_singles/1-cli_latency.nes",
            // "ram_retain/ram_retain.nes",
            // "oamtest3/oam3.nes",
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
            // "apu/blargg_apu_2005.07.30/01.len_ctr.nes",
            // "apu/blargg_apu_2005.07.30/05.len_timing_mode0.nes",
            // "apu/blargg_apu_2005.07.30/08.irq_timing.nes",
            // "apu/test_apu_timers/dmc_pitch.nes",
            // "apu/blargg_apu_2005.07.30/09.reset_timing.nes",
            // "apu/blargg_apu_2005.07.30/10.len_halt_timing.nes",
            // "apu/blargg_apu_2005.07.30/11.len_reload_timing.nes",
            // "cpu_reset/ram_after_reset.nes",
            // "cpu_reset/registers.nes",
            // "full_palette/full_palette.nes",
            // "full_palette/full_palette_smooth.nes",
            // "full_palette/flowing_palette.nes",
            // "nmi_sync/demo_ntsc.nes",
            // "ntsc_torture.nes",
            // "instr_test-v5/all_instrs.nes",
        )).into_iter()
        .cloned(),
        44_100,
    ).unwrap();

    let gui = Gui::new(console);
    gui.run();
}
