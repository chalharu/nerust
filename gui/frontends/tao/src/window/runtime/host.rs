use crate::app_menu::{MenuCommand, UserEvent, imp::AppMenu};
use nerust_factory_nes::NesFactory;
use nerust_gui_runtime::rom::load_rom_path;
use nerust_gui_runtime::settings::{HostBackendIdentity, SettingsApplyPlan, SettingsSnapshot};
use nerust_gui_runtime::shell::NativeShellState;
use nerust_gui_settings::app_state::RememberedWindowSize;
use nerust_gui_settings::input::{KeyboardKey, ShortcutAction};
use nerust_gui_shell::factory::CoreFactory;
use nerust_gui_shell::load::{LoadRequest, MediaObject};
use nerust_gui_shell::session::WindowSize;
use nerust_gui_shell::session::commands::{SessionCommand, SessionCommandOutcome};
use nerust_gui_shell::session::{KeyboardShortcut, SessionError, SessionHandle};
use nerust_gui_shell::settings::defaults::seed::{
    default_app_state, default_local_settings, default_shared_settings,
};
use nerust_gui_shell::settings::i18n::{UiText, text};
use nerust_gui_shell::settings::scaling_factor;
use nerust_screen_video::RenderResult;
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

const DEFAULT_FIT_WINDOW_WIDTH: f64 = 960.0;
const DEFAULT_FIT_WINDOW_HEIGHT: f64 = 720.0;

pub(crate) struct HostState {
    window: Option<Arc<TaoWindow>>,
    session: SessionHandle,
    app_menu: AppMenu,
    shell: NativeShellState,
    pub(crate) settings_window: Option<crate::settings_window::SettingsWindowHandle>,
    settings_open: bool,
    resume_after_settings: bool,
    pending_fullscreen_sync: Option<bool>,
    pub(crate) active: bool,
    auto_paused: bool,
}

impl HostState {
    pub(crate) fn new(app_menu: AppMenu) -> Self {
        let identity = HostBackendIdentity::tao_wgpu();
        let factory: Arc<dyn CoreFactory> = Arc::new(NesFactory);
        let descriptor = factory.system_descriptor();
        let snapshot = SettingsSnapshot {
            shared: default_shared_settings(),
            local: default_local_settings(),
            app_state: default_app_state(),
        };
        let (core, adapter) = factory
            .create_core_and_adapter(&snapshot)
            .expect("failed to create core");
        let session = SessionHandle::new_with_core(identity, descriptor, factory, core, adapter);
        Self {
            window: None,
            session,
            app_menu,
            shell: NativeShellState::new(),
            settings_window: None,
            settings_open: false,
            resume_after_settings: false,
            pending_fullscreen_sync: None,
            active: true,
            auto_paused: false,
        }
    }

    pub(crate) fn session(&self) -> &SessionHandle {
        &self.session
    }

    pub(crate) fn session_mut(&mut self) -> &mut SessionHandle {
        &mut self.session
    }

    pub(crate) fn window(&self) -> Option<&Arc<TaoWindow>> {
        self.window.as_ref()
    }

    pub(crate) fn resume_session(&mut self) {
        let _ = self.session.run_command(SessionCommand::Resume);
    }

    pub(crate) fn pause_session(&mut self) {
        let _ = self.session.run_command(SessionCommand::Pause);
    }

    pub(crate) fn auto_paused(&self) -> bool {
        self.auto_paused
    }

    pub(crate) fn set_auto_paused(&mut self) {
        self.auto_paused = true;
    }

    pub(crate) fn clear_auto_paused(&mut self) {
        self.auto_paused = false;
    }

    pub(crate) fn ensure_window(&mut self, event_loop: &EventLoopWindowTarget<UserEvent>) {
        if self.window.is_some() {
            return;
        }

        let fullscreen = self
            .session
            .settings_snapshot()
            .local
            .video
            .window
            .fullscreen_default;
        let window = Arc::new(
            create_window_builder(
                self.startup_window_size(),
                self.session.window_title(),
                fullscreen,
            )
            .build(event_loop)
            .unwrap(),
        );
        self.app_menu.init_for_window(&window);
        self.window = Some(window);
        self.sync_fullscreen_from_settings();
        self.sync_menu_state();
        self.request_redraw();
        self.refresh_window_title();
    }

    pub(crate) fn is_window(&self, window_id: WindowId) -> bool {
        self.window
            .as_ref()
            .is_some_and(|window| window.id() == window_id)
    }

    pub(crate) fn is_settings_window(&self, window_id: WindowId) -> bool {
        self.settings_window
            .as_ref()
            .is_some_and(|h| h.window.id() == window_id)
    }

    pub(crate) fn window_surface_size(&self) -> Option<SurfaceSize> {
        self.window
            .as_ref()
            .map(|window| window_surface_size(window.inner_size()))
    }

    pub(crate) fn load(&mut self, data: Vec<u8>) -> bool {
        self.load_inner(None, data)
    }

    pub(crate) fn load_with_options(
        &mut self,
        rom_path: Option<PathBuf>,
        data: Vec<u8>,
        _request: LoadRequest,
    ) -> bool {
        self.load_inner(rom_path, data)
    }

