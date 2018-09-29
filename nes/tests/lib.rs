// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

extern crate crc;
extern crate nes;
extern crate std as core;

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
}

struct Scenario(Vec<ScenarioLeaf>);

impl Scenario {
    pub fn new(senarios: &[ScenarioLeaf]) -> Self {
        Scenario(senarios.to_vec())
    }
}

mod branch_timing_tests {
    use super::*;

    #[test]
    fn _1_branch_basics() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/branch_timing_tests/1.Branch_Basics.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            25,
            ScenarioOperation::check_screen(0x081BA42EB6C3294D),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn _2_backward_branch() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/branch_timing_tests/2.Backward_Branch.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            25,
            ScenarioOperation::check_screen(0xE70FF858A009593F),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn _3_forward_branch() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/branch_timing_tests/3.Forward_Branch.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            25,
            ScenarioOperation::check_screen(0xD394B778636B1CEF),
        )]);
        runner.run(scenario);
    }
}

#[test]
fn cpu_dummy_reads() {
    let mut runner = ScenarioRunner::new(
        &mut include_bytes!("../../sample_roms/cpu_dummy_reads.nes")
            .iter()
            .cloned(),
    );
    let scenario = Scenario::new(&vec![ScenarioLeaf::new(
        50,
        ScenarioOperation::check_screen(0x68A285C0C944073D),
    )]);
    runner.run(scenario);
}

mod cpu_dummy_writes {
    use super::*;

    #[test]
    fn cpu_dummy_writes_oam() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/cpu_dummy_writes/cpu_dummy_writes_oam.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            330,
            ScenarioOperation::check_screen(0x6AB7DBF3764D9D43),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn cpu_dummy_writes_ppumem() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/cpu_dummy_writes/cpu_dummy_writes_ppumem.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            240,
            ScenarioOperation::check_screen(0xF8A9BE71A106B451),
        )]);
        runner.run(scenario);
    }
}

mod cpu_exec_space {
    use super::*;

    #[test]
    fn test_cpu_exec_space_ppuio() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/cpu_exec_space/test_cpu_exec_space_ppuio.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            45,
            ScenarioOperation::check_screen(0xB1866B91E4771BAB),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn test_cpu_exec_space_apu() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/cpu_exec_space/test_cpu_exec_space_apu.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            295,
            ScenarioOperation::check_screen(0x28EE2FAC59284B74),
        )]);
        runner.run(scenario);
    }
}

#[test]
fn nestest() {
    let mut runner = ScenarioRunner::new(
        &mut include_bytes!("../../sample_roms/nestest.nes")
            .iter()
            .cloned(),
    );
    let scenario = Scenario::new(&vec![
        ScenarioLeaf::new(15, ScenarioOperation::check_screen(0x43073DD69063B0D2)),
        ScenarioLeaf::new(
            15,
            ScenarioOperation::standard_controller(
                StandardControllerButtonCode::Pad1(ButtonCode::START),
                PadState::Pressed,
            ),
        ),
        ScenarioLeaf::new(
            16,
            ScenarioOperation::standard_controller(
                StandardControllerButtonCode::Pad1(ButtonCode::START),
                PadState::Released,
            ),
        ),
        ScenarioLeaf::new(70, ScenarioOperation::check_screen(0x01A4B722289CD31E)),
        ScenarioLeaf::new(
            70,
            ScenarioOperation::standard_controller(
                StandardControllerButtonCode::Pad1(ButtonCode::SELECT),
                PadState::Pressed,
            ),
        ),
        ScenarioLeaf::new(
            71,
            ScenarioOperation::standard_controller(
                StandardControllerButtonCode::Pad1(ButtonCode::SELECT),
                PadState::Released,
            ),
        ),
        ScenarioLeaf::new(75, ScenarioOperation::check_screen(0xA5763C5F44A6FBED)),
        ScenarioLeaf::new(
            75,
            ScenarioOperation::standard_controller(
                StandardControllerButtonCode::Pad1(ButtonCode::START),
                PadState::Pressed,
            ),
        ),
        ScenarioLeaf::new(
            76,
            ScenarioOperation::standard_controller(
                StandardControllerButtonCode::Pad1(ButtonCode::START),
                PadState::Released,
            ),
        ),
        ScenarioLeaf::new(90, ScenarioOperation::check_screen(0x6FBB66DD65D28A99)),
    ]);
    runner.run(scenario);
}

#[test]
fn instr_timing() {
    let mut runner = ScenarioRunner::new(
        &mut include_bytes!("../../sample_roms/instr_timing/instr_timing.nes")
            .iter()
            .cloned(),
    );
    let scenario = Scenario::new(&vec![ScenarioLeaf::new(
        1330,
        ScenarioOperation::check_screen(0x5E0E057574FF467B),
    )]);
    runner.run(scenario);
}

