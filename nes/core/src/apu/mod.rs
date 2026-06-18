#[cfg(test)]
mod audio_regression_test;
mod dmc;
pub(crate) mod envelope;
#[cfg(test)]
mod fft_test;
mod frame_counter;
pub(crate) mod length_counter;
mod noise;
mod pulse;
pub(crate) mod timer;
mod triangle;

use self::dmc::DMC;
use self::envelope::*;
use self::frame_counter::*;
use self::length_counter::*;
use self::noise::Noise;
use self::pulse::Pulse;
use self::triangle::Triangle;
use crate::Cpu;
use crate::OpenBusReadResult;
use crate::interrupt::{Interrupt, IrqSource};
use crate::persistence_error::PersistenceError;
use nerust_contract_core::audio::AudioBackend;

// // 240Hz フレームシーケンサ
// const FRAME_COUNTER_RATE: f64 = 7457.3875;
// const FRAME_COUNTER_RATE: f64 = 29829.55;
const CLOCK_RATE: u64 = 1_789_773;
const MIN_BULK_SAMPLE_INTERVAL: u64 = 32;

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Clone)]
pub(crate) struct Core {
    #[serde(skip, default = "make_pulse_table")]
    pulse_table: Vec<f32>,
    #[serde(skip, default = "make_tnd_table")]
    tnd_table: Vec<f32>,
    // filter: ChaindFilter<ChaindFilter<PassFilter, PassFilter>, PassFilter>,
    // sample_rate: u32,
    pulse1: Pulse,
    pulse2: Pulse,
    triangle: Triangle,
    noise: Noise,
    dmc: DMC,
    // Channel timers stay CPU-cycle accurate; only exported mixer samples are rate-limited.
    #[serde(default)]
    sample_accumulator: u64,
    frame_counter: FrameCounter,
}

fn make_pulse_table() -> Vec<f32> {
    (0..31)
        .map(|x| 95.52 / (8128.0 / x as f32 + 100.0))
        .collect::<Vec<_>>()
}

fn make_tnd_table() -> Vec<f32> {
    (0..203)
        .map(|x| 163.67 / (24329.0 / x as f32 + 100.0))
        .collect::<Vec<_>>()
}

impl Core {
    pub(crate) fn new(
        // sample_rate: u32,
        interrupt: &mut Interrupt,
    ) -> Self {
        // let sample_reset_cycle = CLOCK_RATE * sample_rate as u64;
        // let filter_sample_rate = CLOCK_RATE as f64 / f64::from(sample_rate);
        let mut result = Self {
            // https://wiki.nesdev.com/w/index.php/APU_Mixer
            pulse_table: make_pulse_table(),
            tnd_table: make_tnd_table(),
            // filter: PassFilter::get_highpass_filter(filter_sample_rate, 90.0)
            //     .chain(PassFilter::get_highpass_filter(filter_sample_rate, 440.0))
            //     .chain(PassFilter::get_lowpass_filter(filter_sample_rate, 14000.0)),
            // sample_rate,
            pulse1: Pulse::new(true),
            pulse2: Pulse::new(false),
            triangle: Triangle::new(),
            noise: Noise::new(),
            dmc: DMC::new(),
            sample_accumulator: 0,
            frame_counter: FrameCounter::new(),
        };
        result.initialize(interrupt);
        result
    }

