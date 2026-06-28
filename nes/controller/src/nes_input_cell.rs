use std::sync::Arc;

use nerust_core_traits::input::{InputCell, InputState};

/// NES 入力セル: [P1 ボタン, P2 ボタン, マイク(0/1)]
///
/// スレッド間で共有する場合は `Arc<NesInputCell>` を使用する。
pub struct NesInputCell(InputCell<3>);

impl std::fmt::Debug for NesInputCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("NesInputCell").field(&self.0.load()).finish()
    }
}

impl NesInputCell {
    pub fn new() -> Self {
        Self(InputCell::new())
    }

    /// GUI 側からボタン状態を書き込む (`[p1, p2, mic]`)。
    pub fn store(&self, p1: u8, p2: u8, mic: bool) {
        self.0.store(&[p1, p2, mic as u8]);
    }

    /// Device 側に渡すための `InputState<3>` 実装。
    pub fn sample(&self) -> [u8; 3] {
        self.0.load()
    }
}

impl InputState<3> for NesInputCell {
    fn sample(&self) -> [u8; 3] {
        self.0.load()
    }
}

/// `Arc<NesInputCell>` をラップして `InputState<3>` を実装する。
pub struct SharedNesInputCell(pub Arc<NesInputCell>);

impl InputState<3> for SharedNesInputCell {
    fn sample(&self) -> [u8; 3] {
        self.0.sample()
    }
}

impl Default for NesInputCell {
    fn default() -> Self {
        Self::new()
    }
}
