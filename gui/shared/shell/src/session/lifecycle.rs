use crate::descriptor::NesConsoleProfile;
use crate::load::NesLoadOptions;
use crate::session::NesSession;
use crate::settings::nes::effective_load_options;
use nerust_console::ConsoleMetrics;
use nerust_console::video::ConsoleVideo;
use nerust_gui_session::commands::{SessionCommand, SessionCommandOutcome};
use nerust_gui_session::core::WindowSize;
use nerust_persistence::model::StateSlotSummary;
use std::path::PathBuf;

impl NesSession {
    pub fn video(&self) -> &ConsoleVideo {
        self.session.video()
    }

    pub fn with_frame_buffer<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        self.session.with_frame_buffer(f)
    }

    pub fn window_size(&self) -> WindowSize {
        self.session.window_size()
    }

    pub fn metrics(&self) -> ConsoleMetrics {
        self.session.metrics()
    }

    pub fn window_title(&self) -> String {
        self.session.window_title()
    }

    pub fn paused(&self) -> bool {
        self.session.paused()
    }

    pub fn loaded(&self) -> bool {
        self.session.loaded()
    }

    pub fn can_pause(&self) -> bool {
        self.session.can_pause()
    }

    pub fn can_resume(&self) -> bool {
        self.session.can_resume()
    }

    pub fn slots(&self) -> &[StateSlotSummary] {
        self.session.slots()
    }

    pub fn active_slot_id(&self) -> Option<u64> {
        self.session.active_slot_id()
    }

    pub fn resume(&mut self) {
        self.session.resume();
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
        let core_options = effective_load_options(&self.settings_snapshot.shared, explicit_options)
            .into_core_options();
        let loaded = self
            .session
            .load_with_options(None, data.clone(), core_options);
        if !loaded {
            return false;
        }

        self.loaded_rom = Some(super::LoadedRom {
            path: rom_path.clone(),
            data: data.clone(),
            explicit_options,
        });
        self.sync_input_from_session();

        let persistence_paths = match self.session.persistence_target() {
            Ok(target) => match self.settings.resolve_persistence_paths_with_import(
                nerust_input_schema::SystemId::Nes,
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
        self.session.configure_persistence_paths(persistence_paths);

        if let Some(path) = rom_path.as_deref()
            && let Err(error) = self.settings.update_last_successful_rom_directory(path)
        {
            log::warn!("failed to update app state: {error}");
        }
        if let Ok(snapshot) = self.settings.snapshot() {
            self.settings_snapshot = snapshot;
        }
        true
    }

    pub fn unload(&mut self) -> bool {
        let unloaded = self.session.unload();
        if unloaded {
            self.loaded_rom = None;
            self.sync_input_from_session();
        }
        unloaded
    }

    pub fn flush_before_exit(&mut self) {
        self.session.flush_before_exit();
    }

    pub fn run_command(&mut self, command: SessionCommand) -> SessionCommandOutcome {
        let outcome = self.session.run_command(command);
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
                self.session
                    .export_state()
                    .map_err(|error| format!("state export failed: {error}"))?,
            )
        } else {
            None
        };

        let mut rebuilt = NesConsoleProfile.build_gui_session(next_settings);
        if let Some(loaded_rom) = self.loaded_rom.clone() {
            let effective_options = effective_rebuild_load_options(
                &self.settings_snapshot,
                next_settings,
                loaded_rom.explicit_options,
            )
            .into_core_options();
            if !rebuilt.load_with_options(None, loaded_rom.data.clone(), effective_options) {
                return Err("ROM reload failed during session rebuild".into());
            }
            let target = rebuilt
                .persistence_target()
                .map_err(|error| format!("persistence target failed: {error}"))?;
            let resolved = self
                .settings
                .resolve_persistence_paths_with_import(
                    nerust_input_schema::SystemId::Nes,
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

        self.session = rebuilt;
        self.sync_input_from_session();
        if was_loaded && was_paused {
            self.session.pause();
        }
        Ok(())
    }
}

fn effective_rebuild_load_options(
    current_settings: &nerust_gui_runtime::settings::SettingsSnapshot,
    _next_settings: &nerust_gui_runtime::settings::SettingsSnapshot,
    explicit_options: NesLoadOptions,
) -> NesLoadOptions {
    // Rebuilds exist to refresh immediate host/runtime changes while preserving the
    // load-time core behavior of the currently running ROM. Deferred core settings
    // apply on the next explicit ROM load instead.
    effective_load_options(&current_settings.shared, explicit_options)
}

#[cfg(test)]
mod tests {
    use super::effective_rebuild_load_options;
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

        let rebuilt = effective_rebuild_load_options(
            &current,
            &next,
            NesLoadOptions {
                mmc3_irq_variant: Some(NesMmc3IrqVariant::Nec),
            },
        );

        assert_eq!(rebuilt.mmc3_irq_variant, Some(NesMmc3IrqVariant::Nec));

        let rebuilt = effective_rebuild_load_options(&current, &next, NesLoadOptions::default());
        assert_eq!(rebuilt.mmc3_irq_variant, None);
    }
}