    pub(crate) fn validate_runtime_state(&self) -> Result<(), PersistenceError> {
        self.pulse1.validate_runtime_state()?;
        self.pulse2.validate_runtime_state()?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn set_pulse_duty_for_test(&mut self, mode: u8, value: u8) {
        self.pulse1.set_duty_for_test(mode, value);
    }

    pub(crate) fn reset(&mut self, interrupt: &mut Interrupt) {
        self.pulse1.reset();
        self.pulse2.reset();
        self.triangle.reset();
        self.noise.reset();
        self.dmc.reset();
        self.sample_accumulator = 0;
        self.frame_counter.reset();
        self.initialize(interrupt);
    }

    fn initialize(&mut self, interrupt: &mut Interrupt) {
        for _ in 0..4 {
            self.step_frame(interrupt);
        }
    }

    pub(crate) fn read_register(
        &mut self,
        address: usize,
        interrupt: &mut Interrupt,
    ) -> OpenBusReadResult {
        match address {
            0x4015 => OpenBusReadResult::new(self.read_status(interrupt), !0x20),
            _ => {
                log::error!("unhandled apu register read at address: 0x{:04X}", address);
                OpenBusReadResult::new(0, 0)
            }
        }
    }

    pub(crate) fn write_register(&mut self, address: usize, value: u8, interrupt: &mut Interrupt) {
        match address {
            0x4000 => self.pulse1.write_control(value),
            0x4001 => self.pulse1.write_sweep(value),
            0x4002 => self.pulse1.write_timer_low(value),
            0x4003 => self.pulse1.write_timer_high(value),
            0x4004 => self.pulse2.write_control(value),
            0x4005 => self.pulse2.write_sweep(value),
            0x4006 => self.pulse2.write_timer_low(value),
            0x4007 => self.pulse2.write_timer_high(value),
            0x4008 => self.triangle.write_control(value),
            0x4009 => (),
            0x4010 => self.dmc.write_control(value, interrupt),
            0x4011 => self.dmc.write_value(value),
            0x4012 => self.dmc.write_address(value),
            0x4013 => self.dmc.write_length(value),
            0x400A => self.triangle.write_timer_low(value),
            0x400B => self.triangle.write_timer_high(value),
            0x400C => self.noise.write_control(value),
            0x400D => (),
            0x400E => self.noise.write_period(value),
            0x400F => self.noise.write_length(value),
            0x4015 => self.write_control(value, interrupt),
            0x4017 => self.frame_counter.write_frame_counter(value, interrupt),
            _ => log::error!("unhandled apu register write at address: 0x{:04X}", address),
        }
    }

    pub(crate) fn dmc_fill(&mut self, value: u8, interrupt: &mut Interrupt) {
        self.dmc.fill(value, interrupt);
    }

    pub(crate) fn dmc_fill_address(&self) -> Option<usize> {
        self.dmc.fill_address()
    }

    fn step_frame(&mut self, interrupt: &mut Interrupt) {
        match self.frame_counter.step_frame_counter(interrupt) {
            FrameType::Half => {
                self.quarter_frame();
                self.half_frame();
            }
            FrameType::Quarter => self.quarter_frame(),
            FrameType::None => (),
        }
        self.step_timer(interrupt);
    }

    pub(crate) fn step<M: AudioBackend>(
        &mut self,
        cpu: &mut Cpu,
        mixer: &mut M,
        mixer_sample_rate: u32,
        expansion_audio_output: f32,
        expansion_audio_inverted: bool,
    ) {
        self.step_frame(cpu.interrupt_mut());
        if self.sample_accumulator >= CLOCK_RATE {
            self.sample_accumulator %= CLOCK_RATE;
        }
        self.sample_accumulator += u64::from(mixer_sample_rate).min(CLOCK_RATE);
        if self.sample_accumulator >= CLOCK_RATE {
            self.sample_accumulator -= CLOCK_RATE;
            self.send_sample(mixer, expansion_audio_output, expansion_audio_inverted);
        }
    }

    pub(crate) fn step_many<M: AudioBackend>(
        &mut self,
        cpu: &mut Cpu,
        mixer: &mut M,
        mixer_sample_rate: u32,
        expansion_audio_output: f32,
        expansion_audio_inverted: bool,
        cycles: u64,
    ) {
        for _ in 0..cycles {
            self.step_frame(cpu.interrupt_mut());
            if self.sample_accumulator >= CLOCK_RATE {
                self.sample_accumulator %= CLOCK_RATE;
            }
            self.sample_accumulator += u64::from(mixer_sample_rate).min(CLOCK_RATE);
            if self.sample_accumulator >= CLOCK_RATE {
                self.sample_accumulator -= CLOCK_RATE;
                self.send_sample(mixer, expansion_audio_output, expansion_audio_inverted);
            }
        }
    }

    pub(crate) fn step_many_batched<M: AudioBackend>(
        &mut self,
        cpu: &mut Cpu,
        mixer: &mut M,
        mixer_sample_rate: u32,
        expansion_audio_output: f32,
        expansion_audio_inverted: bool,
        cycles: u64,
    ) {
        let mut remaining = cycles;
        while remaining > 0 {
            let next_frame = self.frame_counter.cycles_until_next_frame_event();
            let next_dmc_dma = self.dmc.cycles_until_next_dma_request(remaining);
            if next_frame == 1 || next_dmc_dma == 1 {
                self.step(
                    cpu,
                    mixer,
                    mixer_sample_rate,
                    expansion_audio_output,
                    expansion_audio_inverted,
                );
                remaining -= 1;
                continue;
            }

            let segment = remaining
                .min(next_frame - 1)
                .min(next_dmc_dma - 1)
                .min(self.cycles_until_next_sample(mixer_sample_rate));
            self.advance_no_frame_event(
                segment,
                cpu.interrupt_mut(),
                mixer,
                mixer_sample_rate,
                expansion_audio_output,
                expansion_audio_inverted,
            );
            remaining -= segment;
        }
    }

    pub(crate) fn should_step_many_exact(mixer_sample_rate: u32) -> bool {
        u64::from(mixer_sample_rate)
            .min(CLOCK_RATE)
            .saturating_mul(MIN_BULK_SAMPLE_INTERVAL)
            > CLOCK_RATE
    }

    pub(crate) fn cycles_until_next_sample(&self, mixer_sample_rate: u32) -> u64 {
        let increment = u64::from(mixer_sample_rate).min(CLOCK_RATE);
        if increment == 0 {
            return u64::MAX;
        }

        let accumulator = self.sample_accumulator % CLOCK_RATE;
        (CLOCK_RATE - accumulator).div_ceil(increment).max(1)
    }

    pub(crate) fn cycles_until_next_scheduler_event(
        &self,
        interrupt: &Interrupt,
        max_cycles: u64,
    ) -> u64 {
        self.frame_counter
            .cycles_until_next_irq_change(interrupt, max_cycles)
            .min(self.dmc.cycles_until_next_dma_request(max_cycles))
    }

    pub(crate) fn send_sample<M: AudioBackend>(
        &self,
        mixer: &mut M,
        expansion_audio_output: f32,
        expansion_audio_inverted: bool,
    ) {
        mixer.push(self.output(expansion_audio_output, expansion_audio_inverted));
        // let output = self.output();
        // let filtered = self.filter.step(output);
        // speaker.push(((filtered * 65535.0) as i32 - 32768) as i16);
    }

    pub(crate) fn output(
        &self,
        expansion_audio_output: f32,
        expansion_audio_inverted: bool,
    ) -> f32 {
        let apu_output = self.pulse_table
            [usize::from(self.pulse1.output()) + usize::from(self.pulse2.output())]
            + self.tnd_table[3 * usize::from(self.triangle.output())
                + 2 * usize::from(self.noise.output())
                + usize::from(self.dmc.output())];
        if expansion_audio_inverted {
            (0.5 + (apu_output - expansion_audio_output) * 0.5).clamp(0.0, 1.0)
        } else {
            (apu_output + expansion_audio_output).clamp(0.0, 1.0)
        }
    }

    fn quarter_frame(&mut self) {
        self.step_envelope();
    }

    fn half_frame(&mut self) {
        self.step_sweep();
        self.step_length();
    }

    fn step_timer(&mut self, interrupt: &mut Interrupt) {
        self.pulse1.step_length_counter();
        self.pulse2.step_length_counter();
        self.noise.step_length_counter();
        self.pulse1.step_timer();
        self.pulse2.step_timer();
        self.noise.step_timer();
        self.dmc.step_timer(interrupt);
        self.triangle.step_timer();
    }

    fn advance_no_frame_event<M: AudioBackend>(
        &mut self,
        cycles: u64,
        interrupt: &mut Interrupt,
        mixer: &mut M,
        mixer_sample_rate: u32,
        expansion_audio_output: f32,
        expansion_audio_inverted: bool,
    ) {
        debug_assert!(cycles > 0);
        debug_assert!(cycles < self.frame_counter.cycles_until_next_frame_event());
        debug_assert!(cycles <= self.cycles_until_next_sample(mixer_sample_rate));

        self.frame_counter.advance_no_event(cycles);
        self.advance_timers(cycles, interrupt);
        self.advance_sample_accumulator(
            cycles,
            mixer,
            mixer_sample_rate,
            expansion_audio_output,
            expansion_audio_inverted,
        );
    }

    fn advance_timers(&mut self, cycles: u64, interrupt: &mut Interrupt) {
        self.pulse1.step_length_counter();
        self.pulse2.step_length_counter();
        self.noise.step_length_counter();
        self.pulse1.step_timer_many(cycles);
        self.pulse2.step_timer_many(cycles);
        self.noise.step_timer_many(cycles);
        self.dmc.step_timer_many(cycles, interrupt);
        self.triangle.step_timer_many(cycles);
    }

    fn advance_sample_accumulator<M: AudioBackend>(
        &mut self,
        cycles: u64,
        mixer: &mut M,
        mixer_sample_rate: u32,
        expansion_audio_output: f32,
        expansion_audio_inverted: bool,
    ) {
        let increment = u64::from(mixer_sample_rate).min(CLOCK_RATE);
        if increment == 0 {
            return;
        }

        if self.sample_accumulator >= CLOCK_RATE {
            self.sample_accumulator %= CLOCK_RATE;
        }
        let total = self.sample_accumulator + cycles * increment;
        debug_assert!(total < CLOCK_RATE * 2);
        self.sample_accumulator = total % CLOCK_RATE;
        if total >= CLOCK_RATE {
            self.send_sample(mixer, expansion_audio_output, expansion_audio_inverted);
        }
    }

    fn step_envelope(&mut self) {
        self.pulse1.step_envelope();
        self.pulse2.step_envelope();
        self.noise.step_envelope();
        self.triangle.step_counter();
    }

    fn step_sweep(&mut self) {
        self.pulse1.step_sweep();
        self.pulse2.step_sweep();
    }

    fn step_length(&mut self) {
        self.pulse1.step_length();
        self.pulse2.step_length();
        self.noise.step_length();
        self.triangle.step_length();
    }

    fn read_status(&mut self, interrupt: &mut Interrupt) -> u8 {
        let result = (if self.pulse1.get_status() { 1 } else { 0 })
            | (if self.pulse2.get_status() { 2 } else { 0 })
            | (if self.triangle.get_status() { 4 } else { 0 })
            | (if self.noise.get_status() { 8 } else { 0 })
            | (if self.dmc.get_status() { 0x10 } else { 0 })
            | (if interrupt.get_irq(IrqSource::FRAME_COUNTER) {
                0x40
            } else {
                0
            })
            | (if interrupt.get_irq(IrqSource::DMC) {
                0x80
            } else {
                0
            });
        interrupt.clear_irq(IrqSource::FRAME_COUNTER);
        result
    }

    fn write_control(&mut self, value: u8, interrupt: &mut Interrupt) {
        self.pulse1.set_enabled((value & 1) != 0);
        self.pulse2.set_enabled((value & 2) != 0);
        self.triangle.set_enabled((value & 4) != 0);
        self.noise.set_enabled((value & 8) != 0);
        self.dmc.set_enabled((value & 16) != 0, interrupt);
    }
}

#[cfg(test)]
mod tests {
    use super::Core;
    use crate::cpu::Core as Cpu;
    use crate::interrupt::Interrupt;
    use nerust_contract_core::audio::AudioBackend;

