use nerust_core_traits::audio::AudioBackend;
use nerust_input_traits::SystemInputAdapter;
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_core_traits::SystemId;
use thiserror::Error;

use crate::{
    descriptor::{
        SystemDescriptor, SystemSettingsChoiceId, SystemSettingsFieldId, SystemSettingsPageModel,
    },
    emu_core::EmuCore,
    load::{MediaObject, ResolvedLoadRequest, SystemLoadOptions},
};

#[derive(Debug, Error)]
pub enum FactoryError {
    #[error("core creation failed: {0}")]
    Create(String),
    #[error("invalid settings choice: {0}")]
    InvalidChoice(String),
    #[error("load request resolution failed: {0}")]
    Resolve(String),
}

/// システム（NES/SNES）の全知識をカプセル化する factory。
///
/// frontend はこの trait を通じてのみシステムと対話する。
/// 各システムの実装は `gui/factory/{nes,snes}/` クレートで行う。
pub trait CoreFactory {
    /// この factory が扱うシステムの識別子を返す。
    fn system_id(&self) -> SystemId;

    /// この factory が扱うシステムの UI 表示名を返す。
    fn display_name(&self) -> &'static str;

    /// コアと入力アダプタを構築する（rebuild 時にも使用）。
    ///
    /// `speaker` は呼び出し元（SessionHandle）で構築された音声出力。
    fn create_core_and_adapter(
        &self,
        settings: &SettingsSnapshot,
        speaker: Box<dyn AudioBackend>,
    ) -> Result<(EmuCore, Box<dyn SystemInputAdapter>), FactoryError>;

    /// この factory が処理可能なメディアかを判定する。
    fn probe_media(&self, media: &MediaObject) -> bool;

    /// この factory が扱うシステムの descriptor を返す。
    fn system_descriptor(&self) -> SystemDescriptor;

    /// システム設定のページモデルを返す。
    fn settings_page(&self, settings: &SettingsSnapshot) -> SystemSettingsPageModel;

    /// 設定の選択を適用する。
    fn apply_settings_choice(
        &self,
        settings: &mut SettingsSnapshot,
        field: &SystemSettingsFieldId,
        choice: &SystemSettingsChoiceId,
    ) -> Result<(), FactoryError>;

    /// load request を解決する。
    fn resolve_load_request(
        &self,
        settings: &SettingsSnapshot,
        options: SystemLoadOptions,
    ) -> Result<ResolvedLoadRequest, FactoryError>;

    /// デフォルトの load options を返す。
    fn default_load_options(&self) -> SystemLoadOptions;
}
