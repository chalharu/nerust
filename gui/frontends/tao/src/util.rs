//! Shared dirty gate: remembers the last pushed value.
//! One home for every skip-if-unchanged memo.

/// Change gate: true when `value` differs from memory.
/// False means the downstream push can be skipped.
#[derive(Debug, Default)]
pub(crate) struct Dirty<T: PartialEq> {
    last: Option<T>,
}

impl<T: PartialEq> Dirty<T> {
    pub(crate) fn new() -> Self {
        Self { last: None }
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
        let mut memo: Dirty<(u64, u8)> = Dirty::new();
        assert!(memo.check(&(1, 2)));
        assert!(!memo.check(&(1, 2)));
        assert!(memo.check(&(1, 3)));
        memo.clear();
        assert!(memo.check(&(1, 3)));
    }
}
