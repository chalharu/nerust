use nerust_gui_shell::session::osd::{
    HistoryFilter, HistoryLine, HistoryRowView, history_row_view, visible_history,
};
use nerust_settings_core::{
    i18n::{UiText, history_count, text as ui_text},
    model::language::AppLanguage,
};
use panechord::{Binding, Choice, Command, Executor, Keyed, Store, View, Viewport};
use std::{cell::RefCell, rc::Rc};

#[derive(Clone)]
struct Row {
    datetime: String,
    level: String,
    module: String,
    message: String,
    level_style: panechord::Style,
}

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
    rows: Vec<Keyed<Row>>,
    viewport: Viewport,
}

pub(crate) struct Pane {
    model: Store<Model>,
    filter: Store<String>,
    revision: RefCell<Option<(u64, AppLanguage, String)>>,
    cache: RefCell<nerust_gui_shell::session::session_query::HistoryView>,
    executor: Rc<dyn Executor>,
}

fn project(
    lines: &[HistoryLine],
    language: AppLanguage,
    filter: HistoryFilter,
) -> Vec<HistoryRowView> {
    visible_history(lines, filter)
        .into_iter()
        .map(|line| history_row_view(line, language))
        .collect()
}

impl Pane {
    pub(crate) fn new(language: AppLanguage, executor: Rc<dyn Executor>) -> Rc<Self> {
        let label = |key| ui_text(language, key).to_owned();
        Rc::new(Self {
            model: Store::new(
                Model {
                    title: label(UiText::NotificationHistory),
                    level_label: label(UiText::HistoryLevel),
                    clear_label: label(UiText::HistoryClear),
                    close_label: label(UiText::Close),
                    headers: [
                        UiText::HistoryDatetime,
                        UiText::HistoryLevel,
                        UiText::HistoryModule,
                        UiText::HistoryMessage,
                    ]
                    .map(label),
                    count: history_count(language, 0),
                    empty_label: label(UiText::HistoryEmpty),
                    empty: true,
                    choices: HistoryFilter::all()
                        .into_iter()
                        .map(|filter| Choice {
                            id: filter.id().into(),
                            label: filter.label(language).into(),
                        })
                        .collect(),
                    rows: Vec::new(),
                    viewport: Viewport {
                        count: 11,
                        row_height: 24.0,
                        ..Viewport::default()
                    },
                },
                executor.clone(),
            ),
            filter: Store::new(HistoryFilter::All.id().into(), executor.clone()),
            revision: RefCell::new(None),
            cache: RefCell::new(nerust_gui_shell::session::session_query::HistoryView::new()),
            executor,
        })
    }

    pub(crate) fn needs_refresh(&self, revision: u64, language: AppLanguage) -> bool {
        !self.model.is_closed()
            && self.revision.borrow().as_ref()
                != Some(&(revision, language, self.filter.snapshot()))
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
        let rows = project(lines, language, HistoryFilter::from_id(Some(&selection)));
        let count = history_count(language, rows.len());
        let rows: Vec<_> = rows
            .into_iter()
            .enumerate()
            .map(|(index, row)| {
                Keyed::new(
                    index.to_string(),
                    Store::new(
                        Row {
                            datetime: row.datetime,
                            level: row.level.into(),
                            module: row.module,
                            message: row.message,
                            level_style: panechord::Style {
                                placement: panechord::Placement {
                                    width: Some(70.0),
                                    ..Default::default()
                                },
                                text: Some(panechord::TextStyle {
                                    foreground: Some([row.rgb.0, row.rgb.1, row.rgb.2]),
                                    ..Default::default()
                                }),
                                ..Default::default()
                            },
                        },
                        self.executor.clone(),
                    ),
                )
            })
            .collect();
        let _ = self.model.update(move |model| {
            model.empty = rows.is_empty();
            model.rows = rows;
            model.count = count;
            let label = |key| ui_text(language, key).to_owned();
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
            model.choices = HistoryFilter::all()
                .into_iter()
                .map(|filter| Choice {
                    id: filter.id().into(),
                    label: filter.label(language).into(),
                })
                .collect();
        });
        self.revision.replace(Some((revision, language, selection)));
    }

    pub(crate) fn build(self: &Rc<Self>, clear: Rc<dyn Fn()>) -> Result<View, String> {
        let close_view = Rc::new(RefCell::new(None::<panechord::WeakView>));
        let close = close_view.clone();
        let close = self.model.command(
            |_| true,
            move |_, ()| {
                if let Some(view) = close
                    .borrow()
                    .as_ref()
                    .and_then(panechord::WeakView::upgrade)
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
            datetime_label: self.model.bind(|model| model.headers[0].clone(), None),
            severity_label: self.model.bind(|model| model.headers[1].clone(), None),
            module_label: self.model.bind(|model| model.headers[2].clone(), None),
            message_label: self.model.bind(|model| model.headers[3].clone(), None),
            count: self.model.bind(|model| model.count.clone(), None),
            empty_label: self.model.bind(|model| model.empty_label.clone(), None),
            empty: self.model.bind(|model| model.empty, None),
            populated: self.model.bind(|model| !model.empty, None),
            choices: self.model.bind(|model| model.choices.clone(), None),
            selected: self.filter.bind(
                Clone::clone,
                Some(|filter: &mut String, value| *filter = value),
            ),
            rows: self.model.bind(|model| model.rows.clone(), None),
            viewport: self.model.bind(|model| model.viewport, None),
            clear,
            close,
        };
        let view = panechord::try_view!("ui/history.yaml", bindings, panechord)?;
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
    datetime_label: Binding<String>,
    severity_label: Binding<String>,
    module_label: Binding<String>,
    message_label: Binding<String>,
    count: Binding<String>,
    empty_label: Binding<String>,
    empty: Binding<bool>,
    populated: Binding<bool>,
    choices: Binding<Vec<Choice>>,
    selected: Binding<String>,
    rows: Binding<Vec<Keyed<Row>>>,
    viewport: Binding<Viewport>,
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
        let rows = project(&lines, AppLanguage::English, HistoryFilter::Warning);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].message, "event \u{00d7}2");
        assert_eq!(rows[0].level, "Warning");
        assert_eq!(
            project(&lines, AppLanguage::English, HistoryFilter::Error).len(),
            1
        );
    }
}
