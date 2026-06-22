use crate::descriptor::{
    SystemDescriptor, SystemSettingsChoiceId, SystemSettingsFieldId, SystemSettingsPageModel,
};
use crate::emu_core::EmuCore;
use crate::load::{MediaObject, SystemLoadOptions};
use nerust_contract_core::input::SystemInputAdapter;
use nerust_contract_input::SystemId;
use nerust_gui_runtime::settings::SettingsSnapshot;
use thiserror::Error;

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
    fn create_core_and_adapter(
        &self,
        settings: &SettingsSnapshot,
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

    /// load request を解決・検証する。
    fn resolve_load_request(
        &self,
        settings: &SettingsSnapshot,
        options: SystemLoadOptions,
    ) -> Result<(), FactoryError>;

    /// デフォルトの load options を返す。
    fn default_load_options(&self) -> SystemLoadOptions;
}
