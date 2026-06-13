use super::envelope::*;
use super::length_counter::*;
use super::timer::*;

// NTSC
// https://wiki.nesdev.com/w/index.php/APU_Noise
const NOISE_TABLE: [u16; 16] = [
    4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068,
];

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Copy, Clone)]
pub(crate) struct Noise {
    mode: bool,
    shift_register: u16,

    envelope: EnvelopeDao,
    length_counter: LengthCounterDao,
    timer: TimerDao,
}

impl HaveLengthCounterDao for Noise {
    fn length_counter_dao(&self) -> &LengthCounterDao {
        &self.length_counter
    }
    fn length_counter_dao_mut(&mut self) -> &mut LengthCounterDao {
        &mut self.length_counter
    }
}

impl HaveEnvelopeDao for Noise {
    fn envelope_dao(&self) -> &EnvelopeDao {
        &self.envelope
    }
    fn envelope_dao_mut(&mut self) -> &mut EnvelopeDao {
        &mut self.envelope
    }
}

impl HaveLengthCounter for Noise {
    type LengthCounter = Self;
    fn length_counter(&self) -> &Self::LengthCounter {
        self
    }
    fn length_counter_mut(&mut self) -> &mut Self::LengthCounter {
        self
    }
}

impl HaveTimerDao for Noise {
    fn timer_dao(&self) -> &TimerDao {
        &self.timer
    }
    fn timer_dao_mut(&mut self) -> &mut TimerDao {
        &mut self.timer
    }
}

impl Noise {
    pub(crate) fn new() -> Self {
        Self {
            mode: false,
            shift_register: 1,
            envelope: EnvelopeDao::new(),
            length_counter: LengthCounterDao::new(),
            timer: TimerDao::new(),
        }
    }

    pub(crate) fn reset(&mut self) {
        self.length_counter.reset();
        self.envelope.reset();
        self.timer.reset();
        self.timer.set_period(NOISE_TABLE[0] - 1);
        self.mode = false;
        self.shift_register = 1;
    }

    pub(crate) fn write_control(&mut self, value: u8) {
        self.length_counter.set_halt((value & 0x20) != 0);
        self.envelope.set_enabled((value & 0x10) == 0);
        self.envelope.set_period(value & 0x0F);
    }

    pub(crate) fn write_period(&mut self, value: u8) {
        self.mode = (value & 0x80) != 0;
        self.timer
            .set_period(NOISE_TABLE[usize::from(value & 0x0F)] - 1);
    }

    pub(crate) fn write_length(&mut self, value: u8) {
        self.length_counter.set_load(value >> 3);
        self.envelope.restart();
    }

    pub(crate) fn step_timer(&mut self) {
        if self.timer.step_timer() {
            self.shift_register = self.next_shift_register();
        }
    }

    pub(crate) fn step_timer_many(&mut self, cycles: u64) {
        for _ in 0..self.timer.advance(cycles) {
            self.shift_register = self.next_shift_register();
        }
    }

    fn next_shift_register(&self) -> u16 {
        (self.shift_register >> 1)
            | (((self.shift_register & 1)
                ^ ((self.shift_register >> (if self.mode { 6 } else { 1 })) & 1))
                << 14)
    }

