//! Session lifecycle orchestration: load/unload/flush plus the per-tick pump.
//! Domain work lives in sibling modules; this file only sequences the flow.

use nerust_core_traits::CoreOptions;
use nerust_system_traits::factory::load::{DynSystemLoadOptions, MediaObject, ResolvedLoadRequest};

use crate::{
    load::{RomLoadTarget, RomLoaderError, SystemActivationError},
    session::{
        SessionError, SessionHandle,
        osd::{OsdManager, PumpReport, PushInput},
        persistence_setup::{MapperRestore, PersistenceBinder},
    },
};

/// Loader boundary straight on the session: no adapter between `RomLoader` and `SessionHandle`.
impl crate::load::LoadEnvironment for SessionHandle {
    fn default_load_options(&self) -> Option<Box<dyn DynSystemLoadOptions>> {
        SessionHandle::default_load_options(self)
    }

    fn settings_snapshot(&self) -> &nerust_gui_runtime::settings::SettingsSnapshot {
        SessionHandle::settings_snapshot(self)
    }

    fn set_active_system(
        &mut self,
        system_id: &dyn nerust_core_traits::identity::SystemId,
    ) -> Result<(), SystemActivationError> {
        SessionHandle::set_active_system(self, system_id)
    }
}

impl RomLoadTarget for SessionHandle {
    fn load_resolved(
        &mut self,
        media: MediaObject,
        resolved: ResolvedLoadRequest,
    ) -> Result<(), RomLoaderError> {
        SessionHandle::load_resolved(self, media, resolved).map_err(RomLoaderError::from_session)
    }

    fn resume(&mut self) {
        let _ = SessionHandle::run_command(self, crate::session::commands::SessionCommand::Resume);
    }
}

impl SessionHandle {
    pub fn restart_audio(&mut self) {
        if let Some(core) = self.emu.require_live().ok()
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
        // A switch published earlier dies here with the load: restore the
        // pre-switch live state instead of stranding the empty core.
        // Without a pending switch this is a no-op (NER-320).
        let candidate = match self.stage_candidate(&media, resolved.options.as_ref()) {
            Ok(candidate) => candidate,
            Err(error) => {
                self.rollback_pre_switch();
                return Err(error);
            }
        };
        let was_running = self.freeze_previous()?;
        if let Err(error) = self.flush_previous_mapper() {
            let frozen = self.rollback_freeze(was_running, error);
            self.rollback_pre_switch();
            return frozen;
        }
        match self.commit_candidate(candidate, media, resolved.options) {
            Ok(()) => {
                self.discard_pre_switch();
                Ok(())
            }
            Err(error) => {
                self.rollback_pre_switch();
                Err(error)
            }
        }
    }

    /// Prepare the replacement core without touching live state.
    /// Failure here leaves the previous core, persistence, and media untouched.
    fn stage_candidate(
        &mut self,
        media: &MediaObject,
        options: &dyn CoreOptions,
    ) -> Result<super::CoreRuntime, SessionError> {
        let factory = self.active_factory_cloned().ok_or(SessionError::NoCore)?;
        let candidate = super::settings_apply::build_core_for_settings(
            self.speaker_factory(),
            &self.settings.snapshot,
            &factory,
            &self.input.current_assignments,
        )?;
        candidate
            .emu_core
            .load_paused(media, Some(dyn_clone::clone_box(options)))?;
        Ok(candidate)
    }

    /// Freeze the previous core before its final export; reports whether it ran.
    /// A pause failure vetoes replacement before anything else changes.
    fn freeze_previous(&mut self) -> Result<bool, SessionError> {
        let tick = self.tick_metrics();
        let was_running = tick.loaded && !tick.paused;
        if was_running {
            self.emu.require_live()?.pause()?;
        }
        Ok(was_running)
    }

    /// Final export of the previous core; no persistence configuration changed yet.
    fn flush_previous_mapper(&mut self) -> Result<(), SessionError> {
        let exporter = self.emu.exporter()?;
        self.media.persistence.flush_mapper_save(exporter)?;
        Ok(())
    }

    /// Resume the previous core after a vetoed replacement.
    /// A failed resume pairs the flush fault with the rollback fault.
    fn rollback_freeze(
        &mut self,
        was_running: bool,
        error: SessionError,
    ) -> Result<(), SessionError> {
        if was_running
            && !self.execution_held
            && let Err(rollback) = self.emu.require_live()?.resume()
        {
            return Err(SessionError::Rollback {
                apply: Box::new(error),
                rollback: Box::new(rollback.into()),
            });
        }
        Err(error)
    }
    /// Publish the staged core: persistence, media, and execution switch over together.
    fn commit_candidate(
        &mut self,
        candidate: super::CoreRuntime,
        media: MediaObject,
        options: Box<dyn CoreOptions>,
    ) -> Result<(), SessionError> {
        let setup = PersistenceBinder::bind(
            self,
            Some(&candidate.emu_core),
            media.native_path(),
            MapperRestore::Load,
        );
        self.note_persistence_setup(setup);
        self.publish_runtime(super::ActiveRuntime::from(candidate));

        self.remember_last_successful_rom_directory(media.native_path());
        // Move, don't clone: the core borrowed the bytes above; the media travels into storage.
        self.media
            .set_loaded(Some(super::LoadedMedia { media, options }));
        if !self.execution_held {
            // Replacement is already published: a resume rejection is a partial load,
            // never proof that the old media is still active.
            self.emu
                .require_live()?
                .resume()
                .map_err(|_| nerust_emu_thread::OperationError::NoReply)?;
        }
        Ok(())
    }

