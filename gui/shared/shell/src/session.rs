pub mod input;
mod lifecycle;
#[cfg(test)]
mod tests;

use crate::descriptor::NesConsoleProfile;
use crate::descriptor::SystemSessionProfile;
use crate::settings::defaults::seed::{
    default_app_state, default_local_settings, default_shared_settings,
};
use nerust_contract_settings::input::{KeyboardKey, ShortcutAction};
use nerust_gui_runtime::session::GuiSession;
use nerust_gui_runtime::settings::{
    HostBackendIdentity, SettingsApplyPlan, SettingsManager, SettingsSnapshot, derive_apply_plan,
};
use nerust_input_nes::input::NesInputState;
use nerust_input_schema::InputTopologyDescriptor;
use std::collections::BTreeSet;
use std::path::PathBuf;

#[derive(Debug)]
pub(crate) struct SystemSession<P: SystemSessionProfile> {
    pub(super) profile: P,
    pub(super) host_backend: HostBackendIdentity,
    pub(super) session: GuiSession,
    pub(super) settings: SettingsManager,
    pub(super) settings_snapshot: SettingsSnapshot,
    pub(super) loaded_rom: Option<LoadedRom<P::LoadOptions>>,
}

#[derive(Debug, Clone)]
pub(super) struct LoadedRom<T> {
    pub(super) path: Option<PathBuf>,
    pub(super) data: Vec<u8>,
    pub(super) explicit_options: T,
}

#[derive(Debug)]
pub struct NesSession {
    pub(super) system: SystemSession<NesConsoleProfile>,
    pub(super) input: NesInputState,
    pub(super) pressed_keys: BTreeSet<KeyboardKey>,
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

impl<P: SystemSessionProfile> SystemSession<P> {
    pub fn from_gui_session(profile: P, session: GuiSession) -> Self {
        let host_backend = HostBackendIdentity::gtk_opengl();
        let settings = SettingsManager::ephemeral(
            default_shared_settings(),
            default_local_settings(),
            default_app_state(),
        );
        let settings_snapshot = settings.snapshot().expect("ephemeral settings should read");
        Self {
            profile,
            host_backend,
            session,
            settings,
            settings_snapshot,
            loaded_rom: None,
        }
    }

    pub fn new_for_host(profile: P, identity: HostBackendIdentity) -> Self {
        let settings = crate::settings::defaults::manager::load_settings_manager(identity);
        let settings_snapshot = crate::settings::defaults::manager::current_or_default(&settings);
        Self {
            profile,
            host_backend: identity,
            session: profile.build_gui_session(&settings_snapshot),
            settings,
            settings_snapshot,
            loaded_rom: None,
        }
    }
}

impl NesSession {
    pub fn from_gui_session(session: GuiSession) -> Self {
        let mut result = Self {
            system: SystemSession::from_gui_session(NesConsoleProfile, session),
            input: NesInputState::new(),
            pressed_keys: BTreeSet::new(),
        };
        result.sync_input_from_session();
        result
    }

    pub fn new_for_host(identity: HostBackendIdentity) -> Self {
        let mut result = Self {
            system: SystemSession::new_for_host(NesConsoleProfile, identity),
            input: NesInputState::new(),
            pressed_keys: BTreeSet::new(),
        };
        result.sync_input_from_session();
        result
    }

    pub fn settings_snapshot(&self) -> &SettingsSnapshot {
        &self.system.settings_snapshot
    }

    pub fn settings_manager(&self) -> &SettingsManager {
        &self.system.settings
    }

    pub fn input_topology_descriptor(&self) -> InputTopologyDescriptor {
        self.system.profile.input_topology_descriptor()
    }

    pub fn apply_settings(
        &mut self,
        next_settings: SettingsSnapshot,
    ) -> Result<SettingsApplyPlan, String> {
        let previous = self.system.settings_snapshot.clone();
        let plan = derive_apply_plan(self.system.host_backend, &previous, &next_settings);

        if plan.session_rebuild_required {
            self.rebuild_for_settings(&next_settings)
                .map_err(|error| format!("failed to apply settings: {error}"))?;
        }

        if let Err(error) = self.system.settings.save_snapshot(next_settings.clone()) {
            if plan.session_rebuild_required {
                let _ = self.rebuild_for_settings(&previous);
            }
            return Err(format!("failed to save settings: {error}"));
        }

        self.system.settings_snapshot = next_settings;
        self.pressed_keys.clear();
        self.clear_controller_input();
        Ok(plan)
    }
}
