use nerust_gui_shell::session::osd::{
    HistoryFilter, HistoryLine, HistoryRowView, history_row_view, visible_history,
};
use nerust_settings_core::{
    i18n::{I18n, UiText},
    model::language::AppLanguage,
};
use panechord::backend::View;
use panechord::{
    Binding, Choice, Command, Executor, Store, Table, TableCell, TableColumn, TableRow,
    TextOverflow, TextRole, TextStyle,
};
use std::{cell::RefCell, rc::Rc, sync::Arc};

#[derive(Clone)]
struct Model {
    title: String,
    level_label: String,
    clear_label: String,
    close_label: String,
    headers: [String; 4],
    count: String,
    empty_label: String,
    empty: bool,
    choices: Vec<Choice>,
    table: Table,
}

pub(crate) struct Pane {
    model: Store<Model>,
    filter: Store<String>,
    revision: RefCell<Option<(u64, AppLanguage, String)>>,
    cache: RefCell<nerust_gui_shell::session::session_query::HistoryView>,
}

fn project(lines: &[HistoryLine], i18n: I18n, filter: HistoryFilter) -> Vec<HistoryRowView> {
    visible_history(lines, filter)
        .into_iter()
        .map(|line| history_row_view(line, i18n))
        .collect()
}

fn history_table(headers: [String; 4], empty_text: String, rows: Vec<HistoryRowView>) -> Table {
    // Explicit widths on the bounded columns: tao sizes header and body rows
    // independently, so Shrink columns never line up across the rule. Fixed
    // widths pin header and body to the same tracks on every backend (gtk
    // SizeGroups and macOS NSTableColumns honor them too); only the message
    // column stretches. Datetime is always 19 chars of 5/6-scale monospace.
    let widths = [Some(160.0), Some(80.0), Some(150.0), None];
    let columns = headers
        .into_iter()
        .enumerate()
        .map(|(index, title)| TableColumn {
            title,
            width: widths[index],
            text: if index == 0 || index == 2 {
                TextStyle {
                    role: TextRole::Code,
                    scale: 5.0 / 6.0,
                    ..Default::default()
                }
            } else {
                TextStyle::default()
            },
            overflow: match index {
                2 => TextOverflow::EllipsizeMiddle,
                3 => TextOverflow::Wrap,
                _ => TextOverflow::Clip,
            },
            stretch: index == 3,
            hide_empty: false,
        })
        .collect::<Vec<_>>()
        .into();
    let rows = rows
        .into_iter()
        .enumerate()
        .map(|(index, row)| {
            let color = Some([row.rgb.0, row.rgb.1, row.rgb.2]);
            Arc::new(TableRow {
                key: index.to_string(),
                cells: vec![
                    TableCell {
                        text: row.datetime,
                        foreground: None,
                    },
                    TableCell {
                        text: row.level.into(),
                        foreground: color,
                    },
                    TableCell {
                        text: row.module,
                        foreground: None,
                    },
                    TableCell {
                        text: row.message,
                        foreground: None,
                    },
                ],
            })
        })
        .collect::<Vec<_>>()
        .into();
    Table {
        columns,
        rows,
        empty_text,
        column_gap: 12.0,
        row_gap: 2.0,
        row_inset: 0.0,
    }
}

impl Pane {
    pub(crate) fn new(language: AppLanguage, executor: Rc<dyn Executor>) -> Rc<Self> {
        let i18n = I18n::resolve_os(language);
        let label = move |key| i18n.text(key).to_owned();
        let headers = [
            UiText::HistoryDatetime,
            UiText::HistoryLevel,
            UiText::HistoryModule,
            UiText::HistoryMessage,
        ]
        .map(label);
        let empty_label = label(UiText::HistoryEmpty);
        let table = history_table(headers.clone(), empty_label.clone(), Vec::new());
        Rc::new(Self {
            model: Store::new(
                Model {
                    title: label(UiText::NotificationHistory),
                    level_label: label(UiText::HistoryLevel),
                    clear_label: label(UiText::HistoryClear),
                    close_label: label(UiText::Close),
                    headers,
                    count: i18n.history_count(0),
                    empty_label,
                    empty: true,
                    choices: HistoryFilter::all()
                        .into_iter()
                        .map(|filter| Choice {
                            id: filter.id().into(),
                            label: filter.label(i18n).into(),
                        })
                        .collect(),
                    table,
                },
                executor.clone(),
            ),
            filter: Store::new(HistoryFilter::All.id().into(), executor.clone()),
            revision: RefCell::new(None),
            cache: RefCell::new(nerust_gui_shell::session::session_query::HistoryView::new()),
        })
    }

