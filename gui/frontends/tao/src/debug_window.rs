//! SPIKE-ONLY throwaway debugger UI prototype. Do not merge.
//!
//! tao debug window driven by `nerust_gui_viewmodel::debugger` output.
//! Session-requiring messages are executed by this handle against
//! `SessionHandle::spike_*` accessors; the iced program itself stays
//! pure data in / pure data out. NES `panels()` is empty, so the Table
//! view shows the documented STAND-IN until cores own Table descriptors.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use iced::{Length, Size, Task, Theme, mouse, theme};
use iced_tiny_skia::{
    Renderer,
    graphics::compositor::Compositor as _,
    window::{Compositor, Surface, compositor},
};
use iced_winit::{
    Clipboard,
    graphics::Viewport,
    program::{self, Program},
    runtime::user_interface::{Cache, UserInterface},
};
use nerust_core_traits::debugger::{InspectRequest, SpaceInfo, StepUnit};
use nerust_gui_shell::session::SessionHandle;
use nerust_gui_viewmodel::debugger::{
    DebugViewData, hex_lines, register_lines, standin_cpu_table, table_text,
};

#[cfg(target_os = "macos")]
use tao::platform::macos::WindowBuilderExtMacOS;
use tao::{
    event_loop::EventLoopWindowTarget,
    window::{Window as TaoWindow, WindowBuilder},
};

use crate::tao_conversions::{default_font, tao_modifiers_to_iced};

const SPIKE_DUMP_ROWS: u16 = 16;
const SPIKE_DUMP_BASE: u32 = 0;

#[derive(Debug, Clone)]
pub(crate) enum DebugMessage {
    Pause,
    Resume,
    StepFrame,
    StepInsn,
    Refresh,
    SelectSpace(String),
    SetData(DebugViewData),
}

pub(crate) struct DebugAppProgram {
    initial: DebugViewData,
}

impl Program for DebugAppProgram {
    type State = DebugViewData;
    type Message = DebugMessage;
    type Theme = Theme;
    type Renderer = iced_tiny_skia::Renderer;
    type Executor = iced_winit::futures::backend::default::Executor;

    fn name() -> &'static str {
        "nerust_spike_debug"
    }

    fn settings(&self) -> iced::Settings {
        iced::Settings {
            default_font: default_font(),
            default_text_size: iced::Pixels(14.0),
            ..Default::default()
        }
    }

    fn window(&self) -> Option<iced::window::Settings> {
        None
    }

    fn boot(&self) -> (Self::State, Task<Self::Message>) {
        (self.initial.clone(), Task::none())
    }

    fn update(&self, state: &mut Self::State, message: Self::Message) -> Task<Self::Message> {
        // Only SetData mutates state. Session actions (Pause/Resume/Step/
        // Refresh/SelectSpace) are executed by the outer handle, which
        // owns session access the iced program must never see.
        if let DebugMessage::SetData(data) = message {
            *state = data;
        }
        Task::none()
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        _window: iced::window::Id,
    ) -> iced::Element<'a, Self::Message, Self::Theme, Self::Renderer> {
        use iced::widget::{button, column, pick_list, row, scrollable, text};
        let controls = row![
            button("Pause").on_press(DebugMessage::Pause),
            button("Resume").on_press(DebugMessage::Resume),
            button("Step Frame").on_press(DebugMessage::StepFrame),
            button("Step Insn").on_press(DebugMessage::StepInsn),
            button("Refresh").on_press(DebugMessage::Refresh),
        ]
        .spacing(8);
        let space_pick = pick_list(
            state.space_names.clone(),
            state.space_names.get(state.selected_space).cloned(),
            DebugMessage::SelectSpace,
        )
        .placeholder("no spaces");
        let hex = scrollable(
            column(
                state
                    .hex_lines
                    .iter()
                    .map(|l| text(l.as_str()).size(13).into())
                    .collect::<Vec<_>>(),
            )
            .spacing(0),
        )
        .height(Length::FillPortion(3));
        let regs = column(
            state
                .register_lines
                .iter()
                .map(|l| text(l.as_str()).size(13).into())
                .collect::<Vec<_>>(),
        )
        .spacing(0);
        let mut table_col = column![text(state.table_title.as_str()).size(14)];
        for line in &state.table_lines {
            table_col = table_col.push(text(line.as_str()).size(13));
        }
        if state.table_is_standin {
            table_col = table_col.push(text("[spike stand-in: core owns Table]").size(12));
        }
        let status = format!(
            "{} frame={} refresh={}µs{}",
            if state.paused { "paused" } else { "running?" },
            state.frame,
            state.refresh_micros,
            state
                .error
                .as_ref()
                .map(|e| format!(" err={e}"))
                .unwrap_or_default(),
        );
        column![
            controls,
            row![text("space:"), space_pick].spacing(8),
            row![hex, regs, scrollable(table_col)].spacing(12),
            text(status).size(12),
        ]
        .spacing(8)
        .padding(12)
        .into()
    }
}

