pub mod input;
mod lifecycle;
#[cfg(test)]
mod tests;

use crate::descriptor::NesConsoleProfile;
use crate::load::NesLoadOptions;
use crate::settings::defaults::seed::{
    default_app_state, default_local_settings, default_shared_settings,
};
use nerust_contract_settings::input::{KeyboardKey, ShortcutAction};
use nerust_gui_runtime::session::GuiSession;
use nerust_gui_runtime::settings::{
    HostBackendIdentity, SettingsApplyPlan, SettingsManager, SettingsSnapshot, derive_apply_plan,
};
use nerust_input_nes::input::NesInputState;
use std::collections::BTreeSet;
use std::path::PathBuf;

#[derive(Debug)]
pub struct NesSession {
    pub(super) session: GuiSession,
    pub(super) input: NesInputState,
    pub(super) settings: SettingsManager,
    pub(super) settings_snapshot: SettingsSnapshot,
    pub(super) pressed_keys: BTreeSet<KeyboardKey>,
    pub(super) loaded_rom: Option<LoadedRom>,
}

#[derive(Debug, Clone)]
pub(super) struct LoadedRom {
    pub(super) path: Option<PathBuf>,
    pub(super) data: Vec<u8>,
    pub(super) explicit_options: NesLoadOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardShortcut {
    Session(ShortcutAction),
    ToggleFullscreen,
}

impl Default for NesSession {
    fn default() -> Self {
        Self::new_for_host(HostBackendIdentity::gtk_opengl())
    }
}

impl NesSession {
    pub fn from_gui_session(session: GuiSession) -> Self {
        let settings = SettingsManager::ephemeral(
            default_shared_settings(),
            default_local_settings(),
            default_app_state(),
        );
        let settings_snapshot = settings.snapshot().expect("ephemeral settings should read");
        let mut result = Self {
            session,
            input: NesInputState::new(),
            settings,
            settings_snapshot,
            pressed_keys: BTreeSet::new(),
            loaded_rom: None,
        };
        result.sync_input_from_session();
        result
    }

    pub fn new_for_host(identity: HostBackendIdentity) -> Self {
        let settings = crate::settings::defaults::manager::load_settings_manager(identity);
        let settings_snapshot = crate::settings::defaults::manager::current_or_default(&settings);
        let mut result = Self {
            session: NesConsoleProfile.build_gui_session(&settings_snapshot),
            input: NesInputState::new(),
            settings,
            settings_snapshot,
            pressed_keys: BTreeSet::new(),
            loaded_rom: None,
        };
        result.sync_input_from_session();
        result
    }

    pub fn settings_snapshot(&self) -> &SettingsSnapshot {
        &self.settings_snapshot
    }

    pub fn settings_manager(&self) -> &SettingsManager {
        &self.settings
    }

    pub fn apply_settings(
        &mut self,
        next_settings: SettingsSnapshot,
    ) -> Result<SettingsApplyPlan, String> {
        let previous = self.settings_snapshot.clone();
        let plan = derive_apply_plan(&previous, &next_settings);

        if plan.session_rebuild_required {
            self.rebuild_for_settings(&next_settings)
                .map_err(|error| format!("failed to apply settings: {error}"))?;
        }

        if let Err(error) = self.settings.save_snapshot(next_settings.clone()) {
            if plan.session_rebuild_required {
                let _ = self.rebuild_for_settings(&previous);
            }
            return Err(format!("failed to save settings: {error}"));
        }

        self.settings_snapshot = next_settings;
        self.pressed_keys.clear();
        self.clear_controller_input();
        Ok(plan)
    }
}