    struct CapturingMixer {
        samples: Vec<f32>,
        sample_rate: u32,
    }

    impl CapturingMixer {
        fn new(sample_rate: u32) -> Self {
            Self {
                samples: Vec::new(),
                sample_rate,
            }
        }
    }

    impl Default for CapturingMixer {
        fn default() -> Self {
            Self::new(44_100)
        }
    }

    impl AudioBackend for CapturingMixer {
        fn start(&mut self) {}
        fn pause(&mut self) {}
        fn push(&mut self, data: f32) {
            self.samples.push(data);
        }

        fn sample_rate(&self) -> u32 {
            self.sample_rate
        }
    }

    fn configured_test_apu() -> (Core, Interrupt) {
        let mut interrupt = Interrupt::new();
        let mut apu = Core::new(&mut interrupt);
        apu.write_register(0x4015, 0x0F, &mut interrupt);
        apu.write_register(0x4000, 0x3F, &mut interrupt);
        apu.write_register(0x4002, 0x08, &mut interrupt);
        apu.write_register(0x4003, 0xF8, &mut interrupt);
        apu.write_register(0x4008, 0xFF, &mut interrupt);
        apu.write_register(0x400A, 0x20, &mut interrupt);
        apu.write_register(0x400B, 0xF8, &mut interrupt);
        apu.write_register(0x400C, 0x3F, &mut interrupt);
        apu.write_register(0x400E, 0x03, &mut interrupt);
        apu.write_register(0x400F, 0xF8, &mut interrupt);
        apu.write_register(0x4010, 0x0F, &mut interrupt);
        apu.write_register(0x4011, 0x20, &mut interrupt);
        apu.write_register(0x4012, 0x00, &mut interrupt);
        apu.write_register(0x4013, 0x00, &mut interrupt);
        apu.write_register(0x4015, 0x1F, &mut interrupt);
        if interrupt.dmc_dma_request.take().is_some() {
            apu.dmc_fill(0xAA, &mut interrupt);
        }
        apu.write_register(0x4017, 0x00, &mut interrupt);
        (apu, interrupt)
    }