    pub(crate) fn needs_refresh(&self, revision: u64, language: AppLanguage) -> bool {
        if self.model.is_closed() {
            return false;
        }
        let stored = self.revision.borrow();
        match stored.as_ref() {
            Some((stored_revision, stored_language, _))
                if (*stored_revision, *stored_language) != (revision, language) =>
            {
                true
            }
            // Same revision: only a filter change refreshes. The borrow
            // gate avoids the `snapshot()` String clone on quiet ticks.
            Some((_, _, stored_filter)) => self.filter.with(|current| *stored_filter != *current),
            None => true,
        }
    }

    pub(crate) fn snapshot(
        &self,
        application: &nerust_gui_application::Application<crate::SessionAdapter>,
    ) -> Option<(u64, AppLanguage, Vec<HistoryLine>)> {
        let revision = application.session().history_revision();
        let language = application.settings().shared.general.language;
        self.needs_refresh(revision, language).then(|| {
            (
                revision,
                language,
                application
                    .session()
                    .history_lines(&mut self.cache.borrow_mut(), language),
            )
        })
    }

    pub(crate) fn refresh(&self, revision: u64, language: AppLanguage, lines: &[HistoryLine]) {
        if !self.needs_refresh(revision, language) {
            return;
        }
        let selection = self.filter.snapshot();
        let i18n = I18n::resolve_os(language);
        let rows = project(lines, i18n, HistoryFilter::from_id(Some(&selection)));
        let count = i18n.history_count(rows.len());
        let _ = self.model.update(move |model| {
            model.empty = rows.is_empty();
            model.count = count;
            let label = move |key| i18n.text(key).to_owned();
            model.title = label(UiText::NotificationHistory);
            model.level_label = label(UiText::HistoryLevel);
            model.clear_label = label(UiText::HistoryClear);
            model.close_label = label(UiText::Close);
            model.empty_label = label(UiText::HistoryEmpty);
            model.headers = [
                UiText::HistoryDatetime,
                UiText::HistoryLevel,
                UiText::HistoryModule,
                UiText::HistoryMessage,
            ]
            .map(label);
            model.table = history_table(model.headers.clone(), model.empty_label.clone(), rows);
            model.choices = HistoryFilter::all()
                .into_iter()
                .map(|filter| Choice {
                    id: filter.id().into(),
                    label: filter.label(i18n).into(),
                })
                .collect();
        });
        self.revision.replace(Some((revision, language, selection)));
    }

    pub(crate) fn build(self: &Rc<Self>, clear: Rc<dyn Fn()>) -> Result<View, String> {
        let close_view = Rc::new(RefCell::new(None::<panechord::backend::WeakView>));
        let close = close_view.clone();
        let close = self.model.command(
            |_| true,
            move |_, ()| {
                if let Some(view) = close
                    .borrow()
                    .as_ref()
                    .and_then(panechord::backend::WeakView::upgrade)
                {
                    view.close();
                }
            },
            Binding::constant(()),
        );
        let clear = self.model.command(
            |model| !model.empty,
            move |_, ()| clear(),
            Binding::constant(()),
        );
        let bindings = Presentation {
            title: self.model.bind(|model| model.title.clone(), None),
            level_label: self.model.bind(|model| model.level_label.clone(), None),
            clear_label: self.model.bind(|model| model.clear_label.clone(), None),
            close_label: self.model.bind(|model| model.close_label.clone(), None),
            count: self.model.bind(|model| model.count.clone(), None),
            choices: self.model.bind(|model| model.choices.clone(), None),
            selected: self.filter.bind(
                Clone::clone,
                Some(|filter: &mut String, value| *filter = value),
            ),
            table: self.model.bind(|model| model.table.clone(), None),
            clear,
            close,
        };
        let view = panechord::try_view!("ui/history.yaml", bindings, panechord::backend)?;
        close_view.replace(Some(view.downgrade()));
        let pane = self.clone();
        view.on_close(Rc::new(move || {
            pane.model.close();
            pane.filter.close();
        }));
        Ok(view)
    }
}

