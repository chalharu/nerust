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
            "tests/Lan Master/Lan_Master.nes",
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
            // "ppu/oamtest3/oam3.nes",
            // "apu/apu_mixer/noise.nes",
            // "apu/test_apu_timers/square_pitch.nes"
            // "apu/test_apu_timers/triangle_pitch.nes"
            // "apu/test_apu_timers/noise_pitch.nes"
            // "apu/test_apu_timers/dmc_pitch.nes"
            // "apu/apu_phase_reset/apu_phase_reset.nes",
            // "mapper/34_test_src/34_test_2.nes",
            // "ppu/ppu_sprite_hit/ppu_prite_hit.nes"

            // "color_test.nes",
            // "tests/tvpassfail/tv.nes",
            // "apu/test_apu_timers/dmc_pitch.nes",
            // "ppu/ppu_open_bus/ppu_open_bus.nes",
            // "cpu_reset/ram_after_reset.nes",
            // "cpu_reset/registers.nes",
            // "full_palette/full_palette.nes",
            // "full_palette/full_palette_smooth.nes",
            // "full_palette/flowing_palette.nes",
            // "nmi_sync/demo_ntsc.nes",
            // "ntsc_torture.nes",
            // "tests/coredump-v1.3.nes",

            // "tests/240pee-0.15/240pee.nes",
            // "tests/240pee-0.15/240pee-bnrom.nes",
            // "tests/allpads.nes",
            // "tests/ram_retain/ram_retain.nes",

            // "tests/scanline/scanline.nes",
            // "tests/nmi_sync/demo_ntsc.nes",
            // "tests/ntsc_torture.nes",
        )).iter()
        .cloned(),
    ).unwrap();

    let gui = Gui::new(console);
    gui.run();
}
