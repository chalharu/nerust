use std::hash::Hasher;

use crc::{CRC_64_XZ, Crc, Digest};

const CRC64_LEGACY_ECMA: Crc<u64> = Crc::<u64>::new(&CRC_64_XZ);

pub struct Crc64Hasher(Digest<'static, u64>);

impl Crc64Hasher {
    pub fn new() -> Self {
        Self(CRC64_LEGACY_ECMA.digest())
    }
}

pub fn crc64(bytes: &[u8]) -> u64 {
    let mut hasher = Crc64Hasher::new();
    hasher.write(bytes);
    hasher.finish()
}

impl Hasher for Crc64Hasher {
    fn write(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    fn finish(&self) -> u64 {
        self.0.clone().finalize()
    }
}

impl Default for Crc64Hasher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::crc64;

    #[test]
    fn crc64_changes_with_input() {
        let first = [0x00, 0x10, 0x20, 0xFF, 0x40, 0x50, 0x60, 0xFF];
        let second = [0x00, 0x10, 0x20, 0xFF, 0x40, 0x50, 0x61, 0xFF];

        assert_eq!(crc64(&first), crc64(&first));
        assert_ne!(crc64(&first), crc64(&second));
    }

    #[test]
    fn crc64_consistency() {
        let data = [0x00, 0x10, 0x20, 0xFF, 0x40, 0x50, 0x60, 0xFF];
        let hash1 = crc64(&data);
        let hash2 = crc64(&data);
        assert_eq!(hash1, hash2);
    }
}
