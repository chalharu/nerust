// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::fft_test::CPU_CLOCK_HZ;
use crate::cartridge_data::{CartridgeData, CartridgeDataParts};
use crate::controller::standard_controller::StandardController;
use crate::core::Core;
use hound::{SampleFormat, WavReader, WavSpec, WavWriter};
use nerust_contract::{MirrorMode, RomFormat};
use nerust_screen_traits::Screen;
use nerust_sound_traits::MixerInput;
use std::io::Cursor;

const ANALYSIS_WINDOW_SECONDS: f32 = 0.001;
const HALF_FRAME_SECONDS: f32 = 14_914.0 / CPU_CLOCK_HZ;
const QUARTER_FRAME_SECONDS: f32 = HALF_FRAME_SECONDS / 2.0;
const PULSE_TIMER_LOW: u8 = 0x40;
const ENVELOPE_PERIOD: u8 = 0x02;
const MAX_LENGTH_INDEX: u8 = 0x01;
const FRAME_COUNTER_LENGTH_INDEX: u8 = 0x00;
const SILENCE_THRESHOLD: f32 = 0.001;
const DEFAULT_SILENCE_TAIL_SECONDS: f32 = 0.05;
const TEST_SAMPLE_RATE: u32 = 24_000;
const LENGTH_TEST_SAMPLE_RATE: u32 = 12_000;
const LENGTH_TABLE: [u8; 32] = [
    0x0A, 0xFE, 0x14, 0x02, 0x28, 0x04, 0x50, 0x06, 0xA0, 0x08, 0x3C, 0x0A, 0x0E, 0x0C, 0x1A, 0x0E,
    0x0C, 0x10, 0x18, 0x12, 0x30, 0x14, 0x60, 0x16, 0xC0, 0x18, 0x48, 0x1A, 0x10, 0x1C, 0x20, 0x1E,
];

#[derive(Default)]
struct NullScreen;

impl Screen for NullScreen {
    fn push(&mut self, _palette: u8) {}

    fn render(&mut self) {}
}

#[derive(Debug, Clone)]
struct CapturingMixer {
    sample_rate: u32,
    samples: Vec<f32>,
}

impl CapturingMixer {
    fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            samples: Vec::new(),
        }
    }

    fn has_activity(&self) -> bool {
        self.samples
            .iter()
            .any(|sample| sample.abs() > SILENCE_THRESHOLD)
    }

    fn tail_is_silent(&self, silence_seconds: f32) -> bool {
        let tail_len = ((self.sample_rate as f32) * silence_seconds).ceil() as usize;
        if self.samples.len() < tail_len {
            return false;
        }
        self.samples[self.samples.len() - tail_len..]
            .iter()
            .all(|sample| sample.abs() <= SILENCE_THRESHOLD)
    }
}

