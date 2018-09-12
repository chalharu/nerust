// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod apu;
mod cartridge;
pub mod controller;
mod cpu;
mod mirror_mode;
mod ppu;

use self::apu::Core as Apu;
use self::cartridge::Cartridge;
use self::controller::Controller;
use self::cpu::Core as Cpu;
use self::mirror_mode::MirrorMode;
use self::ppu::Core as Ppu;
use failure::Error;

pub struct RGB {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl From<u32> for RGB {
    fn from(value: u32) -> RGB {
        RGB {
            red: ((value >> 16) & 0xFF) as u8,
            green: ((value >> 8) & 0xFF) as u8,
            blue: (value & 0xFF) as u8,
        }
    }
}

pub trait Screen {
    fn set_rgb(&mut self, x: u16, y: u16, color: RGB);
}

pub trait Speaker {
    fn push(&mut self, data: i16);
}

pub struct Console {
    cpu: Cpu,
    ppu: Ppu,
    apu: Apu,
    cartridge: Box<Cartridge>,
    wram: [u8; 2048],
}

impl Console {
    pub fn new<I: Iterator<Item = u8>>(
        input: &mut I,
        sound_sample_rate: u32,
    ) -> Result<Console, Error> {
        Ok(Self {
            cpu: Cpu::new(),
            ppu: Ppu::new(),
            apu: Apu::new(sound_sample_rate),
            cartridge: try!(cartridge::try_from(input)),
            wram: [0; 2048],
        })
    }

    pub fn step<S: Screen, C: Controller, SP: Speaker>(
        &mut self,
        screen: &mut S,
        controller: &mut C,
        speaker: &mut SP,
    ) -> bool {
        let mut result = false;
        self.cpu.step(
            &mut self.ppu,
            &mut self.cartridge,
            controller,
            &mut self.apu,
            &mut self.wram,
        );
        for _ in 0..3 {
            if self.ppu.step(screen, &mut self.cartridge, &mut self.cpu) {
                result = true;
            }
            self.cartridge.step();
        }
        self.apu
            .step(&mut self.cpu.state, &mut self.cartridge, speaker);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::controller::{Buttons, StandardController};
    use super::*;
    use core::collections::VecDeque;
    use core::hash::{Hash, Hasher};
    use crc::crc64;
    use {ScreenBuffer, Speaker};

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
                &mut include_bytes!("../../../sample_roms/branch_timing_tests/1.Branch_Basics.nes")
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
                &mut include_bytes!(
                    "../../../sample_roms/branch_timing_tests/2.Backward_Branch.nes"
                ).iter()
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
                &mut include_bytes!(
                    "../../../sample_roms/branch_timing_tests/3.Forward_Branch.nes"
                ).iter()
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
            &mut include_bytes!("../../../sample_roms/cpu_dummy_reads.nes")
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
                &mut include_bytes!(
                    "../../../sample_roms/cpu_dummy_writes/cpu_dummy_writes_oam.nes"
                ).iter()
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
                &mut include_bytes!(
                    "../../../sample_roms/cpu_dummy_writes/cpu_dummy_writes_ppumem.nes"
                ).iter()
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
                &mut include_bytes!(
                    "../../../sample_roms/cpu_exec_space/test_cpu_exec_space_ppuio.nes"
                ).iter()
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
                &mut include_bytes!(
                    "../../../sample_roms/cpu_exec_space/test_cpu_exec_space_apu.nes"
                ).iter()
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
            &mut include_bytes!("../../../sample_roms/nestest.nes")
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

}