    pub(crate) fn output(&self) -> u8 {
        if (self.shift_register & 1) != 0 {
            0
        } else {
            Envelope::get_volume(self)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fft_test::{
        CPU_CLOCK_HZ, FFT_SAMPLE_COUNT, average_band_power, capture_samples, dominant_frequency,
        dominant_frequency_tolerance, power_spectrum, spectral_flatness,
    };
    use super::{NOISE_TABLE, Noise};

    const SHORT_MODE_SEQUENCE_LENGTH: f32 = 93.0;
    const SHORT_MODE_DOMINANT_HARMONIC: f32 = 31.0;
    const SHORT_LENGTH_HALF_FRAMES: usize = 0x0A;

    fn expected_short_mode_peak(period_index: usize) -> f32 {
        SHORT_MODE_DOMINANT_HARMONIC * CPU_CLOCK_HZ
            / (f32::from(NOISE_TABLE[period_index]) * SHORT_MODE_SEQUENCE_LENGTH)
    }

    fn test_fixed_noise(period_index: u8) -> Noise {
        let mut noise = Noise::new();
        noise.write_control(0x3F);
        noise.length_counter.set_enabled(true);
        noise.write_period(0x80 | period_index);
        noise.write_length(0xF8);
        noise.length_counter.step();
        noise
    }

    fn test_fixed_long_mode_noise(period_index: u8) -> Noise {
        let mut noise = Noise::new();
        noise.write_control(0x3F);
        noise.length_counter.set_enabled(true);
        noise.write_period(period_index);
        noise.write_length(0xF8);
        noise.length_counter.step();
        noise
    }

    #[test]
    fn fft_peak_matches_expected_fixed_noise_frequency() {
        let period_index = 5_usize;
        let mut noise = test_fixed_noise(period_index as u8);
        let samples = capture_samples(FFT_SAMPLE_COUNT, || {
            noise.step_timer();
            f32::from(noise.output())
        });
        let dominant = dominant_frequency(&samples, CPU_CLOCK_HZ);

        assert!(
            (dominant - expected_short_mode_peak(period_index)).abs()
                <= dominant_frequency_tolerance(CPU_CLOCK_HZ, FFT_SAMPLE_COUNT)
        );
    }

    #[test]
    fn long_mode_noise_has_broadband_power_spectrum() {
        let mut noise = test_fixed_long_mode_noise(0);
        let samples = capture_samples(FFT_SAMPLE_COUNT, || {
            noise.step_timer();
            f32::from(noise.output())
        });
        let spectrum = power_spectrum(&samples);
        let low_band = average_band_power(&spectrum, CPU_CLOCK_HZ, 500.0, 2_000.0);
        let mid_band = average_band_power(&spectrum, CPU_CLOCK_HZ, 2_000.0, 8_000.0);
        let high_band = average_band_power(&spectrum, CPU_CLOCK_HZ, 8_000.0, 30_000.0);

        assert!((low_band / mid_band) > 0.6);
        assert!((low_band / mid_band) < 1.6);
        assert!((mid_band / high_band) > 0.6);
        assert!((mid_band / high_band) < 1.6);
        assert!(spectral_flatness(&spectrum, CPU_CLOCK_HZ, 500.0, 30_000.0) > 0.45);
    }

    #[test]
    fn long_mode_shift_register_cycles_through_full_32767_state_sequence() {
        let mut noise = Noise::new();
        noise.write_period(0x00);
        let initial = noise.shift_register;

        for _ in 1..32_767 {
            noise.timer.set_value(0);
            noise.step_timer();
            assert_ne!(noise.shift_register, initial);
        }

        noise.timer.set_value(0);
        noise.step_timer();
        assert_eq!(noise.shift_register, initial);
    }

    #[test]
    fn length_counter_stops_fixed_noise_after_expected_half_frames() {
        let mut noise = Noise::new();
        noise.write_control(0x10 | 0x0F);
        noise.length_counter.set_enabled(true);
        noise.write_period(0x09);
        noise.write_length(0x00);
        noise.length_counter.step();

        for _ in 0..(SHORT_LENGTH_HALF_FRAMES - 1) {
            assert!(noise.length_counter.get_status());
            noise.length_counter.step_frame();
        }

        assert!(noise.length_counter.get_status());
        noise.length_counter.step_frame();
        assert!(!noise.length_counter.get_status());

        let trailing = capture_samples(1_024, || {
            noise.step_timer();
            f32::from(noise.output())
        });
        assert!(trailing.iter().all(|sample| *sample == 0.0));
    }
}
