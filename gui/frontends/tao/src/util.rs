//! Shared change gate: remembers the last pushed value plus a version.

/// Versioned change gate: true when `value` differs from memory.
/// False means the downstream push can be skipped.
#[derive(Debug, Default)]
pub(crate) struct Versioned<T: PartialEq> {
    last: Option<T>,
    version: u64,
}

impl<T: PartialEq> Versioned<T> {
    pub(crate) fn new() -> Self {
        Self {
            last: None,
            version: 0,
        }
    }

    /// Monotonic change count: bumped once per accepted value.
    pub(crate) fn version(&self) -> u64 {
        self.version
    }

    /// True on change (recording `value`); false on repeat.
    pub(crate) fn check(&mut self, value: &T) -> bool
    where
        T: Clone,
    {
        if self.last.as_ref() == Some(value) {
            return false;
        }
        self.last = Some(value.clone());
        self.version += 1;
        true
    }

    /// Single-op take-gate: records `value`, reports whether it changed.
    pub(crate) fn should_sync(&mut self, value: T) -> bool
    where
        T: Clone,
    {
        if self.last.as_ref() == Some(&value) {
            return false;
        }
        self.last = Some(value);
        self.version += 1;
        true
    }

    pub(crate) fn clear(&mut self) {
        self.last = None;
    }

    pub(crate) fn get(&self) -> Option<&T> {
        self.last.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_gates_repeats() {
        let mut memo: Versioned<(u64, u8)> = Versioned::new();
        assert!(memo.check(&(1, 2)));
        assert!(!memo.check(&(1, 2)));
        assert!(memo.check(&(1, 3)));
        memo.clear();
        assert!(memo.check(&(1, 3)));
    }

    #[test]
    fn version_bumps_once_per_change() {
        let mut memo: Versioned<String> = Versioned::new();
        assert_eq!(memo.version(), 0);
        assert!(memo.check(&"a".to_string()));
        assert_eq!(memo.version(), 1);
        assert!(!memo.check(&"a".to_string()));
        assert_eq!(memo.version(), 1);
        assert!(memo.should_sync("b".to_string()));
        assert_eq!(memo.version(), 2);
        assert!(!memo.should_sync("b".to_string()));
        assert_eq!(memo.version(), 2);
    }

    #[test]
    fn should_sync_takes_ownership_atomically() {
        let mut memo: Versioned<u32> = Versioned::new();
        assert!(memo.should_sync(7));
        assert_eq!(memo.get(), Some(&7));
        assert!(!memo.should_sync(7));
        memo.clear();
        assert!(memo.should_sync(7));
    }
}