#[derive(Clone)]
struct Presentation {
    title: Binding<String>,
    level_label: Binding<String>,
    clear_label: Binding<String>,
    close_label: Binding<String>,
    count: Binding<String>,
    choices: Binding<Vec<Choice>>,
    selected: Binding<String>,
    table: Binding<Table>,
    clear: Command,
    close: Command,
}
impl Presentation {
    fn command_clear(&self, _: Binding<String>) -> Command {
        self.clear.clone()
    }
    fn command_close(&self, _: Binding<String>) -> Command {
        self.close.clone()
    }
}

#[cfg(test)]
mod history_operation_tests {
    //! History revision gating and long-row behavior through a real
    //! `SessionAdapter` (NES core). Row projection is pinned by the unit
    //! tests above; these pin the snapshot/refresh wiring headlessly.
    use super::*;
    use crate::session::test_fixture::loaded_adapter;
    use nerust_gui_application::{Application, SessionAction, SettingsPort};

    struct TestExecutor;
    impl Executor for TestExecutor {
        fn spawn(&self, _: std::pin::Pin<Box<dyn std::future::Future<Output = ()>>>) {
            panic!("history flow must not dispatch async work");
        }
    }

    #[test]
    fn revision_gating_and_long_rows() {
        let (adapter, _dir) = loaded_adapter();
        let language = adapter.settings().shared.general.language;
        let mut app = Application::new(adapter);
        let pane = Pane::new(language, Rc::new(TestExecutor));
        // Initial snapshot, then refresh caches the revision.
        let (revision, language, lines) =
            pane.snapshot(&app).expect("first snapshot is always due");
        assert!(lines.is_empty(), "no events recorded yet");
        pane.refresh(revision, language, &lines);
        assert!(pane.snapshot(&app).is_none(), "cached revision is quiet");
        // Slot traffic records events and bumps the revision.
        assert!(
            app.execute(SessionAction::CreateSlot)
                .expect("slot creates")
        );
        let (revision, language, lines) = pane.snapshot(&app).expect("new revision is due");
        pane.refresh(revision, language, &lines);
        assert!(
            pane.model
                .snapshot()
                .table
                .rows
                .iter()
                .any(|row| row.cells.iter().any(|cell| cell.text.contains("Slot"))),
            "slot traffic reaches the grid"
        );
        // Flood past the bounded log: the grid stays capped, newest last.
        for _ in 0..60 {
            assert!(
                app.execute(SessionAction::CreateSlot)
                    .expect("slot creates")
            );
        }
        let (revision, language, lines) = pane.snapshot(&app).expect("flooded revision is due");
        pane.refresh(revision, language, &lines);
        let rows = &pane.model.snapshot().table.rows;
        assert!(!rows.is_empty());
        assert!(
            rows.len() <= 50,
            "bounded history caps long rows: {}",
            rows.len()
        );
        assert!(pane.snapshot(&app).is_none(), "cache catches up");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_gui_shell::session::notification::NotificationLevel;
    #[test]
    fn projection_uses_shared_threshold_and_preformatted_message() {
        let lines: Vec<_> = [
            NotificationLevel::Info,
            NotificationLevel::Warning,
            NotificationLevel::Error,
        ]
        .into_iter()
        .map(|level| HistoryLine {
            datetime: "2026-10-31 12:00:00".into(),
            level,
            module: "nerust::notification",
            text: "event \u{00d7}2".into(),
            count: 2,
        })
        .collect();
        let rows = project(
            &lines,
            I18n::new(AppLanguage::English),
            HistoryFilter::Warning,
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].message, "event \u{00d7}2");
        assert_eq!(rows[0].level, "Warning");
        assert_eq!(
            project(
                &lines,
                I18n::new(AppLanguage::English),
                HistoryFilter::Error
            )
            .len(),
            1
        );
    }
}