struct DebugUiState {
    ui: std::mem::ManuallyDrop<
        UserInterface<'static, DebugMessage, iced::Theme, iced_tiny_skia::Renderer>,
    >,
    instance: program::Instance<DebugAppProgram>,
}

impl DebugUiState {
    fn build_ui(
        instance: &program::Instance<DebugAppProgram>,
        window_id: iced::window::Id,
        bounds: Size,
        cache: Cache,
        renderer: &mut iced_tiny_skia::Renderer,
    ) -> UserInterface<'static, DebugMessage, iced::Theme, iced_tiny_skia::Renderer> {
        unsafe {
            std::mem::transmute::<
                UserInterface<'_, DebugMessage, iced::Theme, iced_tiny_skia::Renderer>,
                UserInterface<'static, DebugMessage, iced::Theme, iced_tiny_skia::Renderer>,
            >(UserInterface::build(
                instance.view(window_id),
                bounds,
                cache,
                renderer,
            ))
        }
    }

    fn process_messages(
        &mut self,
        messages: Vec<DebugMessage>,
        window_id: iced::window::Id,
        bounds: Size,
        renderer: &mut iced_tiny_skia::Renderer,
    ) {
        if messages.is_empty() {
            return;
        }
        let placeholder = std::mem::replace(
            &mut *self.ui,
            Self::build_ui(
                &self.instance,
                window_id,
                bounds,
                Cache::default(),
                renderer,
            ),
        );
        let cache = placeholder.into_cache();
        for msg in messages {
            let _task = self.instance.update(msg);
        }
        let stale = std::mem::replace(
            &mut *self.ui,
            Self::build_ui(&self.instance, window_id, bounds, cache, renderer),
        );
        let _ = stale.into_cache();
    }
}

impl Drop for DebugUiState {
    fn drop(&mut self) {
        unsafe { std::mem::ManuallyDrop::drop(&mut self.ui) };
    }
}

pub(crate) struct DebugWindowHandle {
    pub(crate) window: Arc<TaoWindow>,
    window_id: iced::window::Id,
    ui_state: DebugUiState,
    renderer: DebugRenderer,
    viewport_physical: (u32, u32),
    pub(crate) scale_factor: f32,
    pub(crate) modifiers: iced::keyboard::Modifiers,
    pub(crate) should_close: Arc<AtomicBool>,
    cursor: mouse::Cursor,
    clipboard: Clipboard,
    spaces: Vec<SpaceInfo>,
    selected: usize,
}

pub(crate) struct DebugRenderer {
    compositor: Compositor,
    surface: Surface,
    backend: Renderer,
}

impl DebugRenderer {
    fn present(
        &mut self,
        viewport: &Viewport,
        background_color: iced::Color,
    ) -> Result<(), iced_tiny_skia::graphics::compositor::SurfaceError> {
        self.compositor.present(
            &mut self.backend,
            &mut self.surface,
            viewport,
            background_color,
            || {},
        )
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.compositor
            .configure_surface(&mut self.surface, width, height);
    }
}

