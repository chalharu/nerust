//! Owned notification-history presentation, without native widgets or lifetimes.
//! Build under the session borrow, release it, then visit or cache the model.

use nerust_gui_shell::session::{
    SessionHandle,
    osd::{HistoryFilter, HistoryLine, HistoryRowView, history_row_view, visible_history},
};
use nerust_settings_core::{
    i18n::{UiText, history_count, text as ui_text},
    model::language::AppLanguage,
};
use wakui::scene::control::{ControlDisplay, ControlId, ControlScreen};
use wakui::scene::def::{
    Binding, BoolBinding, ChoicesDef, ContainerDef, ControlDef, LeafDef, ListColumnDef, NodeDef,
    PageDef, PolicyDef, RowTemplateDef, RowsDef, ScreenDef, SelectedDef,
};
use wakui::scene::hints::LayoutHints;
use wakui::scene::inflate::{
    Choices, CustomLeaves, Flags, InflateCtx, Lists, Texts, Value, inflate,
};

/// Localized chrome, including headers in datetime/level/module/message order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryCaptions {
    pub title: String,
    pub close: String,
    pub level: String,
    pub clear: String,
    pub count: String,
    pub empty: String,
    pub headers: [String; 4],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryFilterOption {
    pub filter: HistoryFilter,
    pub label: String,
}

/// Render-ready snapshot. Frontends may clone strings into retained controls or iced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryModel {
    pub language: AppLanguage,
    pub filter: HistoryFilter,
    pub captions: HistoryCaptions,
    pub options: Vec<HistoryFilterOption>,
    pub rows: Vec<HistoryRowView>,
}

impl HistoryModel {
    pub fn build(lines: &[HistoryLine], language: AppLanguage, filter: HistoryFilter) -> Self {
        let rows: Vec<_> = visible_history(lines, filter)
            .into_iter()
            .map(|line| history_row_view(line, language))
            .collect();
        Self {
            language,
            filter,
            captions: HistoryCaptions {
                title: ui_text(language, UiText::NotificationHistory).to_string(),
                close: ui_text(language, UiText::Close).to_string(),
                level: ui_text(language, UiText::HistoryLevel).to_string(),
                clear: ui_text(language, UiText::HistoryClear).to_string(),
                count: history_count(language, rows.len()),
                empty: ui_text(language, UiText::HistoryEmpty).to_string(),
                headers: [
                    UiText::HistoryDatetime,
                    UiText::HistoryLevel,
                    UiText::HistoryModule,
                    UiText::HistoryMessage,
                ]
                .map(|key| ui_text(language, key).to_string()),
            },
            options: HistoryFilter::all()
                .into_iter()
                .map(|filter| HistoryFilterOption {
                    filter,
                    label: filter.label(language).to_string(),
                })
                .collect(),
            rows,
        }
    }

    /// Project the model as a generic control screen through the L1
    /// definition below. Returns the screen plus the stable action IDs
    /// for filter/clear/close.
    pub fn screen(&self, ctx: &mut InflateCtx<String>) -> (ControlScreen, HistoryIds) {
        let resolver = HistoryResolver { model: self };
        let (screen, live) = inflate(
            &history_def(self.rows.is_empty()),
            ctx,
            |key| key.to_string(),
            &resolver,
        );
        ctx.retain_rows(&live);
        let ids = &mut ctx.ids;
        let mut id = |key: &str| ids.id(key.to_string());
        (
            screen,
            HistoryIds {
                filter: id("filter"),
                clear: id("clear"),
                close: id("close"),
            },
        )
    }
}

