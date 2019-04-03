// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod blargg_ppu_tests_2005_09_15b;
mod oam_read;
mod oam_stress;
// mod oamtest3;
mod ppu_open_bus;
mod ppu_read_buffer;
mod ppu_sprite_hit;
mod ppu_sprite_overflow;
mod ppu_vbl_nmi;
mod sprite_hit_tests_2005_10_05;
mod sprite_overflow_tests;
mod vbl_nmi_timing;

// mod scanline;

/*
none    color_test	rainwarrior	Simple display of any chosen color full-screen	thread
passed  blargg_ppu_tests_2005.09.15b	blargg	Miscellaneous PPU tests (palette ram, sprite ram, etc.)	thread
none    full_palette	blargg	Displays the full palette with all emphasis states, demonstrates direct PPU color control	thread
none    nmi_sync	blargg	Verifies NMI timing by creating a specific pattern on the screen (NTSC & PAL versions)	thread
none    ntsc_torture	rainwarrior	NTSC Torture Test displays visual patterns to demonstrate NTSC signal artifacts	thread
pass    oam_read	blargg	Tests OAM reading ($2004), being sure it reads the byte from OAM at the current address in $2003.	thread
pass    oam_stress	blargg	Thoroughly tests OAM address ($2003) and read/write ($2004)	thread
failed  oamtest3	lidnariq	Utility to upload OAM data via $2003/$2004 - can be used to test for the OAMADDR bug behavior	thread 1 thread 2
        palette	rainwarrior	Palette display requiring only scanline-based palette changes, intended to demonstrate the full palette even on less advanced emulators	thread
failed  ppu_open_bus	blargg	Tests behavior when reading from open-bus PPU bits/registers
passed  ppu_read_buffer	bisqwit	Mammoth test pack tests many aspects of the NES system, mostly centering around the PPU $2007 read buffer	thread
passed  ppu_sprite_hit	blargg	Tests sprite 0 hit behavior and timing	thread
passed  ppu_sprite_overflow	blargg	Tests sprite overflow behavior and timing	thread
passed  ppu_vbl_nmi	blargg	Tests the behavior and timing of the NTSC PPU's VBL flag, NMI enable, and NMI interrupt. Timing is tested to an accuracy of one PPU clock.	thread
        scanline	Quietust	Displays a test screen that will contain glitches if certain portions of the emulation are not perfect.
passed  sprite_hit_tests_2005.10.05	blargg	Generally the same as ppu_sprite_hit (older revision of the tests - ppu_sprite_hit is most likely better)	thread
passed  sprite_overflow_tests	blargg	Generally the same as ppu_sprite_overflow (older revision of the tests - ppu_sprite_overflow is most likely better)	thread
        sprdma_and_dmc_dma	blargg	Tests the cycle stealing behavior of the DMC DMA while running Sprite DMAs	thread
        tvpassfail	tepples	NTSC color and NTSC/PAL pixel aspect ratio test ROM	thread
passed  vbl_nmi_timing	blargg	Generally the same as ppu_vbl_nmi (older revision of the tests - ppu_vbl_nmi is most likely better)	thread
*/

use super::*;