impl DebugWindowHandle {
    pub(crate) fn new(
        event_loop: &EventLoopWindowTarget<crate::app_menu::UserEvent>,
    ) -> Option<Self> {
        let should_close = Arc::new(AtomicBool::new(false));

        #[cfg_attr(not(target_os = "macos"), expect(unused_mut))]
        let mut wb = WindowBuilder::new()
            .with_title("Debugger (spike)")
            .with_inner_size(tao::dpi::LogicalSize::new(1100.0, 700.0));
        #[cfg(target_os = "macos")]
        {
            wb = wb.with_automatic_window_tabbing(false);
        }
        let window = Arc::new(match wb.build(event_loop) {
            Ok(w) => w,
            Err(e) => {
                log::error!("spike: failed to create debug window: {e}");
                return None;
            }
        });
        let window_id = iced::window::Id::unique();
        let program = DebugAppProgram {
            initial: DebugViewData {
                error: Some("press Refresh (pause the session first)".to_string()),
                ..Default::default()
            },
        };
        let (instance, _task) = program::Instance::new(program);
        let scale_factor = window.scale_factor() as f32;
        let window_size = window.inner_size();
        let viewport_physical = (window_size.width, window_size.height);

        let mut compositor = compositor::new(
            iced_tiny_skia::Settings {
                default_font: default_font(),
                default_text_size: iced::Pixels(14.0),
            },
            Arc::clone(&window),
        );
        let mut renderer = compositor.create_renderer();
        let surface =
            compositor.create_surface(Arc::clone(&window), window_size.width, window_size.height);
        let bounds = Viewport::with_physical_size(
            Size::new(viewport_physical.0, viewport_physical.1),
            scale_factor,
        )
        .logical_size();
        let ui = DebugUiState::build_ui(
            &instance,
            window_id,
            bounds,
            Cache::default(),
            &mut renderer,
        );

        window.request_redraw();
        Some(Self {
            window,
            window_id,
            ui_state: DebugUiState {
                ui: std::mem::ManuallyDrop::new(ui),
                instance,
            },
            renderer: DebugRenderer {
                compositor,
                surface,
                backend: renderer,
            },
            viewport_physical,
            scale_factor,
            modifiers: iced::keyboard::Modifiers::default(),
            should_close,
            cursor: mouse::Cursor::default(),
            clipboard: Clipboard::unconnected(),
            spaces: Vec::new(),
            selected: 0,
        })
    }

    /// Refresh all views from the session. Times the fetch+format path
    /// for the prototype's performance measurement.
    fn refresh(&mut self, session: &mut SessionHandle) -> DebugViewData {
        let t0 = Instant::now();
        let mut data = DebugViewData::default();
        match session.spike_spaces() {
            Ok(spaces) => {
                data.space_names = spaces.iter().map(|s| s.name.to_string()).collect();
                self.spaces = spaces;
            }
            Err(e) => {
                data.error = Some(format!("spaces unavailable: {e:?}"));
                data.refresh_micros = t0.elapsed().as_micros();
                return data;
            }
        }
        if self.spaces.is_empty() {
            data.error = Some("no spaces: load a ROM first".to_string());
            data.refresh_micros = t0.elapsed().as_micros();
            return data;
        }
        self.selected = self.selected.min(self.spaces.len() - 1);
        data.selected_space = self.selected;
        let space = self.spaces[self.selected].id;
        let req = InspectRequest {
            space: Some(space),
            addr: Some(SPIKE_DUMP_BASE),
            rows: SPIKE_DUMP_ROWS,
        };
        match session.spike_inspect(req) {
            Err(e) => data.error = Some(format!("session unavailable: {e:?}")),
            Ok(Err(nerust_core_traits::debugger::InspectError::NotPaused)) => {
                data.error = Some("running: pause the session to inspect".to_string());
            }
            Ok(Err(nerust_core_traits::debugger::InspectError::Core(e))) => {
                data.error = Some(format!("core refused inspect: {e:?}"));
            }
            Ok(Ok(res)) => {
                data.paused = true;
                data.frame = res.captured_at_frame;
                data.hex_lines = hex_lines(&res.dump);
                let regs: Vec<(&str, u64)> = res.registers.iter().map(|(n, v)| (*n, *v)).collect();
                data.register_lines = register_lines(&regs);
                match res.panels.first() {
                    Some(panel) => {
                        let (title, lines) = table_text(panel);
                        data.table_title = title;
                        data.table_lines = lines;
                    }
                    None => {
                        let panel = standin_cpu_table(&regs);
                        let (title, lines) = table_text(&panel);
                        data.table_title = title;
                        data.table_lines = lines;
                        data.table_is_standin = true;
                    }
                }
            }
        }
        data.refresh_micros = t0.elapsed().as_micros();
        data
    }