/// Static L1 definition for the history screen. Language-independent: every
/// string is a [`Binding`] resolved per frame by [`HistoryResolver`].
/// `empty` toggles the empty-state note; L1 itself holds no conditionals.
fn history_def(empty: bool) -> ScreenDef {
    let chrome = || ControlDef {
        key: String::new(),
        label: Binding::Literal(String::new()),
        accessible: None,
        help: None,
        display: ControlDisplay::Standard,
        enabled: BoolBinding::Literal(true),
        visible: BoolBinding::Literal(true),
        hints: LayoutHints::default(),
        leaf: LeafDef::Action,
    };
    let mut body_children = vec![
        NodeDef::Control(Box::new(ControlDef {
            key: "filter".into(),
            label: Binding::Localized("level".into()),
            accessible: None,
            help: None,
            display: ControlDisplay::Standard,
            enabled: BoolBinding::Literal(true),
            visible: BoolBinding::Literal(true),
            hints: LayoutHints::default(),
            leaf: LeafDef::Choice {
                choices: ChoicesDef::Bound("filter".into()),
                selected: SelectedDef::Bound("filter".into()),
            },
        })),
        NodeDef::Control(Box::new(ControlDef {
            key: "clear".into(),
            label: Binding::Localized("clear".into()),
            ..chrome()
        })),
        NodeDef::Control(Box::new(ControlDef {
            key: "count".into(),
            label: Binding::Localized("count".into()),
            accessible: None,
            help: None,
            display: ControlDisplay::Standard,
            enabled: BoolBinding::Literal(true),
            visible: BoolBinding::Literal(true),
            hints: LayoutHints::default(),
            leaf: LeafDef::Note {
                text: Binding::Localized("count".into()),
            },
        })),
        NodeDef::Control(Box::new(ControlDef {
            key: "table".into(),
            label: Binding::Localized("title".into()),
            accessible: None,
            help: None,
            display: ControlDisplay::Standard,
            enabled: BoolBinding::Literal(true),
            visible: BoolBinding::Literal(true),
            hints: LayoutHints::default(),
            leaf: LeafDef::List {
                columns: ["h0", "h1", "h2", "h3"]
                    .into_iter()
                    .enumerate()
                    .map(|(column, key)| ListColumnDef {
                        id: column.to_string(),
                        header: Binding::Localized(key.into()),
                        width: None,
                    })
                    .collect(),
                rows: RowsDef::Bound {
                    source: "history".into(),
                    template: RowTemplateDef {
                        id_field: "row".into(),
                        cells: ["datetime", "level", "module", "message"]
                            .into_iter()
                            .enumerate()
                            .map(|(column, path)| ControlDef {
                                key: format!("cell{column}"),
                                label: Binding::Field(path.into()),
                                accessible: None,
                                help: None,
                                display: ControlDisplay::Standard,
                                enabled: BoolBinding::Literal(true),
                                visible: BoolBinding::Literal(true),
                                hints: LayoutHints::default(),
                                leaf: LeafDef::Note {
                                    text: Binding::Field(path.into()),
                                },
                            })
                            .collect(),
                    },
                },
            },
        })),
    ];
    if empty {
        body_children.push(NodeDef::Control(Box::new(ControlDef {
            key: "empty".into(),
            label: Binding::Localized("empty".into()),
            accessible: None,
            help: None,
            display: ControlDisplay::Standard,
            enabled: BoolBinding::Literal(true),
            visible: BoolBinding::Literal(true),
            hints: LayoutHints::default(),
            leaf: LeafDef::Note {
                text: Binding::Localized("empty".into()),
            },
        })));
    }
    let chrome_hints = LayoutHints {
        spacing: Some(12.0),
        indent: Some(0.0),
        ..LayoutHints::default()
    };
    ScreenDef {
        title: Binding::Localized("title".into()),
        default_size: None,
        pages: vec![PageDef {
            key: "page".into(),
            label: Binding::Localized("title".into()),
            selected: true,
            body: ContainerDef {
                key: "body".into(),
                label: Binding::Literal(String::new()),
                hints: chrome_hints,
                header: None,
                policy: PolicyDef::Column,
                children: body_children,
            },
        }],
        footer: ContainerDef {
            key: "footer".into(),
            label: Binding::Literal(String::new()),
            hints: chrome_hints,
            header: None,
            policy: PolicyDef::Column,
            children: vec![NodeDef::Control(Box::new(ControlDef {
                key: "close".into(),
                label: Binding::Localized("close".into()),
                accessible: None,
                help: None,
                display: ControlDisplay::Primary,
                enabled: BoolBinding::Literal(true),
                visible: BoolBinding::Literal(true),
                hints: LayoutHints::default(),
                leaf: LeafDef::Action,
            }))],
        },
    }
}

/// Outer-owned resolution for [`history_def`]: captions, filter options,
/// and row fields. The engine never names these domain types.
struct HistoryResolver<'m> {
    model: &'m HistoryModel,
}

impl Texts for HistoryResolver<'_> {
    fn text(&self, key: &str) -> String {
        let captions = &self.model.captions;
        match key {
            "title" => captions.title.clone(),
            "close" => captions.close.clone(),
            "level" => captions.level.clone(),
            "clear" => captions.clear.clone(),
            "count" => captions.count.clone(),
            "empty" => captions.empty.clone(),
            "h0" => captions.headers[0].clone(),
            "h1" => captions.headers[1].clone(),
            "h2" => captions.headers[2].clone(),
            "h3" => captions.headers[3].clone(),
            _ => String::new(),
        }
    }
}

