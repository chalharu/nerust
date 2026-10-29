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
use wakui::scene::control::{
    Capture, ColumnPolicy, Container, Control, ControlDisplay, ControlId, ControlMetadata,
    ControlScreen, IdRegistry, Node, Page,
};
use wakui::scene::leaf::{ActionLeaf, ChoiceLeaf, ColumnDef, ItemRow, ListLeaf, NoteLeaf};

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

    /// Project the model as a generic control screen. Rows become a single
    /// [`ListLeaf`]; flavors render it virtualized or degrade to flat notes.
    /// Returns the screen plus the stable action IDs for filter/clear/close.
    pub fn screen(&self, ids: &mut IdRegistry<HistoryKey>) -> (ControlScreen, HistoryIds) {
        let c = &self.captions;
        let filter = simple(
            ids,
            HistoryKey::Filter,
            c.level.clone(),
            ChoiceLeaf {
                choices: self.options.iter().map(|o| o.label.clone()).collect(),
                selected: self.options.iter().position(|o| o.filter == self.filter),
            },
        );
        let clear = simple(ids, HistoryKey::Clear, c.clear.clone(), ActionLeaf);
        let close = history_close(ids, c.close.clone());
        let count = simple_note(ids, HistoryKey::Count, String::new(), c.count.clone());
        let columns: Vec<ColumnDef> = c
            .headers
            .iter()
            .enumerate()
            .map(|(column, text)| ColumnDef {
                id: column.to_string(),
                header: text.clone(),
                width: None,
            })
            .collect();
        let mut live = std::collections::HashSet::new();
        let mut rows = Vec::new();
        for (index, view) in self.rows.iter().enumerate() {
            // Equal duplicate rows get disjoint paths. This is an occurrence
            // within equal semantic keys, not a table-position identity.
            let occurrence = self.rows[..index].iter().filter(|r| *r == view).count();
            let cells = [
                &view.datetime,
                &view.level.to_string(),
                &view.module,
                &view.message,
            ]
            .into_iter()
            .enumerate()
            .map(|(column, text)| {
                let id = ids.id(HistoryKey::Cell(view.clone(), occurrence, column));
                live.insert(id);
                Control {
                    id,
                    label: String::new(),
                    metadata: ControlMetadata::new(text),
                    display: ControlDisplay::Standard,
                    leaf: Box::new(NoteLeaf { text: text.clone() }),
                }
            })
            .collect();
            let id = ids.id(HistoryKey::Row(view.clone(), occurrence));
            live.insert(id);
            rows.push(ItemRow {
                id,
                depth: 0,
                expanded: None,
                cells,
            });
        }
        // Only history content expires. Chrome keys retain their identity.
        // Returning keys receive fresh IDs; previously issued IDs can never
        // identify different content.
        ids.retain(|key, id| match key {
            HistoryKey::Row(..) | HistoryKey::Cell(..) => live.contains(&id),
            _ => true,
        });
        let table = Control {
            id: ids.id(HistoryKey::Table),
            label: c.title.clone(),
            metadata: ControlMetadata::new(c.title.clone()),
            display: ControlDisplay::Standard,
            leaf: Box::new(ListLeaf { columns, rows }),
        };
        let empty = if self.rows.is_empty() {
            Some(Node::Control(simple_note(
                ids,
                HistoryKey::Empty,
                String::new(),
                c.empty.clone(),
            )))
        } else {
            None
        };
        let mut children = vec![
            Node::Control(filter.clone()),
            Node::Control(clear.clone()),
            Node::Control(count),
            Node::Control(table),
        ];
        children.extend(empty);
        let body = container(ids, HistoryKey::Body, children);
        let screen = ControlScreen {
            title: c.title.clone(),
            pages: vec![Page {
                id: ids.id(HistoryKey::Page),
                label: c.title.clone(),
                selected: true,
                select: simple_note(ids, HistoryKey::Select, c.title.clone(), c.title.clone()),
                body,
            }],
            footer: container(ids, HistoryKey::Footer, vec![Node::Control(close.clone())]),
            message: None,
            capture: Capture {
                target: None,
                prompt: String::new(),
            },
        };
        (
            screen,
            HistoryIds {
                filter: filter.id,
                clear: clear.id,
                close: close.id,
            },
        )
    }
}

/// Stable action IDs minted for the history chrome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryIds {
    pub filter: ControlId,
    pub clear: ControlId,
    pub close: ControlId,
}

/// Stable registry keys for history controls and rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryKey {
    Filter,
    Clear,
    Close,
    Page,
    Select,
    Body,
    Footer,
    Table,
    Count,
    Empty,
    Row(HistoryRowView, usize),
    Cell(HistoryRowView, usize, usize),
}

fn container(ids: &mut IdRegistry<HistoryKey>, key: HistoryKey, children: Vec<Node>) -> Container {
    let mut metadata = ControlMetadata::new("");
    metadata.hints.spacing = Some(12.0);
    metadata.hints.indent = Some(0.0);
    Container {
        id: ids.id(key),
        label: String::new(),
        metadata,
        header: None,
        policy: Box::new(ColumnPolicy),
        children,
    }
}

fn simple<L: wakui::scene::leaf::ControlLeaf>(
    ids: &mut IdRegistry<HistoryKey>,
    key: HistoryKey,
    label: String,
    leaf: L,
) -> Control {
    Control {
        id: ids.id(key),
        label: label.clone(),
        metadata: ControlMetadata::new(label),
        display: ControlDisplay::Standard,
        leaf: Box::new(leaf),
    }
}

fn simple_note(
    ids: &mut IdRegistry<HistoryKey>,
    key: HistoryKey,
    label: String,
    text: String,
) -> Control {
    let accessible = if label.is_empty() {
        text.clone()
    } else {
        label.clone()
    };
    Control {
        id: ids.id(key),
        label,
        metadata: ControlMetadata::new(accessible),
        display: ControlDisplay::Standard,
        leaf: Box::new(NoteLeaf { text }),
    }
}

fn history_close(ids: &mut IdRegistry<HistoryKey>, label: String) -> Control {
    Control {
        id: ids.id(HistoryKey::Close),
        label: label.clone(),
        metadata: ControlMetadata::new(label),
        display: ControlDisplay::Primary,
        leaf: Box::new(ActionLeaf),
    }
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