    fn load_inner(&mut self, rom_path: Option<PathBuf>, data: Vec<u8>) -> bool {
        let media = MediaObject::new(rom_path, data);
        let options = self.session.default_load_options();
        if let Ok(resolved) = self
            .session
            .factory()
            .resolve_load_request(self.session.settings_snapshot(), options)
            && self.session.load_resolved(media, resolved).is_ok()
        {
            let _ = self.session.run_command(SessionCommand::Resume);
            self.after_rom_load();
            return true;
        }
        false
    }

    pub(crate) fn load_path(&mut self, path: &Path) -> bool {
        match load_rom_path(path) {
            Ok(loaded_rom) => {
                let (rom_path, data) = loaded_rom.into_parts();
                self.load_inner(Some(rom_path), data)
            }
            Err(error) => {
                log::warn!("ROM open failed: {error}");
                false
            }
        }
    }

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
            MenuCommand::Settings => {
                self.open_settings_window(event_loop);
                HostAction::None
            }
            MenuCommand::Session(command) => {
                self.apply_session_command(command);
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
            && let Some(key) = keycode_controller_input(input.physical_key)
            && let Some(shortcut) = self.session.handle_keyboard_key(key, pressed)
        {
            self.apply_keyboard_shortcut(shortcut);
        }
    }

    pub(crate) fn clear_keys(&mut self) {
        self.session.clear_input();
    }

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

    pub(crate) fn update_control_flow(&mut self, control_flow: &mut ControlFlow) {
        self.sync_fullscreen_default_from_window();
        self.maybe_refresh_window_title(Instant::now());
        *control_flow = ControlFlow::Wait;

        // On macOS, request_redraw() integrates with CVDisplayLink/vsync.
        // On other platforms, it fires on the next event loop iteration.
        // When inactive, no redraw is requested — event loop sleeps (CPU 0%).
        if self.active
            && let Some(window) = self.window.as_ref()
        {
            window.request_redraw();
        }
    }

    pub(crate) fn on_render_result(&mut self, result: RenderResult) {
        match result {
            RenderResult::Presented => {
                self.shell
                    .on_frame_presented(self.session.metrics().frame_counter);
                self.maybe_refresh_window_title(Instant::now());
            }
            RenderResult::Skipped | RenderResult::Error => {}
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
        self.remember_fit_window_size();
        self.settings_open = false;
        self.resume_after_settings = false;
        self.settings_window.take();
        self.session.flush_before_exit();
        true
    }

    pub(crate) fn apply_settings(
        &mut self,
        settings: SettingsSnapshot,
    ) -> Result<SettingsApplyPlan, SessionError> {
        let plan = self.session.apply_settings(settings)?;
        let fullscreen_default = self
            .session
            .settings_snapshot()
            .local
            .video
            .window
            .fullscreen_default;
        if plan.fullscreen_default_changed && !fullscreen_default {
            self.sync_fullscreen_from_settings();
        }
        if plan.scaling_changed {
            self.update_window_size_for_scaling();
        }
        if plan.fullscreen_default_changed && fullscreen_default {
            self.sync_fullscreen_from_settings();
        }
        self.sync_menu_state();
        self.refresh_window_title();
        self.request_redraw();
        Ok(plan)
    }

    pub(crate) fn on_settings_closed(&mut self) {
        self.settings_window = None;
        if !self.settings_open {
            return;
        }
        self.settings_open = false;
        // The main window may not have received Focused(true) yet (platform
        // quirk).  Ensure the render loop keeps running.
        self.active = true;
        let should_resume = std::mem::take(&mut self.resume_after_settings);
        if should_resume {
            self.apply_session_command(SessionCommand::Resume);
        } else {
            self.sync_menu_state();
            self.refresh_window_title();
        }
    }

    fn open_rom_dialog(&mut self) -> bool {
        FileDialog::new()
            .set_title(text(
                self.session.settings_snapshot().shared.general.language,
                UiText::Open,
            ))
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
            self.settings_open,
            self.session.settings_snapshot().shared.general.language,
        );
    }

