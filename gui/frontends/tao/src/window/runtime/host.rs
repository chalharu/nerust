use crate::app_menu::{MenuCommand, UserEvent, imp::AppMenu};
use nerust_backend_wgpu::RenderResult;
use nerust_contract_settings::input::{KeyboardKey, ShortcutAction};
use nerust_gui_runtime::rom::load_rom_path;
use nerust_gui_runtime::settings::HostBackendIdentity;
use nerust_gui_runtime::shell::NativeShellState;
use nerust_gui_session::commands::{SessionCommand, SessionCommandOutcome};
use nerust_gui_session::core::WindowSize;
use nerust_gui_shell::load::NesLoadOptions;
use nerust_gui_shell::session::{KeyboardShortcut, NesSession};
use nerust_gui_shell::settings::nes::scaling_factor;
use nerust_screen_wgpu::surface::SurfaceSize;
use rfd::FileDialog;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tao::{
    dpi::{LogicalSize as TaoLogicalSize, PhysicalSize as TaoPhysicalSize},
    event::{ElementState, KeyEvent},
    event_loop::{ControlFlow, EventLoopWindowTarget},
    keyboard::KeyCode,
    window::{Fullscreen, Window as TaoWindow, WindowBuilder, WindowId},
};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum HostAction {
    None,
    RomLoaded,
    Exit,
}

pub(crate) struct HostState {
    window: Option<Arc<TaoWindow>>,
    session: NesSession,
    app_menu: AppMenu,
    shell: NativeShellState,
    default_load_options: NesLoadOptions,
}

impl HostState {
    pub(crate) fn new(app_menu: AppMenu, default_load_options: NesLoadOptions) -> Self {
        Self {
            window: None,
            session: NesSession::new_for_host(HostBackendIdentity::tao_wgpu()),
            app_menu,
            shell: NativeShellState::new(),
            default_load_options,
        }
    }

    pub(crate) fn session(&self) -> &NesSession {
        &self.session
    }

    pub(crate) fn window(&self) -> Option<&Arc<TaoWindow>> {
        self.window.as_ref()
    }

    pub(crate) fn resume_session(&mut self) {
        self.session.resume();
    }

    pub(crate) fn ensure_window(&mut self, event_loop: &EventLoopWindowTarget<UserEvent>) {
        if self.window.is_some() {
            return;
        }

        let window = Arc::new(
            create_window_builder(
                self.session.window_size(),
                self.session.window_title(),
                scaling_factor(self.session.settings_snapshot().local.video.scaling),
            )
            .build(event_loop)
            .unwrap(),
        );
        self.app_menu.init_for_window(&window);
        if self
            .session
            .settings_snapshot()
            .local
            .video
            .fullscreen_default
        {
            window.set_fullscreen(Some(Fullscreen::Borderless(None)));
        }
        self.window = Some(window);
        self.sync_menu_state();
        self.request_redraw();
        self.refresh_window_title();
    }

    pub(crate) fn is_window(&self, window_id: WindowId) -> bool {
        self.window
            .as_ref()
            .is_some_and(|window| window.id() == window_id)
    }

    pub(crate) fn window_surface_size(&self) -> Option<SurfaceSize> {
        self.window
            .as_ref()
            .map(|window| window_surface_size(window.inner_size()))
    }

    pub(crate) fn load(&mut self, data: Vec<u8>) -> bool {
        self.load_with_options(None, data, self.default_load_options)
    }

    pub(crate) fn load_with_options(
        &mut self,
        rom_path: Option<PathBuf>,
        data: Vec<u8>,
        options: NesLoadOptions,
    ) -> bool {
        if self.session.load_with_options(rom_path, data, options) {
            self.session.resume();
            self.after_rom_load();
            true
        } else {
            false
        }
    }

    pub(crate) fn load_path(&mut self, path: &Path) -> bool {
        match load_rom_path(path) {
            Ok(loaded_rom) => {
                let (rom_path, data) = loaded_rom.into_parts();
                self.load_with_options(Some(rom_path), data, self.default_load_options)
            }
            Err(error) => {
                log::warn!("ROM open failed: {error}");
                false
            }
        }
    }

    pub(crate) fn on_menu_command(&mut self, command: UserEvent) -> HostAction {
        match command {
            UserEvent::Menu(MenuCommand::Open) => {
                if self.open_rom_dialog() {
                    HostAction::RomLoaded
                } else {
                    HostAction::None
                }
            }
            UserEvent::Menu(MenuCommand::Session(command)) => {
                self.apply_session_command(command);
                HostAction::None
            }
            UserEvent::Menu(MenuCommand::Quit) => {
                if self.prepare_close() {
                    HostAction::Exit
                } else {
                    HostAction::None
                }
            }
        }
    }

    pub(crate) fn on_keyboard_input(&mut self, input: KeyEvent) {
        if let Some(pressed) = element_state_to_pressed(input.state)
            && let Some(key) = keycode_controller_input(input.physical_key)
            && let Some(shortcut) = self.session.handle_keyboard_key(key, pressed)
        {
            self.apply_keyboard_shortcut(shortcut);
        }
    }

