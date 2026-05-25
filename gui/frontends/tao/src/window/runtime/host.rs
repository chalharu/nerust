use crate::app_menu::{MenuCommand, UserEvent, imp::AppMenu};
use nerust_backend_wgpu::RenderResult;
use nerust_gui_runtime::rom::load_rom_path;
use nerust_gui_runtime::shell::NativeShellState;
use nerust_gui_session::commands::{SessionCommand, SessionCommandOutcome};
use nerust_gui_session::core::WindowSize;
use nerust_gui_shell::load::NesLoadOptions;
use nerust_gui_shell::session::NesSession;
use nerust_gui_shell::session::input::NesButton;
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
    window::{Window as TaoWindow, WindowBuilder, WindowId},
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
            session: NesSession::new(),
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
            create_window_builder(self.session.window_size(), self.session.window_title())
                .build(event_loop)
                .unwrap(),
        );
        self.app_menu.init_for_window(&window);
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
        let code = match input.physical_key {
            KeyCode::Space if input.state == ElementState::Pressed && !input.repeat => {
                self.apply_session_command(SessionCommand::TogglePause);
                None
            }
            KeyCode::Escape if input.state == ElementState::Released => {
                self.apply_session_command(SessionCommand::Reset);
                None
            }
            KeyCode::F5 if input.state == ElementState::Released && !input.repeat => {
                self.apply_session_command(SessionCommand::SaveActiveSlotOrNew);
                None
            }
            KeyCode::F8 if input.state == ElementState::Released && !input.repeat => {
                self.apply_session_command(SessionCommand::LoadActiveSlot);
                None
            }
            code => keycode_controller_input(code),
        };

        if let Some(pressed) = element_state_to_pressed(input.state)
            && let Some(controller_input) = code
        {
            self.session
                .handle_player_one_button(controller_input, pressed);
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
}

fn create_window_builder(size: WindowSize, title: String) -> WindowBuilder {
    WindowBuilder::new()
        .with_title(title)
        .with_inner_size(TaoLogicalSize::new(
            f64::from(size.width),
            f64::from(size.height),
        ))
}

fn window_surface_size(size: TaoPhysicalSize<u32>) -> SurfaceSize {
    SurfaceSize::new(size.width, size.height)
}

fn keycode_controller_input(code: KeyCode) -> Option<NesButton> {
    Some(match code {
        KeyCode::KeyZ => NesButton::A,
        KeyCode::KeyX => NesButton::B,
        KeyCode::KeyC => NesButton::Select,
        KeyCode::KeyV => NesButton::Start,
        KeyCode::ArrowUp => NesButton::Up,
        KeyCode::ArrowDown => NesButton::Down,
        KeyCode::ArrowLeft => NesButton::Left,
        KeyCode::ArrowRight => NesButton::Right,
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
    use nerust_gui_shell::session::input::NesButton;
    use tao::keyboard::KeyCode;

    #[test]
    fn keycode_mapping_matches_controller_layout() {
        assert_eq!(keycode_controller_input(KeyCode::KeyZ), Some(NesButton::A));
        assert_eq!(keycode_controller_input(KeyCode::KeyX), Some(NesButton::B));
        assert_eq!(
            keycode_controller_input(KeyCode::ArrowUp),
            Some(NesButton::Up)
        );
        assert_eq!(
            keycode_controller_input(KeyCode::ArrowRight),
            Some(NesButton::Right)
        );
        assert_eq!(keycode_controller_input(KeyCode::Enter), None);
    }
}
