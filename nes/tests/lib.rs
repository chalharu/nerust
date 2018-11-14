// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

extern crate crc;
extern crate hound;
extern crate nes;
extern crate std as core;

#[cfg(test)]
#[macro_use]
mod macros;

mod apu;
mod cpu;
mod input;
mod mapper;
mod ppu;

use self::ButtonCode::*;
use self::PadState::{Pressed, Released};
use self::StandardControllerButtonCode::Pad1;
use crc::crc64;
use nes::gui::filterset::FilterType;
use nes::gui::{LogicalSize, ScreenBuffer};
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
            screen_buffer: ScreenBuffer::new(
                FilterType::None,
                LogicalSize {
                    width: 256,
                    height: 240,
                },
            ),
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

// mod full_palette {
//     use super::*;

//     #[test]
//     fn flowing_palette() {
//         test!(
//             "full_palette/flowing_palette.nes",
//             ScenarioLeaf::check_screen(30, 0xE31EB51722472E30)
//         );
//     }

//     #[test]
//     fn full_palette_smooth() {
//         test!(
//             "full_palette/full_palette_smooth.nes",
//             ScenarioLeaf::check_screen(30, 0xE31EB51722472E30)
//         );
//     }

//     #[test]
//     fn full_palette() {
//         test!(
//             "full_palette/full_palette.nes",
//             ScenarioLeaf::check_screen(30, 0xE31EB51722472E30)
//         );
//     }
// }

// mod nmi_sync {
//     use super::*;

//     #[test]
//     fn demo_ntsc() {
//         test!(
//             "nmi_sync/demo_ntsc.nes",
//             ScenarioLeaf::check_screen(30, 0xE31EB51722472E30)
//         );
//     }

//     #[test]
//     fn demo_pal() {
//         test!(
//             "nmi_sync/demo_pal.nes",
//             ScenarioLeaf::check_screen(30, 0xE31EB51722472E30)
//         );
//     }
// }
