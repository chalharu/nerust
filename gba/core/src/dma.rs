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
    pub source: u32,
    pub destination: u32,
    pub width: u8,
    pub interrupt: bool,
    pub latched_value: u32,
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
    completing: bool,
    completion_interrupt: bool,
}

#[derive(Debug, Default)]
pub struct GbaDma {
    channels: [DmaChannel; 4],
    completion_interrupts: u16,
}

impl GbaDma {
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
            2 => dma.destination = (dma.destination & 0xFFFF0000) | u32::from(value),
            3 => dma.destination = (dma.destination & 0xFFFF) | (u32::from(value) << 16),
            4 => dma.count = value,
            _ => write_control(dma, channel, value),
        }
        true
    }

    pub fn trigger(&mut self, trigger: DmaTrigger) {
        for dma in &mut self.channels {
            if dma.control & 0x8000 != 0
                && timing(dma.control) == trigger
                && !dma.active
                && dma.pending == 0
            {
                dma.pending = 4;
                dma.is_first = true;
            }
        }
    }

    pub fn trigger_channel(&mut self, channel: usize, trigger: DmaTrigger) {
        if channel < 4 {
            let dma = &mut self.channels[channel];
            if dma.control & 0x8000 != 0
                && timing(dma.control) == trigger
                && !dma.active
                && dma.pending == 0
            {
                dma.pending = 4;
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
    pub fn step(&mut self, waitcnt: u16) -> Option<DmaTransfer> {
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
        let width = if dma.control & (1 << 10) != 0 { 4 } else { 2 };
        let source = dma.current_source & !(u32::from(width) - 1);
        let destination = dma.current_destination & !(u32::from(width) - 1);
        let is_seq_src = if dma.is_first {
            false
        } else {
            let prev = dma.prev_src;
            let cur = source;
            let same_block = (cur & !0x1FFFF) == (prev & !0x1FFFF);
            let src_mode = source_mode(dma.control);
            let seq = match src_mode {
                1 => cur == prev.wrapping_sub(u32::from(width)),
                0 => cur == prev.wrapping_add(u32::from(width)),
                _ => false,
            };
            if (0x08000000..=0x0DFFFFFF).contains(&source) {
                seq && same_block
            } else {
                seq
            }
        };
        let is_seq_dst = !dma.is_first;
        let src_wait = dma_bus_wait(source, width, is_seq_src, waitcnt);
        let dst_wait = dma_bus_wait(destination, width, is_seq_dst, waitcnt);
        let both_gamepak = (0x08000000..=0x0DFFFFFF).contains(&source)
            && (0x08000000..=0x0DFFFFFF).contains(&destination);
        let internal: u32 = if both_gamepak { 4 } else { 2 };
        let total_wait = u32::from(src_wait) + u32::from(dst_wait) + internal;
        dma.delay = total_wait.saturating_sub(1) as u8;
        dma.current_source = advance(dma.current_source, source_mode(dma.control), width, false);
        dma.current_destination = advance(
            dma.current_destination,
            destination_mode(dma.control),
            width,
            true,
        );
        dma.prev_src = source;
        dma.prev_dst = destination;
        dma.is_first = false;
        dma.remaining -= 1;
        let finished = dma.remaining == 0;
        if finished {
            dma.completing = true;
            dma.completion_interrupt = dma.control & (1 << 14) != 0;
        }
        Some(DmaTransfer {
            channel,
            source,
            destination,
            width,
            interrupt: false,
            latched_value: dma.latch,
        })
    }

    pub fn take_completion_interrupts(&mut self) -> u16 {
        std::mem::take(&mut self.completion_interrupts)
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
    dma.control = value & 0xFFE0;
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
        dma.remaining = effective_count(channel, dma.count);
        dma.is_first = true;
        dma.prev_src = 0;
        dma.prev_dst = 0;
        dma.delay = 0;
        dma.stalled = false;
        dma.completing = false;
        dma.completion_interrupt = false;
        if timing(dma.control) == DmaTrigger::Immediate {
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
    let repeat = dma.control & (1 << 9) != 0 && timing(dma.control) != DmaTrigger::Immediate;
    dma.active = false;
    dma.pending = 0;
    dma.delay = 0;
    dma.stalled = false;
    dma.completing = false;
    dma.completion_interrupt = false;
    if repeat {
        dma.remaining = effective_count(channel, dma.count);
        dma.is_first = true;
        if destination_mode(dma.control) == 3 {
            dma.current_destination = dma.destination;
        }
    } else {
        dma.control &= !0x8000;
    }
}

fn effective_count(channel: usize, count: u16) -> u32 {
    if count != 0 {
        u32::from(count)
    } else if channel == 3 {
        0x1_0000
    } else {
        0x4000
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
        3 if destination => address.wrapping_add(u32::from(width)),
        _ => address.wrapping_add(u32::from(width)),
    }
}

fn dma_bus_wait(address: u32, width: u8, is_seq: bool, waitcnt: u16) -> u8 {
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
        0x05000000..=0x05FFFFFF => 1,
        0x06000000..=0x06FFFFFF => 1,
        0x07000000..=0x07FFFFFF => 1,
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
            if width == 4 {
                if is_seq { second * 2 } else { first + second }
            } else if is_seq {
                second
            } else {
                first
            }
        }
        0x0E000000..=0x0FFFFFFF => {
            const SRAM_WAIT: [u8; 4] = [4, 3, 2, 8];
            let base = SRAM_WAIT[(waitcnt & 0b11) as usize];
            base.saturating_mul(width)
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
            if let Some(t) = dma.step(0) {
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
            if let Some(t) = dma.step(0) {
                second = Some(t);
                break;
            }
        }
        let second = second.expect("second transfer should complete");
        assert!(!second.interrupt);
        for _ in 0..30 {
            if !dma.is_active() {
                break;
            }
            dma.step(0);
        }
        assert_eq!(dma.take_completion_interrupts(), 1 << (8 + second.channel));
        assert_eq!(dma.read(0x040000DE).unwrap() & 0x8000, 0);
    }
}
