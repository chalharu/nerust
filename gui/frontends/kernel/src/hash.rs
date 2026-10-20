//! FNV-1a content hashing: integers and hashes only, never `format!`.

/// FNV-1a hasher shared by content fingerprints.
#[derive(Debug)]
pub struct Fnv1a(u64);

impl Fnv1a {
    /// Offset-basis seed for a fresh fingerprint.
    pub fn new() -> Self {
        Self(0xcbf29ce484222325)
    }

    /// Terminal digest of the hashed content.
    pub fn finish(&self) -> u64 {
        self.0
    }
}

impl Default for Fnv1a {
    fn default() -> Self {
        Self::new()
    }
}

impl core::hash::Hasher for Fnv1a {
    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::hash::{Hash, Hasher};

    #[test]
    fn digest_is_stable_and_content_sensitive() {
        let mut a = Fnv1a::new();
        7u64.hash(&mut a);
        let mut b = Fnv1a::new();
        7u64.hash(&mut b);
        assert_eq!(a.finish(), b.finish());
        assert_eq!(Hasher::finish(&a), b.finish());
        let mut c = Fnv1a::new();
        8u64.hash(&mut c);
        assert_ne!(a.finish(), c.finish());
    }
}
