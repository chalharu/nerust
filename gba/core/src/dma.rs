#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DmaTrigger {
    #[default]
    Immediate,
    VBlank,
    HBlank,
    Special,
}

#[derive(Clone, Copy, Debug)]
pub struct DmaTransfer {
    pub channel: usize,
    /// Programmed-counter source (drives N/S timing; may descend).
    pub source: u32,
    /// Actual data-read source (forced increment inside GamePak ROM).
    pub data_source: u32,
    pub destination: u32,
    pub width: u8,
    pub latched_value: u32,
    /// True when the burst head issued outside GamePak ROM. The 16-bit
    /// GamePak read-path shift (unit N latches unit N+1's halfword) only
    /// fires for such primed bursts (HW-pinned by nba burst-into-tears,
    /// whose head issues from the OAM mirror): bursts sourced entirely
    /// within ROM stream aligned (HW-pinned by mgba-suite DMA H rows,
    /// which expect the plain forced-increment last word 0xDEAD, not the
    /// shifted 0xBEF1/0xBEEE).
    pub shift_primed: bool,
    /// True when this unit is the only unit of its burst. The 16-bit
    /// GamePak pre-increment read (dest[i] = mem16(src+2+2i)) is a
    /// multi-unit pipeline effect (HW-pinned by nba burst-into-tears,
    /// count 3): single-unit 16-bit reads land on the aligned source
    /// (mgba-suite "ROM load DMA1 16" pins 0xBEEF, not 0xDEAD).
    pub single_unit: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct DmaChannel {
    source: u32,
    destination: u32,
    count: u16,
    control: u16,
    current_source: u32,
    current_destination: u32,
    remaining: u32,
    active: bool,
    delay: u8,
    latch: u32,
    prev_src: u32,
    prev_dst: u32,
    is_first: bool,
    pending: u8,
    stalled: bool,
    /// Latched at enable: whether the burst head issued outside GamePak
    /// ROM (see `DmaTransfer::shift_primed`). The latched source is fixed
    /// for the burst (re-arms keep it), so the flag needs no per-unit
    /// update.
    shift_primed: bool,
    /// Data-stream source, latched at enable alongside `current_source`
    /// and advanced per unit. GamePak-ROM sources always increment (mGBA
    /// dma.c `sourceOffset = width`) while the N/S timing follows the
    /// programmed counter: HW-pinned both ways (mgba-suite DMA data rows
    /// deliver the 4th word up for fixed/inc/dec ROM sources; nba
    /// 128kb-boundary dec rows time the descending counter, e.g. 70 vs
    /// 68 across the 128K line). Outside ROM both streams coincide.
    data_source: u32,
    completing: bool,
    completion_interrupt: bool,
}

#[derive(Debug, Default)]
pub struct GbaDma {
    channels: [DmaChannel; 4],
    completion_interrupts: u16,
}

impl GbaDma {
    /// The bus stays owned through the completion tick: the CPU resumes
    /// once the last unit's delay has fully elapsed and the channel state
    /// has settled (mGBA cpuBlocked until the DMA event completes).
    pub fn is_active(&self) -> bool {
        self.channels.iter().any(|dma| dma.active)
    }

    pub fn read(&self, address: u32) -> Option<u16> {
        let (channel, register) = decode(address)?;
        let dma = self.channels[channel];
        Some(match register {
            0 => dma.source as u16,
            1 => (dma.source >> 16) as u16,
            2 => dma.destination as u16,
            3 => (dma.destination >> 16) as u16,
            4 => dma.count,
            _ => dma.control,
        })
    }

    pub fn write(&mut self, address: u32, value: u16) -> bool {
        let Some((channel, register)) = decode(address) else {
            return false;
        };
        let dma = &mut self.channels[channel];
        match register {
            0 => dma.source = (dma.source & 0xFFFF0000) | u32::from(value),
            1 => dma.source = (dma.source & 0xFFFF) | (u32::from(value) << 16),
            2 | 3 => {
                if register == 2 {
                    dma.destination = (dma.destination & 0xFFFF0000) | u32::from(value);
                } else {
                    dma.destination = (dma.destination & 0xFFFF) | (u32::from(value) << 16);
                }
                // A destination rewrite on an idle enabled channel can turn
                // it into FIFO DMA (enable latched it as a plain transfer);
                // re-latch the 4x32-bit burst. Never touch a running burst.
                if !dma.active
                    && dma.control & 0x8000 != 0
                    && sound_dma(channel, dma.control, dma.destination)
                {
                    dma.remaining = 4;
                }
            }
            4 => dma.count = value,
            _ => write_control(dma, channel, value),
        }
        true
    }

