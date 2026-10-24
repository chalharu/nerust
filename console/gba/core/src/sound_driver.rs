/// HLE for the BIOS sound driver voices (GBATEK "BIOS Sound Functions"):
/// `SoundArea.vchn[]` direct-sound channels with ADSR envelopes ticked by
/// SWI 1Ch, mixed per native-grid sample from `WaveData` byte streams.
///
/// Layout (GBATEK): SoundArea+20 holds 48-byte `SndCh` entries
/// (sf,rv,lv,at,de,su,re,fr,wp,...); WaveData is u16 type/stat, u32
/// freq/loop/size, then signed 8-bit samples (`4000h` = forward loop).
use crate::apu::GbaApu;

/// Bus operations needed by the sound-driver HLE. Implemented for
/// `GbaMemoryBus` in `memory.rs`; the driver only names this trait,
/// so the dependency runs `memory -> sound_driver` and never back.
/// Lives at the crate top (not under `bios`) so `bios -> memory ->
/// sound_driver` stays acyclic.
pub trait SoundDriverBus {
    fn read8(&mut self, addr: u32) -> u8;
    fn read16(&mut self, addr: u32) -> u16;
    fn read32(&mut self, addr: u32) -> u32;
    fn write_hle_bios8(&mut self, addr: u32, value: u8);
    fn apu(&self) -> &GbaApu;
    fn apu_mut(&mut self) -> &mut GbaApu;
}

const SNDCH_BASE: u32 = 20;
const SNDCH_STRIDE: u32 = 48;
const MAX_VOICES: usize = 12;

/// SoundDriverMode playback frequencies (GBATEK bits 16-19, index 1-12).
const PLAYBACK_FREQ: [u32; 12] = [
    5734, 7884, 10512, 13379, 15768, 18157, 21024, 26758, 31536, 36314, 40137, 42048,
];

/// Runtime voice state (the register side lives in SoundArea RAM).
/// Re-exported from `apu`, where the owning `GbaApu::driver_voices` lives.
pub use crate::apu::DriverVoice;

/// Parse SoundDriverMode (GBATEK 1Bh) into (channels, master, play_freq).
/// Zero fields fall back to the documented defaults (8ch, 15, 13379Hz).
pub fn parse_mode(mode: u32) -> (usize, u32, u32) {
    let channels = match (mode >> 8) & 0xF {
        0 => 8,
        n => (n as usize).clamp(1, MAX_VOICES),
    };
    let master = match (mode >> 12) & 0xF {
        0 => 15,
        n => n,
    };
    let freq = match (mode >> 16) & 0xF {
        1..=12 => PLAYBACK_FREQ[((mode >> 16) & 0xF) as usize - 1],
        _ => 13379,
    };
    (channels, master, freq)
}

/// SWI 1Ch body: advance envelopes and latch start/key-off requests.
/// Called by the game every 1/60s; envelopes move one step per call.
pub fn sound_driver_main(bus: &mut impl SoundDriverBus) {
    let area = bus.apu().sound_area;
    if area == 0 {
        return;
    }
    let (channels, _, _) = parse_mode(bus.apu().sound_mode);
    for i in 0..channels {
        tick_voice(bus, area, i);
    }
}

fn tick_voice(bus: &mut impl SoundDriverBus, area: u32, index: usize) {
    let base = area.wrapping_add(SNDCH_BASE + index as u32 * SNDCH_STRIDE);
    let sf = bus.read8(base);
    if sf == 0 {
        bus.apu_mut().driver_voices[index].started = false;
        return;
    }
    if sf & 0x80 != 0 {
        // Start request: latch, reset position/envelope, consume the bit
        // (games set it once per note; the driver owns it afterwards).
        if !bus.apu().driver_voices[index].started {
            let voice = &mut bus.apu_mut().driver_voices[index];
            voice.started = true;
            voice.pos = 0.0;
            voice.env = 0.0;
        }
        bus.write_hle_bios8(base, sf & !0x80);
    }
    if !bus.apu().driver_voices[index].started {
        return;
    }
    let at = bus.read8(base.wrapping_add(4)) as f32;
    let de = bus.read8(base.wrapping_add(5)) as f32;
    let su = bus.read8(base.wrapping_add(6)) as f32;
    let re = bus.read8(base.wrapping_add(7)) as f32;
    let voice = &mut bus.apu_mut().driver_voices[index];
    if sf & 0x40 != 0 {
        // Release: scale down until silence, then stop the channel.
        voice.env = voice.env * re / 256.0;
        if voice.env < 0.5 {
            voice.env = 0.0;
            voice.started = false;
            bus.write_hle_bios8(base, 0);
        }
    } else if voice.env < 255.0 && voice.env < su + 1.0 {
        // Attack up to 255, then decay toward sustain.
        voice.env = (voice.env + at).min(255.0);
    } else if voice.env > su {
        voice.env = (voice.env * de / 256.0).max(su);
    } else {
        voice.env = su;
    }
}

