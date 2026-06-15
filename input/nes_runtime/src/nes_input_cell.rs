use std::sync::Arc;

use nerust_contract_core::input::InputCell;

/// NES 固有の入力セルラッパー。
///
/// `[u8; 2]` の各要素の意味を (P1 ボタン + トリム, P2 ボタン) と明示する。
/// P2 バイトの bit 2 (0x04) はファミコン P2 マイク状態として扱う。
/// 標準 NES ではこのビットは P2 Select だが、ファミコン P2 に Select は存在しないため、
/// マイクが必要ない場合は常に 0 で問題ない。
const P2_MIC_BIT: u8 = 0x04;
pub struct NesInputCell(Arc<InputCell<2>>);

impl std::fmt::Debug for NesInputCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("NesInputCell").field(&self.0).finish()
    }
}

impl NesInputCell {
    pub fn new() -> Self {
        Self(Arc::new(InputCell::new()))
    }

    /// 既存の `Arc<InputCell<2>>` からラップする。
    pub fn from_arc(cell: Arc<InputCell<2>>) -> Self {
        Self(cell)
    }

    /// GUI 側からボタン状態を書き込む。
    /// マイクは P2 バイトの bit 2 にエンコードされる。
    pub fn store(&self, p1: u8, p2: u8, mic: bool) {
        let p2_enc = p2 | if mic { P2_MIC_BIT } else { 0 };
        self.0.store(&[p1, p2_enc]);
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
