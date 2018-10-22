// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

extern crate crc;
extern crate nes;
extern crate std as core;

use self::ButtonCode::*;
use self::PadState::{Pressed, Released};
use self::StandardControllerButtonCode::Pad1;
use crc::crc64;
use nes::gui::ScreenBuffer;
use nes::nes::controller::standard_controller::{Buttons, StandardController};
use nes::nes::{Console, Speaker};
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};

struct TestSpeaker {}

impl TestSpeaker {
    pub fn new() -> Self {
        Self {}
    }
}

impl Speaker for TestSpeaker {
    fn push(&mut self, _data: i16) {}
}

struct ScenarioRunner {
    screen_buffer: ScreenBuffer,
    console: Console,
    controller: StandardController,
    speaker: TestSpeaker,
    frame_counter: u64,
    pad1: Buttons,
    pad2: Buttons,
}

impl ScenarioRunner {
    fn new<I: Iterator<Item = u8>>(input: &mut I) -> Self {
        Self {
            screen_buffer: ScreenBuffer::new(),
            console: Console::new(input, 44_100).unwrap(),
            controller: StandardController::new(),
            speaker: TestSpeaker::new(),
            frame_counter: 0,
            pad1: Buttons::empty(),
            pad2: Buttons::empty(),
        }
    }

    fn run(&mut self, scenario: Scenario) {
        let mut tmpscenario = scenario.0.clone();
        tmpscenario.sort_by(|a, b| a.frame_number.cmp(&b.frame_number));
        let mut scenario = VecDeque::from(tmpscenario);

        while !scenario.is_empty() {
            self.on_update();
            while !scenario.is_empty() && scenario[0].frame_number == self.frame_counter {
                match scenario.pop_front().unwrap().operation {
                    ScenarioOperation::CheckScreen { hash } => {
                        let mut hasher = crc64::Digest::new(crc64::ECMA);
                        self.screen_buffer.hash(&mut hasher);
                        if hasher.finish() != hash {
                            panic!(format!(
                                "assertion failed: `(left == right)` \
                                 (left: `0x{:016X}`, right: `0x{:016X}` frame: {})",
                                hasher.finish(),
                                hash,
                                self.frame_counter
                            ));
                        };
                    }
                    ScenarioOperation::Reset => {
                        self.console.reset();
                    }
                    ScenarioOperation::StandardControllerInput { code, state } => match code {
                        StandardControllerButtonCode::Pad1(code) => {
                            self.pad1 = match state {
                                PadState::Pressed => self.pad1 | Buttons::from(code),
                                PadState::Released => self.pad1 & !(Buttons::from(code)),
                            };
                            self.controller.set_pad1(self.pad1);
                        }
                        StandardControllerButtonCode::Pad2(code) => {
                            self.pad2 = match state {
                                PadState::Pressed => self.pad2 | Buttons::from(code),
                                PadState::Released => self.pad2 & !(Buttons::from(code)),
                            };
                            self.controller.set_pad2(self.pad2);
                        }
                    },
                }
            }
        }
    }

    fn on_update(&mut self) {
        while !self.console.step(
            &mut self.screen_buffer,
            &mut self.controller,
            &mut self.speaker,
        ) {}
        self.frame_counter += 1;
    }
}

#[derive(Debug, Copy, Clone)]
enum ButtonCode {
    A,
    B,
    SELECT,
    START,
    UP,
    DOWN,
    LEFT,
    RIGHT,
}

impl From<ButtonCode> for Buttons {
    fn from(v: ButtonCode) -> Self {
        match v {
            ButtonCode::A => Buttons::A,
            ButtonCode::B => Buttons::B,
            ButtonCode::SELECT => Buttons::SELECT,
            ButtonCode::START => Buttons::START,
            ButtonCode::UP => Buttons::UP,
            ButtonCode::DOWN => Buttons::DOWN,
            ButtonCode::LEFT => Buttons::LEFT,
            ButtonCode::RIGHT => Buttons::RIGHT,
        }
    }
}

