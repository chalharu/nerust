//! Menu/title/fullscreen/settings chrome synced with the session.

use super::HostState;
use super::*;
use nerust_gui_shell::session::access::FrontendSession;

impl HostState {
    pub(crate) fn sync_fullscreen_default_from_window(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let fullscreen = window_is_fullscreen(window);
        let sync = derive_native_fullscreen_sync(
            self.pending_fullscreen_sync,
            fullscreen,
            self.session
                .settings_snapshot()
                .local
                .video
                .window
                .fullscreen_default,
        );
        self.pending_fullscreen_sync = sync.pending_target;
        if !sync.persist_setting {
            return;
        }
        match self.session.set_fullscreen_default(fullscreen) {
            Ok(plan) if plan.fullscreen_default_changed => {
                self.sync_menu_state();
                self.refresh_window_title();
                self.request_redraw();
            }
            Ok(_) => (),
            Err(error) => log::warn!("failed to persist fullscreen setting: {error}"),
        }
    }

    pub(crate) fn apply_settings(
        &mut self,
        settings: SettingsSnapshot,
    ) -> Result<SettingsResult, SessionError> {
        let result = FrontendSession::apply_settings(self, settings)?;
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
        self.settings_window = None;
        if !self.settings_open {
            return;
        }
        self.settings_open = false;
        let should_resume = std::mem::take(&mut self.resume_after_settings);
        if should_resume {
            self.resume();
        } else {
            self.sync_menu_state();
            self.refresh_window_title();
        }
    }

    pub(crate) fn sync_menu_state(&mut self) {
        let snapshot = self.session.settings_snapshot();
        let slots = self.session.slots();
        self.app_menu.update(crate::menu::MenuSync {
            loaded: self.session.loaded(),
            paused: self.session.paused(),
            slots,
            active_slot: self.session.active_slot_id(),
            settings_open: self.settings_open,
            language: snapshot.shared.general.language,
            shortcuts: &snapshot.shared.input.shortcuts.keyboard,
        });
    }

    pub(crate) fn refresh_window_title(&mut self) {
        if let Some(window) = self.window.as_ref() {
            window.set_title(&self.session.window_title());
        }
    }

    pub(super) fn maybe_refresh_window_title(&mut self, now: Instant) {
        if self.shell.should_refresh_title(now) {
            self.refresh_window_title();
        }
    }

    pub(super) fn toggle_fullscreen(&mut self) {
        let fullscreen = if let Some(window) = self.window.as_ref() {
            !window_is_fullscreen(window)
        } else {
            return;
        };
        if let Err(error) = self.persist_fullscreen_default(fullscreen) {
            log::warn!("failed to persist fullscreen setting: {error}");
            if let Some(window) = self.window.as_ref() {
                set_window_fullscreen(window, fullscreen);
            }
            self.request_redraw();
        }
    }

    pub(super) fn open_settings_window(&mut self, event_loop: &EventLoopWindowTarget<UserEvent>) {
        if self.settings_open {
            return;
        }
        self.settings_open = true;
        self.resume_after_settings = self.session.loaded() && !self.session.paused();
        if self.resume_after_settings {
            self.pause();
        } else {
            self.sync_menu_state();
        }

        match crate::aux::settings::window::SettingsWindowHandle::new(
            self.session.settings_snapshot().clone(),
            self.ctx.registry.clone(),
            self.ctx.audio_registry.clone(),
            event_loop,
        ) {
            Some(handle) => self.settings_window = Some(handle),
            None => {
                log::error!("failed to open settings window");
                self.settings_open = false;
                if self.resume_after_settings {
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

    pub(super) fn startup_window_size(&self) -> TaoLogicalSize<f64> {
        match scaling_factor(self.session.settings_snapshot().local.video.window.scaling) {
            Some(scale) => logical_window_size(self.session.window_size(), Some(scale)),
            None => self
                .remembered_fit_window_size()
                .map(logical_size_from_remembered)
                .unwrap_or_else(default_fit_window_size),
        }
    }

    fn update_window_size_for_scaling(&self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let Some(scale) =
            scaling_factor(self.session.settings_snapshot().local.video.window.scaling)
        else {
            return;
        };
        window.set_inner_size(logical_window_size(self.session.window_size(), Some(scale)));
    }

    pub(super) fn sync_fullscreen_from_settings(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let fullscreen = self
            .session
            .settings_snapshot()
            .local
            .video
            .window
            .fullscreen_default;
        if window_is_fullscreen(window) == fullscreen {
            self.pending_fullscreen_sync = None;
            return;
        }
        self.pending_fullscreen_sync = Some(fullscreen);
        set_window_fullscreen(window, fullscreen);
    }

    fn persist_fullscreen_default(
        &mut self,
        fullscreen: bool,
    ) -> Result<SettingsResult, SessionError> {
        let plan = self.session.set_fullscreen_default(fullscreen)?;
        self.sync_fullscreen_from_settings();
        self.sync_menu_state();
        self.refresh_window_title();
        self.request_redraw();
        Ok(SettingsResult {
            renderer_needs_rebuild: plan.renderer_rebuild_required
                || plan.session_rebuild_required
                || plan.window_settings_changed,
            fullscreen_default_changed: plan.fullscreen_default_changed,
            scaling_changed: false,
        })
    }

    fn remembered_fit_window_size(&self) -> Option<RememberedWindowSize> {
        self.session
            .settings_snapshot()
            .app_state
            .window_size("main")
    }

    pub(super) fn remember_fit_window_size(&self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        if window_is_fullscreen(window)
            || scaling_factor(self.session.settings_snapshot().local.video.window.scaling).is_some()
        {
            return;
        }

        let logical_size = window.inner_size().to_logical::<f64>(window.scale_factor());
        let width = logical_size.width.round().max(1.0) as u32;
        let height = logical_size.height.round().max(1.0) as u32;

        if let Err(error) = self
            .session
            .settings_manager()
            .update_window_size(width, height)
        {
            log::warn!("failed to remember tao window size: {error}");
        }
    }
}
