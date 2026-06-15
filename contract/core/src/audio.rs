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
    /// Tier 1 → Tier 2 → Tier 3 の順に初期化を試行し、
    /// 最初に利用可能だったバックエンドを返す。
    ///
    /// 各 Tier は以下の優先順位で確認される:
    ///   1. CPAL  (クロスプラットフォーム)
    ///   2. OpenAL (デスクトップフォールバック)
    ///   3. Null   (常に利用可能)
    pub fn autoselect() -> Self {
        // Tier 1: CPAL
        #[cfg(feature = "cpal")]
        {
            use cpal::traits::HostTrait;
            let host = cpal::default_host();
            if host.default_output_device().is_some() {
                log::info!("autoselect: selected CPAL audio backend (Tier 1)");
                return AudioBackendKind::Cpal;
            }
        }

        // Tier 2: OpenAL
        #[cfg(feature = "openal")]
        {
            if alto::Alto::load_default().is_ok() {
                log::info!("autoselect: selected OpenAL audio backend (Tier 2)");
                return AudioBackendKind::OpenAl;
            }
        }

        // Tier 3: Null (常に利用可能)
        log::info!("autoselect: no audio device found, using Null backend (Tier 3)");
        AudioBackendKind::Null
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
