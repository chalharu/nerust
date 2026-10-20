//! Settings dialog lifecycle: open/close plus applying snapshots.
//! Owned by `HostState`; the iced side lives in `crate::aux::settings`.
use super::HostState;

use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::session::{
    SessionError,
    access::{PauseControl, SessionControl, SettingsResult},
};

use crate::ports::WindowOpener;

impl HostState {
    pub(crate) fn apply_settings(
        &mut self,
        settings: SettingsSnapshot,
    ) -> Result<SettingsResult, SessionError> {
        let result = SessionControl::apply_settings(self, settings)?;
        if result.fullscreen_default_changed {
            self.sync_fullscreen_from_settings();
        }
        if result.scaling_changed {
            self.update_window_size_for_scaling();
        }
        let ctx = self.frame_ctx();
        self.sync_chrome(&ctx, true);
        Ok(result)
    }

    pub(crate) fn on_settings_closed(&mut self) {
        if !self.settings_open() {
            return;
        }
        let should_resume = self.settings.take_resume_after();
        self.settings.close_all();
        if should_resume {
            self.resume();
        } else {
            let ctx = self.frame_ctx();
            self.sync_chrome(&ctx, false);
        }
    }

    pub(super) fn open_settings_window(&mut self, opener: &dyn WindowOpener) {
        if self.settings_open() {
            return;
        }
        let tick = self.session.tick_metrics();
        let resume_after = tick.loaded && !tick.paused;
        self.settings.set_resume_after(resume_after);
        if resume_after {
            self.pause();
        } else {
            let ctx = self.frame_ctx();
            self.sync_menu_state(&ctx);
        }

        match opener.create_aux(
            crate::aux::focus::AuxKind::Settings,
            crate::ports::AuxOpenParams::Settings {
                snapshot: self.session.settings_snapshot().clone(),
                registry: self.ctx.registry.clone(),
                audio: self.ctx.audio_registry.clone(),
            },
        ) {
            Some(handle) => {
                let id = self.aux.insert(handle);
                debug_assert_eq!(
                    self.aux.route_id(crate::aux::focus::AuxKind::Settings),
                    Some(id)
                );
            }
            None => {
                log::error!("failed to open settings window");
                self.settings.close_all();
                if resume_after {
                    self.resume();
                } else {
                    let ctx = self.frame_ctx();
                    self.sync_menu_state(&ctx);
                }
            }
        }
    }

    pub(crate) fn close_settings_window(
        &mut self,
        pending: Option<SettingsSnapshot>,
    ) -> Option<SettingsResult> {
        self.on_settings_closed();
        if let Some(snapshot) = pending {
            match self.apply_settings(snapshot) {
                Ok(plan) => return Some(plan),
                Err(error) => {
                    log::warn!("settings apply failed after close: {error}");
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use crate::runtime::host::TestHost;
    use crate::test_opener::{FakeOpener, OpenerCall};
    use crate::testkit::StubAux;

    #[test]
    fn open_settings_attempt_records_request_and_stays_closed() {
        // Single truth is the registry: headless open fails, so no
        // handle exists and the guard stays closed.
        let support = TestHost::new();
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        host.open_settings_window(&opener);
        assert_eq!(opener.calls(), [OpenerCall::Settings]);
        assert!(!host.settings_open());
    }

    #[test]
    fn open_settings_failure_rolls_back() {
        let support = TestHost::new();
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        host.open_settings_window(&opener);
        assert_eq!(opener.calls(), [OpenerCall::Settings]);
        assert!(!host.settings_open());
        // Failure leaves the guard closed; the dirty gate collapses
        // the pre + rollback syncs into one push.
        let open_flags: Vec<bool> = support
            .menu
            .updates()
            .iter()
            .map(|sync| sync.settings_open)
            .collect();
        assert_eq!(open_flags, [false]);
    }

    #[test]
    fn settings_close_without_open_state_is_noop() {
        let support = TestHost::new();
        let mut host = support.host;
        host.on_settings_closed();
        assert!(!host.settings_open());
        assert!(support.menu.updates().is_empty());
    }

    #[test]
    fn settings_close_without_handle_is_noop() {
        // Without a handle, close is a no-op even with a stale resume
        // flag: the handle is the only open truth.
        let support = TestHost::new();
        let mut host = support.host;
        host.settings.set_resume_after(true);
        host.on_settings_closed();
        assert!(!host.settings_open());
        assert!(host.settings.take_resume_after());
        assert!(support.menu.updates().is_empty());
    }

    #[test]
    fn settings_close_with_open_window_syncs_chrome() {
        use crate::aux::focus::AuxKind;
        let support = TestHost::new();
        let mut host = support.host;
        host.aux.insert(Box::new(StubAux::new(AuxKind::Settings)));
        assert!(host.settings_open());
        // Registry removal is the caller's job; close teardown only
        // clears the resume flag and syncs chrome while still open.
        host.on_settings_closed();
        assert!(host.settings_open());
        assert!(!host.settings.take_resume_after());
        assert_eq!(support.menu.updates().len(), 1);
    }

    #[test]
    fn settings_close_with_resume_flag_resumes() {
        use crate::aux::focus::AuxKind;
        let support = TestHost::new();
        let mut host = support.host;
        host.aux.insert(Box::new(StubAux::new(AuxKind::Settings)));
        host.settings.set_resume_after(true);
        host.on_settings_closed();
        assert!(host.settings_open());
        assert!(!host.settings.take_resume_after());
        assert!(support.menu.updates().is_empty());
    }

    #[test]
    fn close_settings_window_applies_pending_snapshot() {
        let support = TestHost::new();
        let mut host = support.host;
        assert!(host.close_settings_window(None).is_none());
        let snapshot = host.session().settings_snapshot().clone();
        let plan = host
            .close_settings_window(Some(snapshot))
            .expect("identical snapshot applies cleanly");
        assert!(!plan.renderer_needs_rebuild);
    }
}
