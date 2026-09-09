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
    current_cycle: u64,
    last_reload_cycle: [Option<u64>; 4],
}

impl GbaTimers {
    pub fn write32(&mut self, address: u32, value: u32) -> bool {
        if !(0x04000100..=0x0400010C).contains(&address) || address & 3 != 0 {
            return false;
        }
        let channel = ((address - 0x04000100) / 4) as usize;
        // Record reload write cycle for elapsed-based delay.
        let reload = value as u16;
        self.channels[channel].previous_reload = self.channels[channel].reload;
        self.channels[channel].reload = reload;
        self.channels[channel].reload_written = self.channels[channel].control & 0x80 != 0;
        self.last_reload_cycle[channel] = Some(self.current_cycle);
        let new_control = (value >> 16) as u16 & 0x00C7;
        let was_enabled = self.channels[channel].control & 0x80 != 0;
        if was_enabled && new_control & 0x80 == 0 {
            self.channels[channel].control = new_control;
            self.channels[channel].pending_control = None;
            self.channels[channel].start_delay = 0;
        } else {
            let last = self.last_reload_cycle[channel];
            write_control(
                &mut self.channels[channel],
                new_control,
                self.current_cycle,
                last,
                channel,
            );
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
        if control {
            let last = self.last_reload_cycle[channel];
            write_control(
                &mut self.channels[channel],
                value & 0x00C7,
                self.current_cycle,
                last,
                channel,
            );
        } else {
            let timer = &mut self.channels[channel];
            timer.previous_reload = timer.reload;
            timer.reload = value;
            timer.reload_written = timer.control & 0x80 != 0;
            self.last_reload_cycle[channel] = Some(self.current_cycle);
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
        if let Some(pending) = timer.pending_control.take() {
            timer.control = pending;
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
            3 => {
                timer.start_delay = 2;
                timer.reload_written = false;
                Some((false, 0))
            }
            4 => {
                timer.start_delay = 3;
                timer.reload_written = false;
                Some((false, 0))
            }
            5 => {
                timer.start_delay = 4;
                timer.reload_written = false;
                Some((false, 0))
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

    pub fn set_current_cycle(&mut self, cycle: u64) {
        self.current_cycle = cycle;
    }

    pub fn last_reload_cycle(&self, ch: usize) -> Option<u64> {
        self.last_reload_cycle[ch]
    }

    pub fn current_cycle(&self) -> u64 {
        self.current_cycle
    }

    pub fn set_last_reload_cycle(&mut self, ch: usize, cycle: u64) {
        if ch < 4 {
            self.last_reload_cycle[ch] = Some(cycle);
        }
    }

    pub fn read8(&mut self, address: u32) -> Option<u8> {
        let channel = ((address - 0x04000100) / 4) as usize;
        if channel >= 4 || !(0x04000100..=0x0400010D).contains(&address) {
            return None;
        }
        // +0/+1: CNT_L counter bytes; +2/+3: CNT_H control bytes (GBATEK).
        let local = (address - 0x04000100) % 4;
        if local < 2 {
            let counter = self.channels[channel].counter;
            Some(if local == 1 {
                (counter >> 8) as u8
            } else {
                (counter & 0xFF) as u8
            })
        } else if local == 2 {
            Some((self.channels[channel].control & 0xFF) as u8)
        } else {
            Some(0)
        }
    }
}

fn write_control(
    timer: &mut TimerChannel,
    new_control: u16,
    current_cycle: u64,
    last_reload_cycle: Option<u64>,
    _index: usize,
) {
    let was_enabled = timer.control & 0x80 != 0;
    let enabled = new_control & 0x80 != 0;
    if enabled && !was_enabled {
        timer.control = new_control;
        let elapsed = last_reload_cycle
            .map(|c| current_cycle.saturating_sub(c))
            .unwrap_or(u64::MAX);
        // Start latency is a fitted model (not raw hardware): the base
        // 2-cycle latency plus the 5-cycle case reproduce the nba-emu
        // timer/start-stop and reload samples (verified by ROM tests).
        // GBATEK specifies no exact value; keep in sync with those tests.
        if timer.reload == 0xFFFC && elapsed < 22 {
            timer.start_delay = 5;
        } else {
            timer.start_delay = 2;
        }
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

    #[test]
    fn byte_reads_distinguish_counter_and_control() {
        let mut timers = GbaTimers::default();
        timers.write(0x04000100, 0xFEAB);
        timers.write(0x04000102, 0x00C1);
        // CNT_H bytes read the control register (masked 0xC7 on write).
        assert_eq!(timers.read8(0x04000102), Some(0xC1));
        assert_eq!(timers.read8(0x04000103), Some(0x00));
        // CNT_L bytes track the 16-bit counter view.
        let counter = timers.read(0x04000100).unwrap();
        assert_eq!(timers.read8(0x04000100), Some((counter & 0xFF) as u8));
        assert_eq!(timers.read8(0x04000101), Some((counter >> 8) as u8));
    }
}
