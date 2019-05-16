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
        ScenarioLeaf::check_screen(760, 0x9026_DAD6_5555_ECA0)
    );
}

#[test]
fn _1_cli_latency() {
    test!(
        "cpu/cpu_interrupts_v2/rom_singles/1-cli_latency.nes",
        ScenarioLeaf::check_screen(35, 0x06A2_E9A5_AD65_ED0E)
    );
}

#[test]
fn _2_nmi_and_brk() {
    test!(
        "cpu/cpu_interrupts_v2/rom_singles/2-nmi_and_brk.nes",
        ScenarioLeaf::check_screen(115, 0x2FB0_BFAC_269F_D16A)
    );
}

#[test]
fn _3_nmi_and_irq() {
    test!(
        "cpu/cpu_interrupts_v2/rom_singles/3-nmi_and_irq.nes",
        ScenarioLeaf::check_screen(150, 0x9A87_75FF_7A96_97DF)
    );
}

#[test]
fn _4_irq_and_dma() {
    test!(
        "cpu/cpu_interrupts_v2/rom_singles/4-irq_and_dma.nes",
        ScenarioLeaf::check_screen(75, 0x70E9_CDAE_61FC_6CE6)
    );
}

#[test]
fn _5_branch_delays_irq() {
    test!(
        "cpu/cpu_interrupts_v2/rom_singles/5-branch_delays_irq.nes",
        ScenarioLeaf::check_screen(400, 0xDCF0_4EA0_9FBB_490C)
    );
}