    fn handle_event(&mut self, mapped: iced::Event, session: &mut SessionHandle) {
        let mut messages = Vec::new();
        self.ui_state.ui_mut().update(
            &[mapped],
            self.cursor,
            &mut self.renderer.backend,
            &mut self.clipboard,
            &mut messages,
        );
        log::info!("spike: ui produced {} message(s)", messages.len());
        if messages.is_empty() {
            return;
        }
        // Session actions run here, in the handle — never inside the
        // iced program. Each action ends with a refresh into SetData.
        let mut translated = Vec::with_capacity(messages.len() + 1);
        for msg in messages {
            log::info!("spike: translating message: {msg:?}");
            match msg {
                DebugMessage::Pause => {
                    if let Err(e) = session.spike_pause() {
                        let mut data = self.refresh(session);
                        data.error = Some(format!("pause failed: {e:?}"));
                        translated.push(DebugMessage::SetData(data));
                    } else {
                        translated.push(DebugMessage::SetData(self.refresh(session)));
                    }
                }
                DebugMessage::Resume => {
                    if let Err(e) = session.spike_resume() {
                        let mut data = self.refresh(session);
                        data.error = Some(format!("resume failed: {e:?}"));
                        translated.push(DebugMessage::SetData(data));
                    } else {
                        let data = DebugViewData {
                            space_names: self.spaces.iter().map(|s| s.name.to_string()).collect(),
                            selected_space: self.selected,
                            error: Some("running: pause the session to inspect".to_string()),
                            ..Default::default()
                        };
                        translated.push(DebugMessage::SetData(data));
                    }
                }
                DebugMessage::StepFrame => {
                    match session.spike_step(StepUnit::Frame) {
                        r => log::info!("spike: frame step result: {r:?}"),
                    }
                    translated.push(DebugMessage::SetData(self.refresh(session)));
                }
                DebugMessage::StepInsn => {
                    match session.spike_step(StepUnit::Instruction) {
                        r => log::info!("spike: insn step result: {r:?}"),
                    }
                    translated.push(DebugMessage::SetData(self.refresh(session)));
                }
                DebugMessage::Refresh => {
                    translated.push(DebugMessage::SetData(self.refresh(session)));
                }
                DebugMessage::SelectSpace(name) => {
                    if let Some(i) = self.spaces.iter().position(|s| s.name == name.as_str()) {
                        self.selected = i;
                    }
                    translated.push(DebugMessage::SetData(self.refresh(session)));
                }
                DebugMessage::SetData(_) => {}
            }
        }
        let bounds = Viewport::with_physical_size(
            Size::new(self.viewport_physical.0, self.viewport_physical.1),
            self.scale_factor,
        )
        .logical_size();
        self.ui_state.process_messages(
            translated,
            self.window_id,
            bounds,
            &mut self.renderer.backend,
        );
    }

    pub(crate) fn render(&mut self) {
        log::info!("spike: debug render called");
        use iced::advanced::renderer;
        let theme = iced::Theme::Dark;
        let style = <iced::Theme as theme::Base>::base(&theme);
        let vp = Viewport::with_physical_size(
            Size::new(self.viewport_physical.0, self.viewport_physical.1),
            self.scale_factor,
        );
        let redraw_event = iced::Event::Window(iced::window::Event::RedrawRequested(
            std::time::Instant::now(),
        ));
        let _ = self.ui_state.ui_mut().update(
            &[redraw_event],
            self.cursor,
            &mut self.renderer.backend,
            &mut self.clipboard,
            &mut std::vec::Vec::new(),
        );
        self.ui_state.ui_mut().draw(
            &mut self.renderer.backend,
            &theme,
            &renderer::Style {
                text_color: style.text_color,
            },
            self.cursor,
        );
        if let Err(e) = self.renderer.present(&vp, iced::Color::BLACK) {
            log::warn!("spike debug render present failed: {e:?}");
        }
    }

