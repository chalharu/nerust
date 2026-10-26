//! Settings dialog lifecycle: open/close plus applying snapshots.
//! Owned by `HostState`; the iced side lives in `crate::aux::settings`.
use super::HostState;
use nerust_frontend_host::aux::OpenRequest;
use nerust_frontend_host::session::SessionEvent;

use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::session::{
    SessionError,
    access::{SessionControl, SettingsResult},
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
        let changed = self
            .dispatch_session(SessionEvent::SettingsClosed)
            .is_ok_and(|out| out.settings_changed);
        if !changed {
            return;
        }
        let ctx = self.frame_ctx();
        self.sync_chrome(&ctx, false);
    }

    pub(super) fn open_settings_window(&mut self, opener: &dyn WindowOpener) {
        if self.settings_open() {
            return;
        }
        if !self
            .dispatch_session(SessionEvent::SettingsOpened)
            .is_ok_and(|out| out.settings_changed)
        {
            return;
        }
        let ctx = self.frame_ctx();
        self.sync_menu_state(&ctx);

        let OpenRequest::Create(id) = self.aux.request_open(crate::aux::focus::AuxKind::Settings)
        else {
            return;
        };
        let opened = opener
            .create_aux(
                crate::aux::focus::AuxKind::Settings,
                crate::ports::AuxOpenParams::Settings {
                    snapshot: self.session.settings_snapshot().clone(),
                    registry: self.ctx.registry.clone(),
                    audio: self.ctx.audio_registry.clone(),
                },
            )
            .is_some_and(|handle| self.aux.complete_open(id, handle).is_ok());
        if !opened {
            self.aux.cancel_open(id);
            log::error!("failed to open settings window");
            let _ = self.dispatch_session(SessionEvent::SettingsOpenFailed);
            let ctx = self.frame_ctx();
            self.sync_chrome(&ctx, false);
        }
    }

    pub(crate) fn close_settings_window(
        &mut self,
        pending: Option<SettingsSnapshot>,
    ) -> Option<SettingsResult> {
        let result = if let Some(snapshot) = pending {
            match self.apply_settings(snapshot) {
                Ok(plan) => Some(plan),
                Err(error) => {
                    log::warn!("settings apply failed after close: {error}");
                    None
                }
            }
        } else {
            None
        };
        self.on_settings_closed();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::OpenRequest;
    use crate::runtime::host::TestHost;
    use crate::test_opener::{FakeOpener, OpenerCall};
    use crate::testkit::StubAux;

    #[test]
    fn pending_settings_open_skips_native_creation_and_pause_acquisition() {
        use crate::aux::focus::AuxKind;
        let mut host = TestHost::new().host;
        let OpenRequest::Create(id) = host.aux.request_open(AuxKind::Settings) else {
            panic!("expected reservation");
        };
        let opener = FakeOpener::failing("no display");
        host.open_settings_window(&opener);
        assert!(opener.calls().is_empty());
        assert_eq!(host.aux.route_id(AuxKind::Settings), Some(id));
        assert!(!host.pause_policy.paused());
        assert!(host.aux.cancel_open(id));
        host.open_settings_window(&opener);
        assert_eq!(opener.calls(), [OpenerCall::Settings]);
        assert!(!host.settings_open());
        assert!(!host.pause_policy.paused());
    }

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
    fn settings_close_releases_its_owner_even_after_registry_removal() {
        let support = TestHost::new();
        let mut host = support.host;
        host.set_pause_owner(nerust_frontend_kernel::pause::PauseOwner::Settings, true);
        host.on_settings_closed();
        assert!(!host.settings_open());
        assert!(!host.pause_policy.paused());
    }

    #[test]
    fn settings_close_with_open_window_syncs_chrome() {
        use crate::aux::focus::AuxKind;
        let support = TestHost::new();
        let mut host = support.host;
        host.aux.insert(Box::new(StubAux::new(AuxKind::Settings)));
        host.set_pause_owner(nerust_frontend_kernel::pause::PauseOwner::Settings, true);
        assert!(host.settings_open());
        // Registry removal is the caller's job; close teardown only
        // releases its pause owner and syncs chrome while still open.
        host.on_settings_closed();
        assert!(host.settings_open());
        assert!(!host.pause_policy.paused());
        assert_eq!(support.menu.updates().len(), 1);
    }

    #[test]
    fn settings_close_preserves_modal_owner() {
        use crate::aux::focus::AuxKind;
        let support = TestHost::new();
        let mut host = support.host;
        host.aux.insert(Box::new(StubAux::new(AuxKind::Settings)));
        host.set_pause_owner(nerust_frontend_kernel::pause::PauseOwner::Settings, true);
        host.set_pause_owner(nerust_frontend_kernel::pause::PauseOwner::Modal, true);
        host.on_settings_closed();
        assert!(host.settings_open());
        assert!(
            host.pause_policy
                .owns(nerust_frontend_kernel::pause::PauseOwner::Modal)
        );
        assert!(
            !host
                .pause_policy
                .owns(nerust_frontend_kernel::pause::PauseOwner::Settings)
        );
    }

    #[test]
    fn close_settings_window_applies_pending_snapshot() {
        let support = TestHost::new();
        let mut host = support.host;
        assert!(host.close_settings_window(None).is_none());
        let snapshot = host.settings_snapshot().clone();
        let plan = host
            .close_settings_window(Some(snapshot))
            .expect("identical snapshot applies cleanly");
        assert!(!plan.renderer_needs_rebuild);
    }
}
