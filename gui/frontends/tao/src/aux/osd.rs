//! OSD notifications: display queue, transparent overlay window,
//! and history viewer.
//!
//! Shell owns event capture (`NotificationCenter`); this module owns
//! presentation state. `OsdManager` is display-free so headless tests
//! cover queueing, expiry, burst caps, and strip math. The overlay
//! window itself (`OsdWindowHandle`) is a transparent click-through
//! iced shell tracking the main client area.

use iced::{Length, Task};
use iced_winit::program::{self, Program};

use nerust_gui_shell::session::osd::OsdSnapshot;
use nerust_settings_core::model::{language::AppLanguage, local::NotificationPosition};
use tao::event_loop::EventLoopWindowTarget;

use crate::aux::shell::{AuxWindow, WindowScaffold, aux_settings, aux_theme};

// ── Overlay window ───────────────────────────────────────────────

pub(crate) struct OsdState {
    shared: std::sync::Arc<std::sync::Mutex<OsdSnapshot>>,
}

pub(crate) struct OsdProgram {
    shared: std::sync::Arc<std::sync::Mutex<OsdSnapshot>>,
}

impl Program for OsdProgram {
    type State = OsdState;
    type Message = ();
    type Theme = iced::Theme;
    type Renderer = iced_tiny_skia::Renderer;
    type Executor = iced_winit::futures::backend::default::Executor;

    fn name() -> &'static str {
        "nerust_osd"
    }

    fn theme(&self, _state: &Self::State, _window: iced::window::Id) -> Option<Self::Theme> {
        aux_theme()
    }

    fn settings(&self) -> iced::Settings {
        aux_settings()
    }

    fn window(&self) -> Option<iced::window::Settings> {
        None
    }

    fn boot(&self) -> (Self::State, Task<Self::Message>) {
        (
            OsdState {
                shared: std::sync::Arc::clone(&self.shared),
            },
            Task::none(),
        )
    }

    fn update(&self, _state: &mut Self::State, _message: Self::Message) -> Task<Self::Message> {
        Task::none()
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        _window: iced::window::Id,
    ) -> iced::Element<'a, Self::Message, Self::Theme, Self::Renderer> {
        use iced::widget::{column, container, row, text};
        let snapshot = state.shared.lock().map(|s| s.clone()).unwrap_or_default();
        let font = iced::Pixels(snapshot.font_px.max(6.0));
        let label = |glyph: char, body: &str, rgb: (u8, u8, u8)| {
            container(
                row![
                    text(format!("{glyph} ")).size(font),
                    text(body.to_string()).size(font),
                ]
                .spacing(4),
            )
            .padding([2, 6])
            .style(move |_theme: &iced::Theme| container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgba8(
                    0, 0, 0, 0.72,
                ))),
                text_color: Some(iced::Color::from_rgb8(rgb.0, rgb.1, rgb.2)),
                border: iced::Border {
                    radius: 6.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
        };
        // Transient rows anchor at `position`, status at
        // `status_position`: same-corner stacking and split-corner
        // placement both fall out of one two-group column.
        let top_is_transient = matches!(
            snapshot.position,
            NotificationPosition::TopLeft | NotificationPosition::TopRight
        );
        let status_top = matches!(
            snapshot.status_position,
            NotificationPosition::TopLeft | NotificationPosition::TopRight
        );
        let transient: iced::Element<'_, (), iced::Theme, iced_tiny_skia::Renderer> = {
            let mut items = column![].spacing(4);
            for row_data in &snapshot.rows {
                items = items.push(label(row_data.glyph, &row_data.text, row_data.rgb));
            }
            container(items)
                .align_x(match snapshot.position {
                    NotificationPosition::TopLeft | NotificationPosition::BottomLeft => {
                        iced::alignment::Horizontal::Left
                    }
                    NotificationPosition::TopRight | NotificationPosition::BottomRight => {
                        iced::alignment::Horizontal::Right
                    }
                })
                .into()
        };
        let status_widget: iced::Element<'_, (), iced::Theme, iced_tiny_skia::Renderer> =
            match snapshot.status.as_ref() {
                Some(status) => container(label('\u{25cf}', status, (200, 200, 200)))
                    .align_x(match snapshot.status_position {
                        NotificationPosition::TopLeft | NotificationPosition::BottomLeft => {
                            iced::alignment::Horizontal::Left
                        }
                        NotificationPosition::TopRight | NotificationPosition::BottomRight => {
                            iced::alignment::Horizontal::Right
                        }
                    })
                    .into(),
                None => iced::widget::Space::new().into(),
            };
        // Same edge: status pins to the edge side, transient inside.
        let (top, bottom) = if top_is_transient == status_top {
            if top_is_transient {
                (status_widget, transient)
            } else {
                (transient, status_widget)
            }
        } else if top_is_transient {
            (transient, status_widget)
        } else {
            (status_widget, transient)
        };
        container(
            column![
                top,
                iced::widget::Space::new()
                    .width(Length::Fill)
                    .height(Length::Fill),
                bottom,
            ]
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(8)
        .into()
    }
}

pub(crate) struct OsdWindowHandle {
    pub(crate) shell: crate::aux::shell::AuxShell<OsdProgram>,
    shared: std::sync::Arc<std::sync::Mutex<OsdSnapshot>>,
}

