//! Session lifecycle orchestration: load/unload/flush plus the per-tick pump.
//! Domain work lives in sibling modules; this file only sequences the flow.

use nerust_system_traits::factory::load::{MediaObject, ResolvedLoadRequest};

use crate::session::{
    SessionError, SessionHandle,
    osd::{PumpReport, PumpTarget, PushInput},
    persistence_setup::PersistenceBinder,
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
        let tick = self.tick_metrics();
        let options = resolved.options;
        let factory = self.active_factory_cloned().ok_or(SessionError::NoCore)?;
        let candidate = super::settings_apply::build_core_for_settings(
            self.speaker_factory(),
            &self.settings.snapshot,
            &factory,
            &self.input.current_assignments,
        )?;
        candidate
            .emu_core
            .load_paused(&media, Some(options.clone()))?;
        // Freeze the previous core before its final export. Failure vetoes replacement
        // and restores its execution state; no persistence configuration changed yet.
        let was_running = tick.loaded && !tick.paused;
        if was_running {
            self.emu.core()?.pause()?;
        }
        let flush = self
            .media
            .persistence
            .flush_mapper_save(self.emu.exporter()?);
        if let Err(error) = flush {
            if was_running
                && !self.execution_held
                && let Err(rollback) = self.emu.core()?.resume()
            {
                return Err(SessionError::Rollback {
                    apply: Box::new(error.into()),
                    rollback: Box::new(rollback.into()),
                });
            }
            return Err(error.into());
        }
        PersistenceBinder::prepare(self, &candidate.emu_core, media.native_path());
        self.emu.replace(super::ActiveRuntime::from(candidate));
        self.debug_clear_write();
        self.clear_input();
        self.input.invalidate_topology();
        self.rebuild_key_field_map();

        self.remember_last_successful_rom_directory(media.native_path());
        // Move, don't clone: the core borrowed the bytes above; the media travels into storage.
        self.media
            .set_loaded(Some(super::LoadedMedia { media, options }));
        if !self.execution_held {
            // Replacement is already published: a resume rejection is a partial load,
            // never proof that the old media is still active.
            self.emu
                .core()?
                .resume()
                .map_err(|_| nerust_emu_thread::OperationError::NoReply)?;
        }
        Ok(())
    }

    pub fn unload(&mut self) -> Result<(), SessionError> {
        if let Ok(exporter) = self.emu.exporter() {
            self.media.persistence.flush_mapper_save(exporter)?;
        }
        if let Ok(core) = self.emu.core_mut() {
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

    pub(crate) fn drain_notifications(&mut self) -> Vec<super::notification::UiNotification> {
        self.notifications.drain()
    }

    pub fn pump_osd(&mut self, osd: &mut dyn PumpTarget, input: PushInput) -> PumpReport {
        let PushInput {
            policy,
            language,
            now,
            tick,
        } = input;
        let before = osd.generation();
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
        // Hard invariant: `Command` failures enter through `run_command`; external
        // reporters must pick a non-command origin. Rejected loudly in all builds.
        if origin == super::notification::FailureOrigin::Command {
            log::error!("report_external_error rejected Command origin");
            return;
        }
        self.notifications
            .push(super::notification::NotificationKind::command_failure(
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
            notification::{NotificationKind, NotificationLevel, UiNotification},
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
                kind: NotificationKind::Paused,
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