    fn apply_session_command(&mut self, command: SessionCommand) {
        let outcome = self.session.run_command(command).unwrap_or_default();
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

    fn open_settings_window(&mut self, event_loop: &EventLoopWindowTarget<UserEvent>) {
        if self.settings_open {
            return;
        }
        self.settings_open = true;
        self.resume_after_settings = self.session.loaded() && !self.session.paused();
        if self.resume_after_settings {
            self.apply_session_command(SessionCommand::Pause);
        } else {
            self.sync_menu_state();
        }

        match crate::settings_window::SettingsWindowHandle::new(
            self.session.settings_snapshot().clone(),
            event_loop,
        ) {
            Some(handle) => self.settings_window = Some(handle),
            None => {
                log::error!("failed to open settings window");
                self.settings_open = false;
                if self.resume_after_settings {
                    self.apply_session_command(SessionCommand::Resume);
                } else {
                    self.sync_menu_state();
                }
            }
        }
    }

    pub(crate) fn close_settings_window(
        &mut self,
        mut handle: crate::settings_window::SettingsWindowHandle,
    ) -> Option<SettingsApplyPlan> {
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

    fn startup_window_size(&self) -> TaoLogicalSize<f64> {
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

    fn sync_fullscreen_from_settings(&mut self) {
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
    ) -> Result<SettingsApplyPlan, SessionError> {
        let plan = self.session.set_fullscreen_default(fullscreen)?;
        self.sync_fullscreen_from_settings();
        self.sync_menu_state();
        self.refresh_window_title();
        self.request_redraw();
        Ok(plan)
    }

    fn remembered_fit_window_size(&self) -> Option<RememberedWindowSize> {
        self.session
            .settings_snapshot()
            .app_state
            .window_size(&HostBackendIdentity::tao_wgpu().to_string())
    }

    fn remember_fit_window_size(&self) {
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

        if let Err(error) = self.session.settings_manager().update_window_size(
            &HostBackendIdentity::tao_wgpu(),
            width,
            height,
        ) {
            log::warn!("failed to remember tao window size: {error}");
        }
    }
}

fn create_window_builder(
    size: TaoLogicalSize<f64>,
    title: String,
    fullscreen: bool,
) -> WindowBuilder {
    WindowBuilder::new()
        .with_title(title)
        .with_inner_size(size)
        .with_fullscreen(fullscreen_mode(fullscreen))
}

fn window_is_fullscreen(window: &TaoWindow) -> bool {
    window.fullscreen().is_some()
}

fn set_window_fullscreen(window: &TaoWindow, fullscreen: bool) {
    window.set_fullscreen(fullscreen_mode(fullscreen));
}

fn fullscreen_mode(fullscreen: bool) -> Option<Fullscreen> {
    fullscreen.then_some(Fullscreen::Borderless(None))
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct NativeFullscreenSync {
    pending_target: Option<bool>,
    persist_setting: bool,
}

fn derive_native_fullscreen_sync(
    pending_target: Option<bool>,
    fullscreen: bool,
    persisted_fullscreen: bool,
) -> NativeFullscreenSync {
    if let Some(expected) = pending_target
        && fullscreen != expected
    {
        return NativeFullscreenSync {
            pending_target: Some(expected),
            persist_setting: false,
        };
    }

    NativeFullscreenSync {
        pending_target: None,
        persist_setting: fullscreen != persisted_fullscreen,
    }
}

fn window_surface_size(size: TaoPhysicalSize<u32>) -> SurfaceSize {
    SurfaceSize::new(size.width, size.height)
}

fn logical_window_size(size: WindowSize, scaling: Option<u32>) -> TaoLogicalSize<f64> {
    let (width, height) = scaling
        .map(|scale| {
            (
                f64::from(size.width) * f64::from(scale),
                f64::from(size.height) * f64::from(scale),
            )
        })
        .unwrap_or((f64::from(size.width), f64::from(size.height)));
    TaoLogicalSize::new(width, height)
}

fn default_fit_window_size() -> TaoLogicalSize<f64> {
    TaoLogicalSize::new(DEFAULT_FIT_WINDOW_WIDTH, DEFAULT_FIT_WINDOW_HEIGHT)
}

fn logical_size_from_remembered(size: RememberedWindowSize) -> TaoLogicalSize<f64> {
    TaoLogicalSize::new(f64::from(size.width), f64::from(size.height))
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
    use super::{
        NativeFullscreenSync, create_window_builder, derive_native_fullscreen_sync,
        keycode_controller_input,
    };
    use nerust_gui_settings::input::KeyboardKey;
    use tao::{dpi::LogicalSize as TaoLogicalSize, keyboard::KeyCode, window::Fullscreen};

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

    #[test]
    fn window_builder_requests_initial_fullscreen_when_enabled() {
        let builder =
            create_window_builder(TaoLogicalSize::new(960.0, 720.0), "nerust".into(), true);

        assert_eq!(
            builder.window.fullscreen,
            Some(Fullscreen::Borderless(None))
        );
    }

    #[test]
    fn window_builder_skips_initial_fullscreen_when_disabled() {
        let builder =
            create_window_builder(TaoLogicalSize::new(960.0, 720.0), "nerust".into(), false);

        assert_eq!(builder.window.fullscreen, None);
    }

    #[test]
    fn fullscreen_sync_ignores_transient_state_while_transition_is_pending() {
        assert_eq!(
            derive_native_fullscreen_sync(Some(true), false, true),
            NativeFullscreenSync {
                pending_target: Some(true),
                persist_setting: false,
            }
        );
    }

    #[test]
    fn fullscreen_sync_clears_pending_transition_once_window_matches_target() {
        assert_eq!(
            derive_native_fullscreen_sync(Some(true), true, true),
            NativeFullscreenSync {
                pending_target: None,
                persist_setting: false,
            }
        );
    }

    #[test]
    fn fullscreen_sync_persists_native_changes_once_no_transition_is_pending() {
        assert_eq!(
            derive_native_fullscreen_sync(None, false, true),
            NativeFullscreenSync {
                pending_target: None,
                persist_setting: true,
            }
        );
    }
}