#[derive(Debug, Copy, Clone)]
enum StandardControllerButtonCode {
    Pad1(ButtonCode),
    #[allow(dead_code)]
    Pad2(ButtonCode),
}

#[derive(Debug, Copy, Clone)]
enum PadState {
    Pressed,
    Released,
}

#[derive(Debug, Copy, Clone)]
enum ScenarioOperation {
    CheckScreen {
        hash: u64,
    },
    Reset,
    StandardControllerInput {
        code: StandardControllerButtonCode,
        state: PadState,
    },
}
impl ScenarioOperation {
    pub fn check_screen(hash: u64) -> Self {
        ScenarioOperation::CheckScreen { hash }
    }
    pub fn standard_controller(code: StandardControllerButtonCode, state: PadState) -> Self {
        ScenarioOperation::StandardControllerInput { code, state }
    }
    pub fn reset() -> Self {
        ScenarioOperation::Reset
    }
}

#[derive(Debug, Copy, Clone)]
struct ScenarioLeaf {
    frame_number: u64,
    operation: ScenarioOperation,
}

impl ScenarioLeaf {
    pub fn new(frame_number: u64, operation: ScenarioOperation) -> Self {
        Self {
            frame_number,
            operation,
        }
    }
    pub fn check_screen(frame_number: u64, hash: u64) -> Self {
        Self::new(frame_number, ScenarioOperation::check_screen(hash))
    }
    pub fn standard_controller(
        frame_number: u64,
        code: StandardControllerButtonCode,
        state: PadState,
    ) -> Self {
        Self::new(
            frame_number,
            ScenarioOperation::standard_controller(code, state),
        )
    }
    pub fn reset(frame_number: u64) -> Self {
        Self::new(frame_number, ScenarioOperation::reset())
    }
}

struct Scenario(Vec<ScenarioLeaf>);

impl Scenario {
    pub fn new(senarios: &[ScenarioLeaf]) -> Self {
        Scenario(senarios.to_vec())
    }
}

macro_rules! test{
    ($filename:expr, $( $x:expr ),+) => {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!(concat!("../../sample_roms/", $filename))
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ $( $x ),* ]);
        runner.run(scenario);
    }
}

mod branch_timing_tests {
    use super::*;

    #[test]
    fn _1_branch_basics() {
        test!(
            "branch_timing_tests/1.Branch_Basics.nes",
            ScenarioLeaf::check_screen(25, 0x081BA42EB6C3294D)
        );
    }

    #[test]
    fn _2_backward_branch() {
        test!(
            "branch_timing_tests/2.Backward_Branch.nes",
            ScenarioLeaf::check_screen(25, 0xE70FF858A009593F)
        );
    }

    #[test]
    fn _3_forward_branch() {
        test!(
            "branch_timing_tests/3.Forward_Branch.nes",
            ScenarioLeaf::check_screen(25, 0xD394B778636B1CEF)
        );
    }
}

#[test]
fn cpu_dummy_reads() {
    test!(
        "cpu_dummy_reads.nes",
        ScenarioLeaf::check_screen(50, 0x68A285C0C944073D)
    );
}

mod cpu_dummy_writes {
    use super::*;

    #[test]
    fn cpu_dummy_writes_oam() {
        test!(
            "cpu_dummy_writes/cpu_dummy_writes_oam.nes",
            ScenarioLeaf::check_screen(330, 0x6AB7DBF3764D9D43)
        );
    }

    #[test]
    fn cpu_dummy_writes_ppumem() {
        test!(
            "cpu_dummy_writes/cpu_dummy_writes_ppumem.nes",
            ScenarioLeaf::check_screen(240, 0xF8A9BE71A106B451)
        );
    }
}

mod cpu_exec_space {
    use super::*;