#[test]
fn instr_misc() {
    let mut runner = ScenarioRunner::new(
        &mut include_bytes!("../../sample_roms/instr_misc/instr_misc.nes")
            .iter()
            .cloned(),
    );
    let scenario = Scenario::new(&vec![ScenarioLeaf::new(
        344,
        ScenarioOperation::check_screen(0xE00704F6A0376CBE),
    )]);
    runner.run(scenario);
}

#[test]
fn instr_test_v5() {
    let mut runner = ScenarioRunner::new(
        &mut include_bytes!("../../sample_roms/instr_test-v5/all_instrs.nes")
            .iter()
            .cloned(),
    );
    let scenario = Scenario::new(&vec![ScenarioLeaf::new(
        2400,
        ScenarioOperation::check_screen(0x0D3D1CD1F7F9EC0B),
    )]);
    runner.run(scenario);
}

#[test]
fn cpu_timing_test6() {
    let mut runner = ScenarioRunner::new(
        &mut include_bytes!("../../sample_roms/cpu_timing_test6/cpu_timing_test.nes")
            .iter()
            .cloned(),
    );
    let scenario = Scenario::new(&vec![ScenarioLeaf::new(
        639,
        ScenarioOperation::check_screen(0x475DE2E673F715D4),
    )]);
    runner.run(scenario);
}

mod blargg_apu_2005_07_30 {
    use super::*;

    #[test]
    fn _01_len_ctr() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/blargg_apu_2005.07.30/01.len_ctr.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            30,
            ScenarioOperation::check_screen(0x85459C9BE19FB8A0),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn _02_len_table() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/blargg_apu_2005.07.30/02.len_table.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            15,
            ScenarioOperation::check_screen(0x85459C9BE19FB8A0),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn _03_irq_flag() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/blargg_apu_2005.07.30/03.irq_flag.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            30,
            ScenarioOperation::check_screen(0x85459C9BE19FB8A0),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn _04_clock_jitter() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/blargg_apu_2005.07.30/04.clock_jitter.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            30,
            ScenarioOperation::check_screen(0x85459C9BE19FB8A0),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn _05_len_timing_mode0() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/blargg_apu_2005.07.30/05.len_timing_mode0.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            30,
            ScenarioOperation::check_screen(0x85459C9BE19FB8A0),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn _06_len_timing_mode1() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/blargg_apu_2005.07.30/06.len_timing_mode1.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            30,
            ScenarioOperation::check_screen(0x85459C9BE19FB8A0),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn _07_irq_flag_timing() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/blargg_apu_2005.07.30/07.irq_flag_timing.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            30,
            ScenarioOperation::check_screen(0x85459C9BE19FB8A0),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn _08_irq_timing() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/blargg_apu_2005.07.30/08.irq_timing.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            30,
            ScenarioOperation::check_screen(0x85459C9BE19FB8A0),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn _09_reset_timing() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/blargg_apu_2005.07.30/09.reset_timing.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            30,
            ScenarioOperation::check_screen(0x85459C9BE19FB8A0),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn _10_len_halt_timing() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/blargg_apu_2005.07.30/10.len_halt_timing.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            30,
            ScenarioOperation::check_screen(0x85459C9BE19FB8A0),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn _11_len_reload_timing() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/blargg_apu_2005.07.30/11.len_reload_timing.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            30,
            ScenarioOperation::check_screen(0x85459C9BE19FB8A0),
        )]);
        runner.run(scenario);
    }
}

mod cpu_reset {
    use super::*;

    #[test]
    fn ram_after_reset() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/cpu_reset/ram_after_reset.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            45,
            ScenarioOperation::check_screen(0xB1866B91E4771BAB),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn registers() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/cpu_reset/registers.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            295,
            ScenarioOperation::check_screen(0x28EE2FAC59284B74),
        )]);
        runner.run(scenario);
    }
}

mod blargg_ppu_tests_2005_09_15b {
    use super::*;

    #[test]
    fn palette_ram() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/blargg_ppu_tests_2005.09.15b/palette_ram.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            30,
            ScenarioOperation::check_screen(0x85459C9BE19FB8A0),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn power_up_palette() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!(
                "../../sample_roms/blargg_ppu_tests_2005.09.15b/power_up_palette.nes"
            ).iter()
            .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            30,
            ScenarioOperation::check_screen(0x85459C9BE19FB8A0),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn sprite_ram() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/blargg_ppu_tests_2005.09.15b/sprite_ram.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            30,
            ScenarioOperation::check_screen(0x85459C9BE19FB8A0),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn vbl_clear_time() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!(
                "../../sample_roms/blargg_ppu_tests_2005.09.15b/vbl_clear_time.nes"
            ).iter()
            .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            30,
            ScenarioOperation::check_screen(0x85459C9BE19FB8A0),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn vram_access() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/blargg_ppu_tests_2005.09.15b/vram_access.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            30,
            ScenarioOperation::check_screen(0x85459C9BE19FB8A0),
        )]);
        runner.run(scenario);
    }
}

