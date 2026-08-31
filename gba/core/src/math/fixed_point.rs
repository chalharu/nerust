/// 8.8 固定小数点 (s7.8) — GBAアフィンで使用
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fixed8_8(pub i16);

#[allow(clippy::should_implement_trait)]
impl Fixed8_8 {
    pub const fn from_raw(raw: i16) -> Self {
        Self(raw)
    }
    pub const fn from_int(i: i16) -> Self {
        Self(i << 8)
    }
    pub const fn to_raw(self) -> i16 {
        self.0
    }
    pub fn from_f32(v: f32) -> Self {
        Self((v * 256.0) as i16)
    }
    pub fn to_f32(self) -> f32 {
        self.0 as f32 / 256.0
    }
    pub fn mul(self, other: Self) -> Self {
        Self(((self.0 as i32 * other.0 as i32) >> 8) as i16)
    }
    pub fn add(self, other: Self) -> Self {
        Self(self.0.wrapping_add(other.0))
    }
}

/// 20.8 固定小数点 — テクスチャ座標用
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fixed20_8(pub i32);

#[allow(clippy::should_implement_trait)]
impl Fixed20_8 {
    pub const fn from_raw(raw: i32) -> Self {
        Self(raw)
    }
    pub fn from_int(i: i32) -> Self {
        Self(i << 8)
    }
    pub fn add(self, other: Self) -> Self {
        Self(self.0.wrapping_add(other.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed8_8_mul() {
        let a = Fixed8_8::from_int(2);
        let b = Fixed8_8::from_int(3);
        assert_eq!(a.mul(b).to_raw(), Fixed8_8::from_int(6).to_raw());
    }
}
