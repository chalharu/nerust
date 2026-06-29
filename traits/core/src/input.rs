use std::sync::atomic::{AtomicU8, Ordering};

/// N バイトの入力状態をアトミックに共有する。
///
/// 要素毎の atomicity は保証するが、全要素の一貫性は保証しない。
/// NES/SNES では latch 直後にしか参照しないため問題にならない。
pub struct InputCell<const N: usize> {
    inner: [AtomicU8; N],
}

impl<const N: usize> std::fmt::Debug for InputCell<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputCell")
            .field("value", &self.load())
            .finish()
    }
}

impl<const N: usize> InputCell<N> {
    pub fn new() -> Self {
        Self {
            inner: [0; N].map(AtomicU8::new),
        }
    }

    pub fn store(&self, src: &[u8; N]) {
        for (i, v) in src.iter().enumerate() {
            self.inner[i].store(*v, Ordering::Release);
        }
    }

    pub fn load(&self) -> [u8; N] {
        let mut out = [0u8; N];
        for (i, v) in self.inner.iter().enumerate() {
            out[i] = v.load(Ordering::Acquire);
        }
        out
    }
}

impl<const N: usize> Default for InputCell<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Device 側が入力状態を読み取るための trait。
pub trait InputState<const N: usize> {
    fn sample(&self) -> [u8; N];
}

/// GUI 側が入力状態を書き込むための trait。
pub trait InputSink<const N: usize>: InputState<N> {
    fn apply(&mut self, src: &[u8; N]);
}

impl<const N: usize> InputState<N> for std::sync::Arc<InputCell<N>> {
    fn sample(&self) -> [u8; N] {
        self.load()
    }
}

impl<const N: usize> InputSink<N> for std::sync::Arc<InputCell<N>> {
    fn apply(&mut self, src: &[u8; N]) {
        self.store(src);
    }
}
