use std::collections::VecDeque;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct Timer {
    instants: VecDeque<Instant>,
    next_deadline: Instant,
    sleep_margin: Duration,
    frame_interval: Duration,
    fps: f32,
}

/// NTSC の標準フレーム間隔（≈16.67ms）
pub const NTSC_FRAME_INTERVAL: Duration = Duration::from_nanos(16_666_667);

impl Timer {
    /// デフォルト（NTSC ≈ 60fps）
    pub fn new() -> Self {
        Self::new_with_interval(NTSC_FRAME_INTERVAL)
    }

    /// 任意のフレーム間隔
    pub fn new_with_interval(frame_interval: Duration) -> Self {
        let instants = VecDeque::with_capacity(Self::CALC_FRAMES);
        let now = Instant::now();
        Self {
            instants,
            next_deadline: now + frame_interval,
            sleep_margin: Duration::from_nanos(Self::SPIN_WAIT_NANOS),
            frame_interval,
            fps: 0.0,
        }
    }

    const CALC_FRAMES: usize = 64;
    const SPIN_WAIT_NANOS: u64 = 200_000;
    const MAX_CATCH_UP_FRAMES: u32 = 3;

    pub fn wait(&mut self) {
        let now = Instant::now();
        self.record_instant(now);

        let sleep_until = self
            .next_deadline
            .checked_sub(self.sleep_margin)
            .unwrap_or(self.next_deadline);
        if now < sleep_until {
            let wait = sleep_until.duration_since(now);
            thread::sleep(wait);
        }

        let mut waited_until = Instant::now();
        while waited_until < self.next_deadline {
            std::hint::spin_loop();
            waited_until = Instant::now();
        }
        self.next_deadline =
            advance_deadline(self.next_deadline, waited_until, self.frame_interval);
    }

    fn record_instant(&mut self, now: Instant) {
        let len = self.instants.len();
        if len == 0 {
            self.instants.push_back(now);
        } else {
            let duration = now.duration_since(if len >= Self::CALC_FRAMES {
                self.instants.pop_front().unwrap()
            } else {
                *self.instants.front().unwrap()
            });
            self.instants.push_back(now);
            let elapsed_nanos = duration.as_nanos().max(1) as f64;
            self.fps = (1_000_000_000_f64 / elapsed_nanos * len as f64) as f32;
        }
    }

    pub fn as_fps(&self) -> f32 {
        self.fps
    }
}

fn advance_deadline(next_deadline: Instant, now: Instant, frame_interval: Duration) -> Instant {
    if now < next_deadline {
        return next_deadline;
    }

    let late = now.duration_since(next_deadline);
    let max_catch_up = frame_interval
        .checked_mul(Timer::MAX_CATCH_UP_FRAMES)
        .expect("frame interval catch-up window must be representable");
    if late <= max_catch_up {
        // Keep a small backlog for a few frames so brief stalls are recovered by
        // running without sleeping until we rejoin the ideal cadence.
        return next_deadline
            .checked_add(frame_interval)
            .unwrap_or(now + frame_interval);
    }

    let interval_nanos = frame_interval.as_nanos();
    let skipped_intervals = late.as_nanos() / interval_nanos;
    let skipped_intervals = skipped_intervals.saturating_add(1);
    let skipped_intervals = skipped_intervals.min(u128::from(u32::MAX)) as u32;
    next_deadline
        .checked_add(
            frame_interval
                .checked_mul(skipped_intervals)
                .expect("frame interval catch-up step must be representable"),
        )
        .unwrap_or(now + frame_interval)
}

impl Default for Timer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{Timer, advance_deadline};
    use std::time::{Duration, Instant};

    #[test]
    fn slight_lag_keeps_single_frame_catch_up() {
        let frame_interval = Duration::from_millis(16);
        let base = Instant::now();
        let scheduled = base + frame_interval;
        let now = scheduled + Duration::from_millis(4);

        let advanced = advance_deadline(scheduled, now, frame_interval);

        assert_eq!(advanced, scheduled + frame_interval);
        assert!(advanced > now);
    }

    #[test]
    fn long_stall_skips_backlog_in_one_step() {
        let frame_interval = Duration::from_millis(16);
        let base = Instant::now();
        let scheduled = base + frame_interval;
        let now = scheduled + Duration::from_millis(120);

        let advanced = advance_deadline(scheduled, now, frame_interval);
        let max_catch_up = frame_interval
            .checked_mul(Timer::MAX_CATCH_UP_FRAMES)
            .expect("test duration must fit");

        assert!(now.duration_since(scheduled) > max_catch_up);
        assert!(advanced > now);
        assert!(advanced <= now + frame_interval);
    }

    #[test]
    fn multi_frame_lag_within_catch_up_window_preserves_backlog() {
        let frame_interval = Duration::from_millis(16);
        let base = Instant::now();
        let scheduled = base + frame_interval;
        let now = scheduled + frame_interval * 2;

        let advanced = advance_deadline(scheduled, now, frame_interval);
        let max_catch_up = frame_interval
            .checked_mul(Timer::MAX_CATCH_UP_FRAMES)
            .expect("test duration must fit");

        assert!(now.duration_since(scheduled) <= max_catch_up);
        assert_eq!(advanced, scheduled + frame_interval);
        assert!(advanced < now);
    }
}
