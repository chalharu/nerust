use crate::descriptor::{RuntimeHostServices, SystemSettingsPageModel};
use crate::load::{LoadRequest, MediaObject, ResolvedLoadRequest};
use crate::session::SessionHandle;
use nerust_console::state::PreviewFrame;
use nerust_gui_session::commands::{SessionCommand, SessionCommandOutcome};
use nerust_gui_session::core::WindowSize;
use nerust_gui_session::title::window_title;
use nerust_persistence::sidecar::{
    load_mapper_save, write_mapper_save, write_recovery_mapper_save,
};
use nerust_persistence::slots::{
    allocate_next_slot_id, autosave_state_slot_path, delete_state_slot, load_state_slot,
    load_state_slot_for_identity, scan_state_slots_for_identity, state_slot_path,
    write_autosave_state_slot, write_state_slot,
};
use nerust_persistence::thumbnail::ThumbnailSource;
use nerust_persistence::time::latest_saved_slot_id;
use std::io::ErrorKind;
use std::path::Path;

impl SessionHandle {
    pub fn metrics(&self) -> nerust_console::ConsoleMetrics {
        self.runtime.snapshot().metrics
    }

    pub fn window_size(&self) -> WindowSize {
        let profile = self
            .runtime
            .snapshot()
            .video_profile
            .expect("system runtime should publish a video profile");
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

    pub fn input_topology_descriptor(&self) -> nerust_input_schema::InputTopologyDescriptor {
        self.descriptor.input_topology.clone()
    }

    pub fn system_settings_page_model(&self) -> SystemSettingsPageModel {
        self.definition.settings_page(&self.settings_snapshot)
    }

    pub fn apply_system_settings_choice(
        &self,
        settings: &mut nerust_gui_runtime::settings::SettingsSnapshot,
        field: &crate::descriptor::SystemSettingsFieldId,
        choice: &crate::descriptor::SystemSettingsChoiceId,
    ) -> Result<(), String> {
        self.definition
            .apply_settings_choice(settings, field, choice)
    }

    pub fn apply_settings(
        &mut self,
        next_settings: nerust_gui_runtime::settings::SettingsSnapshot,
    ) -> Result<nerust_gui_runtime::settings::SettingsApplyPlan, String> {
        let previous = self.settings_snapshot.clone();
        let plan = nerust_gui_runtime::settings::apply::derive_apply_plan(
            self.host_backend,
            &previous,
            &next_settings,
        );

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
        self.clear_input()?;
        Ok(plan)
    }

    pub fn set_fullscreen_default(
        &mut self,
        fullscreen: bool,
    ) -> Result<nerust_gui_runtime::settings::SettingsApplyPlan, String> {
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
            return Err(format!("failed to save settings: {error}"));
        }

        self.settings_snapshot = next_settings;
        Ok(plan)
    }

    pub fn load(&mut self, media: MediaObject, request: LoadRequest) -> Result<(), String> {
        let resolved = self.resolve_load_request(request, &media)?;
        self.flush_mapper_save()?;
        self.runtime.load(&media, &resolved)?;
        self.loaded_media = Some(super::LoadedMedia {
            media: media.clone(),
            request: resolved,
        });
        self.configure_persistence_for_loaded_media(true);
        self.remember_last_successful_rom_directory(media.path.as_deref());
        self.sync_input_from_runtime();
        Ok(())
    }

    pub fn unload(&mut self) -> Result<bool, String> {
        self.flush_mapper_save()?;
        let unloaded = self.runtime.unload()?;
        if unloaded {
            self.loaded_media = None;
            self.persistence = Default::default();
            self.sync_input_from_runtime();
        }
        Ok(unloaded)
    }

    pub fn flush_before_exit(&mut self) {
        if let Err(error) = self.flush_mapper_save() {
            log::warn!("mapper save flush before close failed: {error}");
        }
    }

