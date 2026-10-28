//! Native auxiliary creation and settings effects. Policy lives in ApplicationHost.
use super::HostState;
use nerust_frontend_host::{
    aux::{AuxId, OpenRequest},
    focus::AuxKind,
};

use nerust_frontend_host::{widget_session::WidgetSession, window::WindowEffects};

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

    pub(crate) fn apply_window_effects(&mut self, effects: WindowEffects) -> bool {
        if let Some(fullscreen) = effects.fullscreen
            && let Some(window) = self.windows.live()
        {
            window.set_fullscreen(super::fullscreen_mode(fullscreen));
        }
        if let Some((width, height)) = effects.resize
            && let Some(window) = self.windows.live()
        {
            let extra = self.windows.menu_height();
            window.set_inner_size(tao::dpi::LogicalSize::new(
                f64::from(width),
                f64::from(height) + extra,
            ));
        }
        if effects.refresh_chrome {
            let ctx = self.frame_ctx();
            self.sync_chrome(&ctx, effects.redraw);
        } else if effects.redraw {
            self.request_redraw();
        }
        effects.rebuild_renderer
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
        let Some(session) = self.prepare_widget_session(id) else {
            return;
        };
        let opened = self.create_aux_window(id, opener, AuxOpenParams::Controls { session });
        if !opened {
            log::error!("failed to open settings window");
            let ctx = self.frame_ctx();
            self.sync_chrome(&ctx, false);
        } else {
            let ctx = self.frame_ctx();
            self.sync_menu_state(&ctx);
        }
    }

    pub(crate) fn prepare_widget_session(&mut self, id: AuxId) -> Option<WidgetSession> {
        match self.core.prepare_widget_window(&mut self.aux.windows, id) {
            Ok(session) => session,
            Err(error) => {
                log::error!("refusing settings window: {error}");
                let ctx = self.frame_ctx();
                self.sync_chrome(&ctx, false);
                None
            }
        }
    }

    /// The caller retains the original generation across native dispatch.
    pub(crate) fn finish_widget_window(&mut self, id: AuxId, close: bool) -> bool {
        if self.aux.windows.kind_of_id(id).is_none() {
            return false;
        }
        let pending = self
            .aux
            .windows
            .mailbox(id)
            .and_then(|m| m.take_submission());
        let result = if close {
            let Some((handle, result)) =
                self.core
                    .close_widget_window(&mut self.aux.windows, id, pending)
            else {
                return false;
            };
            drop(handle);
            result
        } else if let Some(pending) = pending {
            self.core
                .apply_window_submission(&self.aux.windows, id, pending)
        } else {
            return false;
        };
        let effects = match result {
            Ok(effects) => effects,
            Err(error) => {
                log::warn!("widget submission failed: {error}");
                self.core.report_host_error(error.to_string());
                WindowEffects {
                    refresh_chrome: close,
                    ..Default::default()
                }
            }
        };
        if !close {
            self.aux.refresh_widget(id, &mut self.core);
        }
        self.apply_window_effects(effects)
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
                AuxOpenParams::Controls {
                    session: {
                        // This stale/active check runs before consuming native parameters.
                        let mut other = TestHost::new().host;
                        crate::aux::widgets::bindings::tests::prepare(&mut other, AuxKind::History)
                            .1
                    }
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
        host.set_pause_owner(wakui::pause::PauseOwner::Settings, true);
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
        host.set_pause_owner(wakui::pause::PauseOwner::Settings, true);
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
        host.set_pause_owner(wakui::pause::PauseOwner::Settings, true);
        host.set_pause_owner(wakui::pause::PauseOwner::Modal, true);
        host.on_settings_closed();
        assert!(host.settings_open());
        assert!(host.owns_pause(wakui::pause::PauseOwner::Modal));
        assert!(!host.owns_pause(wakui::pause::PauseOwner::Settings));
    }

    #[test]
    fn close_settings_window_applies_pending_submission() {
        let support = TestHost::new();
        let mut host = support.host;
        let (id, mut session) =
            crate::aux::widgets::bindings::tests::prepare(&mut host, AuxKind::Settings);
        crate::aux::widgets::bindings::tests::activate(&mut session, "OK");
        assert!(session.mailbox().has_pending());
        assert!(!host.finish_widget_window(id, true));
        assert!(!host.settings_open());
        assert!(!session.mailbox().has_pending());
    }

    #[test]
    fn stale_close_cannot_consume_reopened_settings_submission_or_release_pause() {
        use crate::aux::widgets::bindings::tests::{activate, prepare};
        use wakui::pause::PauseOwner;

        let mut host = TestHost::new().host;
        let (old, _) = prepare(&mut host, AuxKind::Settings);
        assert!(!host.finish_widget_window(old, true));
        let (current, mut session) = prepare(&mut host, AuxKind::Settings);
        assert_ne!(old, current);
        activate(&mut session, "OK");
        assert!(session.mailbox().has_pending());
        for close in [false, true] {
            assert!(!host.finish_widget_window(old, close));
            assert!(session.mailbox().has_pending());
            assert_eq!(host.aux.route_id(AuxKind::Settings), Some(current));
            assert!(host.owns_pause(PauseOwner::Settings));
        }
        assert!(!host.finish_widget_window(current, true));
        assert!(!session.mailbox().has_pending());
        assert!(!host.owns_pause(PauseOwner::Settings));
        assert!(!host.settings_open());
    }

    #[test]
    fn failed_opaque_save_closes_and_releases_only_settings_pause() {
        use crate::aux::widgets::bindings::tests::{activate, find, prepare};
        use wakui::control::{ControlEvent, ControlInput};
        use wakui::pause::PauseOwner;

        let context = crate::test_session::TestContext::with_loader(
            crate::test_session::FakeRomLoader::succeeding(),
        );
        let root = context.settings_root();
        let path = root.join("config").join("settings.yaml");
        let boot = nerust_frontend_host::bootstrap::boot(
            context.ctx,
            nerust_frontend_host::bootstrap::backend_capabilities(true, Some(true)),
            Some(root),
        )
        .unwrap();
        let mut host = super::HostState::new(
            boot,
            crate::test_menu::MenuRecorderHandle::default().into_box(),
            std::sync::Arc::new(crate::test_dialogs::FakeDialogs::default()),
        );
        let (id, mut session) = prepare(&mut host, AuxKind::Settings);
        host.set_pause_owner(PauseOwner::Modal, true);
        let mute = find(&session.screen(), "Mute");
        session.dispatch(ControlEvent::new(mute.id, ControlInput::Bool(true)));
        activate(&mut session, "OK");
        if path.is_file() {
            std::fs::remove_file(&path).unwrap();
        }
        std::fs::create_dir_all(&path).unwrap();
        assert!(session.mailbox().has_pending());
        assert!(!host.finish_widget_window(id, true));
        assert!(!host.settings_open());
        assert!(host.aux.window(id).is_none());
        assert!(!host.owns_pause(PauseOwner::Settings));
        assert!(host.owns_pause(PauseOwner::Modal));
        assert!(!session.mailbox().has_pending());
        assert!(
            session.screen().message.is_some(),
            "real save failure published"
        );
        assert!(host.session().notification_history().count() > 0);
        std::fs::remove_dir(&path).unwrap();
    }
}
