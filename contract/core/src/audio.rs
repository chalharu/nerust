pub trait AudioBackend: Send {
    fn start(&mut self);
    fn pause(&mut self);
    fn sample_rate(&self) -> u32 {
        48_000
    }
    fn push(&mut self, sample: f32);
}

/// 音声バックエンドの種類
///
/// OS/環境に応じて `autoselect()` で最適なバックエンドを選択する。
/// 実際のインスタンス生成は各バックエンド crate のコンストラクタで行う。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioBackendKind {
    /// CPAL (クロスプラットフォーム、Tier 1)
    Cpal,
    /// OpenAL (デスクトップ、Tier 2)
    OpenAl,
    /// 無音出力 (Tier 3)
    Null,
}

impl AudioBackendKind {
    /// 環境に応じて最適なバックエンドを自動選択する
    pub fn autoselect() -> Self {
        #[cfg(target_os = "android")]
        {
            AudioBackendKind::Cpal
        }
        #[cfg(not(target_os = "android"))]
        {
            AudioBackendKind::OpenAl
        }
    }
}

/// 無音出力バックエンド (Tier 3)
///
/// 常に利用可能で、テスト・CI・ヘッドレス動作に使用する。
pub struct NullAudio;

impl AudioBackend for NullAudio {
    fn start(&mut self) {}
    fn pause(&mut self) {}
    fn push(&mut self, _sample: f32) {}
}
