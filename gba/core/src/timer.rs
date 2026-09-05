#[derive(Clone, Copy, Debug, Default)]
struct TimerChannel {
    reload: u16,
    counter: u16,
    control: u16,
    divider: u16,
    start_delay: u8,
    pending_control: Option<u16>,
    previous_reload: u16,
    reload_written: bool,
}

#[derive(Debug, Default)]
pub struct GbaTimers {
    channels: [TimerChannel; 4],
}

impl GbaTimers {
    pub fn write32(&mut self, address: u32, value: u32) -> bool {
        if !(0x04000100..=0x0400010C).contains(&address) || address & 3 != 0 {
            return false;
        }
        let channel = ((address - 0x04000100) / 4) as usize;
        let timer = &mut self.channels[channel];
        let reload = value as u16;
        timer.previous_reload = timer.reload;
        timer.reload = reload;
        timer.reload_written = timer.control & 0x80 != 0;
        let new_control = (value >> 16) as u16 & 0x00C7;
        let was_enabled = timer.control & 0x80 != 0;
        if was_enabled && new_control & 0x80 == 0 {
            timer.control = new_control;
            timer.pending_control = None;
            timer.start_delay = 0;
        } else {
            write_control(timer, new_control);
        }
        true
    }

    pub fn read(&self, address: u32) -> Option<u16> {
        let (channel, control) = decode(address)?;
        Some(if control {
            self.channels[channel].control
        } else {
            self.channels[channel].counter
        })
    }

    pub fn write(&mut self, address: u32, value: u16) -> bool {
        let Some((channel, control)) = decode(address) else {
            return false;
        };
        let timer = &mut self.channels[channel];
        if control {
            write_control(timer, value & 0x00C7);
        } else {
            timer.previous_reload = timer.reload;
            timer.reload = value;
            timer.reload_written = timer.control & 0x80 != 0;
        }
        true
    }

    /// Advance all four timers by one CPU T-cycle and return Timer IRQ bits 3..6.
    pub fn step(&mut self) -> u16 {
        let mut irq = 0;
        let mut cascade = false;
        for index in 0..4 {
            let (next_cascade, channel_irq) = self.step_channel(index, cascade);
            cascade = next_cascade;
            irq |= channel_irq;
        }
        irq
    }

    fn step_channel(&mut self, index: usize, incoming_cascade: bool) -> (bool, u16) {
        let timer = &mut self.channels[index];
        if timer.control & 0x80 == 0 {
            return (false, 0);
        }
        if let Some((c, irq)) = Self::handle_start_delay(timer, index) {
            return (c, irq);
        }
        let tick = Self::should_tick(timer, index, incoming_cascade);
        let cascade = tick && increment(timer);
        let irq = if cascade && timer.control & (1 << 6) != 0 {
            1 << (3 + index)
        } else {
            0
        };
        if let Some(control) = timer.pending_control.take() {
            timer.control = control;
            timer.start_delay = 0;
        }
        timer.reload_written = false;
        (cascade, irq)
    }

    fn handle_start_delay(timer: &mut TimerChannel, index: usize) -> Option<(bool, u16)> {
        match timer.start_delay {
            1 => {
                timer.counter = timer.reload;
                timer.start_delay = 0;
                timer.reload_written = false;
                Some((false, 0))
            }
            2 => {
                timer.start_delay = 1;
                let cascade = increment(timer);
                let irq = if cascade && timer.control & (1 << 6) != 0 {
                    1 << (3 + index)
                } else {
                    0
                };
                timer.reload_written = false;
                Some((cascade, irq))
            }
            _ => None,
        }
    }

    fn should_tick(timer: &mut TimerChannel, index: usize, cascade: bool) -> bool {
        if index != 0 && timer.control & 4 != 0 {
            return cascade;
        }
        timer.divider = timer.divider.wrapping_add(1);
        let period = [1, 64, 256, 1024][usize::from(timer.control & 3)];
        if timer.divider == period {
            timer.divider = 0;
            true
        } else {
            false
        }
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

fn write_control(timer: &mut TimerChannel, new_control: u16) {
    let was_enabled = timer.control & 0x80 != 0;
    let enabled = new_control & 0x80 != 0;
    if enabled && !was_enabled {
        timer.control = new_control;
        timer.start_delay = 2;
        timer.divider = 0;
    } else if !enabled && was_enabled {
        timer.pending_control = Some(new_control);
    } else {
        timer.control = new_control;
        if !enabled {
            timer.start_delay = 0;
        }
    }
}

fn increment(timer: &mut TimerChannel) -> bool {
    let (value, overflow) = timer.counter.overflowing_add(1);
    let reload = if timer.reload_written {
        timer.previous_reload
    } else {
        timer.reload
    };
    timer.counter = if overflow { reload } else { value };
    overflow
}

fn decode(address: u32) -> Option<(usize, bool)> {
    if !(0x04000100..=0x0400010E).contains(&address) || address & 1 != 0 {
        return None;
    }
    let offset = (address - 0x04000100) as usize;
    Some((offset / 4, offset & 2 != 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reload_prescale_cascade_and_irq() {
        let mut timers = GbaTimers::default();
        timers.write(0x04000100, 0xFFFE);
        timers.write(0x04000104, 0);
        timers.write(0x04000106, 0x0084);
        timers.write(0x04000102, 0x00C0);
        assert_eq!(timers.step(), 0);
        assert_eq!(timers.read(0x04000100), Some(1));
        assert_eq!(timers.step(), 0);
        assert_eq!(timers.read(0x04000100), Some(0xFFFE));
        assert_eq!(timers.step(), 0);
        assert_eq!(timers.step(), 1 << 3);
        assert_eq!(timers.read(0x04000104), Some(1));
        assert_eq!(timers.read(0x04000100), Some(0xFFFE));

        timers.write(0x04000102, 0);
        timers.step();
        timers.write(0x04000100, 0);
        timers.write(0x04000102, 0x0081);
        timers.step();
        timers.step();
        for _ in 0..63 {
            timers.step();
        }
        assert_eq!(timers.read(0x04000100), Some(0));
        timers.step();
        assert_eq!(timers.read(0x04000100), Some(1));
    }
}