    #[test]
    fn test_cpu_exec_space_ppuio() {
        test!(
            "cpu_exec_space/test_cpu_exec_space_ppuio.nes",
            ScenarioLeaf::check_screen(45, 0xB1866B91E4771BAB)
        );
    }

    #[test]
    fn test_cpu_exec_space_apu() {
        test!(
            "cpu_exec_space/test_cpu_exec_space_apu.nes",
            ScenarioLeaf::check_screen(295, 0x28EE2FAC59284B74)
        );
    }
}

#[test]
fn nestest() {
    test!(
        "nestest.nes",
        ScenarioLeaf::check_screen(15, 0x43073DD69063B0D2),
        ScenarioLeaf::standard_controller(15, Pad1(START), Pressed),
        ScenarioLeaf::standard_controller(16, Pad1(START), Released),
        ScenarioLeaf::check_screen(70, 0x01A4B722289CD31E),
        ScenarioLeaf::standard_controller(70, Pad1(SELECT), Pressed),
        ScenarioLeaf::standard_controller(71, Pad1(SELECT), Released),
        ScenarioLeaf::check_screen(75, 0xA5763C5F44A6FBED),
        ScenarioLeaf::standard_controller(75, Pad1(START), Pressed),
        ScenarioLeaf::standard_controller(76, Pad1(START), Released),
        ScenarioLeaf::check_screen(90, 0x6FBB66DD65D28A99)
    );
}

#[test]
fn instr_timing() {
    test!(
        "instr_timing/instr_timing.nes",
        ScenarioLeaf::check_screen(1330, 0x5E0E057574FF467B)
    );
}

#[test]
fn instr_misc() {
    test!(
        "instr_misc/instr_misc.nes",
        ScenarioLeaf::check_screen(344, 0xE00704F6A0376CBE)
    );
}

#[test]
fn instr_test_v5() {
    test!(
        "instr_test-v5/all_instrs.nes",
        ScenarioLeaf::check_screen(2450, 0x0D3D1CD1F7F9EC0B)
    );
}

#[test]
fn cpu_timing_test6() {
    test!(
        "cpu_timing_test6/cpu_timing_test.nes",
        ScenarioLeaf::check_screen(639, 0x475DE2E673F715D4)
    );
}

mod blargg_apu_2005_07_30 {
    use super::*;

    #[test]
    fn _01_len_ctr() {
        test!(
            "blargg_apu_2005.07.30/01.len_ctr.nes",
            ScenarioLeaf::check_screen(30, 0x85459C9BE19FB8A0)
        );
    }

    #[test]
    fn _02_len_table() {
        test!(
            "blargg_apu_2005.07.30/02.len_table.nes",
            ScenarioLeaf::check_screen(15, 0x85459C9BE19FB8A0)
        );
    }

    #[test]
    fn _03_irq_flag() {
        test!(
            "blargg_apu_2005.07.30/03.irq_flag.nes",
            ScenarioLeaf::check_screen(30, 0x85459C9BE19FB8A0)
        );
    }

    #[test]
    fn _04_clock_jitter() {
        test!(
            "blargg_apu_2005.07.30/04.clock_jitter.nes",
            ScenarioLeaf::check_screen(30, 0x85459C9BE19FB8A0)
        );
    }

    #[test]
    fn _05_len_timing_mode0() {
        test!(
            "blargg_apu_2005.07.30/05.len_timing_mode0.nes",
            ScenarioLeaf::check_screen(30, 0x85459C9BE19FB8A0)
        );
    }

    #[test]
    fn _06_len_timing_mode1() {
        test!(
            "blargg_apu_2005.07.30/06.len_timing_mode1.nes",
            ScenarioLeaf::check_screen(30, 0x85459C9BE19FB8A0)
        );
    }

    #[test]
    fn _07_irq_flag_timing() {
        test!(
            "blargg_apu_2005.07.30/07.irq_flag_timing.nes",
            ScenarioLeaf::check_screen(30, 0x85459C9BE19FB8A0)
        );
    }