/// Mix one native-grid sample of driver voices into the APU buffer tail.
/// Voice positions advance at fr per grid second (`fr` is an effective
/// sample rate in Hz; the grid runs at [`MIX_RATE`](crate::apu::MIX_RATE)).
/// Advancing by `fr/playback_freq` here would replay every voice
/// 32768/playback_freq times too fast (2.45x at the default 13379Hz):
/// that quotient is per *driver output sample*, and this function runs
/// once per *grid tick*, not once per driver tick.
pub fn mix_driver_grid(bus: &mut impl SoundDriverBus) {
    let area = bus.apu().sound_area;
    if area == 0 {
        return;
    }
    let (channels, master, _) = parse_mode(bus.apu().sound_mode);
    let mut sum_l = 0.0f32;
    let mut sum_r = 0.0f32;
    for i in 0..channels {
        let base = area.wrapping_add(SNDCH_BASE + i as u32 * SNDCH_STRIDE);
        let voice = bus.apu().driver_voices[i];
        if !voice.started || voice.env <= 0.0 {
            continue;
        }
        let fr = bus.read32(base.wrapping_add(12));
        let wp = bus.read32(base.wrapping_add(16));
        if wp == 0 || fr == 0 {
            continue;
        }
        let stat = bus.read16(wp.wrapping_add(2));
        let size = bus.read32(wp.wrapping_add(12));
        let rate = bus.read32(wp.wrapping_add(4));
        if size == 0 || rate == 0 {
            continue;
        }
        // fr = rate / 2^((180-key-fine/256)/12): effective sample rate.
        let mut pos = voice.pos + f64::from(fr) / f64::from(crate::apu::MIX_RATE);
        let mut idx = pos as u32;
        if idx >= size {
            if stat & 0x4000 != 0 {
                let loop_start = bus.read32(wp.wrapping_add(8));
                let span = size.saturating_sub(loop_start).max(1);
                pos = f64::from(loop_start) + (pos - f64::from(size)) % f64::from(span);
                idx = pos as u32;
            } else {
                bus.apu_mut().driver_voices[i].started = false;
                bus.write_hle_bios8(base, 0);
                continue;
            }
        }
        bus.apu_mut().driver_voices[i].pos = pos;
        let sample = bus.read8(wp.wrapping_add(16 + idx)) as i8 as f32 / 128.0;
        let rv = bus.read8(base.wrapping_add(2)) as f32 / 255.0;
        let lv = bus.read8(base.wrapping_add(3)) as f32 / 255.0;
        let amp = voice.env / 255.0 * (master as f32 / 15.0);
        sum_r += sample * amp * rv;
        sum_l += sample * amp * lv;
    }
    if let Some(tail) = bus.apu_mut().mix_tail_mut() {
        tail.0 = (tail.0 + sum_l * 0.5).clamp(-1.0, 1.0);
        tail.1 = (tail.1 + sum_r * 0.5).clamp(-1.0, 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Minimal `SoundDriverBus` over a byte map plus a real `GbaApu`.
    struct MockBus {
        apu: GbaApu,
        mem: HashMap<u32, u8>,
    }

    impl MockBus {
        fn new() -> Self {
            Self {
                apu: GbaApu::new(),
                mem: HashMap::new(),
            }
        }

        fn write32(&mut self, addr: u32, value: u32) {
            for (i, byte) in value.to_le_bytes().into_iter().enumerate() {
                self.mem.insert(addr + i as u32, byte);
            }
        }
    }

    impl SoundDriverBus for MockBus {
        fn read8(&mut self, addr: u32) -> u8 {
            self.mem.get(&addr).copied().unwrap_or(0)
        }

        fn read16(&mut self, addr: u32) -> u16 {
            u16::from_le_bytes([self.read8(addr), self.read8(addr + 1)])
        }

        fn read32(&mut self, addr: u32) -> u32 {
            u32::from_le_bytes([
                self.read8(addr),
                self.read8(addr + 1),
                self.read8(addr + 2),
                self.read8(addr + 3),
            ])
        }

        fn write_hle_bios8(&mut self, addr: u32, value: u8) {
            self.mem.insert(addr, value);
        }

        fn apu(&self) -> &GbaApu {
            &self.apu
        }

        fn apu_mut(&mut self) -> &mut GbaApu {
            &mut self.apu
        }
    }

    /// Set up one started voice: SndCh0 plays `fr`-Hz effective rate
    /// from a 64-sample table at full volume, instant attack.
    fn started_voice(bus: &mut MockBus, fr: u32) {
        const AREA: u32 = 0x1000;
        const WP: u32 = 0x2000;
        bus.apu.sound_area = AREA;
        let base = AREA + SNDCH_BASE;
        bus.mem.insert(base, 0x80); // start request
        bus.mem.insert(base + 2, 255); // rv
        bus.mem.insert(base + 3, 255); // lv
        bus.mem.insert(base + 4, 255); // at (instant attack)
        bus.mem.insert(base + 6, 255); // su
        bus.write32(base + 12, fr);
        bus.write32(base + 16, WP);
        bus.write32(WP + 4, fr); // WaveData rate
        bus.write32(WP + 12, 64); // size
        for i in 0..64 {
            bus.mem.insert(WP + 16 + i, 0x40);
        }
        sound_driver_main(bus);
        assert!(bus.apu.driver_voices[0].started);
        // Latch one real grid sample so `mix_tail_mut` has a fold-in
        // target (512 T-cycles per native-grid sample).
        for _ in 0..512 {
            bus.apu.tick();
        }
    }

    /// Start SndCh0 with the given ADSR bytes and hold the note: the
    /// start bit is consumed by the first `sound_driver_main` call, and
    /// any nonzero `sf` afterwards sustains until release/stop.
    fn adsr_voice(bus: &mut MockBus, at: u8, de: u8, su: u8, re: u8) {
        const AREA: u32 = 0x1000;
        bus.apu.sound_area = AREA;
        let base = AREA + SNDCH_BASE;
        bus.mem.insert(base, 0x80);
        bus.mem.insert(base + 4, at);
        bus.mem.insert(base + 5, de);
        bus.mem.insert(base + 6, su);
        bus.mem.insert(base + 7, re);
        sound_driver_main(bus);
        assert!(bus.apu.driver_voices[0].started);
        bus.mem.insert(base, 0x01);
    }

    #[test]
    fn mode_defaults_match_gbatek() {
        assert_eq!(parse_mode(0), (8, 15, 13379));
    }

    #[test]
    fn mode_fields_decode() {
        // channels=4, master=10, freq index 3 -> 10512Hz.
        let mode = (4 << 8) | (10 << 12) | (3 << 16);
        assert_eq!(parse_mode(mode), (4, 10, 10512));
    }

    #[test]
    fn driver_voice_advances_at_fr_per_grid_second() {
        // `fr` is an effective sample rate in Hz and `mix_driver_grid`
        // runs once per 32768Hz grid tick, so one call must advance the
        // position by fr/32768 — here 16384Hz -> exactly 0.5/call.
        // (The old fr/playback_freq quotient replayed voices 2.45x fast
        // at the default 13379Hz driver rate.)
        let mut bus = MockBus::new();
        started_voice(&mut bus, 16_384);
        for _ in 0..4 {
            mix_driver_grid(&mut bus);
        }
        assert_eq!(bus.apu.driver_voices[0].pos, 2.0);
    }

    #[test]
    fn driver_voice_pitch_ignores_driver_mixer_rate() {
        // Same voice under the fastest driver rate (index 12, 42048Hz)
        // must advance identically: the grid rate, not the mixer rate,
        // sets the per-call step.
        let mut bus = MockBus::new();
        bus.apu.sound_mode = 12 << 16;
        started_voice(&mut bus, 16_384);
        for _ in 0..4 {
            mix_driver_grid(&mut bus);
        }
        assert_eq!(bus.apu.driver_voices[0].pos, 2.0);
    }

    #[test]
    fn driver_voice_sine_table_plays_at_fr_hz() {
        // End-to-end pitch guard for the fr/MIX_RATE fix: a looped
        // 64-sample sine table at fr = 32768 advances one table step per
        // grid tick, i.e. a 512Hz tone. (The old fr/playback_freq step
        // replayed it at ~1254Hz instead.)
        use crate::apu::fft_test::{
            FFT_SAMPLE_COUNT, GRID_RATE_HZ, average_band_power, dominant_frequency,
            dominant_frequency_tolerance, power_spectrum,
        };
        use std::f64::consts::PI;

        const AREA: u32 = 0x1000;
        const WP: u32 = 0x2000;
        let mut bus = MockBus::new();
        bus.apu.sound_area = AREA;
        let base = AREA + SNDCH_BASE;
        bus.mem.insert(base, 0x80); // start request
        bus.mem.insert(base + 2, 255); // rv
        bus.mem.insert(base + 3, 255); // lv
        bus.mem.insert(base + 4, 255); // at (instant attack)
        bus.mem.insert(base + 6, 255); // su (sustain holds env at 255)
        bus.write32(base + 12, crate::apu::MIX_RATE);
        bus.write32(base + 16, WP);
        bus.mem.insert(WP + 2, 0x00);
        bus.mem.insert(WP + 3, 0x40); // stat: loop
        bus.write32(WP + 4, crate::apu::MIX_RATE);
        bus.write32(WP + 8, 0); // loop start
        bus.write32(WP + 12, 64); // size
        for i in 0..64u32 {
            let sine = (127.0 * (2.0 * PI * f64::from(i) / 64.0).sin()).round() as i8;
            bus.mem.insert(WP + 16 + i, sine as u8);
        }
        sound_driver_main(&mut bus);
        assert!(bus.apu.driver_voices[0].started);

        while bus.apu.grid_buffer().len() < FFT_SAMPLE_COUNT {
            if bus.apu.tick() {
                mix_driver_grid(&mut bus);
            }
        }
        let samples: Vec<f32> = bus
            .apu
            .grid_buffer()
            .iter()
            .take(FFT_SAMPLE_COUNT)
            .map(|sample| sample.0)
            .collect();
        let dominant = dominant_frequency(&samples, GRID_RATE_HZ);
        let tolerance = dominant_frequency_tolerance(GRID_RATE_HZ, FFT_SAMPLE_COUNT);
        assert!(
            (dominant - 512.0).abs() <= tolerance,
            "driver voice should play 512Hz, got {dominant} (tol {tolerance})"
        );
        let spectrum = power_spectrum(&samples);
        let energy = average_band_power(&spectrum, GRID_RATE_HZ, 100.0, 2000.0);
        assert!(energy > 1e-6, "driver voice must sound, energy={energy}");
    }

    #[test]
    fn driver_voice_attack_climbs_linearly_then_holds() {
        let mut bus = MockBus::new();
        adsr_voice(&mut bus, 10, 0, 255, 0);
        // The start call already attacked 0 -> 10.
        let mut envs = Vec::new();
        for _ in 0..24 {
            sound_driver_main(&mut bus);
            envs.push(bus.apu.driver_voices[0].env);
        }
        let expected: Vec<f32> = (2..=25).map(|k| k as f32 * 10.0).collect();
        assert_eq!(envs, expected);
        // 250 + 10 clamps to 255, then sustain holds it there.
        sound_driver_main(&mut bus);
        assert_eq!(bus.apu.driver_voices[0].env, 255.0);
        for _ in 0..3 {
            sound_driver_main(&mut bus);
        }
        assert_eq!(bus.apu.driver_voices[0].env, 255.0);
    }

    #[test]
    fn driver_voice_decay_overshoots_toward_sustain() {
        let mut bus = MockBus::new();
        adsr_voice(&mut bus, 255, 128, 100, 0);
        // The start call attacked straight to 255.
        assert_eq!(bus.apu.driver_voices[0].env, 255.0);
        let mut envs = Vec::new();
        for _ in 0..3 {
            sound_driver_main(&mut bus);
            envs.push(bus.apu.driver_voices[0].env);
        }
        // 255 -> 127.5 -> 100.0 (decay clamps at sustain), then the
        // instant attack re-fires (100.0 < su + 1): the driver's decay
        // pumps between sustain and full scale.
        assert_eq!(envs, vec![127.5, 100.0, 255.0]);
    }

    #[test]
    fn driver_voice_release_fades_to_stop() {
        let mut bus = MockBus::new();
        adsr_voice(&mut bus, 255, 0, 255, 128);
        assert_eq!(bus.apu.driver_voices[0].env, 255.0);
        const AREA: u32 = 0x1000;
        bus.mem.insert(AREA + SNDCH_BASE, 0x40);
        let mut envs = Vec::new();
        for _ in 0..8 {
            sound_driver_main(&mut bus);
            envs.push(bus.apu.driver_voices[0].env);
        }
        assert_eq!(
            envs,
            vec![
                127.5, 63.75, 31.875, 15.9375, 7.96875, 3.984375, 1.9921875, 0.99609375
            ]
        );
        // 0.99609375 x 0.5 falls below 0.5: silence, channel stopped.
        sound_driver_main(&mut bus);
        assert_eq!(bus.apu.driver_voices[0].env, 0.0);
        assert!(!bus.apu.driver_voices[0].started);
        assert_eq!(bus.mem.get(&(AREA + SNDCH_BASE)), Some(&0));
    }
}
