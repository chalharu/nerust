//! Shared versioned change gates for small frontend keys.

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
}
