use std::sync::Arc;

use nerust_contract_core::input::InputCell;

/// NES 入力セル: [P1 ボタン, P2 ボタン, マイク(0/1)]
pub struct NesInputCell(Arc<InputCell<3>>);

impl std::fmt::Debug for NesInputCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("NesInputCell").field(&self.0).finish()
    }
}

impl NesInputCell {
    pub fn new() -> Self {
        Self(Arc::new(InputCell::new()))
    }

    /// 既存の `Arc<InputCell<3>>` からラップする。
    pub fn from_arc(cell: Arc<InputCell<3>>) -> Self {
        Self(cell)
    }

    /// GUI 側からボタン状態を書き込む (`[p1, p2, mic]`)。
    pub fn store(&self, p1: u8, p2: u8, mic: bool) {
        self.0.store(&[p1, p2, mic as u8]);
    }

    /// Device 側へ共有する `Arc<InputCell<3>>` を取得する。
    pub fn share(&self) -> Arc<InputCell<3>> {
        self.0.clone()
    }
}

impl Default for NesInputCell {
    fn default() -> Self {
        Self::new()
    }
}