    pub fn unload(&mut self) -> Result<(), SessionError> {
        // Explicit teardown: no later failed load may resurrect the
        // pre-switch state stashed by an abandoned switch.
        self.discard_pre_switch();
        if let Ok(exporter) = self.emu.exporter() {
            self.media.persistence.flush_mapper_save(exporter)?;
        }
        if let Ok(core) = self.emu.require_live_mut() {
            core.unload()?;
        }
        self.emu.invalidate_debug();
        self.debug_clear_write();
        self.media.clear_runtime();
        Ok(())
    }

    pub fn flush_before_exit(&mut self) {
        if let Some(exporter) = self.emu.exporter().ok()
            && let Err(error) = self.media.persistence.flush_mapper_save(exporter)
        {
            log::warn!("mapper save flush before close failed: {error}");
        }
        // Settings are persisted at their mutation boundary. Reloading or saving
        // again here could overwrite committed app state with an older copy.
    }

    #[cfg(test)]
    pub(crate) fn drain_notifications(&mut self) -> Vec<super::notification::UiNotification> {
        self.notifications.drain()
    }

    pub fn pump_osd(&mut self, osd: &mut OsdManager, input: PushInput) -> PumpReport {
        let PushInput {
            policy,
            language,
            now,
            tick,
        } = input;
        let before = osd.generation();
        let channel = tick.frame_channel();
        self.notifications.drain_into(&mut self.drain_scratch);
        let drained = self.drain_scratch.drain(..);
        osd.push_all_with_policy(drained, &policy, language, now, channel);
        osd.poll(now);
        let modal_queued = osd.pump_modal();
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
        // Hard invariant: `Command` failures enter through `run_command`; external
        // reporters must pick a non-command origin. Rejected loudly in all builds.
        if origin == super::notification::FailureOrigin::Command {
            log::error!("report_external_error rejected Command origin");
            return;
        }
        self.notifications
            .push(super::notification::Message::command_failure(
                origin, "", detail,
            ));
    }
}

#[cfg(test)]
mod lifecycle_cov_tests {
    use crate::session::lifecycle::*;
    use crate::session::test_util::test_session;

    #[test]
    fn empty_notification_pump_still_relocalizes_and_filters_visible_rows() {
        use crate::session::{
            notification::{Message, NotificationLevel, UiNotification},
            osd::{FrameChannel, OsdManager, TickMetrics, TransientPolicy},
        };
        use nerust_settings_core::model::{language::AppLanguage, local::NotificationSettings};
        let mut session = test_session();
        let mut osd = OsdManager::new();
        let now = std::time::Instant::now();
        let policy = TransientPolicy::from(&NotificationSettings::default());
        osd.push_all_with_policy(
            vec![UiNotification {
                level: NotificationLevel::Info,
                message: Message::paused(),
                count: 1,
                seq: 0,
                at: std::time::UNIX_EPOCH,
            }],
            &policy,
            AppLanguage::English,
            now,
            FrameChannel::Composited,
        );
        osd.poll(now);
        assert_eq!(osd.rows()[0].text.as_ref(), "Paused");
        let input = |policy| PushInput {
            policy,
            language: AppLanguage::Japanese,
            now,
            tick: TickMetrics {
                loaded: true,
                ..Default::default()
            },
        };
        assert!(session.pump_osd(&mut osd, input(policy)).changed);
        assert_eq!(osd.rows()[0].text.as_ref(), "一時停止しました");
        let off = TransientPolicy {
            info_enabled: false,
            ..policy
        };
        assert!(session.pump_osd(&mut osd, input(off)).changed);
        assert!(osd.rows().is_empty());
    }

    #[test]
    fn coreless_lifecycle_is_best_effort() {
        use std::sync::Arc;
        let probe = test_session();
        let resolved = probe
            .active_factory_cloned()
            .unwrap()
            .resolve_load_request(
                &crate::session::test_util::test_view(&probe),
                probe
                    .active_factory_cloned()
                    .unwrap()
                    .default_load_options(),
            )
            .unwrap();
        let caps = probe.capabilities;
        let registry = Arc::new(crate::registry::SystemRegistry::new(vec![]));
        let audio = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
        let mut session = SessionHandle::new_ephemeral(caps, registry, audio);
        session.restart_audio();
        assert!(
            session
                .load_resolved(MediaObject::new(None, vec![]), resolved)
                .is_err()
        );
        assert!(session.unload().is_ok());
        session.report_external_error(super::super::notification::FailureOrigin::Host, "x".into());
        assert_eq!(session.drain_notifications().len(), 1);
        // Hard invariant holds in all builds: `Command` origin is rejected, not recorded.
        session.report_external_error(
            super::super::notification::FailureOrigin::Command,
            "y".into(),
        );
        assert!(session.drain_notifications().is_empty());
    }

    #[test]
    fn loaded_unload_clears_runtime() {
        let mut session = test_session();
        session.restart_audio();
        assert!(session.unload().is_ok());
        assert!(!session.tick_metrics().loaded);
    }
}
