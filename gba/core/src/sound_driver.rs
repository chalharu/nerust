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
/// Voice positions advance continuously at fr/playback_freq per grid tick.
pub fn mix_driver_grid(bus: &mut impl SoundDriverBus) {
    let area = bus.apu().sound_area;
    if area == 0 {
        return;
    }
    let (channels, master, play_freq) = parse_mode(bus.apu().sound_mode);
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
        let mut pos = voice.pos + f64::from(fr) / f64::from(play_freq);
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
}
