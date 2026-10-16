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

use nerust_gui_shell::session::{
    notification::NotificationLevel,
    osd::{HistoryLine, OsdSnapshot},
};
use nerust_settings_core::{
    i18n::{UiText, history_count, text as ui_text},
    model::{language::AppLanguage, local::NotificationPosition},
};
use tao::event_loop::EventLoopWindowTarget;

use crate::aux::shell::{AuxWindow, WindowScaffold, aux_settings, aux_theme};

// ── Overlay window ───────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(crate) enum OsdMessage {
    Sync(OsdSnapshot),
}

pub(crate) struct OsdState {
    snapshot: OsdSnapshot,
}

pub(crate) struct OsdProgram;

impl Program for OsdProgram {
    type State = OsdState;
    type Message = OsdMessage;
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
                snapshot: OsdSnapshot::default(),
            },
            Task::none(),
        )
    }

    fn update(&self, state: &mut Self::State, message: Self::Message) -> Task<Self::Message> {
        match message {
            OsdMessage::Sync(snapshot) => {
                state.snapshot = snapshot;
                Task::none()
            }
        }
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        _window: iced::window::Id,
    ) -> iced::Element<'a, Self::Message, Self::Theme, Self::Renderer> {
        use iced::widget::{column, container, row, text};
        let snapshot = &state.snapshot;
        let font = iced::Pixels(snapshot.font_px.max(6.0));
        let label = |tag: &str, body: &str, rgb: (u8, u8, u8)| -> iced::Element<'_, OsdMessage> {
            container(
                row![
                    text(tag.to_string()).size(font),
                    text(body.to_string()).size(font),
                ]
                .spacing(6)
                .align_y(iced::Alignment::Center),
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
            .into()
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
        let transient: iced::Element<'_, OsdMessage> = {
            let mut items = column![].spacing(4);
            for row_data in &snapshot.rows {
                items = items.push(label(row_data.tag, &row_data.text, row_data.rgb));
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
        let status_widget: iced::Element<'_, OsdMessage> = match snapshot.status.as_ref() {
            // Status is ambient (not severity): plain text, no level badge.
            Some(status) => container(
                container(text(status.clone()).size(font))
                    .padding([2, 6])
                    .style(|_theme: &iced::Theme| container::Style {
                        background: Some(iced::Background::Color(iced::Color::from_rgba8(
                            0, 0, 0, 0.72,
                        ))),
                        text_color: Some(iced::Color::from_rgb8(200, 200, 200)),
                        border: iced::Border {
                            radius: 6.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
            )
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
        // Same corner: one anchored column, status edge-most and
        // transient just inside. Split corners anchor independently.
        let same_corner = snapshot.position == snapshot.status_position;
        let body: iced::Element<'_, OsdMessage> = if same_corner {
            let mut items = column![].spacing(4);
            if top_is_transient {
                items = items.push(status_widget);
                items = items.push(transient);
            } else {
                items = items.push(transient);
                items = items.push(status_widget);
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
        } else {
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
            column![
                top,
                iced::widget::Space::new()
                    .width(Length::Fill)
                    .height(Length::Fill),
                bottom,
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        };
        container(body)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(match snapshot.position {
                NotificationPosition::TopLeft | NotificationPosition::BottomLeft => {
                    iced::alignment::Horizontal::Left
                }
                NotificationPosition::TopRight | NotificationPosition::BottomRight => {
                    iced::alignment::Horizontal::Right
                }
            })
            .align_y(match snapshot.position {
                NotificationPosition::TopLeft | NotificationPosition::TopRight => {
                    iced::alignment::Vertical::Top
                }
                NotificationPosition::BottomLeft | NotificationPosition::BottomRight => {
                    iced::alignment::Vertical::Bottom
                }
            })
            .padding(8)
            .into()
    }
}

pub(crate) struct OsdWindowHandle {
    pub(crate) shell: crate::aux::shell::AuxShell<OsdProgram>,
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
        let window = std::sync::Arc::new(window);
        let scaffold = WindowScaffold::open_transparent(window, "OSD overlay")?;
        let (instance, _task) = program::Instance::new(OsdProgram);
        let shell = crate::aux::shell::AuxShell::assemble(
            scaffold,
            instance,
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );
        Some(Self { shell })
    }

    pub(crate) fn sync(&mut self, snapshot: OsdSnapshot) {
        // Message-driven (not shared-state): iced rebuilds the cached
        // tree only on update(), so silent Arc mutation never paints.
        // Self-healing viewport: a fresh transparent window reports a
        // degenerate size, and Resized events are not guaranteed on
        // every backend. Without this the layout collapses to nothing.
        let live = self.shell.window.inner_size();
        self.shell.resize(live.width, live.height);
        let bounds = self.shell.logical_bounds();
        let window_id = self.shell.window_id;
        self.shell.ui_state.process_messages(
            vec![OsdMessage::Sync(snapshot)],
            window_id,
            bounds,
            &mut self.shell.renderer.backend,
            &self.shell.should_close,
        );
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
        // Transparent clear: the surface is alpha-capable, so empty
        // areas composite the game beneath instead of theme white.
        self.shell
            .render_with_background("OSD overlay", iced::Color::TRANSPARENT);
    }
}

impl AuxWindow for OsdWindowHandle {
    fn pump_window_event(&mut self, event: tao::event::WindowEvent) {
        self.shell.handle_common_event(&event);
        self.shell.window.request_redraw();
    }
}

// ── History viewer ─────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(crate) enum HistoryMessage {
    Sync {
        lines: Vec<HistoryLine>,
        language: AppLanguage,
    },
    SetFilter(HistoryFilter),
    Clear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum HistoryFilter {
    #[default]
    All,
    Info,
    Warning,
    Error,
}

/// Short module for the grid: last two segments keep the
/// informative tail within the fixed column (full path stays
/// in the data and in GTK's middle-ellipsis cell).
fn short_module(module: &str) -> &str {
    let mut parts = module.rsplit("::");
    let (last, prev) = (parts.next(), parts.next());
    match (prev, last) {
        (Some(prev), Some(last)) => {
            let cut = module.len() - prev.len() - last.len() - 2;
            &module[cut..]
        }
        _ => module,
    }
}

impl HistoryFilter {
    fn matches(self, level: NotificationLevel) -> bool {
        match self {
            HistoryFilter::All => true,
            HistoryFilter::Info => level == NotificationLevel::Info,
            HistoryFilter::Warning => level == NotificationLevel::Warning,
            HistoryFilter::Error => level == NotificationLevel::Error,
        }
    }

    /// Localized dropdown label. Levels reuse the settings table;
    /// only "All" needs its own key.
    fn label(self, language: AppLanguage) -> &'static str {
        match self {
            HistoryFilter::All => ui_text(language, UiText::HistoryFilterAll),
            HistoryFilter::Info => ui_text(language, UiText::NotificationInfo),
            HistoryFilter::Warning => ui_text(language, UiText::NotificationWarning),
            HistoryFilter::Error => ui_text(language, UiText::Error),
        }
    }

    /// Inverse of [`HistoryFilter::label`]: dropdown options are
    /// localized strings, mapped back on select. Total over our own
    /// four labels in either language.
    fn from_label(label: &str, language: AppLanguage) -> Option<HistoryFilter> {
        [
            HistoryFilter::All,
            HistoryFilter::Info,
            HistoryFilter::Warning,
            HistoryFilter::Error,
        ]
        .into_iter()
        .find(|filter| filter.label(language) == label)
    }
}

pub(crate) struct HistoryState {
    lines: Vec<HistoryLine>,
    filter: HistoryFilter,
    language: AppLanguage,
    clear_requested: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

pub(crate) struct HistoryProgram {
    language: AppLanguage,
    clear_requested: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Program for HistoryProgram {
    type State = HistoryState;
    type Message = HistoryMessage;
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
                lines: Vec::new(),
                filter: HistoryFilter::All,
                language: self.language,
                clear_requested: std::sync::Arc::clone(&self.clear_requested),
            },
            Task::none(),
        )
    }

    fn update(&self, state: &mut Self::State, message: Self::Message) -> Task<Self::Message> {
        match message {
            HistoryMessage::Sync { lines, language } => {
                state.lines = lines;
                state.language = language;
                Task::none()
            }
            HistoryMessage::SetFilter(filter) => {
                state.filter = filter;
                Task::none()
            }
            HistoryMessage::Clear => {
                state
                    .clear_requested
                    .store(true, std::sync::atomic::Ordering::Release);
                Task::none()
            }
        }
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        _window: iced::window::Id,
    ) -> iced::Element<'a, Self::Message, Self::Theme, Self::Renderer> {
        use iced::Length;
        use iced::widget::{button, column, container, pick_list, row, rule, scrollable, text};
        use nerust_gui_shell::session::osd::level_label;
        let language = state.language;
        let visible: Vec<&HistoryLine> = state
            .lines
            .iter()
            .filter(|line| state.filter.matches(line.level))
            .collect();
        // Toolbar above the table (the Console / DevTools / logcat
        // pattern: filter plus clear live together at the top).
        // "Clear History" names the target explicitly so filter-clear
        // and history-clear are never confused (cf. Event Viewer
        // "Clear Filter" vs "Clear Log"); the item count trails right.
        let options: Vec<String> = [
            HistoryFilter::All,
            HistoryFilter::Info,
            HistoryFilter::Warning,
            HistoryFilter::Error,
        ]
        .iter()
        .map(|filter| filter.label(language).to_string())
        .collect();
        let toolbar: iced::Element<'_, HistoryMessage> = row![
            text(ui_text(language, UiText::HistoryLevel)).size(12),
            pick_list(
                options,
                state.filter.label(language).to_string().into(),
                move |label: String| HistoryMessage::SetFilter(
                    HistoryFilter::from_label(&label, language).unwrap_or_default()
                ),
            )
            .placeholder(ui_text(language, UiText::HistoryLevel)),
            button(text(ui_text(language, UiText::HistoryClear)).size(12))
                .on_press(HistoryMessage::Clear),
            iced::widget::Space::new().width(Length::Fill),
            text(history_count(language, visible.len())).size(12),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center)
        .into();
        let mut items = column![].spacing(2);
        // Descriptive nouns, title case, no punctuation (Apple HIG).
        items = items.push(
            row![
                text(ui_text(language, UiText::HistoryDatetime))
                    .size(12)
                    .width(Length::Fixed(150.0)),
                text(ui_text(language, UiText::HistoryLevel))
                    .size(12)
                    .width(Length::Fixed(70.0)),
                text(ui_text(language, UiText::HistoryModule))
                    .size(12)
                    .width(Length::Fixed(220.0)),
                text(ui_text(language, UiText::HistoryMessage))
                    .size(12)
                    .width(Length::Fill),
            ]
            .spacing(12),
        );
        items = items.push(rule::horizontal(1));
        if visible.is_empty() {
            items = items.push(text(ui_text(language, UiText::HistoryEmpty)).size(13));
        }
        for line in visible {
            let level_color = match line.level {
                NotificationLevel::Info => iced::Color::from_rgb8(70, 70, 70),
                NotificationLevel::Warning => iced::Color::from_rgb8(170, 110, 0),
                NotificationLevel::Error => iced::Color::from_rgb8(190, 30, 30),
            };
            let mut message = line.text.clone();
            if line.count > 1 {
                message = format!("{message} ×{}", line.count);
            }
            // Cells clip at their column: overlong values truncate
            // instead of bleeding into the neighbour (HIG).
            items = items.push(
                row![
                    container(
                        text(line.datetime.clone())
                            .size(12)
                            .font(iced::Font::MONOSPACE)
                            .width(Length::Fill)
                    )
                    .clip(true)
                    .width(Length::Fixed(150.0)),
                    container(
                        text(level_label(line.level, language))
                            .size(12)
                            .color(level_color)
                            .width(Length::Fill)
                    )
                    .clip(true)
                    .width(Length::Fixed(70.0)),
                    container(
                        text(short_module(line.module))
                            .size(11)
                            .font(iced::Font::MONOSPACE)
                            .width(Length::Fill)
                    )
                    .clip(true)
                    .width(Length::Fixed(220.0)),
                    container(text(message).size(13).width(Length::Fill))
                        .clip(true)
                        .width(Length::Fill),
                ]
                .spacing(12),
            );
        }
        // 16 content margin, 12 between groups, 8 between controls.
        column![toolbar, scrollable(items)]
            .spacing(12)
            .padding(16)
            .into()
    }
}

