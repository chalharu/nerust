use crate::descriptor::SystemSessionProfile;
use crate::load::NesLoadOptions;
use crate::session::NesSession;
use nerust_console::ConsoleMetrics;
use nerust_console::video::ConsoleVideo;
use nerust_gui_session::commands::{SessionCommand, SessionCommandOutcome};
use nerust_gui_session::core::WindowSize;
use nerust_persistence::model::StateSlotSummary;
use std::path::PathBuf;

impl NesSession {
    pub fn video(&self) -> &ConsoleVideo {
        self.system.session.video()
    }

    pub fn with_frame_buffer<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        self.system.session.with_frame_buffer(f)
    }

    pub fn window_size(&self) -> WindowSize {
        self.system.session.window_size()
    }

    pub fn metrics(&self) -> ConsoleMetrics {
        self.system.session.metrics()
    }

    pub fn window_title(&self) -> String {
        self.system.session.window_title()
    }

    pub fn paused(&self) -> bool {
        self.system.session.paused()
    }

    pub fn loaded(&self) -> bool {
        self.system.session.loaded()
    }

    pub fn can_pause(&self) -> bool {
        self.system.session.can_pause()
    }

    pub fn can_resume(&self) -> bool {
        self.system.session.can_resume()
    }

    pub fn slots(&self) -> &[StateSlotSummary] {
        self.system.session.slots()
    }

    pub fn active_slot_id(&self) -> Option<u64> {
        self.system.session.active_slot_id()
    }

    pub fn resume(&mut self) {
        self.system.session.resume();
    }

    pub fn load(&mut self, rom_path: Option<PathBuf>, data: Vec<u8>) -> bool {
        self.load_with_options(rom_path, data, NesLoadOptions::default())
    }

    pub fn load_with_options(
        &mut self,
        rom_path: Option<PathBuf>,
        data: Vec<u8>,
        explicit_options: NesLoadOptions,
    ) -> bool {
        let core_options = self
            .system
            .profile
            .effective_load_options(&self.system.settings_snapshot, explicit_options);
        let loaded = self
            .system
            .session
            .load_with_options(None, data.clone(), core_options);
        if !loaded {
            return false;
        }

        self.system.loaded_rom = Some(super::LoadedRom {
            path: rom_path.clone(),
            data: data.clone(),
            explicit_options,
        });
        self.sync_input_from_session();

        let persistence_paths = match self.system.session.persistence_target() {
            Ok(target) => match self.system.settings.resolve_persistence_paths_with_import(
                self.system.profile.system_id(),
                rom_path.as_deref(),
                target.rom_identity,
            ) {
                Ok(paths) => Some(paths),
                Err(error) => {
                    log::warn!("failed to resolve persistence paths: {error}");
                    None
                }
            },
            Err(error) => {
                log::warn!("failed to read persistence target: {error}");
                None
            }
        };
        self.system
            .session
            .configure_persistence_paths(persistence_paths);

        if let Some(path) = rom_path.as_deref()
            && let Err(error) = self
                .system
                .settings
                .update_last_successful_rom_directory(path)
        {
            log::warn!("failed to update app state: {error}");
        }
        if let Ok(snapshot) = self.system.settings.snapshot() {
            self.system.settings_snapshot = snapshot;
        }
        true
    }

    pub fn unload(&mut self) -> bool {
        let unloaded = self.system.session.unload();
        if unloaded {
            self.system.loaded_rom = None;
            self.sync_input_from_session();
        }
        unloaded
    }

    pub fn flush_before_exit(&mut self) {
        self.system.session.flush_before_exit();
    }

    pub fn run_command(&mut self, command: SessionCommand) -> SessionCommandOutcome {
        let outcome = self.system.session.run_command(command);
        if outcome.executed
            && matches!(
                command,
                SessionCommand::LoadActiveSlot | SessionCommand::LoadSlot(_)
            )
        {
            self.sync_input_from_session();
        }
        outcome
    }

    pub(super) fn rebuild_for_settings(
        &mut self,
        next_settings: &nerust_gui_runtime::settings::SettingsSnapshot,
    ) -> Result<(), String> {
        let was_loaded = self.loaded();
        let was_paused = self.paused();
        let exported_state = if was_loaded {
            Some(
                self.system
                    .session
                    .export_state()
                    .map_err(|error| format!("state export failed: {error}"))?,
            )
        } else {
            None
        };

        let mut rebuilt = self.system.profile.build_gui_session(next_settings);
        if let Some(loaded_rom) = self.system.loaded_rom.clone() {
            let effective_options = self.system.profile.effective_rebuild_load_options(
                &self.system.settings_snapshot,
                next_settings,
                loaded_rom.explicit_options,
            );
            if !rebuilt.load_with_options(None, loaded_rom.data.clone(), effective_options) {
                return Err("ROM reload failed during session rebuild".into());
            }
            let target = rebuilt
                .persistence_target()
                .map_err(|error| format!("persistence target failed: {error}"))?;
            let resolved = self
                .system
                .settings
                .resolve_persistence_paths_with_import(
                    self.system.profile.system_id(),
                    loaded_rom.path.as_deref(),
                    target.rom_identity,
                )
                .map_err(|error| format!("persistence path resolution failed: {error}"))?;
            rebuilt.configure_persistence_paths(Some(resolved));
            if let Some(exported_state) = exported_state {
                rebuilt
                    .import_state(exported_state.machine_state)
                    .map_err(|error| format!("state import failed: {error}"))?;
                if !was_paused {
                    rebuilt.resume();
                }
            }
        }

        self.system.session = rebuilt;
        self.sync_input_from_session();
        if was_loaded && was_paused {
            self.system.session.pause();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::descriptor::{NesConsoleProfile, SystemSessionProfile};
    use crate::load::{NesLoadOptions, NesMmc3IrqVariant};
    use nerust_contract_options::Mmc3IrqVariant;
    use nerust_contract_settings::app_state::DesktopAppState;
    use nerust_contract_settings::local::HostBackendLocalSettings;
    use nerust_contract_settings::nes::{
        NesCoreSettings, NesSettings, NesVideoFilter, NesVideoSettings,
    };
    use nerust_contract_settings::shared::{DesktopSharedSettings, SystemSettings};
    use nerust_gui_runtime::settings::SettingsSnapshot;
    use nerust_input_schema::SystemId;
    use std::collections::BTreeMap;

    fn snapshot(
        mmc3_irq_variant: Option<Mmc3IrqVariant>,
        filter: NesVideoFilter,
    ) -> SettingsSnapshot {
        SettingsSnapshot {
            shared: DesktopSharedSettings {
                systems: BTreeMap::from([(
                    SystemId::Nes,
                    SystemSettings::Nes(NesSettings {
                        video: NesVideoSettings { filter },
                        core: NesCoreSettings { mmc3_irq_variant },
                    }),
                )]),
                ..DesktopSharedSettings::default()
            },
            local: HostBackendLocalSettings::default(),
            app_state: DesktopAppState::default(),
        }
    }

    #[test]
    fn rebuild_load_options_preserve_current_mmc3_variant() {
        let current = snapshot(None, NesVideoFilter::NtscComposite);
        let next = snapshot(Some(Mmc3IrqVariant::Sharp), NesVideoFilter::NtscRgb);

        let rebuilt = NesConsoleProfile.effective_rebuild_load_options(
            &current,
            &next,
            NesLoadOptions {
                mmc3_irq_variant: Some(NesMmc3IrqVariant::Nec),
            },
        );

        assert_eq!(rebuilt.mmc3_irq_variant, Some(Mmc3IrqVariant::Nec));

        let rebuilt = NesConsoleProfile.effective_rebuild_load_options(
            &current,
            &next,
            NesLoadOptions::default(),
        );
        assert_eq!(rebuilt.mmc3_irq_variant, None);
    }
}
