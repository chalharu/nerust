//! Session lifecycle orchestration: load/unload/flush plus the per-tick
//! pump. Domain work lives in sibling modules (`settings_apply`,
//! `command_exec`, `session_query`, `persistence`); this file only
//! sequences construction, persistence, and notification flow.

use nerust_core_traits::factory::load::{MediaObject, ResolvedLoadRequest};

use crate::session::{
    SessionError, SessionHandle,
    osd::{OsdManager, PumpReport, PushInput},
    persistence_setup::{MapperRestore, PersistenceBinder},
};

impl SessionHandle {
    pub fn restart_audio(&mut self) {
        if let Some(core) = self.emu.core().ok()
            && let Err(error) = core.restart_audio()
        {
            log::warn!("restart_audio failed: {error}");
        }
    }

    pub fn load_resolved(
        &mut self,
        media: MediaObject,
        resolved: ResolvedLoadRequest,
    ) -> Result<(), SessionError> {
        {
            let Some(core) = self.emu.core().ok() else {
                return Err(SessionError::NoCore);
            };
            // Best-effort: a stale battery-save write failure must
            // never veto opening a new ROM.
            if let Err(error) = self.media.persistence.flush_mapper_save(core) {
                log::warn!("mapper-save flush before load failed: {error}");
                self.notifications.push(
                    crate::session::notification::NotificationKind::command_failure(
                        crate::session::notification::FailureOrigin::Bridge,
                        "mapper-flush",
                        error.to_string(),
                    ),
                );
            }
        }
        self.emu
            .core_mut()
            .ok()
            .ok_or(SessionError::NoCore)?
            .load(&media, Some(resolved.options))?;
        self.media.loaded = Some(super::LoadedMedia {
            media: media.clone(),
        });

        PersistenceBinder::bind(self, media.native_path(), MapperRestore::Load);

        self.remember_last_successful_rom_directory(media.native_path());
        Ok(())
    }

    pub fn unload(&mut self) -> Result<(), SessionError> {
        if let Ok(core) = self.emu.core() {
            self.media.persistence.flush_mapper_save(core)?;
        }
        if let Ok(core) = self.emu.core_mut() {
            core.unload()?;
        }
        self.media.loaded = None;
        self.media.persistence.reset();
        Ok(())
    }

    pub fn flush_before_exit(&mut self) {
        if let Some(core) = self.emu.core().ok()
            && let Err(error) = self.media.persistence.flush_mapper_save(core)
        {
            log::warn!("mapper save flush before close failed: {error}");
        }
        // Persist the latest settings to disk.  Reload from the manager first
        // so that any pending changes (e.g. window size from
        // remember_fit_window_size) are not overwritten by a stale snapshot.
        match self.settings.manager.reload() {
            Ok(snapshot) => {
                self.settings.snapshot = snapshot;
            }
            Err(error) => {
                log::warn!("settings reload before exit failed: {error}");
            }
        }
        if let Err(error) = self
            .settings
            .manager
            .save_snapshot(self.settings.snapshot.clone())
        {
            log::warn!("settings save on exit failed: {error}");
        }
    }

    pub(crate) fn drain_notifications(&mut self) -> Vec<super::notification::UiNotification> {
        self.notifications.drain()
    }

    pub fn pump_osd(&mut self, manager: &mut OsdManager, input: PushInput) -> PumpReport {
        let PushInput {
            policy,
            language,
            now,
            tick,
        } = input;
        let before = manager.generation();
        if self.notifications.pending_len() == 0 {
            manager.poll(now);
            let modal_queued = manager.pump_modal();
            return PumpReport {
                changed: manager.generation() != before,
                modal_queued,
            };
        }
        let channel = tick.frame_channel();
        let drained = self.drain_notifications();
        manager.push_all_with_policy(drained, &policy, language, now, channel);
        manager.poll(now);
        let modal_queued = manager.pump_modal();
        PumpReport {
            changed: manager.generation() != before,
            modal_queued,
        }
    }

    pub fn report_external_error(
        &mut self,
        origin: super::notification::FailureOrigin,
        detail: String,
    ) {
        debug_assert!(origin != super::notification::FailureOrigin::Command);
        self.notifications
            .push(super::notification::NotificationKind::command_failure(
                origin, "", detail,
            ));
    }
}