pub(crate) struct HistoryWindowHandle {
    pub(crate) shell: crate::aux::shell::AuxShell<HistoryProgram>,
    clear_requested: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl HistoryWindowHandle {
    pub(crate) fn new(
        event_loop: &EventLoopWindowTarget<crate::event::UserEvent>,
        language: AppLanguage,
    ) -> Option<Self> {
        let title = ui_text(language, UiText::NotificationHistory);
        let scaffold = WindowScaffold::open(
            event_loop,
            title,
            (720.0, 360.0),
            None,
            None,
            "history window",
        )?;
        let clear_requested = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (instance, _task) = program::Instance::new(HistoryProgram {
            language,
            clear_requested: std::sync::Arc::clone(&clear_requested),
        });
        let shell = crate::aux::shell::AuxShell::assemble(
            scaffold,
            instance,
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );
        Some(Self {
            shell,
            clear_requested,
        })
    }

    pub(crate) fn take_clear_request(&self) -> bool {
        self.clear_requested
            .swap(false, std::sync::atomic::Ordering::AcqRel)
    }

    pub(crate) fn sync(&mut self, lines: Vec<HistoryLine>, language: AppLanguage) {
        let bounds = self.shell.logical_bounds();
        let window_id = self.shell.window_id;
        self.shell.ui_state.process_messages(
            vec![HistoryMessage::Sync { lines, language }],
            window_id,
            bounds,
            &mut self.shell.renderer.backend,
            &self.shell.should_close,
        );
        self.shell.window.request_redraw();
    }
}

impl HistoryWindowHandle {
    pub(crate) fn render(&mut self) {
        self.shell.render("history window");
    }
}

impl HistoryWindowHandle {
    fn handle_event(&mut self, mapped: iced::Event) {
        let (messages, outcome) = self.shell.ui_state.pump(
            mapped,
            self.shell.cursor,
            &mut self.shell.clipboard,
            &mut self.shell.renderer.backend,
        );
        if matches!(
            outcome,
            crate::aux::shell::PumpOutcome::Repaint | crate::aux::shell::PumpOutcome::Rebuild
        ) {
            self.shell.window.request_redraw();
        }
        if !messages.is_empty() {
            let bounds = self.shell.logical_bounds();
            self.shell.ui_state.process_messages(
                messages,
                self.shell.window_id,
                bounds,
                &mut self.shell.renderer.backend,
                &self.shell.should_close,
            );
            self.shell.window.request_redraw();
        }
    }
}

impl AuxWindow for HistoryWindowHandle {
    fn pump_window_event(&mut self, event: tao::event::WindowEvent) {
        self.shell.handle_common_event(&event);
        if let Some(mapped) = self.shell.feed_tao_event(event) {
            self.handle_event(mapped);
        }
        self.shell.window.request_redraw();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn en() -> AppLanguage {
        AppLanguage::English
    }

    fn ja() -> AppLanguage {
        AppLanguage::Japanese
    }

    fn line(level: NotificationLevel, text: &str) -> HistoryLine {
        HistoryLine {
            datetime: "2026-10-16 18:24:07".to_string(),
            level,
            module: "nerust_gui_shell::session::notification",
            text: text.to_string(),
            count: 1,
        }
    }

    #[test]
    fn filter_labels_cover_both_languages() {
        assert_eq!(HistoryFilter::All.label(en()), "All");
        assert_eq!(HistoryFilter::All.label(ja()), "すべて");
        assert_eq!(HistoryFilter::Info.label(en()), "Info");
        assert_eq!(HistoryFilter::Info.label(ja()), "情報");
        assert_eq!(HistoryFilter::Warning.label(en()), "Warning");
        assert_eq!(HistoryFilter::Warning.label(ja()), "警告");
        assert_eq!(HistoryFilter::Error.label(en()), "Error");
        assert_eq!(HistoryFilter::Error.label(ja()), "エラー");
    }

    #[test]
    fn filter_label_round_trips_in_both_languages() {
        for language in [en(), ja()] {
            for filter in [
                HistoryFilter::All,
                HistoryFilter::Info,
                HistoryFilter::Warning,
                HistoryFilter::Error,
            ] {
                assert_eq!(
                    HistoryFilter::from_label(filter.label(language), language),
                    Some(filter)
                );
            }
        }
        assert_eq!(HistoryFilter::from_label("???", en()), None);
        assert_eq!(HistoryFilter::from_label("", ja()), None);
    }

    #[test]
    fn filter_matches_only_its_level() {
        assert!(HistoryFilter::All.matches(NotificationLevel::Error));
        assert!(HistoryFilter::Info.matches(NotificationLevel::Info));
        assert!(!HistoryFilter::Info.matches(NotificationLevel::Warning));
        assert!(HistoryFilter::Warning.matches(NotificationLevel::Warning));
        assert!(!HistoryFilter::Warning.matches(NotificationLevel::Error));
        assert!(HistoryFilter::Error.matches(NotificationLevel::Error));
        assert!(!HistoryFilter::Error.matches(NotificationLevel::Info));
    }

    #[test]
    fn short_module_keeps_the_tail() {
        assert_eq!(
            short_module("nerust_gui_shell::session::notification"),
            "session::notification"
        );
        assert_eq!(short_module("single"), "single");
    }

    #[test]
    fn history_sync_sets_lines_and_language() {
        let program = HistoryProgram {
            language: en(),
            clear_requested: Default::default(),
        };
        let (mut state, _) = program.boot();
        assert!(state.lines.is_empty());
        assert_eq!(state.filter, HistoryFilter::All);
        let _ = program.update(
            &mut state,
            HistoryMessage::Sync {
                lines: vec![
                    line(NotificationLevel::Info, "Paused"),
                    line(NotificationLevel::Error, "failed"),
                ],
                language: ja(),
            },
        );
        assert_eq!(state.lines.len(), 2);
        assert_eq!(state.language, ja());
        let _ = program.update(&mut state, HistoryMessage::SetFilter(HistoryFilter::Error));
        assert_eq!(state.filter, HistoryFilter::Error);
    }

    #[test]
    fn history_view_builds_without_a_window() {
        // Element construction is pure (no drawing): the toolbar,
        // headers, rows, empty state, and filter variants all build
        // headless, covering the view fn without a display.
        let program = HistoryProgram {
            language: en(),
            clear_requested: Default::default(),
        };
        let (mut state, _) = program.boot();
        drop(program.view(&state, iced::window::Id::unique()));
        let _ = program.update(
            &mut state,
            HistoryMessage::Sync {
                lines: vec![
                    line(NotificationLevel::Info, "Paused"),
                    line(NotificationLevel::Warning, "empty"),
                    line(NotificationLevel::Error, "failed"),
                ],
                language: ja(),
            },
        );
        drop(program.view(&state, iced::window::Id::unique()));
        state.filter = HistoryFilter::Error;
        drop(program.view(&state, iced::window::Id::unique()));
    }

    #[test]
    fn history_clear_raises_the_flag() {
        let program = HistoryProgram {
            language: en(),
            clear_requested: Default::default(),
        };
        let (mut state, _) = program.boot();
        assert!(
            !program
                .clear_requested
                .load(std::sync::atomic::Ordering::Acquire)
        );
        let _ = program.update(&mut state, HistoryMessage::Clear);
        assert!(
            state
                .clear_requested
                .load(std::sync::atomic::Ordering::Acquire)
        );
    }
}

#[cfg(test)]
mod overlay_tests {
    use super::*;
    use nerust_gui_shell::session::osd::{OsdRow, OsdSnapshot};

    fn snapshot_with_rows() -> OsdSnapshot {
        OsdSnapshot {
            rows: vec![
                OsdRow {
                    text: "Paused".to_string(),
                    rgb: (255, 255, 255),
                    tag: "[INFO]",
                },
                OsdRow {
                    text: "slot is empty".to_string(),
                    rgb: (255, 200, 0),
                    tag: "[WARN]",
                },
            ],
            status: Some("60.0 fps".to_string()),
            ..OsdSnapshot::default()
        }
    }

    #[test]
    fn overlay_sync_and_view_build_headless() {
        let program = OsdProgram;
        let (mut state, _) = program.boot();
        let _ = program.update(&mut state, OsdMessage::Sync(snapshot_with_rows()));
        drop(program.view(&state, iced::window::Id::unique()));
        // Status-less snapshot renders too.
        let _ = program.update(&mut state, OsdMessage::Sync(OsdSnapshot::default()));
        drop(program.view(&state, iced::window::Id::unique()));
    }
}