    pub(crate) fn clear_keys(&mut self) {
        self.session.clear_controller_input();
    }

    pub(crate) fn update_control_flow(&mut self, control_flow: &mut ControlFlow) {
        let now = Instant::now();
        self.maybe_refresh_window_title(now);

        let Some(window) = self.window.as_ref() else {
            *control_flow = ControlFlow::Wait;
            return;
        };

        let metrics = self.session.metrics();
        if self.shell.wants_redraw(metrics.frame_counter) {
            window.request_redraw();
        }

        if self.shell.wants_poll(metrics.loaded, metrics.paused) {
            *control_flow = ControlFlow::WaitUntil(now + NativeShellState::FRAME_POLL_INTERVAL);
        } else {
            *control_flow = ControlFlow::Wait;
        }
    }

    pub(crate) fn on_render_result(&mut self, result: RenderResult) {
        match result {
            RenderResult::Presented => {
                self.shell
                    .on_frame_presented(self.session.metrics().frame_counter);
                self.maybe_refresh_window_title(Instant::now());
            }
            RenderResult::Skipped | RenderResult::Error => {
                self.shell.needs_redraw = true;
            }
        }
    }

    pub(crate) fn clear_event_handler(&self) {
        self.app_menu.clear_event_handler();
    }

    pub(crate) fn request_redraw(&mut self) {
        self.shell.needs_redraw = true;
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    pub(crate) fn prepare_close(&mut self) -> bool {
        self.session.flush_before_exit();
        true
    }

    fn open_rom_dialog(&mut self) -> bool {
        FileDialog::new()
            .set_title("Open ROM")
            .add_filter("NES ROM", &["nes", "zip"])
            .pick_file()
            .is_some_and(|path| self.load_path(&path))
    }

    fn after_rom_load(&mut self) {
        self.sync_menu_state();
        self.request_redraw();
        self.refresh_window_title();
    }

    fn sync_menu_state(&mut self) {
        self.app_menu.update(
            self.session.loaded(),
            self.session.paused(),
            self.session.slots(),
            self.session.active_slot_id(),
        );
    }

    fn apply_session_command(&mut self, command: SessionCommand) {
        let outcome = self.session.run_command(command);
        self.apply_command_outcome(outcome);
    }

    fn apply_command_outcome(&mut self, outcome: SessionCommandOutcome) {
        if outcome.needs_redraw {
            self.request_redraw();
        }
        self.sync_menu_state();
        self.refresh_window_title();
    }

    fn refresh_window_title(&mut self) {
        if let Some(window) = self.window.as_ref() {
            window.set_title(&self.session.window_title());
        }
    }

    fn maybe_refresh_window_title(&mut self, now: Instant) {
        if self.shell.should_refresh_title(now) {
            self.refresh_window_title();
        }
    }

    fn apply_keyboard_shortcut(&mut self, shortcut: KeyboardShortcut) {
        match shortcut {
            KeyboardShortcut::Session(action) => match action {
                ShortcutAction::TogglePause => {
                    self.apply_session_command(SessionCommand::TogglePause)
                }
                ShortcutAction::SaveActiveSlot => {
                    self.apply_session_command(SessionCommand::SaveActiveSlotOrNew)
                }
                ShortcutAction::SelectNextSlot => {
                    self.apply_session_command(SessionCommand::SelectNextSlot)
                }
                ShortcutAction::SelectPreviousSlot => {
                    self.apply_session_command(SessionCommand::SelectPreviousSlot)
                }
                ShortcutAction::LoadActiveSlot => {
                    self.apply_session_command(SessionCommand::LoadActiveSlot)
                }
                ShortcutAction::Reset => self.apply_session_command(SessionCommand::Reset),
                ShortcutAction::ToggleFullscreen => self.toggle_fullscreen(),
            },
            KeyboardShortcut::ToggleFullscreen => self.toggle_fullscreen(),
        }
    }

    fn toggle_fullscreen(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        if window.fullscreen().is_some() {
            window.set_fullscreen(None);
        } else {
            window.set_fullscreen(Some(Fullscreen::Borderless(None)));
        }
    }
}

fn create_window_builder(size: WindowSize, title: String, scaling: Option<u32>) -> WindowBuilder {
    let (width, height) = scaling
        .map(|scale| {
            (
                f64::from(size.width) * f64::from(scale),
                f64::from(size.height) * f64::from(scale),
            )
        })
        .unwrap_or((f64::from(size.width), f64::from(size.height)));
    WindowBuilder::new()
        .with_title(title)
        .with_inner_size(TaoLogicalSize::new(width, height))
}

fn window_surface_size(size: TaoPhysicalSize<u32>) -> SurfaceSize {
    SurfaceSize::new(size.width, size.height)
}

fn keycode_controller_input(code: KeyCode) -> Option<KeyboardKey> {
    Some(match code {
        KeyCode::Digit0 => KeyboardKey::Digit0,
        KeyCode::Digit1 => KeyboardKey::Digit1,
        KeyCode::Digit2 => KeyboardKey::Digit2,
        KeyCode::Digit3 => KeyboardKey::Digit3,
        KeyCode::Digit4 => KeyboardKey::Digit4,
        KeyCode::Digit5 => KeyboardKey::Digit5,
        KeyCode::Digit6 => KeyboardKey::Digit6,
        KeyCode::Digit7 => KeyboardKey::Digit7,
        KeyCode::Digit8 => KeyboardKey::Digit8,
        KeyCode::Digit9 => KeyboardKey::Digit9,
        KeyCode::KeyA => KeyboardKey::KeyA,
        KeyCode::KeyB => KeyboardKey::KeyB,
        KeyCode::KeyC => KeyboardKey::KeyC,
        KeyCode::KeyD => KeyboardKey::KeyD,
        KeyCode::KeyE => KeyboardKey::KeyE,
        KeyCode::KeyF => KeyboardKey::KeyF,
        KeyCode::KeyG => KeyboardKey::KeyG,
        KeyCode::KeyH => KeyboardKey::KeyH,
        KeyCode::KeyI => KeyboardKey::KeyI,
        KeyCode::KeyJ => KeyboardKey::KeyJ,
        KeyCode::KeyK => KeyboardKey::KeyK,
        KeyCode::KeyL => KeyboardKey::KeyL,
        KeyCode::KeyM => KeyboardKey::KeyM,
        KeyCode::KeyN => KeyboardKey::KeyN,
        KeyCode::KeyO => KeyboardKey::KeyO,
        KeyCode::KeyP => KeyboardKey::KeyP,
        KeyCode::KeyQ => KeyboardKey::KeyQ,
        KeyCode::KeyR => KeyboardKey::KeyR,
        KeyCode::KeyS => KeyboardKey::KeyS,
        KeyCode::KeyT => KeyboardKey::KeyT,
        KeyCode::KeyU => KeyboardKey::KeyU,
        KeyCode::KeyV => KeyboardKey::KeyV,
        KeyCode::KeyW => KeyboardKey::KeyW,
        KeyCode::KeyZ => KeyboardKey::KeyZ,
        KeyCode::KeyX => KeyboardKey::KeyX,
        KeyCode::KeyY => KeyboardKey::KeyY,
        KeyCode::ArrowUp => KeyboardKey::ArrowUp,
        KeyCode::ArrowDown => KeyboardKey::ArrowDown,
        KeyCode::ArrowLeft => KeyboardKey::ArrowLeft,
        KeyCode::ArrowRight => KeyboardKey::ArrowRight,
        KeyCode::Enter => KeyboardKey::Enter,
        KeyCode::Escape => KeyboardKey::Escape,
        KeyCode::Space => KeyboardKey::Space,
        KeyCode::Tab => KeyboardKey::Tab,
        KeyCode::F1 => KeyboardKey::F1,
        KeyCode::F2 => KeyboardKey::F2,
        KeyCode::F3 => KeyboardKey::F3,
        KeyCode::F4 => KeyboardKey::F4,
        KeyCode::F5 => KeyboardKey::F5,
        KeyCode::F6 => KeyboardKey::F6,
        KeyCode::F7 => KeyboardKey::F7,
        KeyCode::F8 => KeyboardKey::F8,
        KeyCode::F9 => KeyboardKey::F9,
        KeyCode::F10 => KeyboardKey::F10,
        KeyCode::F11 => KeyboardKey::F11,
        KeyCode::F12 => KeyboardKey::F12,
        _ => return None,
    })
}

fn element_state_to_pressed(state: ElementState) -> Option<bool> {
    Some(match state {
        ElementState::Pressed => true,
        ElementState::Released => false,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::keycode_controller_input;
    use nerust_contract_settings::input::KeyboardKey;
    use tao::keyboard::KeyCode;

    #[test]
    fn keycode_mapping_matches_controller_layout() {
        assert_eq!(
            keycode_controller_input(KeyCode::KeyZ),
            Some(KeyboardKey::KeyZ)
        );
        assert_eq!(
            keycode_controller_input(KeyCode::KeyX),
            Some(KeyboardKey::KeyX)
        );
        assert_eq!(
            keycode_controller_input(KeyCode::ArrowUp),
            Some(KeyboardKey::ArrowUp)
        );
        assert_eq!(
            keycode_controller_input(KeyCode::ArrowRight),
            Some(KeyboardKey::ArrowRight)
        );
        assert_eq!(
            keycode_controller_input(KeyCode::Enter),
            Some(KeyboardKey::Enter)
        );
        assert_eq!(
            keycode_controller_input(KeyCode::Digit1),
            Some(KeyboardKey::Digit1)
        );
    }
}
