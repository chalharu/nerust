//! Shared change gate + content hash: single home in the frontend kernel.
//! Both native frontends share these; see `nerust_frontend_kernel`.

pub(crate) use nerust_frontend_kernel::{Fnv1a, Versioned, level_visible};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_is_stable_and_content_sensitive() {
        use core::hash::Hash;
        let mut a = Fnv1a::new();
        7u64.hash(&mut a);
        let mut b = Fnv1a::new();
        7u64.hash(&mut b);
        assert_eq!(a.finish(), b.finish());
        let mut c = Fnv1a::new();
        8u64.hash(&mut c);
        assert_ne!(a.finish(), c.finish());
    }

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
        assert!(!memo.check(&"b".to_string()));
        assert_eq!(memo.version(), 2);
    }

    #[test]
    fn check_takes_reference_and_gates_repeats() {
        let mut memo: Versioned<u32> = Versioned::new();
        assert!(memo.check(&7));
        assert_eq!(memo.get(), Some(&7));
        assert!(!memo.check(&7));
        memo.clear();
        assert!(memo.check(&7));
    }
}
