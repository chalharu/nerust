//! Session presentation sync: menu, title, and fullscreen state.
use super::HostState;
use super::{
    ChromeScalars, MenuSig, derive_native_fullscreen_sync, set_window_fullscreen,
    window_is_fullscreen,
};
use nerust_gui_shell::session::{SessionError, access::SettingsResult};
use std::time::Instant;

impl HostState {
    pub(crate) fn sync_fullscreen_default_from_window(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let fullscreen = window_is_fullscreen(window.raw());
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
        let settings_open = self.settings_open();
        let snapshot = self.session.settings_snapshot();
        let language = snapshot.shared.general.language;
        let active_slot = self.session.active_slot_id();
        let tick = self.session.tick_metrics();
        let title = self.session.window_title(&tick);
        let slots = self.session.slots();
        let shortcuts = &snapshot.shared.input.shortcuts.keyboard;
        // Cheap gate first: scalars plus lengths, no `Arc` clones.
        if self.chrome.cheap_same(&ChromeScalars {
            loaded: tick.loaded,
            paused: tick.paused,
            settings_open,
            language,
            active_slot,
            slots_len: slots.len(),
            shortcuts_len: shortcuts.len(),
            title: &title,
        }) {
            let generation = MenuSig::fingerprint(slots, shortcuts);
            if self.chrome.menu_same(generation, slots, shortcuts) {
                return;
            }
        }
        // Changed: clone the slices into `Arc`s once and push both
        // halves through their own gates.
        let generation = MenuSig::fingerprint(slots, shortcuts);
        let sig = MenuSig {
            loaded: tick.loaded,
            paused: tick.paused,
            settings_open,
            language,
            active_slot,
            slots: slots.to_vec().into(),
            shortcuts: shortcuts.to_vec().into(),
            generation,
        };
        self.app_menu.update(crate::menu::MenuSync {
            loaded: sig.loaded,
            paused: sig.paused,
            slots: &sig.slots,
            active_slot: sig.active_slot,
            settings_open: sig.settings_open,
            language: sig.language,
            shortcuts: &sig.shortcuts,
        });
        self.chrome.push_menu(sig);
        if !self.chrome.title_same(&title) {
            self.chrome.push_title(title.clone());
            self.push_title(&title);
        }
    }

    fn push_title(&mut self, title: &str) {
        if let Some(window) = self.window.as_ref() {
            window.set_title(title);
        }
    }

    pub(crate) fn refresh_window_title(&mut self) {
        // Title-only gate: never touches the menu signature, so the
        // 500ms throttle path cannot mark menu state it did not push.
        let tick = self.session.tick_metrics();
        let title = self.session.window_title(&tick);
        if !self.chrome.title_same(&title) {
            self.chrome.push_title(title.clone());
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
            !window_is_fullscreen(window.raw())
        } else {
            return;
        };
        if let Err(error) = self.persist_fullscreen_default(fullscreen) {
            log::warn!("failed to persist fullscreen setting: {error}");
            if let Some(window) = self.window.as_ref() {
                set_window_fullscreen(window.raw(), fullscreen);
            }
            self.request_redraw();
        }
    }

    /// Refresh chrome after a ROM (re)load: menu, title, frame.
    pub(super) fn after_rom_load(&mut self) {
        self.sync_chrome(true);
    }

    /// Single chrome-sync choke point: menu, title, and optionally a frame.
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
        if window_is_fullscreen(window.raw()) == fullscreen {
            self.pending_fullscreen_sync = None;
            return;
        }
        self.pending_fullscreen_sync = Some(fullscreen);
        set_window_fullscreen(window.raw(), fullscreen);
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
