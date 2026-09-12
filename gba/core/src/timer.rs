#[derive(Clone, Copy, Debug, Default)]
struct TimerChannel {
    reload: u16,
    counter: u16,
    control: u16,
    start_delay: u8,
    pending_control: Option<u16>,
    /// CNT_L writes land with a one-tick delay (nba timer/reload 7/7: an
    /// overwrite in the last cycle before overflow still loads the OLD
    /// reload; earlier overwrites use the new value). Applied at the end of
    /// the next tick; flushed immediately on enable so start loads it.
    reload_pending: Option<u16>,
}

#[derive(Debug, Default)]
pub struct GbaTimers {
    channels: [TimerChannel; 4],
    /// Free-running system prescaler (HW: the divider chain never resets,
    /// enabling a timer does not touch its phase). Ticks once per T-cycle;
    /// prescaled channels sample its taps. Wrapping at 65536 is
    /// phase-transparent (65536 is a multiple of every period).
    prescaler: u16,
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
        self.channels[channel].reload_pending = Some(reload);
        self.last_reload_cycle[channel] = Some(self.current_cycle);
        let new_control = (value >> 16) as u16 & 0x00C7;
        let was_enabled = self.channels[channel].control & 0x80 != 0;
        if was_enabled && new_control & 0x80 == 0 {
            // 32-bit disable lands immediately, while a 16-bit CNT_H stop
            // defers one tick via pending_control (write_control). The
            // asymmetry is HW-pinned, not an oversight: nba start-stop
            // (16-bit stop, 2ND=8) observes the counter ticking once more
            // after the stop write, and nba reload (32-bit reset/start)
            // pins the immediate path (7/7). Do not "unify" them.
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
            // GBATEK Timers: writing CNT_L initializes the reload value only
            // (never the running counter); it lands with a one-tick delay
            // (see reload_pending).
            let timer = &mut self.channels[channel];
            timer.reload_pending = Some(value);
            self.last_reload_cycle[channel] = Some(self.current_cycle);
        }
        true
    }

    /// Advance all four timers by one CPU T-cycle and return Timer IRQ bits 3..6.
    pub fn step(&mut self) -> u16 {
        self.prescaler = self.prescaler.wrapping_add(1);
        let prescaler = self.prescaler;
        let mut irq = 0;
        let mut cascade = false;
        for index in 0..4 {
            let (next_cascade, channel_irq) = self.step_channel(index, cascade, prescaler);
            cascade = next_cascade;
            irq |= channel_irq;
        }
        irq
    }

    fn step_channel(
        &mut self,
        index: usize,
        incoming_cascade: bool,
        prescaler: u16,
    ) -> (bool, u16) {
        let timer = &mut self.channels[index];
        if timer.control & 0x80 == 0 {
            return (false, 0);
        }
        // CNT_L landing is uniform one tick: consume a pending reload
        // even inside start-delay ticks. (Previously the early return
        // below skipped the take, delaying delay-window overwrites by
        // up to 2 extra ticks. The skip was an implementation artifact
        // of the early return, not modeled HW behavior: nba schedules
        // OnReloadWritten unconditionally +1. The normal path below is
        // untouched, so overflow-vs-landing races keep their order.
        // Verified zero-effect across the full 123-case manifest.)
        if timer.start_delay != 0
            && let Some(reload) = timer.reload_pending.take()
        {
            timer.reload = reload;
        }
        if let Some((c, irq)) = Self::handle_start_delay(timer, index, prescaler) {
            // mGBA GBATimerUpdate cascades synchronously: a lower timer's
            // overflow still clocks this counter during enable latency
            // (applied after the state's own action, so the state-2 reload
            // load keeps mGBA's preload-then-++ order).
            if incoming_cascade {
                let cascade_out = increment(timer);
                let cascade_irq = if cascade_out && timer.control & (1 << 6) != 0 {
                    1 << (3 + index)
                } else {
                    0
                };
                return (c || cascade_out, irq | cascade_irq);
            }
            return (c, irq);
        }
        let tick = Self::should_tick(timer, index, incoming_cascade, prescaler);
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
        // A CNT_L write lands one tick after it is issued: an overwrite in
        // the last cycle before overflow still loads the old reload.
        if let Some(reload) = timer.reload_pending.take() {
            timer.reload = reload;
        }
        (cascade, irq)
    }

    fn handle_start_delay(
        timer: &mut TimerChannel,
        index: usize,
        prescaler: u16,
    ) -> Option<(bool, u16)> {
        match timer.start_delay {
            1 => {
                // Counter was loaded during the first latency tick (below);
                // this tick is idle.
                timer.start_delay = 0;
                Some((false, 0))
            }
            2 => {
                // Enabling takes one cycle to load the reload value. The
                // stale counter only ticks (and can overflow, even
                // cascading) when the prescaler tap fires this tick, and
                // never for count-up enables (which load the reload
                // outright) -- nba OnControlWritten / issue #331. An
                // unconditional stale tick fabricates a +1 overflow IRQ on
                // prescaled re-enables (mgba-suite timers prologues reuse
                // TM0 across prescalers with a 0xFFFF stale), spawning a
                // dispatch race HW never runs. (/1 taps fire every tick,
                // so tick-before-reload and all 0b behavior is unchanged.)
                timer.start_delay = 1;
                let count_up = index != 0 && timer.control & 4 != 0;
                if count_up {
                    timer.counter = timer.reload;
                    return Some((false, 0));
                }
                let period = [1, 64, 256, 1024][usize::from(timer.control & 3)];
                if prescaler & (period - 1) != period - 1 {
                    timer.counter = timer.reload;
                    return Some((false, 0));
                }
                let cascade = increment(timer);
                let irq = if cascade && timer.control & (1 << 6) != 0 {
                    1 << (3 + index)
                } else {
                    0
                };
                timer.counter = timer.reload;
                Some((cascade, irq))
            }
            3 => {
                timer.start_delay = 2;
                Some((false, 0))
            }
            4 => {
                timer.start_delay = 3;
                Some((false, 0))
            }
            5 => {
                timer.start_delay = 4;
                Some((false, 0))
            }
            _ => None,
        }
    }

    fn should_tick(timer: &TimerChannel, index: usize, cascade: bool, prescaler: u16) -> bool {
        if index != 0 && timer.control & 4 != 0 {
            return cascade;
        }
        // Sample the shared prescaler tap: the channel ticks when the tap
        // carries out. Identical phase to seeding from the global cycle at
        // enable (prescaler == bus tcycle, both 0-init and +1/tick), but
        // the phase now survives disable/re-enable without a jump, as HW.
        let period = [1, 64, 256, 1024][usize::from(timer.control & 3)];
        prescaler & (period - 1) == period - 1
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
    _current_cycle: u64,
    _last_reload_cycle: Option<u64>,
    _index: usize,
) {
    let was_enabled = timer.control & 0x80 != 0;
    let enabled = new_control & 0x80 != 0;
    if enabled && !was_enabled {
        timer.control = new_control;
        // A reload written together with (or just before) the enable is
        // visible to the startup load: flush the one-tick landing delay.
        if let Some(reload) = timer.reload_pending.take() {
            timer.reload = reload;
        }
        // GBATEK Timers / HW determinism: the reload value is loaded on the
        // first latency tick (see handle_start_delay), so the counter keeps
        // its stale value here. A stale tick (even overflow) may fire before
        // the load (nba tick-before-reload).
        // Fixed 2-cycle start latency (was a reload/elapsed fit that only
        // ever triggered for 0xFFFC and broke the cancel-irq race).
        timer.start_delay = 2;
        // No phase seeding: the shared prescaler is free-running and never
        // reset by enables (mGBA lastEvent = now & ~tickMask;
        // NBA prescaler_offset = now & mask).
    } else if !enabled && was_enabled {
        timer.pending_control = Some(new_control);
    } else {
        // Any direct control write supersedes a deferred stop: leaving a
        // stale pending stop would kill a later start on the next tick.
        timer.control = new_control;
        timer.pending_control = None;
        if !enabled {
            timer.start_delay = 0;
        }
    }
}

fn increment(timer: &mut TimerChannel) -> bool {
    let (value, overflow) = timer.counter.overflowing_add(1);
    // GBATEK Timers: the CURRENT reload value is copied into the counter on
    // overflow. A mid-run CNT_L write only retargets future overflows.
    timer.counter = if overflow { timer.reload } else { value };
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
        // Enabling loads the reload value at once; the 2-cycle start
        // latency ticks are idle (no stale-counter increment mid-startup).
        assert_eq!(timers.read(0x04000100), Some(0xFFFE));
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
        // /64 samples the free-running prescaler tap: reset the phase so
        // the first tick lands deterministically. Enable at prescaler 0,
        // 2 start-delay ticks (prescaler 1, 2), then the tap carries at
        // prescaler 63: 60 more steps keep the counter at 0, the 61st
        // ticks it to 1.
        timers.prescaler = 0;
        timers.step();
        timers.step();
        for _ in 0..60 {
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
