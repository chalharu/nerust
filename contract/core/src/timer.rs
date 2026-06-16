use std::time::{Duration, Instant};

/// フレームレート制御用タイマー。
///
/// 起動時刻を基準に、フレーム番号から期待終了時刻を計算し、
/// 現在時刻との差分を返す。累積誤差を補正する。
///
/// ```
/// let timer = FrameTimer::new(Duration::from_secs_f64(1.0 / 60.0));
///
/// // フレーム N を描画後、フレーム N+1 の開始まで待つ:
/// if let Some(dur) = timer.remaining_until(frame_index + 1) {
///     thread::sleep(dur);
/// }
/// ```
pub struct FrameTimer {
    start: Instant,
    frame_interval: Duration,
}

impl FrameTimer {
    pub fn new(frame_interval: Duration) -> Self {
        Self {
            start: Instant::now(),
            frame_interval,
        }
    }

    /// フレーム `frame_index` の期待時刻までの残り時間を返す。
    ///
    /// 期待時刻を過ぎている場合は `None` を返す（フレームスキップの判断に使える）。
    pub fn remaining_until(&self, frame_index: u64) -> Option<Duration> {
        let target = self.start + self.frame_interval.mul_f64(frame_index as f64);
        target.checked_duration_since(Instant::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn first_frame_waits_for_one_interval() {
        let timer = FrameTimer::new(Duration::from_millis(100));
        let remaining = timer.remaining_until(1).unwrap();
        assert!(remaining > Duration::from_millis(95));
        assert!(remaining <= Duration::from_millis(100));
    }

    #[test]
    fn frame_zero_returns_immediately_or_slightly_late() {
        let timer = FrameTimer::new(Duration::from_millis(100));
        // frame 0's deadline is at Instant::now(), which may already have passed
        // by the time remaining_until is called (Some(0ns) or None)
        match timer.remaining_until(0) {
            Some(dur) => assert!(dur.as_nanos() < 1_000_000),
            None => {} // acceptable: slightly late
        }
    }

    #[test]
    fn slow_frame_returns_none() {
        let timer = FrameTimer::new(Duration::from_millis(1));
        // frame 1's deadline is 1ms from start. Sleeping 10ms guarantees we're past it.
        thread::sleep(Duration::from_millis(10));
        assert!(timer.remaining_until(1).is_none());
    }
}
