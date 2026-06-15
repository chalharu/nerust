use std::sync::Arc;

use nerust_contract_core::input::InputCell;

/// NES 固有の入力セルラッパー。
///
/// `[u8; 2]` の各要素の意味を (P1 ボタン, P2 ボタン) と明示する。
pub struct NesInputCell(Arc<InputCell<2>>);

impl NesInputCell {
    pub fn new() -> Self {
        Self(Arc::new(InputCell::new()))
    }

    /// GUI 側からボタン状態を書き込む。
    pub fn store(&self, p1: u8, p2: u8) {
        self.0.store(&[p1, p2]);
    }

    /// Device 側へ共有する `Arc<InputCell<2>>` を取得する。
    pub fn share(&self) -> Arc<InputCell<2>> {
        self.0.clone()
    }
}

impl Default for NesInputCell {
    fn default() -> Self {
        Self::new()
    }
}
