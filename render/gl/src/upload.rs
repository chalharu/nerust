//! Impl-owned OSD upload budget plus byte staging: the port crate keeps
//! only the `OsdUploadPolicy`/`OsdUploadPolicyFactory` traits.

use nerust_render_traits::renderer::{OsdUploadPolicy, OsdUploadPolicyFactory};

/// Bounded retry: floor of one attempt, saturating counter.
#[derive(Debug)]
pub struct BoundedUploadPolicy {
    max: u8,
    fails: u8,
}

impl BoundedUploadPolicy {
    /// Zero would drop live layers on the first transient failure, so
    /// the floor is one attempt: the constructor clamps, not callers.
    pub fn new(max: u8) -> Self {
        Self {
            max: max.max(1),
            fails: 0,
        }
    }
}

impl OsdUploadPolicy for BoundedUploadPolicy {
    fn record_failure(&mut self) -> bool {
        self.fails = self.fails.saturating_add(1);
        self.fails >= self.max
    }

    fn reset(&mut self) {
        self.fails = 0;
    }
}

/// Factory minting one bounded budget per rebuilt overlay view.
/// Renderers hold this, never a concrete policy type.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicyFactory {
    max: u8,
}

impl RetryPolicyFactory {
    /// Budget source for one view lifetime; clamps at mint time.
    pub fn new(max: u8) -> Self {
        Self { max }
    }
}

impl OsdUploadPolicyFactory for RetryPolicyFactory {
    fn create_policy(&self) -> Box<dyn OsdUploadPolicy> {
        Box::new(BoundedUploadPolicy::new(self.max))
    }
}

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
    use super::{BoundedUploadPolicy, RetryPolicyFactory, StagingBuf};
    use nerust_render_traits::renderer::{OsdUploadPolicy as _, OsdUploadPolicyFactory as _};

    #[test]
    fn bounded_policy_pins_single_budget() {
        let mut policy = BoundedUploadPolicy::new(0);
        // Zero clamps to one attempt, never drops live layers at once.
        assert!(policy.record_failure());
        policy.reset();
        let mut policy = BoundedUploadPolicy::new(3);
        assert!(!policy.record_failure());
        assert!(!policy.record_failure());
        assert!(policy.record_failure());
        assert!(policy.record_failure());
        policy.reset();
        assert!(!policy.record_failure());
        // Saturation: hundreds of failures never wrap back to retry.
        let mut policy = BoundedUploadPolicy::new(3);
        for _ in 0..300 {
            policy.record_failure();
        }
        assert!(policy.record_failure());
    }

    #[test]
    fn retry_factory_mints_independent_budgets() {
        let factory = RetryPolicyFactory::new(3);
        let mut first = factory.create_policy();
        let mut second = factory.create_policy();
        assert!(!first.record_failure());
        assert!(!first.record_failure());
        // A second view starts with a fresh budget, not the spent one.
        assert!(!second.record_failure());
        assert!(!second.record_failure());
        assert!(first.record_failure());
        assert!(second.record_failure());
        // Zero clamps at mint time.
        let mut policy = RetryPolicyFactory::new(0).create_policy();
        assert!(policy.record_failure());
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
