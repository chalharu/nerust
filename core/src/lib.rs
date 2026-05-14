#![allow(
    dead_code,
    reason = "emulator components include future mapper/APU hooks"
)]

// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod apu;
mod cartridge;
pub mod controller;
mod cpu;
mod ppu;
mod status;

use self::apu::Core as Apu;
use self::cartridge::Cartridge;
use self::controller::Controller;
use self::cpu::Core as Cpu;
use self::cpu::trace_jit::TraceJit;
use self::ppu::Core as Ppu;
use self::status::mirror_mode::MirrorMode;
use nerust_screen_traits::Screen;
use nerust_sound_traits::MixerInput;
use std::sync::OnceLock;

const TRACE_JIT_ENV: &str = "NERUST_TRACE_JIT";

pub type Error = Box<dyn std::error::Error + Send + Sync + 'static>;

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
pub struct Core {
    cpu: Cpu,
    ppu: Ppu,
    apu: Apu,
    cartridge: Box<dyn Cartridge>,
    #[serde(skip)]
    trace_jit: Option<Box<TraceJit>>,
}

impl Core {
    pub fn new<I: Iterator<Item = u8>>(input: &mut I) -> Result<Core, Error> {
        let mut cpu = Cpu::new();
        let mut cartridge = cartridge::try_from(input)?;
        let apu = Apu::new(cpu.interrupt_mut(), cartridge.as_mut());
        Ok(Self {
            cpu,
            ppu: Ppu::new(),
            apu,
            cartridge,
            trace_jit: None,
        })
    }

    pub fn reset(&mut self) {
        self.cpu.reset();
        self.ppu.reset();
        self.apu
            .reset(self.cpu.interrupt_mut(), self.cartridge.as_mut());
        if let Some(trace_jit) = self.trace_jit.as_mut() {
            trace_jit.clear();
        }
    }

    pub fn step<S: Screen, M: MixerInput>(
        &mut self,
        screen: &mut S,
        controller: &mut dyn Controller,
        mixer: &mut M,
    ) -> bool {
        self.step_cycle(screen, controller, mixer, mixer.sample_rate())
    }

    pub fn run_frame<S: Screen, M: MixerInput>(
        &mut self,
        screen: &mut S,
        controller: &mut dyn Controller,
        mixer: &mut M,
    ) -> u64 {
        let mixer_sample_rate = mixer.sample_rate();
        if trace_jit_runtime_enabled() {
            self.run_frame_trace_jit(screen, controller, mixer, mixer_sample_rate)
        } else {
            self.run_frame_interpreted(screen, controller, mixer, mixer_sample_rate)
        }
    }

    #[inline(always)]
    fn run_frame_interpreted<S: Screen, M: MixerInput>(
        &mut self,
        screen: &mut S,
        controller: &mut dyn Controller,
        mixer: &mut M,
        mixer_sample_rate: u32,
    ) -> u64 {
        let mut cycles = 0;
        loop {
            cycles += 1;
            if self.step_cycle(screen, controller, mixer, mixer_sample_rate) {
                return cycles;
            }
        }
    }

    #[inline(never)]
    fn run_frame_trace_jit<S: Screen, M: MixerInput>(
        &mut self,
        screen: &mut S,
        controller: &mut dyn Controller,
        mixer: &mut M,
        mixer_sample_rate: u32,
    ) -> u64 {
        let mut cycles = 0;
        self.trace_jit
            .get_or_insert_with(|| Box::new(TraceJit::default()));
        loop {
            if self.cpu.is_trace_jit_boundary() {
                let trace_budget = self
                    .ppu
                    .trace_jit_safe_cpu_cycles()
                    .min(self.apu.trace_jit_safe_cpu_cycles());
                let trace_cycles = {
                    let trace_jit = self
                        .trace_jit
                        .as_deref_mut()
                        .expect("trace JIT cache should be initialized");
                    self.cpu.try_run_trace_jit(
                        trace_jit,
                        &mut self.ppu,
                        self.cartridge.as_mut(),
                        controller,
                        &mut self.apu,
                        trace_budget,
                    )
                };
                if let Some(trace_cycles) = trace_cycles {
                    cycles += trace_cycles;
                    if self.advance_devices(screen, mixer, mixer_sample_rate, trace_cycles) {
                        return cycles;
                    }
                    continue;
                }
            }

            cycles += 1;
            if self.step_cycle(screen, controller, mixer, mixer_sample_rate) {
                return cycles;
            }
        }
    }

    #[inline(always)]
    fn step_cycle<S: Screen, M: MixerInput>(
        &mut self,
        screen: &mut S,
        controller: &mut dyn Controller,
        mixer: &mut M,
        mixer_sample_rate: u32,
    ) -> bool {
        // 1CPUサイクルにつき、APUは1、PPUはNTSC=>3,PAL=>3.2となる
        let mut result = false;
        self.cpu.step(
            &mut self.ppu,
            self.cartridge.as_mut(),
            controller,
            &mut self.apu,
        );
        for _ in 0..3 {
            if self
                .ppu
                .step(screen, self.cartridge.as_mut(), self.cpu.interrupt_mut())
            {
                result = true;
            }
        }
        self.cartridge.step();
        self.apu.step(
            &mut self.cpu,
            self.cartridge.as_mut(),
            mixer,
            mixer_sample_rate,
        );

        result
    }

    fn advance_devices<S: Screen, M: MixerInput>(
        &mut self,
        screen: &mut S,
        mixer: &mut M,
        mixer_sample_rate: u32,
        cycles: u64,
    ) -> bool {
        let mut result = false;
        for _ in 0..cycles {
            result |= self.ppu.advance_trace_jit(
                screen,
                self.cartridge.as_mut(),
                self.cpu.interrupt_mut(),
                3,
            );
            self.cartridge.step();
            self.apu.step(
                &mut self.cpu,
                self.cartridge.as_mut(),
                mixer,
                mixer_sample_rate,
            );
        }
        result
    }

    #[inline(always)]
    fn step_devices<S: Screen, M: MixerInput>(
        &mut self,
        screen: &mut S,
        mixer: &mut M,
        mixer_sample_rate: u32,
    ) -> bool {
        let mut result = false;
        for _ in 0..3 {
            if self
                .ppu
                .step(screen, self.cartridge.as_mut(), self.cpu.interrupt_mut())
            {
                result = true;
            }
        }
        self.cartridge.step();
        self.apu.step(
            &mut self.cpu,
            self.cartridge.as_mut(),
            mixer,
            mixer_sample_rate,
        );

        result
    }
}

fn trace_jit_runtime_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os(TRACE_JIT_ENV).is_some())
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Clone, Copy)]
struct OpenBus {
    data: u8,
}

impl OpenBus {
    pub(crate) fn new() -> Self {
        Self { data: 0 }
    }

    pub(crate) fn unite(&mut self, data: OpenBusReadResult) -> u8 {
        let result = (self.data & !data.mask) | (data.data & data.mask);
        self.data = result;
        result
    }
}

#[derive(Debug, Copy, Clone)]
pub struct OpenBusReadResult {
    pub data: u8,
    pub mask: u8,
}

impl OpenBusReadResult {
    pub fn new(data: u8, mask: u8) -> Self {
        Self { data, mask }
    }
}
