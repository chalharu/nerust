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

    /// Cancellation is checked between bounded reads; no stale whole-file I/O.
    pub(crate) fn read_latest(
        &self,
        seq: u64,
        mut reader: impl std::io::Read,
    ) -> std::io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        let mut chunk = [0; 64 * 1024];
        loop {
            if !self.is_latest(seq) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "ROM fetch cancelled",
                ));
            }
            let count = reader.read(&mut chunk)?;
            if !self.is_latest(seq) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "ROM fetch cancelled",
                ));
            }
            if count == 0 {
                return Ok(bytes);
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
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

    #[test]
    fn cancellation_stops_reads_instead_of_publishing_partial_bytes() {
        let seq = FetchSeq::new();
        let epoch = seq.next();
        struct CancellingReader<'a>(&'a FetchSeq);
        impl std::io::Read for CancellingReader<'_> {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                buffer[0] = 1;
                self.0.next();
                Ok(1)
            }
        }
        assert_eq!(
            seq.read_latest(epoch, CancellingReader(&seq))
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::Interrupted
        );
        let epoch = seq.next();
        assert_eq!(
            seq.read_latest(epoch, std::io::Cursor::new([1, 2, 3]))
                .unwrap(),
            [1, 2, 3]
        );
    }

    #[test]
    fn cancellation_invalidates_already_published_completion_at_consumption() {
        let seq = FetchSeq::new();
        let completed = seq.next();
        assert!(seq.is_latest(completed));
        let published = Some((completed, vec![1, 2, 3]));
        seq.next(); // Unload/suspend, including after publication.
        assert!(
            published
                .filter(|(epoch, _)| seq.is_latest(*epoch))
                .is_none()
        );
    }
}
