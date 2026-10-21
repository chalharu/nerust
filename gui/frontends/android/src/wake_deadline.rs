//! One idle policy shared by retries and OSD expiration; never busy-poll.
use std::time::Instant;

pub(crate) fn next_deadline(retry: Option<Instant>, osd: Option<Instant>) -> Option<Instant> {
    retry.into_iter().chain(osd).min()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn earliest_deadline_wins_and_idle_waits() {
        let now = Instant::now();
        let later = now + Duration::from_secs(2);
        assert_eq!(next_deadline(None, None), None);
        assert_eq!(next_deadline(Some(later), None), Some(later));
        assert_eq!(next_deadline(Some(later), Some(now)), Some(now));
        assert_eq!(next_deadline(Some(now), Some(later)), Some(now));
    }
}