impl OsdWindowHandle {
    /// Transparent click-through overlay tracking the main client area.
    /// Returns `None` when the platform refuses transparency (OSD off).
    pub(crate) fn new(
        event_loop: &EventLoopWindowTarget<crate::event::UserEvent>,
        position: (i32, i32),
        size: (f64, f64),
    ) -> Option<Self> {
        use tao::window::WindowBuilder;
        let window = WindowBuilder::new()
            .with_title("nerust OSD")
            .with_inner_size(tao::dpi::LogicalSize::new(size.0, size.1))
            .with_position(tao::dpi::LogicalPosition::new(
                position.0 as f64,
                position.1 as f64,
            ))
            .with_transparent(true)
            .with_decorations(false)
            .with_always_on_top(true)
            .build(event_loop)
            .map_err(|error| {
                log::warn!("OSD overlay unavailable: {error}");
            })
            .ok()?;
        if let Err(error) = window.set_ignore_cursor_events(true) {
            log::warn!("OSD click-through refused: {error:?}");
            return None;
        }
        log::debug!("OSD window created: id={:?}", window.id());
        let window = std::sync::Arc::new(window);
        let scaffold = WindowScaffold::open_transparent(window, "OSD overlay")?;
        let shared = std::sync::Arc::new(std::sync::Mutex::new(OsdSnapshot::default()));
        let (instance, _task) = program::Instance::new(OsdProgram {
            shared: std::sync::Arc::clone(&shared),
        });
        let shell = crate::aux::shell::AuxShell::assemble(
            scaffold,
            instance,
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );
        Some(Self { shell, shared })
    }

    pub(crate) fn sync(&mut self, snapshot: OsdSnapshot) {
        if let Ok(mut shared) = self.shared.lock() {
            *shared = snapshot;
        }
        self.shell.window.request_redraw();
    }

    pub(crate) fn move_to(&mut self, position: (i32, i32), size: (f64, f64)) {
        self.shell
            .window
            .set_outer_position(tao::dpi::LogicalPosition::new(
                position.0 as f64,
                position.1 as f64,
            ));
        self.shell
            .window
            .set_inner_size(tao::dpi::LogicalSize::new(size.0, size.1));
    }
}

impl OsdWindowHandle {
    pub(crate) fn render(&mut self) {
        self.shell.render("OSD overlay");
    }
}

impl AuxWindow for OsdWindowHandle {
    fn pump_window_event(&mut self, event: tao::event::WindowEvent) {
        self.shell.handle_common_event(&event);
        self.shell.window.request_redraw();
    }
}

// ── History viewer ─────────────────────────────────────────────

pub(crate) struct HistoryState {
    shared: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

pub(crate) struct HistoryProgram {
    shared: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl Program for HistoryProgram {
    type State = HistoryState;
    type Message = ();
    type Theme = iced::Theme;
    type Renderer = iced_tiny_skia::Renderer;
    type Executor = iced_winit::futures::backend::default::Executor;

    fn name() -> &'static str {
        "nerust_notification_history"
    }

    fn theme(&self, _state: &Self::State, _window: iced::window::Id) -> Option<Self::Theme> {
        aux_theme()
    }

    fn settings(&self) -> iced::Settings {
        aux_settings()
    }

    fn window(&self) -> Option<iced::window::Settings> {
        None
    }

    fn boot(&self) -> (Self::State, Task<Self::Message>) {
        (
            HistoryState {
                shared: std::sync::Arc::clone(&self.shared),
            },
            Task::none(),
        )
    }

    fn update(&self, _state: &mut Self::State, _message: Self::Message) -> Task<Self::Message> {
        Task::none()
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        _window: iced::window::Id,
    ) -> iced::Element<'a, Self::Message, Self::Theme, Self::Renderer> {
        use iced::widget::{column, scrollable, text};
        let lines = state.shared.lock().map(|s| s.clone()).unwrap_or_default();
        let mut items = column![].spacing(2);
        for line in &lines {
            items = items.push(text(line.clone()).size(13));
        }
        scrollable(items).into()
    }
}

pub(crate) struct HistoryWindowHandle {
    pub(crate) shell: crate::aux::shell::AuxShell<HistoryProgram>,
    shared: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl HistoryWindowHandle {
    pub(crate) fn new(
        event_loop: &EventLoopWindowTarget<crate::event::UserEvent>,
        language: AppLanguage,
    ) -> Option<Self> {
        let title = match language {
            AppLanguage::Japanese => "通知履歴",
            _ => "Notification history",
        };
        let scaffold = WindowScaffold::open(
            event_loop,
            title,
            (420.0, 360.0),
            None,
            None,
            "history window",
        )?;
        let shared = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let (instance, _task) = program::Instance::new(HistoryProgram {
            shared: std::sync::Arc::clone(&shared),
        });
        let shell = crate::aux::shell::AuxShell::assemble(
            scaffold,
            instance,
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );
        Some(Self { shell, shared })
    }

    pub(crate) fn sync(&mut self, lines: Vec<String>) {
        if let Ok(mut shared) = self.shared.lock() {
            *shared = lines;
        }
        self.shell.window.request_redraw();
    }
}

impl HistoryWindowHandle {
    pub(crate) fn render(&mut self) {
        self.shell.render("history window");
    }
}

impl AuxWindow for HistoryWindowHandle {
    fn pump_window_event(&mut self, event: tao::event::WindowEvent) {
        self.shell.handle_common_event(&event);
        self.shell.window.request_redraw();
    }
}
