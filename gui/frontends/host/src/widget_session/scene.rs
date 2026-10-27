//! Private domain conversion and input binding. Native adapters never import this.

use std::collections::HashSet;

use crate::history::{HistoryInput, HistoryModel, HistoryViewId};
use crate::{control::*, settings::*};
use nerust_gui_shell::session::osd::HistoryRowView;
use nerust_gui_viewmodel::settings::dto::BindingValueView;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ContentKey {
    system: SettingsSystemId,
    label: String,
    occurrence: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Key {
    Control(SettingsControlId),
    Layout(SettingsLayoutId),
    Tabs(SettingsTabsId),
    Tab(SettingsTabId),
    Page(SettingsPage),
    // Legacy empty tabs/conflict notes lack domain identity. Include their
    // content rather than letting an index identify unrelated dynamic content.
    EmptyControl(ContentKey),
    EmptyLayout(ContentKey),
    EmptyTab(ContentKey),
    Conflict(ContentKey),
    ConflictRow(ContentKey),
    HistoryFilter,
    HistoryClear,
    HistoryClose,
    HistoryPage,
    HistorySelect,
    HistoryBody,
    HistoryFooter,
    HistoryTable,
    HistoryHeaders,
    HistoryHeader(usize),
    HistoryEmpty,
    HistoryCount,
    HistoryRow(HistoryRowView, usize),
    HistoryCell(HistoryRowView, usize, usize),
}

#[derive(Default)]
pub(super) struct SceneBuilder {
    ids: IdRegistry<Key>,
    empty_keys: Vec<(SettingsTabId, ContentKey)>,
    conflict_keys: Vec<(SettingsControlId, ContentKey)>,
}

impl SceneBuilder {
    pub(super) fn settings(
        &mut self,
        screen: &SettingsScreen,
    ) -> (ControlScreen, Vec<(ControlId, SettingsControlId)>) {
        self.empty_keys.clear();
        self.conflict_keys.clear();
        for page in &screen.pages {
            collect_empty(&page.body, &mut self.empty_keys);
            collect_conflicts(&page.body, &mut self.conflict_keys);
        }
        collect_empty(&screen.footer, &mut self.empty_keys);
        collect_conflicts(&screen.footer, &mut self.conflict_keys);
        let mut bindings = Vec::new();
        let pages = screen
            .pages
            .iter()
            .map(|page| Page {
                id: self.ids.id(Key::Page(page.id)),
                label: page.label.clone(),
                selected: page.selected,
                select: self.control(&page.select, &mut bindings),
                body: self.layout(&page.body, &mut bindings),
            })
            .collect();
        let footer = self.layout(&screen.footer, &mut bindings);
        let target = screen.capture.target.as_ref().and_then(|target| {
            bindings
                .iter()
                .find(|(_, id)| *id == SettingsControlId::Binding(target.clone()))
                .map(|(id, _)| *id)
        });
        (
            ControlScreen {
                title: screen.title.clone(),
                pages,
                footer,
                message: screen.message.clone(),
                capture: Capture {
                    target,
                    prompt: screen.capture.prompt.clone(),
                },
            },
            bindings,
        )
    }

    fn control(
        &mut self,
        c: &SettingsControl,
        bindings: &mut Vec<(ControlId, SettingsControlId)>,
    ) -> Control {
        let key = match &c.id {
            SettingsControlId::Tab(id @ SettingsTabId::EmptyAttachment { .. }) => {
                Key::EmptyControl(self.empty_key(id))
            }
            id @ SettingsControlId::Conflict { .. } => Key::Conflict(self.conflict_key(id)),
            _ => Key::Control(c.id.clone()),
        };
        let id = self.ids.id(key);
        bindings.push((id, c.id.clone()));
        use SettingsControlKind as K;
        let kind = match &c.kind {
            K::Toggle { value } => ControlKind::Toggle { value: *value },
            K::Choice { choices, selected } => ControlKind::Choice {
                choices: choices.clone(),
                selected: *selected,
            },
            K::Number {
                value,
                min,
                max,
                step,
                unit,
                formatted,
            } => ControlKind::Number {
                value: *value,
                min: *min,
                max: *max,
                step: *step,
                unit: unit.clone(),
                formatted: formatted.clone(),
            },
            K::Text {
                value,
                placeholder,
                browse_label,
            } => ControlKind::Text {
                value: value.clone(),
                placeholder: placeholder.clone(),
                browse_label: browse_label.clone(),
            },
            K::Binding {
                value,
                change_label,
                clear_label,
            } => ControlKind::Binding {
                text: match value {
                    BindingValueView::Unbound(t)
                    | BindingValueView::Bound(t)
                    | BindingValueView::Capturing(t) => t.clone(),
                },
                capturing: matches!(value, BindingValueView::Capturing(_)),
                change_label: change_label.clone(),
                clear_label: clear_label.clone(),
            },
            K::Action => ControlKind::Action,
            K::Note { text } => ControlKind::Note { text: text.clone() },
        };
        Control {
            id,
            label: c.label.clone(),
            metadata: metadata(&c.metadata),
            display: match c.display {
                SettingsDisplay::Standard => ControlDisplay::Standard,
                SettingsDisplay::Primary => ControlDisplay::Primary,
                SettingsDisplay::Secondary => ControlDisplay::Secondary,
                SettingsDisplay::Note => ControlDisplay::Note,
                SettingsDisplay::Heading => ControlDisplay::Heading,
            },
            kind,
        }
    }

    fn layout(
        &mut self,
        l: &SettingsLayout,
        bindings: &mut Vec<(ControlId, SettingsControlId)>,
    ) -> Layout {
        let key = match &l.id {
            SettingsLayoutId::Tab(id @ SettingsTabId::EmptyAttachment { .. }) => {
                Key::EmptyLayout(self.empty_key(id))
            }
            SettingsLayoutId::Row(id @ SettingsControlId::Conflict { .. }) => {
                Key::ConflictRow(self.conflict_key(id))
            }
            _ => Key::Layout(l.id.clone()),
        };
        Layout {
            id: self.ids.id(key),
            label: l.label.clone(),
            metadata: metadata(&l.metadata),
            spacing: l.spacing,
            indent: l.indent,
            header: l.header.as_ref().map(|c| self.control(c, bindings)),
            children: l
                .children
                .iter()
                .map(|n| match n {
                    SettingsNode::Control(c) => Node::Control(self.control(c, bindings)),
                    SettingsNode::Row(l) => Node::Row(self.layout(l, bindings)),
                    SettingsNode::Column(l) => Node::Column(self.layout(l, bindings)),
                    SettingsNode::Group(l) => Node::Group(self.layout(l, bindings)),
                    SettingsNode::Tabs(t) => Node::Tabs(Tabs {
                        id: self.ids.id(Key::Tabs(t.id.clone())),
                        metadata: metadata(&t.metadata),
                        selected: t.selected,
                        tabs: t
                            .tabs
                            .iter()
                            .map(|tab| Tab {
                                id: self.ids.id(
                                    if matches!(tab.id, SettingsTabId::EmptyAttachment { .. }) {
                                        Key::EmptyTab(self.empty_key(&tab.id))
                                    } else {
                                        Key::Tab(tab.id.clone())
                                    },
                                ),
                                label: tab.label.clone(),
                                selected: tab.selected,
                                select: self.control(&tab.select, bindings),
                                body: self.layout(&tab.body, bindings),
                            })
                            .collect(),
                    }),
                })
                .collect(),
        }
    }

    pub(super) fn history_filter(&mut self) -> ControlId {
        self.ids.id(Key::HistoryFilter)
    }
    pub(super) fn history_clear(&mut self) -> ControlId {
        self.ids.id(Key::HistoryClear)
    }
    pub(super) fn history_close(&mut self) -> ControlId {
        self.ids.id(Key::HistoryClose)
    }

    /// Map a native control ID back to the domain history action.
    pub(super) fn history_action(&mut self, id: ControlId) -> Option<HistoryViewId> {
        if id == self.history_filter() {
            Some(HistoryViewId::Filter)
        } else if id == self.history_clear() {
            Some(HistoryViewId::Clear)
        } else if id == self.history_close() {
            Some(HistoryViewId::Close)
        } else {
            None
        }
    }

    pub(super) fn history(&mut self, model: &HistoryModel) -> ControlScreen {
        let c = &model.captions;
        let filter = self.simple(
            Key::HistoryFilter,
            c.level.clone(),
            ControlKind::Choice {
                choices: model.options.iter().map(|o| o.label.clone()).collect(),
                selected: model.options.iter().position(|o| o.filter == model.filter),
            },
        );
        let clear = self.simple(Key::HistoryClear, c.clear.clone(), ControlKind::Action);
        let close = self.simple(Key::HistoryClose, c.close.clone(), ControlKind::Action);
        let count = self.simple(
            Key::HistoryCount,
            String::new(),
            ControlKind::Note {
                text: c.count.clone(),
            },
        );
        let headers = c
            .headers
            .iter()
            .enumerate()
            .map(|(column, text)| {
                Node::Control(self.simple(
                    Key::HistoryHeader(column),
                    String::new(),
                    ControlKind::Note { text: text.clone() },
                ))
            })
            .collect();
        let headers = self.simple_layout(Key::HistoryHeaders, headers);
        let mut rows = vec![Node::Row(headers)];
        let mut live_history_ids = HashSet::new();
        for (index, row) in model.rows.iter().enumerate() {
            // Equal duplicate rows get disjoint paths. This is an occurrence
            // within equal semantic keys, not a table-position identity.
            let occurrence = model.rows[..index].iter().filter(|r| *r == row).count();
            let cells = [
                &row.datetime,
                &row.level.to_string(),
                &row.module,
                &row.message,
            ]
            .into_iter()
            .enumerate()
            .map(|(column, text)| {
                let cell = self.simple(
                    Key::HistoryCell(row.clone(), occurrence, column),
                    String::new(),
                    ControlKind::Note { text: text.clone() },
                );
                live_history_ids.insert(cell.id);
                Node::Control(cell)
            })
            .collect();
            let row = self.simple_layout(Key::HistoryRow(row.clone(), occurrence), cells);
            live_history_ids.insert(row.id);
            rows.push(Node::Row(row));
        }
        // Only history content expires. Settings keys retain their identity even
        // across hidden topology. Returning history keys may receive fresh IDs;
        // previously issued IDs can never identify different content.
        self.ids.retain(|key, id| match key {
            Key::HistoryRow(..) | Key::HistoryCell(..) => live_history_ids.contains(&id),
            _ => true,
        });
        if model.rows.is_empty() {
            rows.push(Node::Control(self.simple(
                Key::HistoryEmpty,
                String::new(),
                ControlKind::Note {
                    text: c.empty.clone(),
                },
            )));
        }
        let table = self.simple_layout(Key::HistoryTable, rows);
        let select = self.simple(
            Key::HistorySelect,
            c.title.clone(),
            ControlKind::Note {
                text: c.title.clone(),
            },
        );
        let body = self.simple_layout(
            Key::HistoryBody,
            vec![
                Node::Control(filter),
                Node::Control(clear),
                Node::Control(count),
                Node::Table(table),
            ],
        );
        ControlScreen {
            title: c.title.clone(),
            pages: vec![Page {
                id: self.ids.id(Key::HistoryPage),
                label: c.title.clone(),
                selected: true,
                select,
                body,
            }],
            footer: self.simple_layout(Key::HistoryFooter, vec![Node::Control(close)]),
            message: None,
            capture: Capture {
                target: None,
                prompt: String::new(),
            },
        }
    }

    fn simple(&mut self, key: Key, label: String, kind: ControlKind) -> Control {
        let accessible = match &kind {
            ControlKind::Note { text } if label.is_empty() => text.clone(),
            _ => label.clone(),
        };
        Control {
            id: self.ids.id(key),
            label,
            metadata: ControlMetadata::new(accessible),
            display: ControlDisplay::Standard,
            kind,
        }
    }

    fn simple_layout(&mut self, key: Key, children: Vec<Node>) -> Layout {
        Layout {
            id: self.ids.id(key),
            label: String::new(),
            metadata: ControlMetadata::new(""),
            spacing: 12,
            indent: 0,
            header: None,
            children,
        }
    }

    fn empty_key(&self, id: &SettingsTabId) -> ContentKey {
        self.empty_keys
            .iter()
            .find(|(domain, _)| domain == id)
            .expect("empty tab collected before conversion")
            .1
            .clone()
    }

    fn conflict_key(&self, id: &SettingsControlId) -> ContentKey {
        self.conflict_keys
            .iter()
            .find(|(domain, _)| domain == id)
            .expect("conflict collected before conversion")
            .1
            .clone()
    }
}

fn note_text(control: &SettingsControl) -> String {
    match &control.kind {
        SettingsControlKind::Note { text } => text.clone(),
        _ => control.label.clone(),
    }
}

fn collect_empty(layout: &SettingsLayout, keys: &mut Vec<(SettingsTabId, ContentKey)>) {
    for node in &layout.children {
        match node {
            SettingsNode::Tabs(tabs) => {
                for tab in &tabs.tabs {
                    if let SettingsTabId::EmptyAttachment { system, .. } = &tab.id {
                        let occurrence = keys
                            .iter()
                            .filter(|(_, key)| key.system == *system && key.label == tab.label)
                            .count();
                        keys.push((
                            tab.id.clone(),
                            ContentKey {
                                system: system.clone(),
                                label: tab.label.clone(),
                                occurrence,
                            },
                        ));
                    }
                    collect_empty(&tab.body, keys);
                }
            }
            SettingsNode::Row(l) | SettingsNode::Column(l) | SettingsNode::Group(l) => {
                collect_empty(l, keys)
            }
            SettingsNode::Control(_) => (),
        }
    }
}

fn collect_conflicts(layout: &SettingsLayout, keys: &mut Vec<(SettingsControlId, ContentKey)>) {
    fn collect(control: &SettingsControl, keys: &mut Vec<(SettingsControlId, ContentKey)>) {
        if let SettingsControlId::Conflict { system, .. } = &control.id {
            let label = note_text(control);
            let occurrence = keys
                .iter()
                .filter(|(_, key)| key.system == *system && key.label == label)
                .count();
            keys.push((
                control.id.clone(),
                ContentKey {
                    system: system.clone(),
                    label,
                    occurrence,
                },
            ));
        }
    }
    if let Some(control) = &layout.header {
        collect(control, keys);
    }
    for node in &layout.children {
        match node {
            SettingsNode::Control(c) => collect(c, keys),
            SettingsNode::Tabs(tabs) => {
                for tab in &tabs.tabs {
                    collect_conflicts(&tab.body, keys);
                }
            }
            SettingsNode::Row(l) | SettingsNode::Column(l) | SettingsNode::Group(l) => {
                collect_conflicts(l, keys)
            }
        }
    }
}

fn metadata(m: &SettingsMetadata) -> ControlMetadata {
    ControlMetadata {
        enabled: m.enabled,
        visible: m.visible,
        accessible_label: m.accessible_label.clone(),
        help: m.help.clone(),
    }
}

pub(super) fn settings_input(
    input: crate::control::ControlInput,
) -> Option<crate::settings::ControlInput> {
    use crate::{control::ControlInput as I, settings::ControlInput as O};
    Some(match input {
        I::Bool(v) => O::Bool(v),
        I::Select(v) => O::Select(v),
        I::Number(v) => O::Number(v),
        I::Text(v) => O::Text(v),
        I::Activate => O::Activate,
        I::CaptureClear => O::CaptureClear,
        I::CaptureKey(v) => O::CaptureKey(v),
        I::DirectorySelected { .. } => return None,
    })
}

pub(super) fn history_input(input: crate::control::ControlInput) -> Option<HistoryInput> {
    use crate::control::ControlInput as I;
    Some(match input {
        I::Select(index) => HistoryInput::Select(index),
        I::Activate => HistoryInput::Activate,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_gui_shell::session::{
        notification::NotificationLevel,
        osd::{HistoryFilter, HistoryLine},
    };
    use nerust_settings_core::model::language::AppLanguage;

    fn entries(scene: &mut SceneBuilder) -> Vec<(Key, Id)> {
        let mut entries = Vec::new();
        scene.ids.retain(|key, id| {
            entries.push((key.clone(), id));
            true
        });
        entries
    }

    fn history_entries(scene: &mut SceneBuilder) -> Vec<(Key, Id)> {
        entries(scene)
            .into_iter()
            .filter(|(key, _)| matches!(key, Key::HistoryRow(..) | Key::HistoryCell(..)))
            .collect()
    }

    #[test]
    fn history_retention_is_bounded_and_clear_releases_all_row_and_cell_keys() {
        let mut scene = SceneBuilder::default();
        let mut model = HistoryModel::build(&[], AppLanguage::English, HistoryFilter::All);
        scene.history(&model);
        let chrome = entries(&mut scene);
        let mut issued = HashSet::new();
        for index in 0..1000 {
            model = HistoryModel::build(
                &[HistoryLine {
                    datetime: "2026-10-27 12:00:00".into(),
                    level: NotificationLevel::Info,
                    module: "test::source",
                    text: format!("row-{index}"),
                    count: 1,
                }],
                AppLanguage::English,
                HistoryFilter::All,
            );
            scene.history(&model);
            let retained = history_entries(&mut scene);
            assert_eq!(retained.len(), 5);
            assert_eq!(entries(&mut scene).len(), chrome.len() + 5);
            for (_, id) in retained {
                assert!(issued.insert(id), "expired IDs must never be recycled");
            }
        }
        let returning = model.clone();
        model.rows.clear();
        scene.history(&model);
        assert!(history_entries(&mut scene).is_empty());
        assert_eq!(entries(&mut scene), chrome);
        scene.history(&returning);
        for (_, id) in history_entries(&mut scene) {
            assert!(issued.insert(id), "returning keys must receive fresh IDs");
        }
    }

    #[test]
    fn surviving_duplicate_rows_and_cells_keep_ids_across_reorder_and_removal() {
        let mut scene = SceneBuilder::default();
        let lines = ["duplicate", "obsolete", "duplicate", "survivor"].map(|text| HistoryLine {
            datetime: "2026-10-27 12:00:00".into(),
            level: NotificationLevel::Info,
            module: "test::source",
            text: text.into(),
            count: 1,
        });
        let mut model = HistoryModel::build(&lines, AppLanguage::English, HistoryFilter::All);
        scene.history(&model);
        let old = history_entries(&mut scene);
        assert_eq!(old.len(), 20);
        assert_eq!(
            old.iter().map(|(_, id)| id).collect::<HashSet<_>>().len(),
            20
        );
        let original = model.rows.clone();
        model.rows = vec![
            original[3].clone(),
            original[0].clone(),
            original[2].clone(),
        ];
        scene.history(&model);
        let retained = history_entries(&mut scene);
        assert_eq!(retained.len(), 15);
        for (key, id) in &retained {
            assert!(old.contains(&(key.clone(), *id)));
        }
        // Removing one equal row retains occurrence zero, not table position.
        model.rows.pop();
        scene.history(&model);
        let retained = history_entries(&mut scene);
        assert_eq!(retained.len(), 10);
        for (key, id) in retained {
            assert!(old.contains(&(key.clone(), id)));
            match key {
                Key::HistoryRow(_, occurrence) | Key::HistoryCell(_, occurrence, _) => {
                    assert_eq!(occurrence, 0);
                }
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn history_pruning_does_not_remove_settings_keys() {
        let mut scene = SceneBuilder::default();
        let key = Key::Control(SettingsControlId::Volume);
        let id = scene.ids.id(key.clone());
        scene.history(&HistoryModel::build(
            &[],
            AppLanguage::English,
            HistoryFilter::All,
        ));
        assert!(entries(&mut scene).contains(&(key, id)));
    }
}
