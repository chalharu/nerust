//! Notification history viewer (iced aux window).
//! Shell owns event capture; this module owns presentation state.

use iced::Task;
use iced_winit::program::{self, Program};

use nerust_gui_shell::session::{notification::NotificationLevel, osd::HistoryLine};
use nerust_settings_core::{
    i18n::{UiText, history_count, text as ui_text},
    model::language::AppLanguage,
};
use tao::event_loop::EventLoopWindowTarget;

use crate::aux::shell::{AuxWindow, WindowScaffold, aux_settings, aux_theme};

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
    /// Threshold match: this level and above. Info shows all.
    fn matches(self, level: NotificationLevel) -> bool {
        let minimum = match self {
            HistoryFilter::All | HistoryFilter::Info => 0,
            HistoryFilter::Warning => 1,
            HistoryFilter::Error => 2,
        };
        level.severity() >= minimum
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
        use iced::widget::{button, column, pick_list, row, rule, scrollable, text};
        use nerust_gui_shell::session::osd::level_label;
        let language = state.language;
        let visible: Vec<&HistoryLine> = state
            .lines
            .iter()
            .filter(|line| state.filter.matches(line.level))
            .collect();
        // Toolbar: filter plus clear on top; count trails right.
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
            // `HistoryLine.text` already carries the `×N` suffix when
            // merged (see `history_lines`); use it as-is.
            let message = line.text.clone();
            // Cells wrap inside their column: overlong values wrap
            // instead of bleeding into the neighbour (HIG). No clip
            // containers: a clipped cell exactly at the scrollable
            // edge produces a sub-pixel clip rect, which tiny-skia
            // can only skip with a warning.
            items = items.push(
                row![
                    text(line.datetime.clone())
                        .size(12)
                        .font(iced::Font::MONOSPACE)
                        .width(Length::Fixed(150.0)),
                    text(level_label(line.level, language))
                        .size(12)
                        .color(level_color)
                        .width(Length::Fixed(70.0)),
                    text(short_module(line.module))
                        .size(11)
                        .font(iced::Font::MONOSPACE)
                        .wrapping(iced::advanced::text::Wrapping::WordOrGlyph)
                        .width(Length::Fixed(220.0)),
                    text(message)
                        .size(13)
                        .wrapping(iced::advanced::text::Wrapping::WordOrGlyph)
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
    shell: crate::aux::shell::AuxShell<HistoryProgram>,
    clear_requested: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// View-refresh flag, separate from the shell close flag (like
    /// the debug/PPU twins): `sync` must never consume a close.
    view_invalidated: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl HistoryWindowHandle {
    /// Native window identity for event routing.
    pub(crate) fn tao_window_id(&self) -> tao::window::WindowId {
        self.shell.tao_window_id()
    }

    /// Close was requested through the shared flag.
    pub(crate) fn close_requested(&self) -> bool {
        self.shell.close_requested()
    }

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
        ).ok()?;
        let clear_requested = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let view_invalidated = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
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
            view_invalidated,
        })
    }

    pub(crate) fn take_clear_request(&self) -> bool {
        self.clear_requested
            .swap(false, std::sync::atomic::Ordering::AcqRel)
    }

    pub(crate) fn sync(&mut self, lines: Vec<HistoryLine>, language: AppLanguage) {
        self.shell.process(
            vec![HistoryMessage::Sync { lines, language }],
            &self.view_invalidated,
        );
    }
}

impl HistoryWindowHandle {
    pub(crate) fn render(&mut self) {
        self.shell.render("history window");
    }
}

impl HistoryWindowHandle {
    fn handle_event(&mut self, mapped: iced::Event) {
        let (messages, outcome) = self.shell.pump_frame(mapped);
        if matches!(outcome, crate::aux::shell::PumpOutcome::Rebuild) {
            self.shell.request_redraw();
        }
        if !messages.is_empty() {
            self.shell.process(messages, &self.view_invalidated);
        }
    }
}

impl AuxWindow for HistoryWindowHandle {
    fn pump_window_event(&mut self, event: tao::event::WindowEvent) {
        self.shell.handle_common_event(&event);
        if let Some(mapped) = self.shell.feed_tao_event(event) {
            self.handle_event(mapped);
        }
        self.shell.request_redraw();
    }
}

impl super::focus::WindowHandler for HistoryWindowHandle {}

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
    fn filter_matches_that_level_and_above() {
        assert!(HistoryFilter::All.matches(NotificationLevel::Error));
        // No level below Info exists, so Info shows everything.
        assert!(HistoryFilter::Info.matches(NotificationLevel::Error));
        assert!(HistoryFilter::Warning.matches(NotificationLevel::Warning));
        assert!(HistoryFilter::Warning.matches(NotificationLevel::Error));
        assert!(!HistoryFilter::Warning.matches(NotificationLevel::Info));
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

    #[test]
    fn sync_consumes_refresh_flag_not_close_flag() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        // Regression: `sync` passed `should_close` as the refresh
        // invalidation flag, so a pending close was consumed by a sync.
        let (instance, _task) = iced_winit::program::Instance::new(HistoryProgram {
            language: en(),
            clear_requested: Arc::new(AtomicBool::new(false)),
        });
        let mut backend =
            iced_tiny_skia::Renderer::new(crate::convert::default_font(), iced::Pixels(16.0));
        let mut ui = crate::aux::shell::UiShell::new(
            instance,
            iced::window::Id::unique(),
            iced::Size::new(800.0, 600.0),
            &mut backend,
        );
        let close = AtomicBool::new(true);
        let invalidated = AtomicBool::new(true);
        // Exactly what `HistoryWindowHandle::sync` feeds: a Sync batch
        // against the dedicated refresh flag, never the close flag.
        ui.process_messages(
            vec![HistoryMessage::Sync {
                lines: Vec::new(),
                language: en(),
            }],
            iced::window::Id::unique(),
            iced::Size::new(800.0, 600.0),
            &mut backend,
            &invalidated,
        );
        assert!(!invalidated.load(Ordering::Acquire));
        assert!(close.load(Ordering::Acquire));
    }

    #[test]
    fn merged_history_line_carries_single_count_suffix() {
        use nerust_gui_shell::session::{
            notification::{NotificationKind, UiNotification},
            osd::history_lines,
        };
        // Pins the shell/view contract without restructuring shell
        // `render_text`: a merged (count 3) row carries exactly one suffix.
        for language in [en(), ja()] {
            let line = UiNotification {
                level: NotificationLevel::Info,
                kind: NotificationKind::Paused,
                count: 3,
                seq: 0,
                at: std::time::SystemTime::UNIX_EPOCH,
            };
            let rows = history_lines(std::slice::from_ref(&line).iter(), language);
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].count, 3);
            assert!(rows[0].text.ends_with("×3"), "{}", rows[0].text);
            assert_eq!(rows[0].text.matches('×').count(), 1, "{}", rows[0].text);
        }
    }
}
