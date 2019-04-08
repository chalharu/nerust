// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn _01_len_ctr() {
    test!(
        "apu/blargg_apu_2005.07.30/01.len_ctr.nes",
        ScenarioLeaf::check_screen(30, 0xE31E_B517_2247_2E30)
    );
}

#[test]
fn _02_len_table() {
    test!(
        "apu/blargg_apu_2005.07.30/02.len_table.nes",
        ScenarioLeaf::check_screen(15, 0xE31E_B517_2247_2E30)
    );
}

#[test]
fn _03_irq_flag() {
    test!(
        "apu/blargg_apu_2005.07.30/03.irq_flag.nes",
        ScenarioLeaf::check_screen(30, 0xE31E_B517_2247_2E30)
    );
}

#[test]
fn _04_clock_jitter() {
    test!(
        "apu/blargg_apu_2005.07.30/04.clock_jitter.nes",
        ScenarioLeaf::check_screen(30, 0xE31E_B517_2247_2E30)
    );
}

#[test]
fn _05_len_timing_mode0() {
    test!(
        "apu/blargg_apu_2005.07.30/05.len_timing_mode0.nes",
        ScenarioLeaf::check_screen(30, 0xE31E_B517_2247_2E30)
    );
}

#[test]
fn _06_len_timing_mode1() {
    test!(
        "apu/blargg_apu_2005.07.30/06.len_timing_mode1.nes",
        ScenarioLeaf::check_screen(30, 0xE31E_B517_2247_2E30)
    );
}

#[test]
fn _07_irq_flag_timing() {
    test!(
        "apu/blargg_apu_2005.07.30/07.irq_flag_timing.nes",
        ScenarioLeaf::check_screen(30, 0xE31E_B517_2247_2E30)
    );
}

#[test]
fn _08_irq_timing() {
    test!(
        "apu/blargg_apu_2005.07.30/08.irq_timing.nes",
        ScenarioLeaf::check_screen(30, 0xE31E_B517_2247_2E30)
    );
}

#[test]
fn _09_reset_timing() {
    test!(
        "apu/blargg_apu_2005.07.30/09.reset_timing.nes",
        ScenarioLeaf::check_screen(30, 0xE31E_B517_2247_2E30)
    );
}

#[test]
fn _10_len_halt_timing() {
    test!(
        "apu/blargg_apu_2005.07.30/10.len_halt_timing.nes",
        ScenarioLeaf::check_screen(30, 0xE31E_B517_2247_2E30)
    );
}

#[test]
fn _11_len_reload_timing() {
    test!(
        "apu/blargg_apu_2005.07.30/11.len_reload_timing.nes",
        ScenarioLeaf::check_screen(30, 0xE31E_B517_2247_2E30)
    );
}
