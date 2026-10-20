//! Session lifecycle orchestration: load/unload/flush plus the per-tick pump.
//! Domain work lives in sibling modules; this file only sequences the flow.

use nerust_core_traits::factory::load::{MediaObject, ResolvedLoadRequest};

use crate::session::{
    SessionError, SessionHandle,
    osd::{PumpReport, PumpTarget, PushInput},
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
            let Some(exporter) = self.emu.exporter().ok() else {
                return Err(SessionError::NoCore);
            };
            // Best-effort: a stale battery-save write failure never vetoes opening a new ROM.
            // The typed chain renders at the notification boundary.
            if let Err(error) = self.media.persistence.flush_mapper_save(exporter) {
                log::warn!("mapper-save flush before load failed: {error}");
                self.notifications.push(
                    crate::session::notification::NotificationKind::command_failure_from(
                        crate::session::notification::FailureOrigin::Bridge,
                        "mapper-flush",
                        &error,
                    ),
                );
            }
        }
        self.emu
            .core_mut()
            .ok()
            .ok_or(SessionError::NoCore)?
            .load(&media, Some(resolved.options))?;

        PersistenceBinder::bind(self, media.native_path(), MapperRestore::Load);

        self.remember_last_successful_rom_directory(media.native_path());
        // Move, don't clone: the core borrowed the bytes above; the media travels into storage.
        self.media.set_loaded(Some(super::LoadedMedia { media }));
        Ok(())
    }

    pub fn unload(&mut self) -> Result<(), SessionError> {
        if let Ok(exporter) = self.emu.exporter() {
            self.media.persistence.flush_mapper_save(exporter)?;
        }
        if let Ok(core) = self.emu.core_mut() {
            core.unload()?;
        }
        self.media.clear_runtime();
        Ok(())
    }

    pub fn flush_before_exit(&mut self) {
        if let Some(exporter) = self.emu.exporter().ok()
            && let Err(error) = self.media.persistence.flush_mapper_save(exporter)
        {
            log::warn!("mapper save flush before close failed: {error}");
        }
        // Reload from the manager before persisting, so pending changes
        // are not overwritten by a stale snapshot.
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

    pub fn pump_osd(&mut self, osd: &mut impl PumpTarget, input: PushInput) -> PumpReport {
        let PushInput {
            policy,
            language,
            now,
            tick,
        } = input;
        let before = osd.generation();
        if self.notifications.pending_len() == 0 {
            osd.poll_queue(now);
            let modal_queued = osd.pump_modal_queue();
            return PumpReport {
                changed: osd.generation() != before,
                modal_queued,
            };
        }
        let channel = tick.frame_channel();
        let drained = self.drain_notifications();
        osd.push_notifications(drained, &policy, language, now, channel);
        osd.poll_queue(now);
        let modal_queued = osd.pump_modal_queue();
        PumpReport {
            changed: osd.generation() != before,
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
