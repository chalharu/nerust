//! Shared OSD upload budget plus byte staging: the budget lives in
//! the port crate so every backend mints the same policy.

pub use nerust_render_traits::renderer::UploadBudget;

/// Grow-only byte staging for texture uploads. No steady allocs; ports
/// take `&mut [u8]` slices staged from here, never the buffer itself.
#[derive(Debug, Default)]
pub struct StagingBuf(Vec<u8>);

impl StagingBuf {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fallible allocation for full-surface uploads; a failed allocation must
    /// drop/retry the overlay rather than aborting the emulator.
    pub fn try_staging(
        &mut self,
        need: usize,
    ) -> Result<&mut [u8], std::collections::TryReserveError> {
        if need > self.0.len() {
            self.0.try_reserve(need - self.0.len())?;
            self.0.resize(need, 0);
        }
        Ok(&mut self.0[..need])
    }

    /// Grow-only staging of at least `need` initialized bytes.
    pub fn staging(&mut self, need: usize) -> &mut [u8] {
        if self.0.len() < need {
            self.0.resize(need, 0);
        }
        &mut self.0[..need]
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::StagingBuf;
    use nerust_render_traits::renderer::{OsdUploadPolicy as _, UploadBudget};

    #[test]
    fn budget_saturates_instead_of_wrapping() {
        let mut policy = UploadBudget::new(3);
        for _ in 0..300 {
            policy.record_failure();
        }
        assert!(policy.record_failure());
        policy.reset();
        assert!(!policy.record_failure());
    }

    #[test]
    fn staging_buf_grows_once_and_reuses() {
        let mut buf = StagingBuf::new();
        assert!(buf.is_empty());
        buf.staging(8).fill(0xAB);
        assert_eq!(buf.len(), 8);
        let ptr = buf.staging(4).as_ptr();
        // Smaller need reuses the allocation without shrinking.
        assert_eq!(buf.staging(4).as_ptr(), ptr);
        assert_eq!(buf.len(), 8);
        assert_eq!(&buf.staging(8)[..4], &[0xAB; 4]);
        // Growth initializes bytes before exposing a safe slice.
        assert_eq!(&buf.staging(10)[8..], &[0, 0]);
    }

    #[test]
    fn failed_staging_allocation_preserves_existing_bytes() {
        let mut buf = StagingBuf::new();
        buf.try_staging(4).unwrap().fill(7);
        assert!(buf.try_staging(usize::MAX).is_err());
        assert_eq!(buf.len(), 4);
        assert_eq!(buf.try_staging(4).unwrap(), &[7; 4]);
    }
}