    pub fn trigger(&mut self, trigger: DmaTrigger) {
        for (channel, dma) in self.channels.iter_mut().enumerate() {
            if dma.control & 0x8000 != 0
                && timing_for(channel, dma.control) == trigger
                && !dma.active
                && dma.pending == 0
            {
                // mGBA `when = now + 3`: the DMA owns the bus 3 cycles
                // after the start condition fires.
                dma.pending = 3;
                dma.is_first = true;
            }
        }
    }

    pub fn trigger_channel(&mut self, channel: usize, trigger: DmaTrigger) {
        if channel < 4 {
            let dma = &mut self.channels[channel];
            if dma.control & 0x8000 != 0
                && timing_for(channel, dma.control) == trigger
                && !dma.active
                && dma.pending == 0
            {
                dma.pending = 3;
                dma.is_first = true;
            }
        }
    }

    pub fn tick_pending(&mut self) {
        for dma in &mut self.channels {
            if dma.pending > 0 {
                dma.pending -= 1;
                if dma.pending == 0 && dma.control & 0x8000 != 0 {
                    dma.active = true;
                    dma.is_first = true;
                }
            }
        }
    }

    pub fn has_pending(&self) -> bool {
        self.channels.iter().any(|dma| dma.pending > 0)
    }

    /// Produce at most one bus transfer. Lower-numbered active channels have priority.
    /// `stall` maps an address to the display-controller contention wait
    /// (0 outside video RAM / VBlank); the bus owner pays it like the CPU.
    pub fn step(&mut self, waitcnt: u16, stall: &dyn Fn(u32) -> u8) -> Option<DmaTransfer> {
        let channel = self.channels.iter().position(|dma| dma.active)?;
        let dma = &mut self.channels[channel];
        if dma.delay != 0 {
            dma.delay -= 1;
            if dma.delay != 0 {
                return None;
            }
        }
        if dma.completing {
            let interrupt = dma.completion_interrupt;
            finish(dma, channel);
            if interrupt {
                self.completion_interrupts |= 1 << (8 + channel);
            }
            return None;
        }
        let raw_width = if dma.control & (1 << 10) != 0 { 4 } else { 2 };
        let raw_dest = dma.current_destination & !(u32::from(raw_width) - 1);
        // Sound-FIFO DMA always moves 32-bit units (GBATEK DMA).
        let width = if sound_dma(channel, dma.control, raw_dest) {
            4
        } else {
            raw_width
        };
        let source = dma.current_source & !(u32::from(width) - 1);
        let destination = dma.current_destination & !(u32::from(width) - 1);
        // The 16-bit GamePak read path pre-increments (memory.rs: the bus
        // carries source+2, dest[i] = mem16(src+2+2i), HW-pinned by nba
        // burst-into-tears). The shift fires when the read lands in ROM,
        // i.e. the source or the source+2 is GamePak: unit 0 of
        // burst-into-tears issues from the OAM mirror (0x07FFFFFE) but its
        // bus read lands at ROM[0]. Sequentiality describes gaps between
        // bus addresses, so the N/S state machine must track the phantom
        // stream: an OAM-mirror -> ROM logical transition (burst-into-tears
        // unit 1) is a sequential ROM ROM gap on the bus (S, not N), which
        // is exactly the 2-cycle TIME overshoot (43 vs 41). Region waits
        // below stay on the logical address; only the stream position is
        // phantom. 32-bit units have no shift (128kb-boundary pins), and
        // non-ROM 16-bit sources are unaffected (nba latch pins IWRAM
        // 16-bit data exact: 0x12341234).
        let lands_in_rom = is_rom(source) || is_rom(source.wrapping_add(2));
        let bus_src = if width == 2 && lands_in_rom {
            source.wrapping_add(2)
        } else {
            source
        };
        let is_seq_src = if dma.is_first {
            false
        } else {
            let prev = dma.prev_src;
            let cur = bus_src;
            let same_block = (cur & !0x1FFFF) == (prev & !0x1FFFF);
            let src_mode = source_mode(dma.control);
            let seq = match src_mode {
                1 => cur == prev.wrapping_sub(u32::from(width)),
                0 => cur == prev.wrapping_add(u32::from(width)),
                // GBATEK transfer rate ("Except for the first data unit,
                // all units are transferred by sequential reads and writes",
                // mGBA dma.c caches Seq for every later unit).
                _ => true,
            };
            if (0x08000000..=0x0DFFFFFF).contains(&bus_src) {
                // 128K blocks force N (GBATEK GamePak Prefetch), except the
                // final unit: N/S describes the gap to a successor access,
                // and the last unit has none (nba 128kb-boundary late-cross
                // measures S-cost while early/mid crosses measure N).
                seq && (same_block || dma.remaining == 1)
            } else {
                seq
            }
        };
        // GBATEK transfer rate ("Except for the first data unit, all
        // units are transferred by sequential reads and writes", mGBA
        // dma.c caches Seq for every later unit): every destination mode,
        // including fixed, is sequential after the first unit.
        let is_seq_dst = !dma.is_first;
        let src_wait = dma_bus_wait(source, width, is_seq_src, waitcnt, stall(source));
        let dst_wait = dma_bus_wait(destination, width, is_seq_dst, waitcnt, stall(destination));
        // GBATEK DMA transfer timing: 2N+2(n-1)S+xI, where the per-unit
        // cost is N/S waits only. The xI internal overhead is a SINGLE
        // per-burst term (2I, 4I when both ends are GamePak), charged with
        // the first unit. (Per-unit internal overcharges by 2/unit; the old
        // code hid that with a -1 hack on the first two units plus a zeroed
        // I/O wait. With burst-start xI the HBlank sampling rate is exactly
        // 2.0 cycles/unit with the documented I/O wait restored, and both
        // the HBlank and video sweep phases match their HW-pinned edges.)
        // 128K blocks force N (GBATEK GamePak Prefetch), except the final
        // unit: N/S describes the gap to a successor, and the last unit
        // has none (nba 128kb-boundary late-cross measures S-cost).
        let total_wait = u32::from(src_wait) + u32::from(dst_wait);
        let both_gamepak = is_rom(source) && is_rom(destination);
        let internal: u32 = if dma.is_first {
            if both_gamepak { 4 } else { 2 }
        } else {
            0
        };
        dma.delay = (total_wait + internal) as u8;
        dma.current_source = advance(dma.current_source, source_mode(dma.control), width, false);
        // Data stream: forced increment inside GamePak ROM (mGBA
        // `sourceOffset = width`, re-evaluated per unit like mGBA's
        // region-crossing refresh); programmed mode elsewhere, where it
        // coincides with the counter above.
        let data_source = dma.data_source & !(u32::from(width) - 1);
        let data_mode = if is_rom(data_source) {
            0
        } else {
            source_mode(dma.control)
        };
        dma.data_source = advance(dma.data_source, data_mode, width, false);
        if sound_dma(channel, dma.control, destination) {
            // GBATEK DMA: sound FIFO transfers never increment the
            // destination; the 4x32-bit burst always lands in the FIFO.
        } else {
            dma.current_destination = advance(
                dma.current_destination,
                destination_mode(dma.control),
                width,
                true,
            );
        }
        dma.prev_src = bus_src;
        dma.prev_dst = destination;
        let was_first = dma.is_first;
        dma.is_first = false;
        dma.remaining -= 1;
        let finished = dma.remaining == 0;
        if finished {
            dma.completing = true;
            dma.completion_interrupt = dma.control & (1 << 14) != 0;
            // No single-unit completion tails (completion_extra /
            // completion_tail_single removed): they compensated the
            // pre-Phase-A CPU model, which lacked the load/store
            // fetch-stream-break charge. Under the corrected model nba
            // force-nseq (88/88) and suite Trivial DMA pass without them;
            // multi-unit fits never saw them anyway.
        }
        Some(DmaTransfer {
            channel,
            source,
            data_source,
            destination,
            width,
            shift_primed: dma.shift_primed,
            latched_value: dma.latch,
            // `remaining` already counts down past this unit, and
            // `was_first` marks the burst head: only a lone unit
            // (remaining == 0 after decrement with was_first) skips the
            // pre-increment; every unit of a multi-unit burst shifts,
            // including the last (burst-into-tears TIME pin).
            single_unit: dma.remaining == 0 && was_first,
        })
    }