#[cfg(test)]
mod history_headless_tests {
    //! Closed-pane, filter-gating and command-projection paths.
    use super::*;
    use nerust_gui_shell::session::notification::NotificationLevel;

    struct NoDispatch;
    impl Executor for NoDispatch {
        fn spawn(&self, _: std::pin::Pin<Box<dyn std::future::Future<Output = ()>>>) {
            panic!("history headless must not dispatch");
        }
    }

    fn lines() -> Vec<HistoryLine> {
        [NotificationLevel::Info, NotificationLevel::Warning]
            .into_iter()
            .map(|level| HistoryLine {
                datetime: "2026-10-31 12:00:00".into(),
                level,
                module: "m",
                text: "t".into(),
                count: 1,
            })
            .collect()
    }

    #[test]
    fn closed_pane_needs_no_refresh() {
        let pane = Pane::new(AppLanguage::English, Rc::new(NoDispatch));
        pane.model.close();
        pane.filter.close();
        assert!(!pane.needs_refresh(99, AppLanguage::English));
    }

    #[test]
    fn refresh_is_noop_without_revision_or_filter_change() {
        let pane = Pane::new(AppLanguage::English, Rc::new(NoDispatch));
        let data = lines();
        pane.refresh(7, AppLanguage::English, &data);
        assert!(pane.model.update(|model| model.count = "sentinel".into()));
        pane.refresh(7, AppLanguage::English, &data);
        assert_eq!(pane.model.snapshot().count, "sentinel");
        assert!(pane.model.update(|model| model.count = "other".into()));
        assert!(
            pane.filter
                .update(|selected| *selected = HistoryFilter::Warning.id().into())
        );
        assert!(pane.needs_refresh(7, AppLanguage::English));
        pane.refresh(7, AppLanguage::English, &data);
        assert_ne!(pane.model.snapshot().count, "other");
    }

    #[test]
    fn presentation_commands_mirror_model_gates() {
        let pane = Pane::new(AppLanguage::English, Rc::new(NoDispatch));
        pane.refresh(7, AppLanguage::English, &lines());
        let build = || Presentation {
            title: pane.model.bind(|model| model.title.clone(), None),
            level_label: pane.model.bind(|model| model.level_label.clone(), None),
            clear_label: pane.model.bind(|model| model.clear_label.clone(), None),
            close_label: pane.model.bind(|model| model.close_label.clone(), None),
            count: pane.model.bind(|model| model.count.clone(), None),
            choices: pane.model.bind(|model| model.choices.clone(), None),
            selected: pane.filter.bind(
                Clone::clone,
                Some(|filter: &mut String, value| *filter = value),
            ),
            table: pane.model.bind(|model| model.table.clone(), None),
            clear: pane
                .model
                .command(|model| !model.empty, |_, _: ()| {}, Binding::constant(())),
            close: pane
                .model
                .command(|_| true, |_, _: ()| {}, Binding::constant(())),
        };
        let presentation = build();
        assert!(
            presentation
                .command_clear(Binding::constant(String::new()))
                .enabled
                .get()
        );
        assert!(
            presentation
                .command_close(Binding::constant(String::new()))
                .enabled
                .get()
        );
        assert!(pane.model.update(|model| model.empty = true));
        assert!(
            !build()
                .command_clear(Binding::constant(String::new()))
                .enabled
                .get()
        );
    }

    #[test]
    #[should_panic(expected = "history headless")]
    fn executor_rejects_async_work() {
        NoDispatch.spawn(Box::pin(async {}));
    }
}