impl Flags for HistoryResolver<'_> {}

impl Lists for HistoryResolver<'_> {
    fn list_len(&self, source: &str) -> usize {
        if source == "history" {
            self.model.rows.len()
        } else {
            0
        }
    }

    fn value(&self, source: &str, index: usize, path: &str) -> Value {
        if source != "history" {
            return Value::Text(String::new());
        }
        let Some(view) = self.model.rows.get(index) else {
            return Value::Text(String::new());
        };
        match path {
            "datetime" => Value::Text(view.datetime.clone()),
            "level" => Value::Text(view.level.to_string()),
            "module" => Value::Text(view.module.clone()),
            "message" => Value::Text(view.message.clone()),
            // Full-row identity: equal duplicate rows share the key and
            // the engine disambiguates them by occurrence.
            "row" => Value::Text(
                [
                    view.datetime.as_str(),
                    view.level,
                    view.module.as_str(),
                    view.message.as_str(),
                ]
                .join(""),
            ),
            _ => Value::Text(String::new()),
        }
    }
}

impl Choices for HistoryResolver<'_> {
    fn choices(&self, source: &str) -> Vec<String> {
        if source == "filter" {
            self.model.options.iter().map(|o| o.label.clone()).collect()
        } else {
            Vec::new()
        }
    }

    fn selected(&self, source: &str) -> Option<usize> {
        if source == "filter" {
            self.model
                .options
                .iter()
                .position(|o| o.filter == self.model.filter)
        } else {
            None
        }
    }
}

impl CustomLeaves for HistoryResolver<'_> {}

/// Stable action IDs minted for the history chrome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryIds {
    pub filter: ControlId,
    pub clear: ControlId,
    pub close: ControlId,
}

/// Domain-side identity for bindable history controls.
/// Native code only sees opaque framework IDs; this is the view-defs side key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryViewId {
    Filter,
    Clear,
    Close,
}

/// Framework-agnostic input for history controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryInput {
    Select(usize),
    Activate,
}

/// Domain events produced by history binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryEvent {
    SetFilter(HistoryFilter),
    Clear,
    Close,
}

/// History produces no native effects. Uninhabited and future-proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryEffect {}

impl wakui::app::binding::ViewDefinition for HistoryModel {
    type ViewId = HistoryViewId;
    type ViewInput = HistoryInput;
    type DomainEvent = HistoryEvent;

    /// Partial: history controls accept only Select/Activate.
    fn convert(input: wakui::scene::control::ControlInput) -> Option<HistoryInput> {
        use wakui::scene::control::ControlInput as I;
        Some(match input {
            I::Select(index) => HistoryInput::Select(index),
            I::Activate => HistoryInput::Activate,
            _ => return None,
        })
    }

    fn bind(&self, id: &HistoryViewId, input: HistoryInput) -> Option<Vec<HistoryEvent>> {
        match (id, input) {
            (HistoryViewId::Filter, HistoryInput::Select(index)) => {
                Some(vec![HistoryEvent::SetFilter(
                    self.options.get(index)?.filter,
                )])
            }
            (HistoryViewId::Clear, HistoryInput::Activate) => Some(vec![HistoryEvent::Clear]),
            (HistoryViewId::Close, HistoryInput::Activate) => Some(vec![HistoryEvent::Close]),
            _ => None,
        }
    }
}

/// Input-identity gate only: no retained model, native state or presentation borrow.
/// Invalidate when reopening or switching sessions, or if an update was not delivered.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HistoryController {
    key: Option<(u64, AppLanguage, HistoryFilter)>,
}

impl HistoryController {
    pub fn new() -> Self {
        Self::default()
    }

    /// Unchanged inputs return before retrieving or formatting any history lines.
    pub fn update(
        &mut self,
        session: &mut SessionHandle,
        filter: HistoryFilter,
    ) -> Option<HistoryModel> {
        let language = session.settings_snapshot().shared.general.language;
        let key = (session.notification_history_seq(), language, filter);
        if self.key == Some(key) {
            return None;
        }
        let model = HistoryModel::build(
            session.notification_history_lines_cached(language),
            language,
            filter,
        );
        self.key = Some(key);
        Some(model)
    }

    pub fn invalidate(&mut self) {
        self.key = None;
    }
}

#[cfg(test)]
mod tests;