    #[test]
    fn inverted_expansion_audio_mix_centers_silence_and_flips_contribution() {
        let mut interrupt = Interrupt::new();
        let apu = Core::new(&mut interrupt);

        assert!((apu.output(0.0, false) - 0.0).abs() < f32::EPSILON);
        assert!((apu.output(0.0, true) - 0.5).abs() < f32::EPSILON);
        assert!(apu.output(0.25, true) < 0.5);
    }

    #[test]
    fn step_many_matches_repeated_step_across_samples_and_frame_events() {
        let (mut exact, interrupt) = configured_test_apu();
        let mut batched = exact.clone();
        let mut exact_cpu = Cpu::new();
        let mut batched_cpu = Cpu::new();
        *exact_cpu.interrupt_mut() = interrupt;
        *batched_cpu.interrupt_mut() = interrupt;
        let mut exact_mixer = CapturingMixer::default();
        let mut batched_mixer = CapturingMixer::default();
        let sample_rate = exact_mixer.sample_rate();

        for _ in 0..40_000 {
            exact.step(&mut exact_cpu, &mut exact_mixer, sample_rate, 0.0, false);
        }
        batched.step_many_batched(
            &mut batched_cpu,
            &mut batched_mixer,
            sample_rate,
            0.0,
            false,
            40_000,
        );

        assert_eq!(batched_mixer.samples, exact_mixer.samples);
        assert_eq!(format!("{:?}", batched), format!("{:?}", exact));
        assert_eq!(
            format!("{:?}", batched_cpu.interrupt_ref()),
            format!("{:?}", exact_cpu.interrupt_ref())
        );
    }

    #[test]
    fn step_many_exact_matches_repeated_step_at_high_sample_rate() {
        let (mut exact, interrupt) = configured_test_apu();
        let mut many = exact.clone();
        let mut exact_cpu = Cpu::new();
        let mut many_cpu = Cpu::new();
        *exact_cpu.interrupt_mut() = interrupt;
        *many_cpu.interrupt_mut() = interrupt;
        let mut exact_mixer = CapturingMixer::new(192_000);
        let mut many_mixer = CapturingMixer::new(192_000);
        let sample_rate = exact_mixer.sample_rate();

        assert!(Core::should_step_many_exact(sample_rate));
        for _ in 0..40_000 {
            exact.step(&mut exact_cpu, &mut exact_mixer, sample_rate, 0.0, false);
        }
        many.step_many(
            &mut many_cpu,
            &mut many_mixer,
            sample_rate,
            0.0,
            false,
            40_000,
        );

        assert_eq!(many_mixer.samples, exact_mixer.samples);
        assert_eq!(format!("{:?}", many), format!("{:?}", exact));
        assert_eq!(
            format!("{:?}", many_cpu.interrupt_ref()),
            format!("{:?}", exact_cpu.interrupt_ref())
        );
    }
}
