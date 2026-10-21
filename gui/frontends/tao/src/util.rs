//! Shared change gate: single home in the frontend kernel.
//! Both native frontends share these; see `nerust_frontend_kernel`.

pub(crate) use nerust_frontend_kernel::{Versioned, level_visible};

#[cfg(test)]
mod tests {
    use super::*;

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