    pub(crate) fn set_scale_factor(&mut self, sf: f32) {
        self.scale_factor = sf;
    }

    pub(crate) fn set_modifiers(&mut self, modifiers: tao::keyboard::ModifiersState) {
        self.modifiers = tao_modifiers_to_iced(modifiers);
    }

    pub(crate) fn update_modifiers_from_tao_event(&mut self, event: &tao::event::WindowEvent) {
        if let tao::event::WindowEvent::ModifiersChanged(state) = event {
            self.set_modifiers(*state);
        }
    }

    pub(crate) fn handle_tao_event(
        &mut self,
        event: tao::event::WindowEvent,
        session: &mut SessionHandle,
    ) {
        // Reuse the settings-window converter: cursor / modifiers /
        // close-request handling is identical for any iced window.
        let mut cursor = std::mem::replace(&mut self.cursor, mouse::Cursor::Unavailable);
        let mut modifiers = self.modifiers;
        let should_close = self.should_close.clone();
        let scale_factor = self.scale_factor;
        let mapped = super::settings_window::convert_tao_window_event(
            event,
            &mut cursor,
            scale_factor,
            &mut modifiers,
            &should_close,
        );
        self.cursor = cursor;
        self.modifiers = modifiers;
        if let Some(iced_event) = mapped {
            self.handle_event(iced_event, session);
        }
    }

    pub(crate) fn resize(&mut self, width: u32, height: u32) {
        self.viewport_physical = (width, height);
        self.renderer.resize(width, height);
    }

    /// SPIKE-ONLY instrumentation: refresh once outside the event path
    /// (used by the env-gated auto-open) and log the fetch+format time.
    pub(crate) fn refresh_now(&mut self, session: &mut SessionHandle) {
        let data = self.refresh(session);
        log::info!("spike: debug refresh took {}µs", data.refresh_micros);
        let bounds = Viewport::with_physical_size(
            Size::new(self.viewport_physical.0, self.viewport_physical.1),
            self.scale_factor,
        )
        .logical_size();
        self.ui_state.process_messages(
            vec![DebugMessage::SetData(data)],
            self.window_id,
            bounds,
            &mut self.renderer.backend,
        );
    }

    pub(crate) fn should_close_now(&self) -> bool {
        self.should_close.load(Ordering::Acquire)
    }
}

impl DebugUiState {
    fn ui_mut(
        &mut self,
    ) -> &mut UserInterface<'static, DebugMessage, iced::Theme, iced_tiny_skia::Renderer> {
        &mut self.ui
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_data() -> DebugViewData {
        DebugViewData {
            space_names: vec!["CPU".to_string()],
            hex_lines: vec!["0000: DE AD".to_string()],
            register_lines: vec!["a = 0x1".to_string()],
            table_title: "CPU (spike stand-in)".to_string(),
            table_lines: vec!["a: a | 0x1".to_string()],
            table_is_standin: true,
            paused: true,
            frame: 7,
            refresh_micros: 42,
            ..Default::default()
        }
    }

    #[test]
    fn program_view_builds_headless() {
        // Builds the Element tree only: no renderer, no window.
        let program = DebugAppProgram {
            initial: sample_data(),
        };
        let (state, _) = program.boot();
        let _element = program.view(&state, iced::window::Id::unique());
    }

    #[test]
    fn program_update_applies_set_data_only() {
        let program = DebugAppProgram {
            initial: DebugViewData::default(),
        };
        let (mut state, _) = program.boot();
        let _ = program.update(&mut state, DebugMessage::Refresh);
        assert!(state.hex_lines.is_empty());
        let _ = program.update(&mut state, DebugMessage::SetData(sample_data()));
        assert_eq!(state.hex_lines, vec!["0000: DE AD".to_string()]);
        assert_eq!(state.frame, 7);
    }
}
