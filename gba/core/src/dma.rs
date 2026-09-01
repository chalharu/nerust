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
}

#[derive(Debug, Default)]
pub struct GbaDma {
    channels: [DmaChannel; 4],
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
            if dma.control & 0x8000 != 0 && timing(dma.control) == trigger && !dma.active {
                dma.active = true;
                dma.delay = 3;
            }
        }
    }

    /// Produce at most one bus transfer. Lower-numbered active channels have priority.
    pub fn step(&mut self) -> Option<DmaTransfer> {
        let channel = self.channels.iter().position(|dma| dma.active)?;
        let dma = &mut self.channels[channel];
        if dma.delay != 0 {
            dma.delay -= 1;
            return None;
        }
        let width = if dma.control & (1 << 10) != 0 { 4 } else { 2 };
        let source = dma.current_source & !(u32::from(width) - 1);
        let destination = dma.current_destination & !(u32::from(width) - 1);
        dma.current_source = advance(dma.current_source, source_mode(dma.control), width, false);
        dma.current_destination = advance(
            dma.current_destination,
            destination_mode(dma.control),
            width,
            true,
        );
        dma.remaining -= 1;
        let finished = dma.remaining == 0;
        let interrupt = finished && dma.control & (1 << 14) != 0;
        if finished {
            finish(dma, channel);
        }
        Some(DmaTransfer {
            channel,
            source,
            destination,
            width,
            interrupt,
            latched_value: dma.latch,
        })
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
        if timing(dma.control) == DmaTrigger::Immediate {
            dma.active = true;
            dma.delay = 3;
        }
    } else if dma.control & 0x8000 == 0 {
        dma.active = false;
    }
}

fn finish(dma: &mut DmaChannel, channel: usize) {
    let repeat = dma.control & (1 << 9) != 0 && timing(dma.control) != DmaTrigger::Immediate;
    dma.active = false;
    if repeat {
        dma.remaining = effective_count(channel, dma.count);
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
        assert!(dma.step().is_none());
        assert!(dma.step().is_none());
        assert!(dma.step().is_none());
        let first = dma.step().unwrap();
        assert_eq!(
            (first.source, first.destination, first.width),
            (0x02001000, 0x03002000, 4)
        );
        let second = dma.step().unwrap();
        assert!(second.interrupt);
        assert_eq!(dma.read(0x040000DE).unwrap() & 0x8000, 0);
    }
}
