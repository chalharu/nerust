// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn cpu_interrupts() {
    test!(
        "cpu/cpu_interrupts_v2/cpu_interrupts.nes",
        ScenarioLeaf::check_screen(760, 0x404D35A34AC3F6CD)
    );
}

#[test]
fn _1_cli_latency() {
    test!(
        "cpu/cpu_interrupts_v2/rom_singles/1-cli_latency.nes",
        ScenarioLeaf::check_screen(35, 0xFEBD53AC40E8D9AB)
    );
}

#[test]
fn _2_nmi_and_brk() {
    test!(
        "cpu/cpu_interrupts_v2/rom_singles/2-nmi_and_brk.nes",
        ScenarioLeaf::check_screen(115, 0xADBD3EA1BFDEE953)
    );
}

#[test]
fn _3_nmi_and_irq() {
    test!(
        "cpu/cpu_interrupts_v2/rom_singles/3-nmi_and_irq.nes",
        ScenarioLeaf::check_screen(150, 0x7DA93875EBD717A6)
    );
}

#[test]
fn _4_irq_and_dma() {
    test!(
        "cpu/cpu_interrupts_v2/rom_singles/4-irq_and_dma.nes",
        ScenarioLeaf::check_screen(75, 0x24388D1CB8DCFFE8)
    );
}

#[test]
fn _5_branch_delays_irq() {
    test!(
        "cpu/cpu_interrupts_v2/rom_singles/5-branch_delays_irq.nes",
        ScenarioLeaf::check_screen(400, 0x2FC680FF95AC89AA)
    );
}