    #[test]
    fn _08_irq_timing() {
        test!(
            "blargg_apu_2005.07.30/08.irq_timing.nes",
            ScenarioLeaf::check_screen(30, 0x85459C9BE19FB8A0)
        );
    }

    #[test]
    fn _09_reset_timing() {
        test!(
            "blargg_apu_2005.07.30/09.reset_timing.nes",
            ScenarioLeaf::check_screen(30, 0x85459C9BE19FB8A0)
        );
    }

    #[test]
    fn _10_len_halt_timing() {
        test!(
            "blargg_apu_2005.07.30/10.len_halt_timing.nes",
            ScenarioLeaf::check_screen(30, 0x85459C9BE19FB8A0)
        );
    }

    #[test]
    fn _11_len_reload_timing() {
        test!(
            "blargg_apu_2005.07.30/11.len_reload_timing.nes",
            ScenarioLeaf::check_screen(30, 0x85459C9BE19FB8A0)
        );
    }
}

mod cpu_reset {
    use super::*;

    #[test]
    fn ram_after_reset() {
        test!(
            "cpu_reset/ram_after_reset.nes",
            ScenarioLeaf::check_screen(155, 0x6C18F33A360A267A),
            ScenarioLeaf::reset(156),
            ScenarioLeaf::check_screen(255, 0xA70256FE525B5712)
        );
    }

    #[test]
    fn registers() {
        test!(
            "cpu_reset/registers.nes",
            ScenarioLeaf::check_screen(155, 0x6C18F33A360A267A),
            ScenarioLeaf::reset(156),
            ScenarioLeaf::check_screen(255, 0x15A2A5B1C285B8CE)
        );
    }
}

mod blargg_ppu_tests_2005_09_15b {
    use super::*;

    #[test]
    fn palette_ram() {
        test!(
            "blargg_ppu_tests_2005.09.15b/palette_ram.nes",
            ScenarioLeaf::check_screen(30, 0x85459C9BE19FB8A0)
        );
    }

    #[test]
    fn power_up_palette() {
        test!(
            "blargg_ppu_tests_2005.09.15b/power_up_palette.nes",
            ScenarioLeaf::check_screen(30, 0x85459C9BE19FB8A0)
        );
    }

    #[test]
    fn sprite_ram() {
        test!(
            "blargg_ppu_tests_2005.09.15b/sprite_ram.nes",
            ScenarioLeaf::check_screen(30, 0x85459C9BE19FB8A0)
        );
    }

    #[test]
    fn vbl_clear_time() {
        test!(
            "blargg_ppu_tests_2005.09.15b/vbl_clear_time.nes",
            ScenarioLeaf::check_screen(30, 0x85459C9BE19FB8A0)
        );
    }

    #[test]
    fn vram_access() {
        test!(
            "blargg_ppu_tests_2005.09.15b/vram_access.nes",
            ScenarioLeaf::check_screen(30, 0x85459C9BE19FB8A0)
        );
    }
}

mod full_palette {
    use super::*;

    #[test]
    fn flowing_palette() {
        test!(
            "full_palette/flowing_palette.nes",
            ScenarioLeaf::check_screen(30, 0x85459C9BE19FB8A0)
        );
    }

    #[test]
    fn full_palette_smooth() {
        test!(
            "full_palette/full_palette_smooth.nes",
            ScenarioLeaf::check_screen(30, 0x85459C9BE19FB8A0)
        );
    }

    #[test]
    fn full_palette() {
        test!(
            "full_palette/full_palette.nes",
            ScenarioLeaf::check_screen(30, 0x85459C9BE19FB8A0)
        );
    }

}

mod nmi_sync {
    use super::*;

    #[test]
    fn demo_ntsc() {
        test!(
            "nmi_sync/demo_ntsc.nes",
            ScenarioLeaf::check_screen(30, 0x85459C9BE19FB8A0)
        );
    }