    pub fn run_command(
        &mut self,
        command: SessionCommand,
    ) -> Result<SessionCommandOutcome, String> {
        match command {
            SessionCommand::Pause => {
                if self.paused() {
                    Ok(SessionCommandOutcome::default())
                } else {
                    self.runtime.pause();
                    Ok(SessionCommandOutcome {
                        executed: true,
                        needs_redraw: false,
                    })
                }
            }
            SessionCommand::Resume => {
                if self.paused() {
                    self.runtime.resume();
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
                self.runtime.reset()?;
                Ok(SessionCommandOutcome {
                    executed: true,
                    needs_redraw: false,
                })
            }
            SessionCommand::CreateSlot => {
                self.create_slot();
                Ok(SessionCommandOutcome {
                    executed: true,
                    needs_redraw: false,
                })
            }
            SessionCommand::SaveActiveSlotOrNew => {
                self.save_active_slot_or_new();
                Ok(SessionCommandOutcome {
                    executed: true,
                    needs_redraw: false,
                })
            }
            SessionCommand::LoadActiveSlot => {
                let was_paused = self.paused();
                let executed = self.load_active_slot();
                Ok(SessionCommandOutcome {
                    executed,
                    needs_redraw: executed && was_paused && !self.paused(),
                })
            }
            SessionCommand::SelectActiveSlot(slot_id) => {
                self.persistence.active_slot_id = Some(slot_id);
                Ok(SessionCommandOutcome {
                    executed: true,
                    needs_redraw: false,
                })
            }
            SessionCommand::SaveSlot(slot_id) => {
                self.save_slot(slot_id, false);
                Ok(SessionCommandOutcome {
                    executed: true,
                    needs_redraw: false,
                })
            }
            SessionCommand::LoadSlot(slot_id) => {
                let was_paused = self.paused();
                let executed = self.load_slot(slot_id);
                Ok(SessionCommandOutcome {
                    executed,
                    needs_redraw: executed && was_paused && !self.paused(),
                })
            }
            SessionCommand::DeleteSlot(slot_id) => {
                self.delete_slot(slot_id);
                Ok(SessionCommandOutcome {
                    executed: true,
                    needs_redraw: false,
                })
            }
            SessionCommand::SelectNextSlot => Ok(SessionCommandOutcome {
                executed: self.select_adjacent_slot(true).is_some(),
                needs_redraw: false,
            }),
            SessionCommand::SelectPreviousSlot => Ok(SessionCommandOutcome {
                executed: self.select_adjacent_slot(false).is_some(),
                needs_redraw: false,
            }),
        }
    }

    pub fn slots(&self) -> &[nerust_persistence::model::StateSlotSummary] {
        &self.persistence.slots
    }

    pub fn active_slot_id(&self) -> Option<u64> {
        self.persistence.active_slot_id
    }

    pub fn save_hidden_lifecycle_state(&mut self) -> bool {
        if !self.loaded() {
            return false;
        }
        let Some(sidecars) = self.persistence.sidecars.as_ref() else {
            return false;
        };
        let Some(identity) = self.persistence_identity() else {
            return false;
        };
        match self.runtime.export_state() {
            Ok(export) => {
                let preview = export.preview.as_ref().map(preview_to_thumbnail_source);
                match write_autosave_state_slot(
                    &sidecars.states_dir,
                    &export.state_blob,
                    identity,
                    preview.as_ref(),
                ) {
                    Ok(_) => true,
                    Err(error) => {
                        log::warn!("saving hidden lifecycle state failed: {error}");
                        false
                    }
                }
            }
            Err(error) => {
                log::warn!("hidden lifecycle state export failed: {error}");
                false
            }
        }
    }

    pub fn load_hidden_lifecycle_state(&mut self) -> bool {
        if !self.loaded() {
            return false;
        }
        let Some(sidecars) = self.persistence.sidecars.as_ref() else {
            return false;
        };
        let Some(identity) = self.persistence_identity() else {
            return false;
        };
        let path = autosave_state_slot_path(&sidecars.states_dir);
        match load_state_slot_for_identity(&path, identity) {
            Ok(Some(slot)) => {
                if slot.summary.emulator_version != env!("CARGO_PKG_VERSION") {
                    log::warn!(
                        "ignoring hidden lifecycle state from emulator version {} on {}",
                        slot.summary.emulator_version,
                        env!("CARGO_PKG_VERSION")
                    );
                    clear_hidden_lifecycle_state_path(&path);
                    return false;
                }
                if let Err(error) = self.runtime.import_state(&slot.machine_state) {
                    log::warn!("hidden lifecycle state import failed: {error}");
                    clear_hidden_lifecycle_state_path(&path);
                    false
                } else {
                    self.sync_input_from_runtime();
                    true
                }
            }
            Ok(None) => {
                clear_hidden_lifecycle_state_path(&path);
                false
            }
            Err(nerust_persistence::error::PersistenceError::Io(error))
                if error.kind() == ErrorKind::NotFound =>
            {
                false
            }
            Err(error) => {
                log::warn!("loading hidden lifecycle state failed: {error}");
                clear_hidden_lifecycle_state_path(&path);
                false
            }
        }
    }

    pub fn clear_hidden_lifecycle_state(&mut self) {
        let Some(sidecars) = self.persistence.sidecars.as_ref() else {
            return;
        };
        let path = autosave_state_slot_path(&sidecars.states_dir);
        clear_hidden_lifecycle_state_path(&path);
    }

    pub fn persistence_identity(&self) -> Option<nerust_contract_persistence::PersistenceIdentity> {
        let media = self.runtime.canonical_media_identity()?;
        Some(nerust_contract_persistence::PersistenceIdentity {
            system_id: self.descriptor.system_id,
            media,
        })
    }

    fn resolve_load_request(
        &self,
        request: LoadRequest,
        media: &MediaObject,
    ) -> Result<ResolvedLoadRequest, String> {
        let (system_id, options) = match request {
            LoadRequest::Auto => (
                self.descriptor.system_id,
                self.definition.default_load_options(),
            ),
            LoadRequest::Explicit { system_id, options } => (system_id, options),
        };
        if system_id != self.descriptor.system_id {
            return Err(format!("unsupported system id: {system_id:?}"));
        }
        if !self.definition.probe_media(media) {
            return Err("media probe failed for active system definition".into());
        }
        self.definition
            .resolve_load_request(&self.settings_snapshot, options)
    }

    fn rebuild_for_settings(
        &mut self,
        next_settings: &nerust_gui_runtime::settings::SettingsSnapshot,
    ) -> Result<(), String> {
        let was_loaded = self.loaded();
        let was_paused = self.paused();
        let exported_state = if was_loaded {
            Some(self.runtime.export_state()?)
        } else {
            None
        };
        let restored_runtime_state = exported_state.is_some();

        let mut rebuilt_runtime = self.definition.create_runtime(
            &RuntimeHostServices {
                host_backend: self.host_backend,
            },
            next_settings,
        )?;
        let rebuilt_adapter = self.definition.create_input_adapter(next_settings);

        if let Some(loaded_media) = self.loaded_media.clone() {
            rebuilt_runtime.load(&loaded_media.media, &loaded_media.request)?;
            if let Some(exported_state) = exported_state.as_ref() {
                rebuilt_runtime.import_state(&exported_state.state_blob)?;
                if !was_paused {
                    rebuilt_runtime.resume();
                }
            }
        }

        self.runtime = rebuilt_runtime;
        self.input_adapter = rebuilt_adapter;
        self.sync_input_from_runtime();
        if was_loaded {
            self.configure_persistence_for_loaded_media(!restored_runtime_state);
            if was_paused {
                self.runtime.pause();
            }
        }
        Ok(())
    }

    fn configure_persistence_for_loaded_media(&mut self, load_mapper_save: bool) {
        if let Some(loaded_media) = self.loaded_media.clone() {
            let identity_opt = self.persistence_identity();
            log::info!(
                "configure_persistence_for_loaded_media: loaded_media path={:?} persistence_identity={:?}",
                loaded_media.media.path.as_deref(),
                identity_opt
            );
            let persistence_paths = identity_opt.and_then(|identity| {
                let resolved = self.resolve_persistence_paths(loaded_media.media.path.as_deref(), identity);
                if resolved.is_none() {
                    log::info!(
                        "configure_persistence_for_loaded_media: failed to resolve persistence paths for identity {:?} and path {:?}",
                        identity,
                        loaded_media.media.path.as_deref()
                    );
                } else {
                    log::info!(
                        "configure_persistence_for_loaded_media: resolved persistence paths: {:?}",
                        resolved
                    );
                }
                resolved
            });
            self.configure_persistence_paths(persistence_paths, load_mapper_save);
        } else {
            self.persistence = Default::default();
        }
    }

    fn resolve_persistence_paths(
        &self,
        rom_path: Option<&Path>,
        identity: nerust_contract_persistence::PersistenceIdentity,
    ) -> Option<nerust_persistence::sidecar::SidecarPaths> {
        match identity.media {
            nerust_contract_persistence::CanonicalMediaIdentity::Rom(rom_identity) => self
                .settings
                .resolve_persistence_paths_with_import(identity.system_id, rom_path, rom_identity)
                .map_err(|error| {
                    log::warn!("failed to resolve persistence paths: {error}");
                    error
                })
                .ok(),
        }
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

    fn load_mapper_save_if_available(&mut self) -> Result<(), String> {
        let Some(sidecars) = self.persistence.sidecars.as_ref() else {
            return Ok(());
        };
        if let Some(bytes) =
            load_mapper_save(&sidecars.mapper_save_path).map_err(|error| error.to_string())?
        {
            self.runtime.import_mapper_save(bytes)?;
        }
        Ok(())
    }

    fn flush_mapper_save(&mut self) -> Result<(), String> {
        let Some(sidecars) = self.persistence.sidecars.as_ref() else {
            return Ok(());
        };
        if !self.persistence.mapper_save_flush_allowed {
            if self.persistence.mapper_save_recovery_written {
                return Ok(());
            }
            if let Some(bytes) = self.runtime.export_mapper_save()? {
                let path = write_recovery_mapper_save(&sidecars.mapper_save_path, &bytes)
                    .map_err(|error| error.to_string())?;
                self.persistence.mapper_save_recovery_written = true;
                log::warn!(
                    "mapper save auto-load failed earlier; wrote recovery save to {}",
                    path.display()
                );
            }
            return Ok(());
        }
        match self.runtime.export_mapper_save()? {
            Some(bytes) => write_mapper_save(&sidecars.mapper_save_path, &bytes)
                .map_err(|error| error.to_string()),
            None => Ok(()),
        }
    }

    fn configure_persistence_paths(
        &mut self,
        persistence_paths: Option<nerust_persistence::sidecar::SidecarPaths>,
        load_mapper_save: bool,
    ) {
        self.persistence.sidecars = persistence_paths;
        self.persistence.mapper_save_flush_allowed = true;
        self.persistence.mapper_save_recovery_written = false;
        self.persistence.active_slot_id = None;
        self.refresh_slots();
        self.persistence.active_slot_id = latest_saved_slot_id(&self.persistence.slots);
        if load_mapper_save && let Err(error) = self.load_mapper_save_if_available() {
            self.persistence.mapper_save_flush_allowed = false;
            log::warn!("mapper save auto-load failed: {error}");
        }
    }

    fn refresh_slots(&mut self) {
        self.persistence.slots = if let (Some(sidecars), Some(identity)) = (
            self.persistence.sidecars.as_ref(),
            self.persistence_identity(),
        ) {
            match scan_state_slots_for_identity(&sidecars.states_dir, identity) {
                Ok(slots) => slots,
                Err(error) => {
                    log::warn!("slot scan failed: {error}");
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };
        if self.persistence.active_slot_id.is_some_and(|slot_id| {
            !self
                .persistence
                .slots
                .iter()
                .any(|slot| slot.slot_id == slot_id)
        }) {
            self.persistence.active_slot_id = None;
        }
    }

    fn save_active_slot_or_new(&mut self) {
        if self.persistence.sidecars.is_none() {
            log::info!(
                "save_active_slot_or_new: no persistence sidecars configured; cannot save state"
            );
            return;
        }
        let sidecars = self.persistence.sidecars.as_ref().unwrap();
        let slot_id = self.persistence.active_slot_id.or_else(|| {
            allocate_next_slot_id(&sidecars.states_dir)
                .map_err(|error| {
                    log::warn!("allocating state slot failed: {error}");
                    error
                })
                .ok()
        });
        match slot_id {
            Some(slot_id) => {
                log::info!("save_active_slot_or_new: saving to slot {}", slot_id);
                self.save_slot(slot_id, true);
            }
            None => {
                log::warn!("save_active_slot_or_new: failed to allocate slot id");
            }
        }
    }

    fn create_slot(&mut self) {
        let Some(sidecars) = self.persistence.sidecars.as_ref() else {
            return;
        };
        match allocate_next_slot_id(&sidecars.states_dir) {
            Ok(slot_id) => self.save_slot(slot_id, true),
            Err(error) => log::warn!("allocating state slot failed: {error}"),
        }
    }

    fn save_slot(&mut self, slot_id: u64, make_active: bool) {
        if self.persistence.sidecars.is_none() {
            log::info!(
                "save_slot: no persistence.sidecars configured; cannot save slot {}",
                slot_id
            );
            return;
        }
        let sidecars = self.persistence.sidecars.as_ref().unwrap();
        let identity_opt = self.persistence_identity();
        if identity_opt.is_none() {
            log::info!(
                "save_slot: no persistence identity available; cannot save slot {}",
                slot_id
            );
            return;
        }
        let identity = identity_opt.unwrap();
        log::info!(
            "save_slot: writing slot {} (make_active={}) to {}",
            slot_id,
            make_active,
            sidecars.states_dir.display()
        );
        match self.runtime.export_state() {
            Ok(export) => {
                let preview = export.preview.as_ref().map(|preview| ThumbnailSource {
                    width: preview.width,
                    height: preview.height,
                    rgba: preview.rgba.clone(),
                });
                match write_state_slot(
                    &sidecars.states_dir,
                    slot_id,
                    &export.state_blob,
                    identity,
                    preview.as_ref(),
                ) {
                    Ok(_) => {
                        if make_active {
                            self.persistence.active_slot_id = Some(slot_id);
                        }
                        self.refresh_slots();
                        log::info!("save_slot: saved slot {}", slot_id);
                    }
                    Err(error) => log::warn!("saving state slot failed: {error}"),
                }
            }
            Err(error) => log::warn!("state export failed: {error}"),
        }
    }

    fn load_active_slot(&mut self) -> bool {
        self.persistence
            .active_slot_id
            .is_some_and(|slot_id| self.load_slot(slot_id))
    }

    fn load_slot(&mut self, slot_id: u64) -> bool {
        let Some(sidecars) = self.persistence.sidecars.as_ref() else {
            return false;
        };
        match load_state_slot(&state_slot_path(&sidecars.states_dir, slot_id)) {
            Ok(slot) => {
                if let Err(error) = self.runtime.import_state(&slot.machine_state) {
                    log::warn!("state import failed: {error}");
                    false
                } else {
                    self.persistence.active_slot_id = Some(slot_id);
                    self.sync_input_from_runtime();
                    self.refresh_slots();
                    true
                }
            }
            Err(error) => {
                log::warn!("loading state slot failed: {error}");
                false
            }
        }
    }

    fn delete_slot(&mut self, slot_id: u64) {
        let Some(sidecars) = self.persistence.sidecars.as_ref() else {
            return;
        };
        match delete_state_slot(&state_slot_path(&sidecars.states_dir, slot_id)) {
            Ok(()) => {
                if self.persistence.active_slot_id == Some(slot_id) {
                    self.persistence.active_slot_id = None;
                }
                self.refresh_slots();
            }
            Err(error) => log::warn!("deleting state slot failed: {error}"),
        }
    }

    fn select_adjacent_slot(&mut self, forward: bool) -> Option<u64> {
        let next_slot_id = adjacent_slot_id(
            &self.persistence.slots,
            self.persistence.active_slot_id,
            forward,
        )?;
        self.persistence.active_slot_id = Some(next_slot_id);
        Some(next_slot_id)
    }
}

fn adjacent_slot_id(
    slots: &[nerust_persistence::model::StateSlotSummary],
    active_slot_id: Option<u64>,
    forward: bool,
) -> Option<u64> {
    if slots.is_empty() {
        return None;
    }
    let current_index = active_slot_id
        .and_then(|active| slots.iter().position(|slot| slot.slot_id == active))
        .unwrap_or_else(|| {
            if forward {
                slots.len().saturating_sub(1)
            } else {
                0
            }
        });
    let next_index = if forward {
        (current_index + 1) % slots.len()
    } else if current_index == 0 {
        slots.len() - 1
    } else {
        current_index - 1
    };
    Some(slots[next_index].slot_id)
}

fn clear_hidden_lifecycle_state_path(path: &Path) {
    if let Err(error) = delete_state_slot(path) {
        log::warn!("deleting hidden lifecycle state failed: {error}");
    }
}

fn preview_to_thumbnail_source(preview: &PreviewFrame) -> ThumbnailSource {
    ThumbnailSource {
        width: preview.width,
        height: preview.height,
        rgba: preview.rgba.clone(),
    }
}
