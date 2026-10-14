//! Menu/keyboard dispatch, ROM lifecycle, and the session delegate.

use super::HostState;
use super::*;
use nerust_gui_shell::session::access::FrontendSession;

impl HostState {
    pub(crate) fn on_menu_command(
        &mut self,
        command: MenuCommand,
        event_loop: &EventLoopWindowTarget<UserEvent>,
    ) -> HostAction {
        if self.settings_open {
            return match command {
                MenuCommand::Quit => {
                    if self.prepare_close() {
                        HostAction::Exit
                    } else {
                        HostAction::None
                    }
                }
                _ => HostAction::None,
            };
        }
        match command {
            MenuCommand::Open => {
                if self.open_rom_dialog() {
                    HostAction::RomLoaded
                } else {
                    HostAction::None
                }
            }
            MenuCommand::Close => {
                self.close_rom();
                HostAction::None
            }
            MenuCommand::Settings => {
                self.open_settings_window(event_loop);
                HostAction::None
            }
            MenuCommand::Debugger => {
                self.open_debug_windows(event_loop, false);
                HostAction::None
            }
            MenuCommand::Session(command) => {
                self.run_command(command);
                self.sync_menu_state();
                self.refresh_window_title();
                HostAction::None
            }
            MenuCommand::Quit => {
                if self.prepare_close() {
                    HostAction::Exit
                } else {
                    HostAction::None
                }
            }
        }
    }

    pub(crate) fn on_keyboard_input(&mut self, input: KeyEvent) {
        if self.settings_open {
            return;
        }
        if let Some(pressed) = element_state_to_pressed(input.state)
            && let Some(key) = crate::convert::keycode_to_key(input.physical_key)
            && let Some(shortcut) = self.session.handle_keyboard_key(key, pressed)
        {
            self.apply_keyboard_shortcut(shortcut);
        }
    }

    pub(crate) fn clear_keys(&mut self) {
        self.session.clear_input();
    }

    fn open_rom_dialog(&mut self) -> bool {
        let mut dialog = FileDialog::new().set_title(text(
            self.session.settings_snapshot().shared.general.language,
            UiText::Open,
        ));
        for factory in self.ctx.registry.all() {
            let extensions = factory.supported_extensions();
            if !extensions.is_empty() {
                dialog = dialog.add_filter(format!("{} ROM", factory.display_name()), extensions);
            }
        }
        let Some(path) = dialog.pick_file() else {
            return false;
        };
        if let Err(detail) = self.load_path(&path) {
            show_rom_load_error(
                self.session.settings_snapshot().shared.general.language,
                &detail,
            );
            return false;
        }
        true
    }

    pub(super) fn after_rom_load(&mut self) {
        self.sync_menu_state();
        self.request_redraw();
        self.refresh_window_title();
    }

    fn close_rom(&mut self) {
        FrontendSession::unload(self);
        self.sync_menu_state();
        self.request_redraw();
        self.refresh_window_title();
    }

    fn apply_keyboard_shortcut(&mut self, shortcut: KeyboardShortcut) {
        use nerust_gui_shell::session::access::dispatch_session_action;
        match shortcut {
            // The debugger open rides the same schema as every other
            // shortcut (default F4, rebindable in Settings): it only
            // sets the flag here because opening needs the event loop,
            // which the runtime arm owns.
            KeyboardShortcut::OpenDebugger => {
                self.debug.request_debug_open();
                return;
            }
            KeyboardShortcut::Session(action) => dispatch_session_action(self, action),
            KeyboardShortcut::ToggleFullscreen => self.toggle_fullscreen(),
        }
        self.sync_menu_state();
        self.refresh_window_title();
    }
}

impl FrontendSession for HostState {
    fn pause(&mut self) {
        if let Err(e) = self.session.run_command(SessionCommand::Pause) {
            log::warn!("session command failed: {e}");
        }
    }
    fn resume(&mut self) {
        if let Err(e) = self.session.run_command(SessionCommand::Resume) {
            log::warn!("session command failed: {e}");
        }
    }
    fn toggle_pause(&mut self) {
        if let Err(e) = self.session.run_command(SessionCommand::TogglePause) {
            log::warn!("session command failed: {e}");
        }
    }
    fn save_active_slot(&mut self) -> bool {
        self.session
            .run_command(SessionCommand::SaveActiveSlotOrNew)
            .unwrap_or_default()
            .executed
    }
    fn load_active_slot(&mut self) -> bool {
        self.session
            .run_command(SessionCommand::LoadActiveSlot)
            .unwrap_or_default()
            .executed
    }
    fn select_next_slot(&mut self) {
        if let Err(e) = self.session.run_command(SessionCommand::SelectNextSlot) {
            log::warn!("session command failed: {e}");
        }
    }
    fn select_previous_slot(&mut self) {
        if let Err(e) = self.session.run_command(SessionCommand::SelectPreviousSlot) {
            log::warn!("session command failed: {e}");
        }
    }
    fn load_slot(&mut self, slot_id: u64) -> bool {
        self.session
            .run_command(SessionCommand::LoadSlot(slot_id))
            .unwrap_or_default()
            .executed
    }
    fn save_slot(&mut self, slot_id: u64) {
        if let Err(e) = self.session.run_command(SessionCommand::SaveSlot(slot_id)) {
            log::warn!("session command failed: {e}");
        }
    }
    fn delete_slot(&mut self, slot_id: u64) {
        if let Err(e) = self
            .session
            .run_command(SessionCommand::DeleteSlot(slot_id))
        {
            log::warn!("session command failed: {e}");
        }
    }
    fn select_slot(&mut self, slot_id: u64) {
        if let Err(e) = self
            .session
            .run_command(SessionCommand::SelectActiveSlot(slot_id))
        {
            log::warn!("session command failed: {e}");
        }
    }
    fn create_slot(&mut self) {
        if let Err(e) = self.session.run_command(SessionCommand::CreateSlot) {
            log::warn!("session command failed: {e}");
        }
    }
    fn reset(&mut self) {
        if let Err(e) = self.session.run_command(SessionCommand::Reset) {
            log::warn!("session command failed: {e}");
        }
    }
    fn unload(&mut self) {
        if let Err(error) = self.session.unload() {
            log::warn!("ROM unload failed: {error}");
        }
    }
    fn run_command(&mut self, command: SessionCommand) {
        if let Err(e) = self.session.run_command(command) {
            log::warn!("session command failed: {e}");
        }
    }
    fn apply_settings(
        &mut self,
        settings: SettingsSnapshot,
    ) -> Result<SettingsResult, SessionError> {
        let plan = self.session.apply_settings(settings)?;
        Ok(SettingsResult {
            // The renderer follows presentation, not the session: a
            // live filter swap rebuilds no core but still needs the new
            // GPU assets (same rule as the android twin).
            renderer_needs_rebuild: plan.renderer_rebuild_required
                || plan.session_rebuild_required
                || plan.window_settings_changed,
            fullscreen_default_changed: plan.fullscreen_default_changed,
            scaling_changed: plan.scaling_changed,
        })
    }
    fn set_fullscreen_default(&mut self, fullscreen: bool) -> Result<SettingsResult, SessionError> {
        let plan = self.session.set_fullscreen_default(fullscreen)?;
        Ok(SettingsResult {
            renderer_needs_rebuild: plan.renderer_rebuild_required
                || plan.session_rebuild_required
                || plan.window_settings_changed,
            fullscreen_default_changed: plan.fullscreen_default_changed,
            scaling_changed: false,
        })
    }
}
