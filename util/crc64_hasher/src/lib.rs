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