impl MixerInput for CapturingMixer {
    fn push(&mut self, data: f32) {
        self.samples.push(data);
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

#[derive(Debug, Clone)]
struct AudioCapture {
    sample_rate: u32,
    samples: Vec<f32>,
}

impl AudioCapture {
    fn new(sample_rate: u32, samples: Vec<f32>) -> Self {
        Self {
            sample_rate,
            samples,
        }
    }

    fn rms_envelope(&self, window_seconds: f32) -> Vec<f32> {
        let window_len = ((self.sample_rate as f32) * window_seconds).round() as usize;
        let window_len = window_len.max(1);
        self.samples.chunks(window_len).map(rms).collect::<Vec<_>>()
    }

    fn active_window_range(&self, window_seconds: f32, threshold_ratio: f32) -> (usize, usize) {
        let envelope = self.rms_envelope(window_seconds);
        let peak = envelope.iter().copied().fold(0.0, f32::max);
        let threshold = peak * threshold_ratio;
        let first = envelope
            .iter()
            .position(|value| *value >= threshold)
            .expect("audio should become active");
        let last = envelope
            .iter()
            .rposition(|value| *value >= threshold)
            .expect("audio should remain active for at least one window");
        (first, last)
    }

    fn first_active_time(&self, window_seconds: f32, threshold_ratio: f32) -> f32 {
        self.active_window_range(window_seconds, threshold_ratio).0 as f32 * window_seconds
    }

    fn active_duration(&self, window_seconds: f32, threshold_ratio: f32) -> f32 {
        let (first, last) = self.active_window_range(window_seconds, threshold_ratio);
        (last + 1 - first) as f32 * window_seconds
    }

    fn segment_rms(&self, start_seconds: f32, end_seconds: f32) -> f32 {
        assert!(end_seconds > start_seconds);
        let start =
            ((start_seconds * self.sample_rate as f32).floor() as usize).min(self.samples.len());
        let end = ((end_seconds * self.sample_rate as f32).ceil() as usize).min(self.samples.len());
        assert!(
            end > start,
            "requested segment should include audio samples"
        );
        rms(&self.samples[start..end])
    }
}

fn rms(samples: &[f32]) -> f32 {
    let energy = samples.iter().map(|sample| sample * sample).sum::<f32>();
    (energy / samples.len() as f32).sqrt()
}

fn encode_wav(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = WavWriter::new(
            &mut cursor,
            WavSpec {
                channels: 1,
                sample_rate,
                bits_per_sample: 16,
                sample_format: SampleFormat::Int,
            },
        )
        .expect("in-memory WAV writer should initialize");
        for &sample in samples {
            let pcm = (sample.clamp(0.0, 1.0) * f32::from(i16::MAX)).round() as i16;
            writer
                .write_sample(pcm)
                .expect("in-memory WAV write should succeed");
        }
        writer.finalize().expect("WAV writer should finalize");
    }
    cursor.into_inner()
}

fn decode_wav(bytes: &[u8]) -> AudioCapture {
    let mut reader = WavReader::new(Cursor::new(bytes)).expect("WAV bytes should decode");
    let sample_rate = reader.spec().sample_rate;
    let samples = reader
        .samples::<i16>()
        .map(|sample| f32::from(sample.expect("PCM sample should decode")) / f32::from(i16::MAX))
        .collect::<Vec<_>>();
    AudioCapture::new(sample_rate, samples)
}

fn emit_lda_immediate(program: &mut Vec<u8>, value: u8) {
    program.extend_from_slice(&[0xA9, value]);
}

fn emit_sta_absolute(program: &mut Vec<u8>, address: u16) {
    program.extend_from_slice(&[0x8D, address as u8, (address >> 8) as u8]);
}

fn emit_write(program: &mut Vec<u8>, address: u16, value: u8) {
    emit_lda_immediate(program, value);
    emit_sta_absolute(program, address);
}

fn build_apu_rom(writes: &[(u16, u8)]) -> Vec<u8> {
    const PROGRAM_START: u16 = 0x8000;

    let mut program = Vec::new();
    program.push(0x78); // SEI
    emit_write(&mut program, 0x4015, 0x00);
    for address in 0x4000..=0x400F {
        emit_write(&mut program, address, 0x00);
    }
    for &(address, value) in writes {
        emit_write(&mut program, address, value);
    }

    let loop_address = PROGRAM_START + program.len() as u16;
    program.extend_from_slice(&[0x4C, loop_address as u8, (loop_address >> 8) as u8]);

    let mut rom = vec![
        0x4E, 0x45, 0x53, 0x1A, 0x02, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00,
    ];
    rom.resize(16 + 0x8000 + 0x2000, 0);

    let prg = &mut rom[16..16 + 0x8000];
    prg[..program.len()].copy_from_slice(&program);
    for vector in [0x7FFA, 0x7FFC, 0x7FFE] {
        prg[vector] = PROGRAM_START as u8;
        prg[vector + 1] = (PROGRAM_START >> 8) as u8;
    }

    rom
}

fn pulse_envelope_rom(frame_counter_value: u8) -> Vec<u8> {
    build_apu_rom(&[
        (0x4000, 0x80 | ENVELOPE_PERIOD),
        (0x4001, 0x00),
        (0x4002, PULSE_TIMER_LOW),
        (0x4015, 0x01),
        (0x4003, MAX_LENGTH_INDEX << 3),
        (0x4017, frame_counter_value),
    ])
}

fn pulse_constant_volume_rom(length_index: u8, frame_counter_value: u8) -> Vec<u8> {
    build_apu_rom(&[
        (0x4000, 0x80 | 0x10 | 0x0F),
        (0x4001, 0x00),
        (0x4002, PULSE_TIMER_LOW),
        (0x4015, 0x01),
        (0x4003, length_index << 3),
        (0x4017, frame_counter_value),
    ])
}

fn cartridge_data_from_rom(rom: &[u8]) -> CartridgeData {
    const HEADER_LEN: usize = 16;
    const PRG_ROM_LEN: usize = 0x8000;
    const CHR_ROM_LEN: usize = 0x2000;

    assert_eq!(
        rom.len(),
        HEADER_LEN + PRG_ROM_LEN + CHR_ROM_LEN,
        "test ROM should have the expected NROM layout",
    );

    CartridgeData::new(CartridgeDataParts {
        format: RomFormat::INes,
        prog_rom: rom[HEADER_LEN..HEADER_LEN + PRG_ROM_LEN].to_vec(),
        char_rom: rom[HEADER_LEN + PRG_ROM_LEN..].to_vec(),
        pram_length: 0,
        save_pram_length: 0,
        vram_length: 0,
        save_vram_length: 0,
        mapper_type: 0,
        mirror_mode: MirrorMode::Horizontal,
        has_battery: false,
        sub_mapper_type: 0,
        trainer: Vec::new(),
    })
    .expect("test cartridge data should be valid")
}

fn run_rom_until_silence(
    rom: Vec<u8>,
    sample_rate: u32,
    max_frames: usize,
    silence_tail_seconds: f32,
) -> AudioCapture {
    let cartridge_data = cartridge_data_from_rom(&rom);
    let mut core = Core::new(cartridge_data).expect("test ROM should load");
    core.reset();

    let mut screen = NullScreen;
    let mut controller = StandardController::new();
    let mut mixer = CapturingMixer::new(sample_rate);

    for _ in 0..max_frames {
        core.run_frame(&mut screen, &mut controller, &mut mixer);
        if mixer.has_activity() && mixer.tail_is_silent(silence_tail_seconds) {
            break;
        }
    }

    decode_wav(&encode_wav(&mixer.samples, sample_rate))
}

fn four_step_half_tick_cycles(index: usize) -> u64 {
    let sequence = (index / 2) as u64;
    match index % 2 {
        0 => 14_913 + sequence * 29_830,
        1 => 29_829 + sequence * 29_830,
        _ => unreachable!(),
    }
}

fn five_step_half_tick_cycles(index: usize) -> u64 {
    if index == 0 {
        return 0;
    }

    let adjusted = index - 1;
    let sequence = (adjusted / 2) as u64;
    match adjusted % 2 {
        0 => 14_913 + sequence * 37_282,
        1 => 37_281 + sequence * 37_282,
        _ => unreachable!(),
    }
}

fn five_step_quarter_tick_cycles(index: usize) -> u64 {
    if index == 0 {
        return 0;
    }

    let adjusted = index - 1;
    let sequence = (adjusted / 4) as u64;
    match adjusted % 4 {
        0 => 7_457 + sequence * 37_282,
        1 => 14_913 + sequence * 37_282,
        2 => 22_371 + sequence * 37_282,
        3 => 37_281 + sequence * 37_282,
        _ => unreachable!(),
    }
}

#[test]
fn rom_driven_envelope_decay_matches_expected_step_period_and_curve() {
    let capture = run_rom_until_silence(
        pulse_envelope_rom(0x80),
        TEST_SAMPLE_RATE,
        30,
        DEFAULT_SILENCE_TAIL_SECONDS,
    );
    let onset = capture.first_active_time(ANALYSIS_WINDOW_SECONDS, 0.45);
    let drop_times = (1..=5)
        .map(|drop| {
            five_step_quarter_tick_cycles((usize::from(ENVELOPE_PERIOD) + 1) * drop) as f32
                / CPU_CLOCK_HZ
        })
        .collect::<Vec<_>>();
    let plateau_levels = (0..5)
        .map(|step| {
            let start = if step == 0 { 0.0 } else { drop_times[step - 1] };
            let end = drop_times[step];
            let duration = end - start;
            capture.segment_rms(onset + start + duration * 0.2, onset + end - duration * 0.2)
        })
        .collect::<Vec<_>>();

    assert!(
        onset < 0.003,
        "5-step immediate clock should start the envelope almost immediately, got {onset:.4}s"
    );

    let base_level = plateau_levels[0];
    for (step, level) in plateau_levels.iter().copied().enumerate() {
        let expected_ratio = (15 - step) as f32 / 15.0;
        let actual_ratio = level / base_level;
        assert!(
            (actual_ratio - expected_ratio).abs() <= 0.08,
            "plateau {step} ratio should follow linear decay (expected {expected_ratio:.3}, got {actual_ratio:.3})"
        );
        if step > 0 {
            assert!(
                plateau_levels[step - 1] > level,
                "plateau {step} should be quieter than plateau {}",
                step - 1
            );
        }
    }

    let first_drop_pre =
        capture.segment_rms(onset + drop_times[0] - 0.003, onset + drop_times[0] - 0.001);
    let first_drop_post =
        capture.segment_rms(onset + drop_times[0] + 0.001, onset + drop_times[0] + 0.003);
    let second_drop_pre =
        capture.segment_rms(onset + drop_times[1] - 0.003, onset + drop_times[1] - 0.001);
    let second_drop_post =
        capture.segment_rms(onset + drop_times[1] + 0.001, onset + drop_times[1] + 0.003);

    assert!(
        (first_drop_pre / base_level - 1.0).abs() <= 0.08,
        "audio should still be at the initial envelope level just before the first decay boundary"
    );
    assert!(
        (first_drop_post / base_level - (14.0 / 15.0)).abs() <= 0.08,
        "audio should match the first decayed envelope level immediately after the first boundary"
    );
    assert!(
        (second_drop_pre / base_level - (14.0 / 15.0)).abs() <= 0.08,
        "audio should remain on the first decayed level until the second boundary"
    );
    assert!(
        (second_drop_post / base_level - (13.0 / 15.0)).abs() <= 0.08,
        "audio should match the second decayed envelope level immediately after the second boundary"
    );
}

#[test]
fn rom_driven_length_counter_matches_each_table_duration() {
    for (index, counter_value) in LENGTH_TABLE.iter().copied().enumerate() {
        let capture = run_rom_until_silence(
            pulse_constant_volume_rom(index as u8, 0x00),
            LENGTH_TEST_SAMPLE_RATE,
            140,
            0.03,
        );
        let duration = capture.active_duration(ANALYSIS_WINDOW_SECONDS, 0.5);
        let expected = f32::from(counter_value) * HALF_FRAME_SECONDS;

        assert!(
            (duration - expected).abs() <= 0.005,
            "length index {index:#04X} should stay active for {expected:.4}s, got {duration:.4}s"
        );
    }
}

#[test]
fn rom_driven_frame_counter_modes_change_envelope_onset_and_length_duration() {
    let envelope_4_step = run_rom_until_silence(
        pulse_envelope_rom(0x00),
        TEST_SAMPLE_RATE,
        30,
        DEFAULT_SILENCE_TAIL_SECONDS,
    );
    let envelope_5_step = run_rom_until_silence(
        pulse_envelope_rom(0x80),
        TEST_SAMPLE_RATE,
        30,
        DEFAULT_SILENCE_TAIL_SECONDS,
    );
    let onset_4_step = envelope_4_step.first_active_time(ANALYSIS_WINDOW_SECONDS, 0.45);
    let onset_5_step = envelope_5_step.first_active_time(ANALYSIS_WINDOW_SECONDS, 0.45);

    assert!(
        (onset_4_step - QUARTER_FRAME_SECONDS).abs() <= 0.003,
        "4-step mode should wait about one quarter frame before the envelope starts, got {onset_4_step:.4}s"
    );
    assert!(
        onset_5_step < 0.003,
        "5-step mode should clock the envelope immediately, got {onset_5_step:.4}s"
    );
    assert!(
        onset_4_step - onset_5_step >= QUARTER_FRAME_SECONDS * 0.6,
        "5-step mode should start audibly earlier than 4-step mode"
    );

    let duration_4_step = run_rom_until_silence(
        pulse_constant_volume_rom(FRAME_COUNTER_LENGTH_INDEX, 0x00),
        LENGTH_TEST_SAMPLE_RATE,
        30,
        0.03,
    )
    .active_duration(ANALYSIS_WINDOW_SECONDS, 0.5);
    let duration_5_step = run_rom_until_silence(
        pulse_constant_volume_rom(FRAME_COUNTER_LENGTH_INDEX, 0x80),
        LENGTH_TEST_SAMPLE_RATE,
        30,
        0.03,
    )
    .active_duration(ANALYSIS_WINDOW_SECONDS, 0.5);
    let length_value = usize::from(LENGTH_TABLE[usize::from(FRAME_COUNTER_LENGTH_INDEX)]);
    let expected_4_step = four_step_half_tick_cycles(length_value - 1) as f32 / CPU_CLOCK_HZ;
    let expected_5_step = five_step_half_tick_cycles(length_value - 1) as f32 / CPU_CLOCK_HZ;

    assert!(
        (duration_4_step - expected_4_step).abs() <= 0.005,
        "4-step duration should match the unadjusted length counter duration"
    );
    assert!(
        (duration_5_step - expected_5_step).abs() <= 0.005,
        "5-step duration should reflect the immediate half-frame clock"
    );
    assert!(
        (duration_5_step - duration_4_step - HALF_FRAME_SECONDS).abs() <= 0.005,
        "5-step mode should last about one half-frame longer than 4-step mode for this short length"
    );
}