    pub fn take_completion_interrupts(&mut self) -> u16 {
        std::mem::take(&mut self.completion_interrupts)
    }

    /// Find an enabled Special channel (1 or 2) targeting a sound FIFO,
    /// for timer-overflow-driven sound DMA (GBATEK SOUNDCNT_H). Like the
    /// transfer path, a Repeat-less channel is not FIFO DMA.
    pub fn sound_channel_for_fifo(&self, fifo_b: bool) -> Option<usize> {
        let want = if fifo_b { 0x0400_00A4 } else { 0x0400_00A0 };
        [1, 2].into_iter().find(|&channel| {
            let dma = &self.channels[channel];
            dma.control & 0x8000 != 0
                && timing(dma.control) == DmaTrigger::Special
                && dma.control & (1 << 9) != 0
                && (dma.destination & !3) == want
        })
    }

    /// DMA3 video-capture (special) transfer armed (enabled + special timing).
    pub fn has_video_transfer(&self) -> bool {
        let dma = &self.channels[3];
        dma.control & 0x8000 != 0 && timing(dma.control) == DmaTrigger::Special
    }

    /// Stop a DMA3 video transfer (NBA `StopVideoTransferDMA`).
    pub fn stop_video_transfer(&mut self) {
        let dma = &mut self.channels[3];
        if dma.control & 0x8000 != 0 && timing(dma.control) == DmaTrigger::Special {
            dma.control &= !0x8000;
            dma.active = false;
            dma.pending = 0;
            dma.delay = 0;
            dma.stalled = false;
            dma.completing = false;
            dma.completion_interrupt = false;
        }
    }

