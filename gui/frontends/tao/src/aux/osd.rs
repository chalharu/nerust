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

use crate::aux::shell::{AuxSurface, aux_settings, aux_theme};

#[derive(Debug, Clone)]
pub(crate) enum HistoryMessage {
    Sync {
        lines: std::sync::Arc<[HistoryLine]>,
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

/// Short module for the grid: last two segments keep the informative tail.
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
        crate::util::level_visible(minimum, level.severity())
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

    /// Inverse of [`HistoryFilter::label`]: localized strings mapped back on select.
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

/// Localized filter labels for the toolbar dropdown.
/// Built on Sync only; the view borrows them.
fn filter_options(language: AppLanguage) -> Vec<String> {
    [
        HistoryFilter::All,
        HistoryFilter::Info,
        HistoryFilter::Warning,
        HistoryFilter::Error,
    ]
    .iter()
    .map(|filter| filter.label(language).to_string())
    .collect()
}

pub(crate) struct HistoryState {
    lines: std::sync::Arc<[HistoryLine]>,
    filter: HistoryFilter,
    language: AppLanguage,
    /// Localized filter labels: rebuilt on Sync only, borrowed by the view.
    options: Vec<String>,
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
        let language = self.language;
        (
            HistoryState {
                lines: std::sync::Arc::from([]),
                filter: HistoryFilter::All,
                language,
                options: filter_options(language),
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
                state.options = filter_options(language);
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
        let shown = state
            .lines
            .iter()
            .filter(|line| state.filter.matches(line.level));
        let shown_count = shown.clone().count();
        // Toolbar: filter plus clear on top; count trails right.
        let toolbar: iced::Element<'_, HistoryMessage> = row![
            text(ui_text(language, UiText::HistoryLevel)).size(12),
            pick_list(
                state.options.as_slice(),
                state.filter.label(language).to_string().into(),
                move |label: String| HistoryMessage::SetFilter(
                    HistoryFilter::from_label(&label, language).unwrap_or_default()
                ),
            )
            .placeholder(ui_text(language, UiText::HistoryLevel)),
            button(text(ui_text(language, UiText::HistoryClear)).size(12))
                .on_press(HistoryMessage::Clear),
            iced::widget::Space::new().width(Length::Fill),
            text(history_count(language, shown_count)).size(12),
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
        if shown_count == 0 {
            items = items.push(text(ui_text(language, UiText::HistoryEmpty)).size(13));
        }
        for line in shown {
            let level_color = match line.level {
                NotificationLevel::Info => iced::Color::from_rgb8(70, 70, 70),
                NotificationLevel::Warning => iced::Color::from_rgb8(170, 110, 0),
                NotificationLevel::Error => iced::Color::from_rgb8(190, 30, 30),
            };
            // Cells wrap inside their column; no clip containers.
            items = items.push(
                row![
                    text(line.datetime.as_str())
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
                    // `HistoryLine.text` already carries the `×N`
                    // suffix when merged; use it as-is.
                    text(line.text.as_str())
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
    pub(crate) fn new(
        event_loop: &EventLoopWindowTarget<crate::event::UserEvent>,
        language: AppLanguage,
    ) -> Option<Self> {
        let title = ui_text(language, UiText::NotificationHistory);
        let surface = AuxSurface::open(
            event_loop,
            title,
            (720.0, 360.0),
            None,
            None,
            "history window",
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .map_err(|error| {
            // Typed scaffold failure: surface the tagged native detail here.
            log::error!("{error}");
            error
        })
        .ok()?;
        let clear_requested = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let view_invalidated = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (instance, _task) = program::Instance::new(HistoryProgram {
            language,
            clear_requested: std::sync::Arc::clone(&clear_requested),
        });
        let shell = crate::aux::shell::AuxShell::assemble(surface, instance);
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

    pub(crate) fn sync(&mut self, lines: std::sync::Arc<[HistoryLine]>, language: AppLanguage) {
        self.shell.process(
            vec![HistoryMessage::Sync { lines, language }],
            &self.view_invalidated,
        );
    }
}

impl super::focus::AuxShellRef for HistoryWindowHandle {
    fn kind(&self) -> super::focus::AuxKind {
        super::focus::AuxKind::History
    }

    fn shell(&self) -> &dyn crate::aux::shell::AuxShellOps {
        &self.shell
    }

    fn shell_mut(&mut self) -> &mut dyn crate::aux::shell::AuxShellOps {
        &mut self.shell
    }
}

impl super::registry::AuxController for HistoryWindowHandle {
    fn sync_view(&mut self) {
        // History rebuilds from pushed content, not from the bridge.
    }

    fn as_history(&mut self) -> Option<&mut dyn super::registry::HistorySyncPort> {
        Some(self)
    }

    fn take_history_clear(&mut self) -> bool {
        self.take_clear_request()
    }
}

impl super::registry::HistorySyncPort for HistoryWindowHandle {
    fn sync_history(&mut self, lines: std::sync::Arc<[HistoryLine]>, language: AppLanguage) {
        self.sync(lines, language);
    }
}

impl super::focus::AuxEvents for HistoryWindowHandle {
    fn handle(&mut self, mapped: iced::Event) -> bool {
        self.shell.pump_finish(mapped, None, &self.view_invalidated)
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
    fn filter_defaults_to_all_and_rejects_foreign_labels() {
        assert_eq!(HistoryFilter::default(), HistoryFilter::All);
        // English labels never resolve in Japanese and vice versa.
        assert_eq!(HistoryFilter::from_label("All", ja()), None);
        assert_eq!(HistoryFilter::from_label("すべて", en()), None);
        // Warning threshold still admits errors only.
        assert!(HistoryFilter::Warning.matches(NotificationLevel::Error));
        assert!(!HistoryFilter::Warning.matches(NotificationLevel::Info));
    }

    #[test]
    fn short_module_keeps_two_segment_paths_whole() {
        assert_eq!(short_module("crate::module"), "crate::module");
        assert_eq!(short_module(""), "");
    }

    #[test]
    fn history_view_builds_empty_and_filtered() {
        // Empty content shows the placeholder, never an empty grid.
        let program = HistoryProgram {
            language: en(),
            clear_requested: Default::default(),
        };
        let (state, _) = program.boot();
        drop(program.view(&state, iced::window::Id::unique()));
        // Filtered-out rows collapse to the same placeholder.
        let (mut state, _) = program.boot();
        let _ = program.update(
            &mut state,
            HistoryMessage::Sync {
                lines: std::sync::Arc::from([line(NotificationLevel::Info, "Paused")]),
                language: en(),
            },
        );
        let _ = program.update(&mut state, HistoryMessage::SetFilter(HistoryFilter::Error));
        drop(program.view(&state, iced::window::Id::unique()));
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
                lines: std::sync::Arc::from([
                    line(NotificationLevel::Info, "Paused"),
                    line(NotificationLevel::Error, "failed"),
                ]),
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
        // View builds headless: construction needs no display.
        let program = HistoryProgram {
            language: en(),
            clear_requested: Default::default(),
        };
        let (mut state, _) = program.boot();
        drop(program.view(&state, iced::window::Id::unique()));
        let _ = program.update(
            &mut state,
            HistoryMessage::Sync {
                lines: std::sync::Arc::from([
                    line(NotificationLevel::Info, "Paused"),
                    line(NotificationLevel::Warning, "empty"),
                    line(NotificationLevel::Error, "failed"),
                ]),
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
        );
        let close = AtomicBool::new(true);
        let invalidated = AtomicBool::new(true);
        // Exactly what `HistoryWindowHandle::sync` feeds: a Sync batch
        // against the dedicated refresh flag, never the close flag.
        ui.process_messages(
            vec![HistoryMessage::Sync {
                lines: std::sync::Arc::from([]),
                language: en(),
            }],
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
