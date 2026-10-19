//! Session presentation sync: menu, title, and fullscreen state.
//! Window sizing helpers live in `window`; the settings dialog in
//! `settings`.
use super::HostState;
use super::{derive_native_fullscreen_sync, set_window_fullscreen, window_is_fullscreen};
use nerust_gui_shell::session::{SessionError, access::SettingsResult};
use std::time::Instant;

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

    pub(crate) fn sync_menu_state(&mut self) {
        let snapshot = self.session.settings_snapshot();
        let slots = self.session.slots().to_vec();
        let tick = self.session.tick_metrics();
        let sig = super::ChromeSig {
            loaded: tick.loaded,
            paused: tick.paused,
            settings_open: self.settings_open(),
            language: snapshot.shared.general.language,
            active_slot: self.session.active_slot_id(),
            slots,
            shortcuts: snapshot.shared.input.shortcuts.keyboard.clone(),
            title: self.session.window_title(),
        };
        if self.last_chrome_sig.as_ref() == Some(&sig) {
            return;
        }
        let title = sig.title.clone();
        self.app_menu.update(crate::menu::MenuSync {
            loaded: sig.loaded,
            paused: sig.paused,
            slots: &sig.slots,
            active_slot: sig.active_slot,
            settings_open: sig.settings_open,
            language: sig.language,
            shortcuts: &sig.shortcuts,
        });
        self.last_chrome_sig = Some(sig);
        self.push_title(&title);
    }

    fn push_title(&mut self, title: &str) {
        if let Some(window) = self.window.as_ref() {
            window.set_title(title);
        }
    }

    pub(crate) fn refresh_window_title(&mut self) {
        let title = self.session.window_title();
        let changed = self
            .last_chrome_sig
            .as_ref()
            .is_none_or(|sig| sig.title != title);
        if changed {
            // Keep the signature truthful when the title moves alone
            // (500ms throttle path); the menu half refreshes on its
            // own next change.
            if let Some(sig) = self.last_chrome_sig.as_mut() {
                sig.title = title.clone();
            }
            self.push_title(&title);
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

    /// Refresh chrome after a ROM (re)load: menu, title, frame.
    pub(super) fn after_rom_load(&mut self) {
        self.sync_chrome(true);
    }

    /// Single chrome-sync choke point: menu, title, and optionally a
    /// frame. Call sites that re-derived the triad in varying order
    /// converge here; omitting the redraw stays explicit at the call.
    pub(crate) fn sync_chrome(&mut self, redraw: bool) {
        self.sync_menu_state();
        self.refresh_window_title();
        if redraw {
            self.request_redraw();
        }
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

    pub(super) fn persist_fullscreen_default(
        &mut self,
        fullscreen: bool,
    ) -> Result<SettingsResult, SessionError> {
        let plan = self.session.set_fullscreen_default(fullscreen)?;
        self.sync_fullscreen_from_settings();
        self.sync_chrome(true);
        Ok(SettingsResult {
            renderer_needs_rebuild: plan.renderer_rebuild_required
                || plan.session_rebuild_required
                || plan.window_settings_changed,
            fullscreen_default_changed: plan.fullscreen_default_changed,
            scaling_changed: false,
        })
    }
}
