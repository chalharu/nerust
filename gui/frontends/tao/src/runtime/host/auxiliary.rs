//! Native auxiliary creation and settings effects. Policy lives in ApplicationHost.
use super::HostState;
use nerust_frontend_host::{
    aux::{AuxId, OpenRequest},
    focus::AuxKind,
};

use nerust_gui_runtime::settings::SettingsSnapshot;
#[cfg(test)]
use nerust_gui_shell::session::SessionError;
use nerust_gui_shell::session::access::SettingsResult;

use crate::ports::{AuxOpenParams, WindowOpener};

impl HostState {
    pub(crate) fn reserve_aux_window(&mut self, kind: AuxKind) -> Option<OpenRequest> {
        match self.core.reserve_window(&mut self.aux.windows, kind) {
            Ok(request) => request,
            Err(error) => {
                log::warn!("auxiliary reservation failed: {error}");
                None
            }
        }
    }

    /// One native creation/completion path for every common auxiliary Window.
    pub(crate) fn create_aux_window(
        &mut self,
        id: AuxId,
        opener: &dyn WindowOpener,
        params: AuxOpenParams,
    ) -> bool {
        let Some(kind) = self.aux.windows.kind_of_id(id) else {
            return false;
        };
        if self.aux.windows.get(kind).is_some() {
            return false;
        }
        let window = self.aux.window(id).expect("current reservation").clone();
        let opened = opener
            .create_aux(window, params)
            .is_some_and(|handle| self.aux.complete_open(id, handle).is_ok());
        if !opened {
            self.core.cancel_window(&mut self.aux.windows, id);
        }
        opened
    }

    #[cfg(test)]
    pub(crate) fn apply_settings(
        &mut self,
        settings: SettingsSnapshot,
    ) -> Result<SettingsResult, SessionError> {
        let result = self.core.apply_settings(settings)?;
        self.apply_native_settings(&result);
        Ok(result)
    }

    fn apply_native_settings(&mut self, result: &SettingsResult) {
        if result.fullscreen_default_changed {
            self.sync_fullscreen_from_settings();
        }
        if result.scaling_changed {
            self.update_window_size_for_scaling();
        }
        let ctx = self.frame_ctx();
        self.sync_chrome(&ctx, true);
    }

    #[cfg(test)]
    pub(crate) fn on_settings_closed(&mut self) {
        let changed = self.core.settings_closed();
        if !changed {
            return;
        }
        let ctx = self.frame_ctx();
        self.sync_chrome(&ctx, false);
    }

    pub(super) fn open_settings_window(&mut self, opener: &dyn WindowOpener) {
        let Some(OpenRequest::Create(id)) = self.reserve_aux_window(AuxKind::Settings) else {
            return;
        };
        let controller = match self.core.prepare_settings_window(
            &mut self.aux.windows,
            id,
            self.ctx.registry.clone(),
            self.ctx.audio_registry.clone(),
        ) {
            Ok(Some(controller)) => controller,
            Ok(None) => return,
            Err(error) => {
                log::error!("refusing settings window: {error}");
                let ctx = self.frame_ctx();
                self.sync_chrome(&ctx, false);
                return;
            }
        };
        let program = crate::aux::widgets::bindings::SettingsBinding::from_controller(controller);
        let opened = self.create_aux_window(id, opener, AuxOpenParams::Settings { program });
        if !opened {
            log::error!("failed to open settings window");
            let ctx = self.frame_ctx();
            self.sync_chrome(&ctx, false);
        } else {
            // Publish only the settled native lifecycle state. A failed open
            // must not flash a guarded menu before its immediate rollback.
            let ctx = self.frame_ctx();
            self.sync_menu_state(&ctx);
        }
    }

    pub(crate) fn close_settings_window(
        &mut self,
        pending: Option<SettingsSnapshot>,
    ) -> Option<SettingsResult> {
        let closed = if let Some(id) = self.aux.route_id(crate::aux::focus::AuxKind::Settings) {
            let (handle, result) = self.core.close_window(&mut self.aux.windows, id, pending)?;
            drop(handle);
            result
        } else {
            self.core.finish_settings(pending)
        };
        let result = match closed {
            Ok(result) => result,
            Err(error) => {
                log::warn!("settings apply failed after close: {error}");
                None
            }
        };
        if let Some(plan) = &result {
            self.apply_native_settings(plan);
        }
        let ctx = self.frame_ctx();
        self.sync_chrome(&ctx, false);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::OpenRequest;
    use super::{AuxKind, AuxOpenParams};
    use crate::runtime::host::TestHost;
    use crate::test_opener::{FakeOpener, OpenerCall};
    use crate::testkit::StubAux;

    #[test]
    fn generic_creation_rejects_stale_and_active_ids_before_native_effects() {
        use nerust_settings_core::model::language::AppLanguage;
        let mut host = TestHost::new().host;
        let Some(OpenRequest::Create(old)) = host.reserve_aux_window(AuxKind::History) else {
            panic!("expected reservation");
        };
        assert!(host.core.cancel_window(&mut host.aux.windows, old));
        let current = host.aux.insert(Box::new(StubAux::new(AuxKind::History)));
        let opener = FakeOpener::failing("must not create a native window");
        for id in [old, current] {
            assert!(!host.create_aux_window(
                id,
                &opener,
                AuxOpenParams::History {
                    language: AppLanguage::English
                },
            ));
        }
        assert!(opener.calls().is_empty());
        assert_eq!(host.aux.route_id(AuxKind::History), Some(current));
    }

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
        assert!(!host.pause_held());
        assert!(host.aux.cancel_open(id));
        host.open_settings_window(&opener);
        assert_eq!(opener.calls(), [OpenerCall::Settings]);
        assert!(!host.settings_open());
        assert!(!host.pause_held());
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
        assert!(!host.pause_held());
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
        assert!(!host.pause_held());
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
        assert!(host.owns_pause(nerust_frontend_kernel::pause::PauseOwner::Modal));
        assert!(!host.owns_pause(nerust_frontend_kernel::pause::PauseOwner::Settings));
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
