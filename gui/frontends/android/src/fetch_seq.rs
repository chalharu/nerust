//! Fetch sequencing for the single picker worker.
//! Every request takes a sequence id; the worker drops stale completions.

use std::sync::atomic::{AtomicU64, Ordering};

/// Monotonic fetch generation: one worker, latest request wins.
#[derive(Debug, Default)]
pub(crate) struct FetchSeq(AtomicU64);

impl FetchSeq {
    pub(crate) const fn new() -> Self {
        Self(AtomicU64::new(0))
    }

    /// Next generation for a new request; wraparound keeps monotonicity.
    pub(crate) fn next(&self) -> u64 {
        self.0.fetch_add(1, Ordering::AcqRel).wrapping_add(1)
    }

    /// A completion publishes only when its generation is still current.
    pub(crate) fn is_latest(&self, seq: u64) -> bool {
        self.0.load(Ordering::Acquire) == seq
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_request_wins_and_stale_drops() {
        let seq = FetchSeq::new();
        let first = seq.next();
        let second = seq.next();
        assert!(!seq.is_latest(first));
        assert!(seq.is_latest(second));
    }
}
