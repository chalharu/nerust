use std::path::Path;

use crate::{
    load::{MediaObject, ResolvedLoadRequest},
    session::{
        SessionError, SessionHandle,
        commands::{SessionCommand, SessionCommandOutcome},
        title::window_title,
    },
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowSize {
    pub width: f32,
    pub height: f32,
}

impl SessionHandle {
    pub fn metrics(&self) -> crate::session::metrics::ConsoleMetrics {
        self.emu_core.metrics()
    }

    pub fn window_size(&self) -> WindowSize {
        let profile = self.emu_core.render_profile();
        WindowSize {
            width: profile.physical_size.width,
            height: profile.physical_size.height,
        }
    }

    pub fn window_title(&self) -> String {
        let metrics = self.metrics();
        window_title(metrics.paused, metrics)
    }

    pub fn loaded(&self) -> bool {
        self.metrics().loaded
    }

    pub fn paused(&self) -> bool {
        self.metrics().paused
    }

    pub fn can_pause(&self) -> bool {
        let metrics = self.metrics();
        metrics.loaded && !metrics.paused
    }

    pub fn can_resume(&self) -> bool {
        let metrics = self.metrics();
        metrics.loaded && metrics.paused
    }

    pub fn apply_settings(
        &mut self,
        next_settings: nerust_gui_runtime::settings::SettingsSnapshot,
    ) -> Result<nerust_gui_runtime::settings::SettingsApplyPlan, SessionError> {
        let previous = self.settings_snapshot.clone();
        let plan = nerust_gui_runtime::settings::apply::derive_apply_plan(
            self.host_backend,
            &previous,
            &next_settings,
        );

        if plan.session_rebuild_required {
            self.rebuild_for_settings(&next_settings)?;
        } else if plan.audio_volume_changed {
            let volume =
                f32::from(next_settings.local.audio.master_volume_percent.min(100)) / 100.0;
            let volume = if next_settings.local.audio.muted {
                0.0
            } else {
                volume
            };
            let _ = self.emu_core.set_volume(volume);
        }

        if let Err(error) = self.settings.save_snapshot(next_settings.clone()) {
            if plan.session_rebuild_required {
                let _ = self.rebuild_for_settings(&previous);
            }
            return Err(SessionError::Settings(error));
        }

        self.settings_snapshot = next_settings;
        self.pressed_keys.clear();
        self.clear_input();
        Ok(plan)
    }

    pub fn set_fullscreen_default(
        &mut self,
        fullscreen: bool,
    ) -> Result<nerust_gui_runtime::settings::SettingsApplyPlan, SessionError> {
        if self.settings_snapshot.local.video.window.fullscreen_default == fullscreen {
            return Ok(nerust_gui_runtime::settings::SettingsApplyPlan::default());
        }

        let mut next_settings = self.settings_snapshot.clone();
        next_settings.local.video.window.fullscreen_default = fullscreen;
        let plan = nerust_gui_runtime::settings::apply::derive_apply_plan(
            self.host_backend,
            &self.settings_snapshot,
            &next_settings,
        );

        if let Err(error) = self.settings.save_snapshot(next_settings.clone()) {
            return Err(SessionError::Settings(error));
        }

        self.settings_snapshot = next_settings;
        Ok(plan)
    }

    pub fn load_resolved(
        &mut self,
        media: MediaObject,
        resolved: ResolvedLoadRequest,
    ) -> Result<(), SessionError> {
        self.persistence.flush_mapper_save(&self.emu_core)?;
        self.emu_core.load(&media, resolved.core_options_bytes)?;
        self.loaded_media = Some(super::LoadedMedia {
            media: media.clone(),
        });

        let sidecars = self
            .emu_core
            .canonical_media_identity()
            .and_then(|identity| {
                log::info!(
                    "load_resolved: identity={:?} path={:?}",
                    identity,
                    media.path.as_deref()
                );
                self.resolve_persistence_paths(media.path.as_deref(), &identity)
                    .inspect(|_| {
                        log::info!("load_resolved: resolved persistence paths");
                    })
            });
        if let Some(sidecars) = sidecars {
            self.persistence
                .configure(sidecars.states_dir, sidecars.mapper_save_path);
            self.persistence.refresh_slots(&self.emu_core);
            let _ = self.persistence.load_mapper_save_if_needed(&self.emu_core);
        } else {
            log::info!("load_resolved: failed to resolve persistence paths");
        }

        self.remember_last_successful_rom_directory(media.path.as_deref());
        Ok(())
    }

    pub fn unload(&mut self) -> Result<(), SessionError> {
        self.persistence.flush_mapper_save(&self.emu_core)?;
        self.emu_core.unload()?;
        self.loaded_media = None;
        self.persistence.reset();
        Ok(())
    }

    pub fn flush_before_exit(&mut self) {
        if let Err(error) = self.persistence.flush_mapper_save(&self.emu_core) {
            log::warn!("mapper save flush before close failed: {error}");
        }
    }

    pub fn run_command(
        &mut self,
        command: SessionCommand,
    ) -> Result<SessionCommandOutcome, SessionError> {
        match command {
            SessionCommand::Pause => {
                if self.paused() {
                    Ok(SessionCommandOutcome::default())
                } else {
                    self.emu_core.pause()?;
                    Ok(SessionCommandOutcome {
                        executed: true,
                        needs_redraw: false,
                    })
                }
            }
            SessionCommand::Resume => {
                if self.paused() {
                    self.emu_core.resume()?;
                    Ok(SessionCommandOutcome {
                        executed: true,
                        needs_redraw: self.loaded(),
                    })
                } else {
                    Ok(SessionCommandOutcome::default())
                }
            }
            SessionCommand::TogglePause => {
                if self.paused() {
                    self.run_command(SessionCommand::Resume)
                } else {
                    self.run_command(SessionCommand::Pause)
                }
            }
            SessionCommand::Reset => {
                self.emu_core.reset()?;
                Ok(SessionCommandOutcome {
                    executed: true,
                    needs_redraw: false,
                })
            }
            SessionCommand::CreateSlot => {
                self.persistence.create_slot(&self.emu_core);
                Ok(SessionCommandOutcome {
                    executed: true,
                    needs_redraw: false,
                })
            }
            SessionCommand::SaveActiveSlotOrNew => {
                self.persistence.save_active_slot_or_new(&self.emu_core);
                Ok(SessionCommandOutcome {
                    executed: true,
                    needs_redraw: false,
                })
            }
            SessionCommand::LoadActiveSlot => {
                let was_paused = self.paused();
                let executed = self.persistence.load_active_slot(&self.emu_core);
                Ok(SessionCommandOutcome {
                    executed,
                    needs_redraw: executed && was_paused && !self.paused(),
                })
            }
            SessionCommand::SelectActiveSlot(slot_id) => {
                self.persistence.select_active_slot(slot_id);
                Ok(SessionCommandOutcome {
                    executed: true,
                    needs_redraw: false,
                })
            }
            SessionCommand::SaveSlot(slot_id) => {
                self.persistence.save_slot(slot_id, &self.emu_core, false);
                Ok(SessionCommandOutcome {
                    executed: true,
                    needs_redraw: false,
                })
            }
            SessionCommand::LoadSlot(slot_id) => {
                let was_paused = self.paused();
                let executed = self.persistence.load_slot(slot_id, &self.emu_core);
                Ok(SessionCommandOutcome {
                    executed,
                    needs_redraw: executed && was_paused && !self.paused(),
                })
            }
            SessionCommand::DeleteSlot(slot_id) => {
                self.persistence.delete_slot(slot_id, &self.emu_core);
                Ok(SessionCommandOutcome {
                    executed: true,
                    needs_redraw: false,
                })
            }
            SessionCommand::SelectNextSlot => Ok(SessionCommandOutcome {
                executed: self.persistence.select_adjacent_slot(true).is_some(),
                needs_redraw: false,
            }),
            SessionCommand::SelectPreviousSlot => Ok(SessionCommandOutcome {
                executed: self.persistence.select_adjacent_slot(false).is_some(),
                needs_redraw: false,
            }),
        }
    }

    pub fn slots(&self) -> &[nerust_persistence::model::StateSlotSummary] {
        self.persistence.slots()
    }

    pub fn active_slot_id(&self) -> Option<u64> {
        self.persistence.active_slot_id()
    }

    pub fn save_hidden_lifecycle_state(&mut self) -> bool {
        self.persistence.save_hidden(&self.emu_core)
    }

    pub fn load_hidden_lifecycle_state(&mut self) -> bool {
        self.persistence.load_hidden(&self.emu_core)
    }

    pub fn clear_hidden_lifecycle_state(&mut self) {
        self.persistence.clear_hidden();
    }

    fn rebuild_for_settings(
        &mut self,
        next_settings: &nerust_gui_runtime::settings::SettingsSnapshot,
    ) -> Result<(), SessionError> {
        let was_loaded = self.loaded();
        let was_paused = self.paused();
        let exported_core_bytes = if was_loaded {
            Some(self.emu_core.save_state_raw()?)
        } else {
            None
        };
        let restored_runtime_state = exported_core_bytes.is_some();

        let (rebuilt_core, rebuilt_adapter) =
            self.factory.create_core_and_adapter(next_settings)?;

        if let Some(loaded_media) = self.loaded_media.clone() {
            rebuilt_core.load(&loaded_media.media, Vec::new())?;
            if let Some(core_bytes) = exported_core_bytes.as_ref() {
                rebuilt_core.load_state_raw(core_bytes.clone())?;
                if !was_paused {
                    rebuilt_core.resume()?;
                }
            }
        }

        self.emu_core = rebuilt_core;
        self.input_adapter = rebuilt_adapter;
        if was_loaded {
            let rom_path = self
                .loaded_media
                .as_ref()
                .and_then(|m| m.media.path.as_deref());
            let sidecars = self
                .emu_core
                .canonical_media_identity()
                .and_then(|identity| {
                    log::info!(
                        "rebuild_for_settings: identity={:?} path={:?}",
                        identity,
                        rom_path
                    );
                    self.resolve_persistence_paths(rom_path, &identity)
                });
            if let Some(sidecars) = sidecars {
                self.persistence
                    .configure(sidecars.states_dir, sidecars.mapper_save_path);
                self.persistence.refresh_slots(&self.emu_core);
            }
            if !restored_runtime_state {
                let _ = self.persistence.load_mapper_save_if_needed(&self.emu_core);
            }
            if was_paused {
                self.emu_core.pause()?;
            }
        }
        Ok(())
    }

    fn resolve_persistence_paths(
        &self,
        rom_path: Option<&Path>,
        identity: &nerust_contract_core::identity::SystemIdentity,
    ) -> Option<nerust_persistence::sidecar::SidecarPaths> {
        self.settings
            .resolve_persistence_paths_with_import(identity.system_id, rom_path, identity)
            .map_err(|error| {
                log::warn!("failed to resolve persistence paths: {error}");
                error
            })
            .ok()
    }

    fn remember_last_successful_rom_directory(&mut self, path: Option<&Path>) {
        if let Some(path) = path
            && let Err(error) = self.settings.update_last_successful_rom_directory(path)
        {
            log::warn!("failed to update app state: {error}");
        }
        if let Ok(snapshot) = self.settings.snapshot() {
            self.settings_snapshot = snapshot;
        }
    }
}
