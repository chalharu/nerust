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
        self.sync_menu_state();
        self.refresh_window_title();
        self.request_redraw();
        Ok(result)
    }

    pub(crate) fn on_settings_closed(&mut self) {
        if self.settings.window.is_none() {
            return;
        }
        self.settings.window = None;
        let should_resume = std::mem::take(&mut self.settings.resume_after);
        if should_resume {
            self.resume();
        } else {
            self.sync_menu_state();
            self.refresh_window_title();
        }
    }

    pub(super) fn open_settings_window(&mut self, opener: &dyn WindowOpener) {
        if self.settings_open() {
            return;
        }
        self.settings.resume_after = self.session.loaded() && !self.session.paused();
        if self.settings.resume_after {
            self.pause();
        } else {
            self.sync_menu_state();
        }

        match opener.create_settings(
            self.session.settings_snapshot().clone(),
            self.ctx.registry.clone(),
            self.ctx.audio_registry.clone(),
        ) {
            Some(handle) => self.settings.window = Some(handle),
            None => {
                log::error!("failed to open settings window");
                if self.settings.resume_after {
                    self.resume();
                } else {
                    self.sync_menu_state();
                }
            }
        }
    }

    pub(crate) fn close_settings_window(
        &mut self,
        mut handle: crate::aux::settings::window::SettingsWindowHandle,
    ) -> Option<SettingsResult> {
        let pending = handle.take_pending_apply();
        drop(handle);
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

    #[test]
    fn open_settings_attempt_records_request_and_stays_closed() {
        // Single truth is the handle: headless open fails, so no handle
        // exists and the guard stays closed.
        let support = TestHost::new();
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        host.open_settings_window(&opener);
        assert_eq!(opener.calls(), [OpenerCall::Settings]);
        assert!(host.settings.window.is_none());
        assert!(!host.settings_open());
    }

    #[test]
    fn open_settings_failure_rolls_back() {
        let support = TestHost::new();
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        host.open_settings_window(&opener);
        assert_eq!(opener.calls(), [OpenerCall::Settings]);
        assert!(host.settings.window.is_none());
        assert!(!host.settings_open());
        // Failure syncs closed around the attempt (pre + rollback).
        let open_flags: Vec<bool> = support
            .menu
            .updates()
            .iter()
            .map(|sync| sync.settings_open)
            .collect();
        assert_eq!(open_flags, [false, false]);
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
        host.settings.resume_after = true;
        host.on_settings_closed();
        assert!(!host.settings_open());
        assert!(host.settings.resume_after);
        assert!(support.menu.updates().is_empty());
    }
}
