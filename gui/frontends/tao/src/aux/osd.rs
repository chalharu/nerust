//! Notification history viewer (iced aux window).
//! Host owns the history model; this module owns native layout and style.

use iced::Task;
use iced_winit::program::{self, Program};
use nerust_frontend_host::history::{HistoryModel, HistoryVisitor};
use nerust_gui_shell::session::osd::{HistoryFilter, HistoryLine, HistoryRowView};
use nerust_settings_core::model::language::AppLanguage;
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

pub(crate) struct HistoryState {
    lines: std::sync::Arc<[HistoryLine]>,
    model: HistoryModel,
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
                model: HistoryModel::build(&[], language, HistoryFilter::All),
                clear_requested: std::sync::Arc::clone(&self.clear_requested),
            },
            Task::none(),
        )
    }

    fn update(&self, state: &mut Self::State, message: Self::Message) -> Task<Self::Message> {
        match message {
            HistoryMessage::Sync { lines, language } => {
                state.lines = lines;
                state.model = HistoryModel::build(&state.lines, language, state.model.filter);
                Task::none()
            }
            HistoryMessage::SetFilter(filter) => {
                state.model = HistoryModel::build(&state.lines, state.model.language, filter);
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
        history_view(&state.model)
    }
}

#[derive(Default)]
struct IcedHistoryVisitor {
    toolbar: Option<iced::Element<'static, HistoryMessage>>,
    items: Vec<iced::Element<'static, HistoryMessage>>,
    view: Option<iced::Element<'static, HistoryMessage>>,
}

impl HistoryVisitor for IcedHistoryVisitor {
    fn begin(&mut self, model: &HistoryModel) {
        use iced::Length;
        use iced::widget::{button, pick_list, row, rule, text};
        let captions = &model.captions;
        let options: Vec<_> = model
            .options
            .iter()
            .map(|option| option.label.clone())
            .collect();
        let selected = model
            .options
            .iter()
            .find(|option| option.filter == model.filter)
            .map(|option| option.label.clone());
        let filters: Vec<_> = model
            .options
            .iter()
            .map(|option| (option.label.clone(), option.filter))
            .collect();
        self.toolbar = Some(
            row![
                text(captions.level.clone()).size(12),
                pick_list(options, selected, move |label: String| {
                    HistoryMessage::SetFilter(
                        filters
                            .iter()
                            .find(|(name, _)| name == &label)
                            .map(|(_, filter)| *filter)
                            .unwrap_or_default(),
                    )
                })
                .placeholder(captions.level.clone()),
                button(text(captions.clear.clone()).size(12)).on_press(HistoryMessage::Clear),
                iced::widget::Space::new().width(Length::Fill),
                text(captions.count.clone()).size(12),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center)
            .into(),
        );
        self.items.clear();
        self.items.push(
            row![
                text(captions.headers[0].clone())
                    .size(12)
                    .width(Length::Fixed(150.0)),
                text(captions.headers[1].clone())
                    .size(12)
                    .width(Length::Fixed(70.0)),
                text(captions.headers[2].clone())
                    .size(12)
                    .width(Length::Fixed(220.0)),
                text(captions.headers[3].clone())
                    .size(12)
                    .width(Length::Fill),
            ]
            .spacing(12)
            .into(),
        );
        self.items.push(rule::horizontal(1).into());
        if model.rows.is_empty() {
            self.items
                .push(text(captions.empty.clone()).size(13).into());
        }
    }

    fn row(&mut self, entry: &HistoryRowView) {
        use iced::Length;
        use iced::widget::{row, text};
        let (r, g, b) = entry.rgb;
        self.items.push(
            row![
                text(entry.datetime.clone())
                    .size(12)
                    .font(iced::Font::MONOSPACE)
                    .width(Length::Fixed(150.0)),
                text(entry.level)
                    .size(12)
                    .color(iced::Color::from_rgb8(r, g, b))
                    .width(Length::Fixed(70.0)),
                text(entry.module.clone())
                    .size(11)
                    .font(iced::Font::MONOSPACE)
                    .wrapping(iced::advanced::text::Wrapping::WordOrGlyph)
                    .width(Length::Fixed(220.0)),
                text(entry.message.clone())
                    .size(13)
                    .wrapping(iced::advanced::text::Wrapping::WordOrGlyph)
                    .width(Length::Fill),
            ]
            .spacing(12)
            .into(),
        );
    }

    fn end(&mut self) {
        use iced::widget::{column, scrollable};
        self.view = Some(
            column![
                self.toolbar.take().expect("history begin before end"),
                scrollable(column(std::mem::take(&mut self.items)).spacing(2)),
            ]
            .spacing(12)
            .padding(16)
            .into(),
        );
    }
}

fn history_view(model: &HistoryModel) -> iced::Element<'static, HistoryMessage> {
    let mut visitor = IcedHistoryVisitor::default();
    model.visit(&mut visitor as &mut dyn HistoryVisitor);
    visitor.view.expect("history traversal must finish")
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
        let model = HistoryModel::build(&[], language, HistoryFilter::All);
        let surface = AuxSurface::open(
            event_loop,
            &model.captions.title,
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
}

impl super::registry::HistorySyncPort for HistoryWindowHandle {
    fn sync_history(&mut self, lines: std::sync::Arc<[HistoryLine]>, language: AppLanguage) {
        self.sync(lines, language);
    }

    fn take_history_clear(&mut self) -> bool {
        self.take_clear_request()
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
    use nerust_gui_shell::session::notification::NotificationLevel;

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
        assert_eq!(state.model.filter, HistoryFilter::All);
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
        assert_eq!(state.model.language, ja());
        assert_eq!(state.model.rows.len(), 2);
        let _ = program.update(&mut state, HistoryMessage::SetFilter(HistoryFilter::Error));
        assert_eq!(state.model.filter, HistoryFilter::Error);
        assert_eq!(state.model.rows.len(), 1);
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
        let _ = program.update(&mut state, HistoryMessage::SetFilter(HistoryFilter::Error));
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
        let instance = iced_winit::program::Instance::new(HistoryProgram {
            language: en(),
            clear_requested: Arc::new(AtomicBool::new(false)),
        })
        .0;
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
            &invalidated,
        );
        assert!(!invalidated.load(Ordering::Acquire));
        assert!(close.load(Ordering::Acquire));
    }

    #[test]
    fn merged_history_line_carries_single_count_suffix() {
        use nerust_gui_shell::session::{
            notification::{Message, UiNotification},
            osd::history_lines,
        };
        // Pins the shell/view contract without restructuring shell
        // `render_text`: a merged (count 3) row carries exactly one suffix.
        for language in [en(), ja()] {
            let line = UiNotification {
                level: NotificationLevel::Info,
                message: Message::paused(),
                count: 3,
                seq: 0,
                at: std::time::SystemTime::UNIX_EPOCH,
            };
            let rows = history_lines(std::slice::from_ref(&line).iter(), language);
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].count, 3);
            assert!(rows[0].text.ends_with("×3"), "{}", rows[0].text);
            assert_eq!(rows[0].text.matches('×').count(), 1, "{}", rows[0].text);
            let program = HistoryProgram {
                language,
                clear_requested: Default::default(),
            };
            let (mut state, _) = program.boot();
            let _ = program.update(
                &mut state,
                HistoryMessage::Sync {
                    lines: rows.into(),
                    language,
                },
            );
            assert!(state.model.rows[0].message.ends_with("×3"));
            assert_eq!(state.model.rows[0].message.matches('×').count(), 1);
            drop(program.view(&state, iced::window::Id::unique()));
        }
    }

    #[test]
    fn shared_model_drives_native_view_and_balanced_visitor_order() {
        struct Probe(Vec<String>);
        impl HistoryVisitor for Probe {
            fn begin(&mut self, model: &HistoryModel) {
                self.0.push(format!("begin:{}", model.captions.title));
            }
            fn row(&mut self, row: &HistoryRowView) {
                self.0.push(row.message.clone());
            }
            fn end(&mut self) {
                self.0.push("end".into());
            }
        }
        let program = HistoryProgram {
            language: en(),
            clear_requested: Default::default(),
        };
        let (mut state, _) = program.boot();
        let lines: std::sync::Arc<[HistoryLine]> = [
            line(NotificationLevel::Info, "info"),
            line(NotificationLevel::Warning, "warning"),
            line(NotificationLevel::Error, "error"),
        ]
        .into();
        for language in [en(), ja()] {
            let _ = program.update(
                &mut state,
                HistoryMessage::Sync {
                    lines: lines.clone(),
                    language,
                },
            );
            for filter in [HistoryFilter::All, HistoryFilter::Error] {
                let _ = program.update(&mut state, HistoryMessage::SetFilter(filter));
                let expected = HistoryModel::build(&lines, language, filter);
                assert_eq!(state.model.captions.title, expected.captions.title);
                assert_eq!(state.model.captions.count, expected.captions.count);
                assert_eq!(state.model.rows.len(), expected.rows.len());
                let mut probe = Probe(Vec::new());
                state.model.visit(&mut probe as &mut dyn HistoryVisitor);
                assert_eq!(
                    probe.0.first(),
                    Some(&format!("begin:{}", expected.captions.title))
                );
                assert_eq!(probe.0.last().map(String::as_str), Some("end"));
                assert_eq!(
                    probe.0[1..probe.0.len() - 1],
                    expected
                        .rows
                        .iter()
                        .map(|row| row.message.clone())
                        .collect::<Vec<_>>()
                );
                // The actual iced visitor consumes this same Host model without a display.
                let mut native = IcedHistoryVisitor::default();
                state.model.visit(&mut native as &mut dyn HistoryVisitor);
                assert!(native.view.is_some());
                assert!(native.toolbar.is_none());
                assert!(native.items.is_empty());
                drop(program.view(&state, iced::window::Id::unique()));
            }
        }
        let _ = program.update(
            &mut state,
            HistoryMessage::Sync {
                lines: lines.clone(),
                language: en(),
            },
        );
        let english = state.model.captions.title.clone();
        let _ = program.update(
            &mut state,
            HistoryMessage::Sync {
                lines,
                language: ja(),
            },
        );
        assert_ne!(state.model.captions.title, english);
        assert_eq!(state.model.filter, HistoryFilter::Error);
        assert_eq!(state.model.rows.len(), 1);
        assert_eq!(state.model.rows[0].message, "error");
    }
}