    #[test]
    fn demo_pal() {
        test!(
            "nmi_sync/demo_pal.nes",
            ScenarioLeaf::check_screen(30, 0x85459C9BE19FB8A0)
        );
    }
}

mod sprite_hit_tests_2005_10_05 {
    use super::*;

    #[test]
    fn _01_basics() {
        test!(
            "sprite_hit_tests_2005.10.05/01.basics.nes",
            ScenarioLeaf::check_screen(36, 0x89392E806F5682F4)
        );
    }

    #[test]
    fn _02_alignment() {
        test!(
            "sprite_hit_tests_2005.10.05/02.alignment.nes",
            ScenarioLeaf::check_screen(34, 0x75D8550D59B6F72B)
        );
    }

    #[test]
    fn _03_corners() {
        test!(
            "sprite_hit_tests_2005.10.05/03.corners.nes",
            ScenarioLeaf::check_screen(34, 0x2983264967F6A253)
        );
    }

    #[test]
    fn _04_flip() {
        test!(
            "sprite_hit_tests_2005.10.05/04.flip.nes",
            ScenarioLeaf::check_screen(34, 0x9BAF184F5F15E8A7)
        );
    }

    #[test]
    fn _05_left_clip() {
        test!(
            "sprite_hit_tests_2005.10.05/05.left_clip.nes",
            ScenarioLeaf::check_screen(34, 0x14DE22738C3636C0)
        );
    }

    #[test]
    fn _06_right_edge() {
        test!(
            "sprite_hit_tests_2005.10.05/06.right_edge.nes",
            ScenarioLeaf::check_screen(34, 0x2270DD899C0E1480)
        );
    }

    #[test]
    fn _07_screen_bottom() {
        test!(
            "sprite_hit_tests_2005.10.05/07.screen_bottom.nes",
            ScenarioLeaf::check_screen(34, 0x5571EB62B8928090)
        );
    }

    #[test]
    fn _08_double_height() {
        test!(
            "sprite_hit_tests_2005.10.05/08.double_height.nes",
            ScenarioLeaf::check_screen(34, 0xC5EE8DB0ABBD48ED)
        );
    }

    #[test]
    fn _09_timing_basics() {
        test!(
            "sprite_hit_tests_2005.10.05/09.timing_basics.nes",
            ScenarioLeaf::check_screen(60, 0x8CED0595749BE2DA)
        );
    }

    #[test]
    fn _10_timing_order() {
        test!(
            "sprite_hit_tests_2005.10.05/10.timing_order.nes",
            ScenarioLeaf::check_screen(60, 0xBDE510E7036C02DD)
        );
    }

    #[test]
    fn _11_edge_timing() {
        test!(
            "sprite_hit_tests_2005.10.05/11.edge_timing.nes",
            ScenarioLeaf::check_screen(60, 0xB3C59FBA25A122C8)
        );
    }
}

mod sprite_overflow_tests {
    use super::*;

    #[test]
    fn _1_basics() {
        test!(
            "sprite_overflow_tests/1.Basics.nes",
            ScenarioLeaf::check_screen(36, 0x64673F9E8279B5DA)
        );
    }

    #[test]
    fn _2_details() {
        test!(
            "sprite_overflow_tests/2.Details.nes",
            ScenarioLeaf::check_screen(36, 0x6857729005806691)
        );
    }

    #[test]
    fn _3_timing() {
        test!(
            "sprite_overflow_tests/3.Timing.nes",
            ScenarioLeaf::check_screen(36, 0x89392E806F5682F4)
        );
    }

    #[test]
    fn _4_obscure() {
        test!(
            "sprite_overflow_tests/4.Obscure.nes",
            ScenarioLeaf::check_screen(36, 0x89392E806F5682F4)
        );
    }

    #[test]
    fn _5_emulator() {
        test!(
            "sprite_overflow_tests/5.Emulator.nes",
            ScenarioLeaf::check_screen(36, 0x0F70D5EEDE382586)
        );
    }
}