mod full_palette {
    use super::*;

    #[test]
    fn flowing_palette() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/full_palette/flowing_palette.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            30,
            ScenarioOperation::check_screen(0x85459C9BE19FB8A0),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn full_palette_smooth() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/full_palette/full_palette_smooth.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            30,
            ScenarioOperation::check_screen(0x85459C9BE19FB8A0),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn full_palette() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/full_palette/full_palette.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            30,
            ScenarioOperation::check_screen(0x85459C9BE19FB8A0),
        )]);
        runner.run(scenario);
    }

}

mod nmi_sync {
    use super::*;

    #[test]
    fn demo_ntsc() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/nmi_sync/demo_ntsc.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            30,
            ScenarioOperation::check_screen(0x85459C9BE19FB8A0),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn demo_pal() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/nmi_sync/demo_pal.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            30,
            ScenarioOperation::check_screen(0x85459C9BE19FB8A0),
        )]);
        runner.run(scenario);
    }
}

mod sprite_hit_tests_2005_10_05 {
    use super::*;

    #[test]
    fn _01_basics() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/sprite_hit_tests_2005.10.05/01.basics.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            36,
            ScenarioOperation::check_screen(0x89392E806F5682F4),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn _02_alignment() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/sprite_hit_tests_2005.10.05/02.alignment.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            34,
            ScenarioOperation::check_screen(0x75D8550D59B6F72B),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn _03_corners() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/sprite_hit_tests_2005.10.05/03.corners.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            34,
            ScenarioOperation::check_screen(0x2983264967F6A253),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn _04_flip() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/sprite_hit_tests_2005.10.05/04.flip.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            34,
            ScenarioOperation::check_screen(0x9BAF184F5F15E8A7),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn _05_left_clip() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/sprite_hit_tests_2005.10.05/05.left_clip.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            34,
            ScenarioOperation::check_screen(0x14DE22738C3636C0),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn _06_right_edge() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/sprite_hit_tests_2005.10.05/06.right_edge.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            34,
            ScenarioOperation::check_screen(0x2270DD899C0E1480),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn _07_screen_bottom() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!(
                "../../sample_roms/sprite_hit_tests_2005.10.05/07.screen_bottom.nes"
            ).iter()
            .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            34,
            ScenarioOperation::check_screen(0x5571EB62B8928090),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn _08_double_height() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!(
                "../../sample_roms/sprite_hit_tests_2005.10.05/08.double_height.nes"
            ).iter()
            .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            34,
            ScenarioOperation::check_screen(0xC5EE8DB0ABBD48ED),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn _09_timing_basics() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!(
                "../../sample_roms/sprite_hit_tests_2005.10.05/09.timing_basics.nes"
            ).iter()
            .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            60,
            ScenarioOperation::check_screen(0x8CED0595749BE2DA),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn _10_timing_order() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!(
                "../../sample_roms/sprite_hit_tests_2005.10.05/10.timing_order.nes"
            ).iter()
            .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            60,
            ScenarioOperation::check_screen(0xBDE510E7036C02DD),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn _11_edge_timing() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/sprite_hit_tests_2005.10.05/11.edge_timing.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            60,
            ScenarioOperation::check_screen(0xB3C59FBA25A122C8),
        )]);
        runner.run(scenario);
    }
}

mod sprite_overflow_tests {
    use super::*;

    #[test]
    fn _1_basics() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/sprite_overflow_tests/1.Basics.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            36,
            ScenarioOperation::check_screen(0x64673F9E8279B5DA),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn _2_details() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/sprite_overflow_tests/2.Details.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            36,
            ScenarioOperation::check_screen(0x6857729005806691),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn _3_timing() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/sprite_overflow_tests/3.Timing.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            36,
            ScenarioOperation::check_screen(0x89392E806F5682F4),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn _4_obscure() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/sprite_overflow_tests/4.Obscure.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            36,
            ScenarioOperation::check_screen(0x89392E806F5682F4),
        )]);
        runner.run(scenario);
    }

    #[test]
    fn _5_emulator() {
        let mut runner = ScenarioRunner::new(
            &mut include_bytes!("../../sample_roms/sprite_overflow_tests/5.Emulator.nes")
                .iter()
                .cloned(),
        );
        let scenario = Scenario::new(&vec![ScenarioLeaf::new(
            36,
            ScenarioOperation::check_screen(0x0F70D5EEDE382586),
        )]);
        runner.run(scenario);
    }
}