    pub fn update_latch(&mut self, channel: usize, width: u8, value: u32) {
        self.channels[channel].latch = if width == 2 {
            let halfword = value & 0xFFFF;
            halfword | (halfword << 16)
        } else {
            value
        };
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

fn write_control(dma: &mut DmaChannel, channel: usize, value: u16) {
    let was_enabled = dma.control & 0x8000 != 0;
    // GBATEK DMA: DRQ (bit 11) exists on DMA3 only (mGBA masks 0xF7E0 below).
    // NOTE: GBATEK's "Repeat must be zero if DRQ is set" is a programming
    // constraint, not latch behavior: the mgba-suite io-read HW capture
    // reads back both bits (DMA3CNT_HI == 0xFFE0), so the latch is kept
    // verbatim here. The guard lives in finish(): a DRQ channel never
    // re-arms.
    dma.control = value & if channel == 3 { 0xFFE0 } else { 0xF7E0 };
    if dma.control & 0x8000 != 0 && !was_enabled {
        dma.current_source = dma.source
            & if channel == 0 {
                0x07FF_FFFF
            } else {
                0x0FFF_FFFF
            };
        dma.current_destination = dma.destination
            & if channel == 3 {
                0x0FFF_FFFF
            } else {
                0x07FF_FFFF
            };
        dma.remaining = if sound_dma(channel, dma.control, dma.destination) {
            // GBATEK DMA: sound transfers ignore CNT_L and always move
            // 4x32-bit per timer overflow.
            4
        } else {
            effective_count(channel, dma.count)
        };
        dma.is_first = true;
        dma.prev_src = 0;
        dma.prev_dst = 0;
        dma.shift_primed = !is_rom(dma.current_source);
        dma.data_source = dma.current_source;
        dma.delay = 0;
        dma.stalled = false;
        dma.completing = false;
        dma.completion_interrupt = false;
        if timing_for(channel, dma.control) == DmaTrigger::Immediate {
            // GBATEK/mGBA startup + the enabling bus cycle: event-triggered
            // DMA starts 3 cycles after its trigger (pending=3), but an
            // Immediate channel pays one more cycle for the CNT_H enabling
            // write itself (nba start-delay reads 20, not 19).
            dma.pending = 4;
            dma.active = false;
        }
    } else if dma.control & 0x8000 == 0 {
        dma.active = false;
        dma.pending = 0;
        dma.delay = 0;
        dma.stalled = false;
        dma.completing = false;
        dma.completion_interrupt = false;
    }
}

fn finish(dma: &mut DmaChannel, channel: usize) {
    let repeat = dma.control & (1 << 9) != 0;
    // GBATEK DMA3: Repeat must be zero when DRQ is set; such a combo is a
    // programming error, so it never re-arms (the latch itself is kept
    // verbatim for HW readback; see write_control).
    let repeat = repeat && !(channel == 3 && dma.control & 0x0800 != 0);
    dma.active = false;
    dma.pending = 0;
    dma.delay = 0;
    dma.stalled = false;
    dma.completing = false;
    dma.completion_interrupt = false;
    if repeat {
        dma.remaining = if sound_dma(channel, dma.control, dma.destination) {
            4
        } else {
            effective_count(channel, dma.count)
        };
        dma.is_first = true;
        if destination_mode(dma.control) == 3 {
            // Reload with the same masking as enable-time latching.
            dma.current_destination = dma.destination
                & if channel == 3 {
                    0x0FFF_FFFF
                } else {
                    0x07FF_FFFF
                };
        }
        if timing_for(channel, dma.control) == DmaTrigger::Immediate {
            // Immediate has no recurring start condition, so Repeat cannot
            // re-arm it (mGBA dma.c forces noRepeat for TIMING_NOW):
            // clear Enable like the non-repeat path instead of looping.
            dma.control &= !0x8000;
        }
    } else {
        dma.control &= !0x8000;
    }
}

fn effective_count(channel: usize, count: u16) -> u32 {
    // GBATEK DMA: channels 0-2 count 14 bits (0 = 0x4000), channel 3 16 bits.
    if channel == 3 {
        if count != 0 {
            u32::from(count)
        } else {
            0x1_0000
        }
    } else {
        let masked = u32::from(count & 0x3FFF);
        if masked != 0 { masked } else { 0x4000 }
    }
}

fn timing(control: u16) -> DmaTrigger {
    match (control >> 12) & 3 {
        1 => DmaTrigger::VBlank,
        2 => DmaTrigger::HBlank,
        3 => DmaTrigger::Special,
        _ => DmaTrigger::Immediate,
    }
}

/// Per-channel start timing. GBATEK DMA Start Timing: Special on DMA0 is
/// Prohibited — it has no start source, so it never fires (mGBA never
/// schedules CUSTOM on ch0). Return the raw timing so no enable, trigger,
/// or repeat path can mistake it for Immediate.
fn timing_for(_channel: usize, control: u16) -> DmaTrigger {
    timing(control)
}

/// Sound-FIFO DMA (GBATEK "DMA-Sound Playback Procedure"): a Special-timed
/// transfer with Repeat set, targeting FIFO_A/B, always moves 4x32-bit
/// with a fixed destination. GBATEK restricts sound DMA to channels 1/2
/// (DMA0 Special is Prohibited, DMA3 Special is Video Capture), so the
/// channel gates the quirk: other channels fall through to normal timing.
fn sound_dma(channel: usize, control: u16, destination: u32) -> bool {
    (channel == 1 || channel == 2)
        && timing(control) == DmaTrigger::Special
        && control & (1 << 9) != 0
        && is_fifo_dest(destination)
}

fn is_fifo_dest(destination: u32) -> bool {
    matches!(destination & !3, 0x0400_00A0 | 0x0400_00A4)
}

fn is_rom(address: u32) -> bool {
    (0x08000000..=0x0DFFFFFF).contains(&address)
}

fn source_mode(control: u16) -> u16 {
    (control >> 7) & 3
}

fn destination_mode(control: u16) -> u16 {
    (control >> 5) & 3
}

fn advance(address: u32, mode: u16, width: u8, destination: bool) -> u32 {
    match mode {
        1 => address.wrapping_sub(u32::from(width)),
        2 => address,
        // GBATEK marks source mode 3 "Prohibited"; de-facto HW/mGBA behavior
        // is increment, which is what the fallthrough implements.
        // Destination mode 3 is Increment+Reload (reload handled at finish).
        3 if destination => address.wrapping_add(u32::from(width)),
        _ => address.wrapping_add(u32::from(width)),
    }
}

fn dma_bus_wait(address: u32, width: u8, is_seq: bool, waitcnt: u16, stall: u8) -> u8 {
    match address {
        0x00000000..=0x00003FFF => 1,
        0x02000000..=0x02FFFFFF => {
            if width == 4 {
                6
            } else {
                3
            }
        }
        0x03000000..=0x03FFFFFF => 1,
        0x04000000..=0x040003FE => 1,
        // GBATEK bus widths: Palette/VRAM 16bit=1, 32bit=2 (+display stall).
        0x05000000..=0x05FFFFFF => (if width == 4 { 2 } else { 1 }) + stall,
        0x06000000..=0x06FFFFFF => (if width == 4 { 2 } else { 1 }) + stall,
        // DMA owns the bus but still contends with the display controller
        // on video memory (nba burst-into-tears: 3 draw-phase OAM accesses
        // stall +1 each; without them TIME reads 38 instead of 41).
        0x07000000..=0x07FFFFFF => 1 + stall,
        0x08000000..=0x0DFFFFFF => {
            const FIRST: [u8; 4] = [4, 3, 2, 8];
            let (first_shift, second_shift, second_slow) = match address {
                0x08000000..=0x09FFFFFF => (2, 4, 2),
                0x0A000000..=0x0BFFFFFF => (5, 7, 4),
                _ => (8, 10, 8),
            };
            let first = FIRST[((waitcnt >> first_shift) & 0b11) as usize];
            let second = if (waitcnt >> second_shift) & 1 == 0 {
                second_slow
            } else {
                1
            };
            // DMA uses the same Game Pak access timing as the CPU
            // (1 base + waits: N16=5/S16=3 at WS0 defaults).
            if width == 4 {
                if is_seq {
                    second * 2 + 2
                } else {
                    first + second + 2
                }
            } else if is_seq {
                second + 1
            } else {
                first + 1
            }
        }
        0x0E000000..=0x0FFFFFFF => {
            // Same as the CPU path: waitstates + 1 base, no width
            // multiplier (8-bit SRAM bus; wide accesses move one byte).
            const SRAM_WAIT: [u8; 4] = [4, 3, 2, 8];
            SRAM_WAIT[(waitcnt & 0b11) as usize] + 1
        }
        _ => 1,
    }
}

fn decode(address: u32) -> Option<(usize, usize)> {
    if !(0x040000B0..=0x040000DE).contains(&address) || address & 1 != 0 {
        return None;
    }
    let offset = (address - 0x040000B0) as usize;
    Some((offset / 12, (offset % 12) / 2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immediate_transfer_latches_and_completes() {
        let mut dma = GbaDma::default();
        dma.write(0x040000D4, 0x1000);
        dma.write(0x040000D6, 0x0200);
        dma.write(0x040000D8, 0x2000);
        dma.write(0x040000DA, 0x0300);
        dma.write(0x040000DC, 2);
        dma.write(0x040000DE, 0xC400);
        let mut first = None;
        for _ in 0..30 {
            dma.tick_pending();
            if let Some(t) = dma.step(0, &|_| 0) {
                first = Some(t);
                break;
            }
        }
        let first = first.expect("first transfer should complete");
        assert_eq!(
            (first.source, first.destination, first.width),
            (0x02001000, 0x03002000, 4)
        );
        let mut second = None;
        for _ in 0..30 {
            if let Some(t) = dma.step(0, &|_| 0) {
                second = Some(t);
                break;
            }
        }
        let second = second.expect("second transfer should complete");
        // Completion IRQs arrive via take_completion_interrupts (one tick
        // after the final write; GBATEK only says "upon end of Word Count").
        for _ in 0..30 {
            if !dma.is_active() {
                break;
            }
            dma.step(0, &|_| 0);
        }
        assert_eq!(dma.take_completion_interrupts(), 1 << (8 + second.channel));
        assert_eq!(dma.read(0x040000DE).unwrap() & 0x8000, 0);
    }

    #[test]
    fn sound_dma_ignores_count_and_fixes_destination() {
        // GBATEK DMA: Special FIFO transfers always move 4x32-bit with a
        // fixed destination, regardless of CNT_L/width/mode bits.
        let mut dma = GbaDma::default();
        dma.write(0x040000BC, 0x1000);
        dma.write(0x040000BE, 0x0200);
        dma.write(0x040000C0, 0x00A0);
        dma.write(0x040000C2, 0x0400);
        dma.write(0x040000C4, 100); // CNT_L ignored for sound
        // 16-bit + dst increment + repeat + IRQ + Special + enable
        dma.write(0x040000C6, 0x8000 | 0x3000 | 0x4000 | 0x0200 | (2 << 5));
        // Special timing waits for its trigger (here: timer overflow).
        dma.trigger_channel(1, DmaTrigger::Special);
        let mut units = Vec::new();
        for _ in 0..60 {
            dma.tick_pending();
            if let Some(t) = dma.step(0, &|_| 0) {
                units.push((t.source, t.destination, t.width));
            }
            if !dma.is_active() && !dma.has_pending() && units.len() >= 4 {
                break;
            }
        }
        assert_eq!(units.len(), 4);
        for (src, dst, width) in &units {
            assert_eq!(*width, 4);
            assert_eq!(*dst, 0x0400_00A0);
            let _ = src;
        }
        // Sources advance by 4 despite the 16-bit control bit.
        assert_eq!(units[1].0 - units[0].0, 4);
    }
}
