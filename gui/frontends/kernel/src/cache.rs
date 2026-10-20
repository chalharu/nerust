//! Shared change gates: versioned memos plus Eq-gated snapshot shares.

/// Versioned change gate: true when `value` differs from memory.
/// False means the downstream push can be skipped.
#[derive(Debug, Default)]
pub struct Versioned<T: PartialEq> {
    last: Option<T>,
    version: u64,
}

impl<T: PartialEq> Versioned<T> {
    /// Empty gate: nothing remembered yet.
    pub fn new() -> Self {
        Self {
            last: None,
            version: 0,
        }
    }

    /// Monotonic change count: bumped once per accepted value.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// True on change (recording `value`); false on repeat.
    pub fn check(&mut self, value: &T) -> bool
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

    /// Forget the remembered value: the next `check` always passes.
    pub fn clear(&mut self) {
        self.last = None;
    }

    /// Remembered value, if any.
    pub fn get(&self) -> Option<&T> {
        self.last.as_ref()
    }
}

/// Eq-gated snapshot share: clones the value only when it changed
/// since the last frame; steady frames reuse one `Arc`.
#[derive(Debug)]
pub struct SnapshotCache<T: PartialEq> {
    last: Option<std::sync::Arc<T>>,
}

impl<T: PartialEq> Default for SnapshotCache<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: PartialEq> SnapshotCache<T> {
    /// Empty cache: the first `get` always clones.
    pub fn new() -> Self {
        Self { last: None }
    }

    /// Shared snapshot: the cached `Arc` on repeat, a fresh clone on change.
    pub fn get(&mut self, current: &T) -> std::sync::Arc<T>
    where
        T: Clone,
    {
        if let Some(last) = &self.last
            && last.as_ref() == current
        {
            return std::sync::Arc::clone(last);
        }
        let fresh = std::sync::Arc::new(current.clone());
        self.last = Some(std::sync::Arc::clone(&fresh));
        fresh
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
        assert!(memo.check(&"b".to_string()));
        assert_eq!(memo.version(), 2);
    }

    #[test]
    fn snapshot_cache_reuses_arc_on_steady_values() {
        let mut cache: SnapshotCache<Vec<u8>> = SnapshotCache::new();
        let value = vec![1u8, 2, 3];
        let first = cache.get(&value);
        let second = cache.get(&value);
        assert!(std::sync::Arc::ptr_eq(&first, &second));
        let changed = vec![1u8, 2, 4];
        let third = cache.get(&changed);
        assert!(!std::sync::Arc::ptr_eq(&first, &third));
        assert_eq!(*third, changed);
    }
}
