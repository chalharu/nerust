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
    /// First-overflow tick since the channel's fresh enable, per channel.
    /// The first take samples a boundary later than the line alone allows
    /// (the take#1 delay is relative to the first overflow, not the
    /// enable: longer reloads overflow past any enable window).
    last_ovf1_cycle: [Option<u64>; 4],
    /// Whether the channel's fresh enable carried a fresh reload (a CNT_L
    /// write landed on the same tick: an atomic 32-bit enable. Split
    /// CNT_L+CNT_H enables use a reload that was set earlier (it cannot
    /// land while stopped, so it is still pending but aged).
    last_enable_fresh_reload: [bool; 4],
    /// Overflows since the last fresh enable, per channel (saturates).
    /// The first overflow primes downstream sample pipelines (sound
    /// FIFO: arms without consuming); later overflows drain.
    overflows_since_enable: [u8; 4],
    /// Timer0 IF acks since the last fresh enable (saturates). A second
    /// take answering a third overflow has seen exactly one ack: the
    /// second overflow arrived masked and was discarded.
    timer0_acks_since_enable: u8,
    /// Sample latency of the run's first timer0 take (take#1), in
    /// T-cycles. Middle-take entries key on it (storm grid-phase proxy).
    /// Reset on enable; the latest first-take wins (None = none yet).
    take1_latency: Option<u64>,
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
        // Stops defer one tick like 16-bit writes (both widths share the
        // +1-tick control events). The deferred 16-bit stop is HW-pinned
        // (nba start-stop 2ND=8); no HW test covers the 32-bit stop, so
        // the unified model rules here.
        if write_control(&mut self.channels[channel], new_control) {
            // A fresh enable re-primes downstream sample pipelines: the
            // next overflow arms without consuming (alyosha fifo_4 pins
            // the skipped first pop).
            self.overflows_since_enable[channel] = 0;
            self.last_ovf1_cycle[channel] = None;
            // A 32-bit write always carries a fresh reload (set above).
            self.last_enable_fresh_reload[channel] = true;
            if channel == 0 {
                self.timer0_acks_since_enable = 0;
                self.take1_latency = None;
            }
        }
        true
    }

    /// Count a timer0 IF ack (handler ends clear the serviced flag).
    pub fn note_timer0_ack(&mut self) {
        self.timer0_acks_since_enable = self.timer0_acks_since_enable.saturating_add(1);
    }

    /// Timer0 IF acks since the last fresh enable.
    pub fn timer0_acks_since_enable(&self) -> u8 {
        self.timer0_acks_since_enable
    }

    /// Record the run's first timer0 take latency (latest first-take
    /// wins: the sync take's latency is overwritten by the run's own).
    pub fn record_take1_latency(&mut self, latency: u64) {
        self.take1_latency = Some(latency);
    }

    /// Sample latency of the run's first timer0 take, if any.
    pub fn take1_latency(&self) -> Option<u64> {
        self.take1_latency
    }

    /// First-overflow tick since the channel's fresh enable, if any.
    pub fn last_ovf1_cycle(&self, channel: usize) -> Option<u64> {
        self.last_ovf1_cycle[channel]
    }

    /// Whether the channel's fresh enable carried a fresh reload.
    pub fn last_enable_fresh_reload(&self, channel: usize) -> bool {
        self.last_enable_fresh_reload[channel]
    }

    /// Prescaler selector bits of a channel (T4 gate: fast timers only).
    pub fn prescaler_bits(&self, channel: usize) -> u16 {
        self.channels[channel].control & 3
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
            // A CNT_H enable lands with a fresh reload only when the CNT_L
            // write happened on this same tick (an atomic 32-bit enable
            // sets both just above; an earlier split write is aged).
            let fresh_reload = self.last_reload_cycle[channel] == Some(self.current_cycle);
            if write_control(&mut self.channels[channel], value & 0x00C7) {
                self.overflows_since_enable[channel] = 0;
                self.last_ovf1_cycle[channel] = None;
                self.last_enable_fresh_reload[channel] = fresh_reload;
                if channel == 0 {
                    self.timer0_acks_since_enable = 0;
                    self.take1_latency = None;
                }
            }
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
        self.step_full().0
    }

    /// Advance one T-cycle, returning Timer IRQ bits 3..6 plus raw
    /// overflow bits 0..3. Overflows clock downstream hardware (sound
    /// FIFO sample drains, count-up timers) whether or not the timer's
    /// IRQ is enabled; only the IRQ bits may raise IF.
    pub fn step_full(&mut self) -> (u16, u16) {
        self.prescaler = self.prescaler.wrapping_add(1);
        let prescaler = self.prescaler;
        let mut irq = 0;
        let mut overflow = 0;
        let mut cascade = false;
        for index in 0..4 {
            let (next_cascade, channel_irq) = self.step_channel(index, cascade, prescaler);
            cascade = next_cascade;
            irq |= channel_irq;
            if next_cascade {
                overflow |= 1 << index;
                if self.overflows_since_enable[index] == 0 {
                    self.last_ovf1_cycle[index] = Some(self.current_cycle);
                }
                self.overflows_since_enable[index] =
                    self.overflows_since_enable[index].saturating_add(1);
            }
        }
        (irq, overflow)
    }

    /// Overflows since this channel's last fresh enable (saturates at 255).
    pub fn overflows_since_enable(&self, channel: usize) -> u8 {
        self.overflows_since_enable[channel]
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
        // CNT_L landing is a uniform one tick, even inside start-delay ticks.
        if timer.start_delay != 0
            && let Some(reload) = timer.reload_pending.take()
        {
            timer.reload = reload;
        }
        if let Some((c, irq)) = Self::handle_start_delay(timer, index, prescaler) {
            // Cascades apply synchronously: a lower timer's overflow still
            // clocks this counter during enable latency (applied after the
            // state's own action, so the state-2 reload load keeps
            // preload-then-++ order).
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
                // Stale counter ticks only if the prescaler tap fires, never
                // for count-up enables; otherwise re-enables gain a spurious
                // overflow IRQ.
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

fn write_control(timer: &mut TimerChannel, new_control: u16) -> bool {
    let was_enabled = timer.control & 0x80 != 0;
    let enabled = new_control & 0x80 != 0;
    // A stop queued this same tick (no tick elapsed) followed by an enable
    // is a restart, not a running write: the disable never took effect.
    let restarted = enabled && timer.pending_control.is_some();
    if (enabled && !was_enabled) || restarted {
        timer.control = new_control;
        // A fresh start supersedes any deferred stop: leaving a stale
        // pending stop would kill the new run a tick later.
        timer.pending_control = None;
        // A reload written together with (or just before) the enable is
        // visible to the startup load: flush the one-tick landing delay.
        if let Some(reload) = timer.reload_pending.take() {
            timer.reload = reload;
        }
        // Reload loads on the first latency tick; a stale tick may fire
        // before the load. Start latency is a fixed 2 cycles.
        timer.start_delay = 2;
        // No phase seeding: the shared prescaler is free-running and never
        // reset by enables.
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
    (enabled && !was_enabled) || restarted
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
