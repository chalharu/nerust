//! L1 expansion engine: [`crate::scene::def::ScreenDef`] plus an outer
//! [`Resolver`] becomes an L2 [`crate::scene::control::ControlScreen`].
//!
//! Role ports mirror `app/roles.rs`: narrow traits ([`Texts`], [`Flags`],
//! [`Lists`], [`Choices`], [`CustomLeaves`]) with an empty [`Resolver`]
//! facade. Row projection depends only on [`Lists`]; every method has a
//! default so future additions never break implementors (OCP).
//! All methods degrade silently (empty values, note fallback) so a binding
//! failure can never panic a backend.

use std::collections::HashMap;

use crate::scene::control::{
    AbsolutePolicy, Capture, ColumnPolicy, Container, Control, ControlDisplay, ControlId,
    ControlMetadata, ControlScreen, FlexPolicy, GroupPolicy, IdRegistry, LayoutPolicy, Node, Page,
    RowPolicy, Tab, TablePolicy, Tabs,
};
use crate::scene::def::{
    Binding, BoolBinding, ChoicesDef, ContainerDef, ControlDef, LeafDef, NodeDef, NumBinding,
    PageDef, PolicyDef, RowsDef, ScreenDef, SelectedDef, TabDef, TabsDef,
};
use crate::scene::leaf::{
    ActionLeaf, ChoiceLeaf, ControlLeaf, ItemRow, ListLeaf, NoteLeaf, NumberLeaf, TextLeaf,
    ToggleLeaf,
};

/// Framework-primitive resolved value. Domain types convert into this;
/// the engine never names them.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Text(String),
    Bool(bool),
    Num(u32),
}

/// Localized text table.
pub trait Texts {
    fn text(&self, key: &str) -> String {
        let _ = key;
        String::new()
    }
}

/// Screen-level boolean state (enablement, visibility, toggles).
pub trait Flags {
    fn flag(&self, key: &str) -> bool {
        let _ = key;
        false
    }
}

/// Row sources plus per-item field reads. Row projection depends only on this.
pub trait Lists {
    fn list_len(&self, source: &str) -> usize {
        let _ = source;
        0
    }
    fn value(&self, source: &str, index: usize, path: &str) -> Value {
        let _ = (source, index, path);
        Value::Text(String::new())
    }
}

/// Choice content and selection.
pub trait Choices {
    fn choices(&self, source: &str) -> Vec<String> {
        let _ = source;
        Vec::new()
    }
    fn selected(&self, source: &str) -> Option<usize> {
        let _ = source;
        None
    }
}

/// External leaf entry point. `None` degrades to a note carrying `kind`,
/// preserving order and visibility.
pub trait CustomLeaves {
    fn build(&self, kind: &str, fields: &[(String, Value)]) -> Option<Box<dyn ControlLeaf>> {
        let _ = (kind, fields);
        None
    }
}

/// Facade: the empty combination only. `inflate` takes this; subunits take
/// the narrow ports above.
pub trait Resolver: Texts + Flags + Lists + Choices + CustomLeaves {}

impl<T: Texts + Flags + Lists + Choices + CustomLeaves> Resolver for T {}

/// Frame-persistent expansion state. Owns the [`IdRegistry`] so surviving
/// keys keep stable IDs, and remembers row-managed keys so vanished rows
/// stop consuming identity without touching chrome keys.
#[derive(Debug)]
pub struct InflateCtx<K: Eq> {
    /// Registry shared with hand-built screens during migration.
    pub ids: IdRegistry<K>,
    managed: Vec<K>,
}

impl<K: Eq> Default for InflateCtx<K> {
    fn default() -> Self {
        Self {
            ids: IdRegistry::default(),
            managed: Vec::new(),
        }
    }
}

impl<K: Eq> InflateCtx<K> {
    /// Drop row-managed keys that the latest expansion did not regenerate.
    /// Chrome keys are never touched. Call with the live set returned by
    /// [`inflate`]; returning keys receive fresh IDs on reappearance.
    pub fn retain_rows(&mut self, live: &[K])
    where
        K: Clone,
    {
        let old = std::mem::take(&mut self.managed);
        self.ids
            .retain(|key, _| !old.contains(key) || live.contains(key));
        self.managed = live.to_vec();
    }
}

/// Expand `def` into a snapshot. Returns the screen plus the live row-managed
/// key set for [`InflateCtx::retain_rows`].
pub fn inflate<K: Eq + Clone>(
    def: &ScreenDef,
    ctx: &mut InflateCtx<K>,
    key: impl Fn(&str) -> K,
    resolver: &dyn Resolver,
) -> (ControlScreen, Vec<K>) {
    let key = &key;
    let mut live = Vec::new();
    let title = text_of(&def.title, resolver, None);
    let pages = def
        .pages
        .iter()
        .map(|page| build_page(page, &mut ctx.ids, key, resolver, &mut live))
        .collect();
    let footer = build_container(&def.footer, &mut ctx.ids, key, resolver, &mut live, None);
    (
        ControlScreen {
            title,
            pages,
            footer,
            message: None,
            capture: Capture {
                target: None,
                prompt: String::new(),
            },
        },
        live,
    )
}

/// Row context: which item of which source a template cell reads.
#[derive(Clone, Copy)]
struct Row<'a> {
    lists: &'a dyn Lists,
    source: &'a str,
    index: usize,
}

/// Key-string constructors shared with outer crates. Event tables rebuild
/// row control IDs through these instead of duplicating the namespace
/// format; row projection builds keys the same way.
pub fn row_key(source: &str, identity: &str, occurrence: usize) -> String {
    format!("{source}/{identity}#{occurrence}")
}

/// Cell key nested under its row namespace.
pub fn cell_key(row: &str, cell: &str) -> String {
    format!("{row}/{cell}")
}

fn value_text(value: &Value) -> String {
    match value {
        Value::Text(text) => text.clone(),
        Value::Bool(flag) => flag.to_string(),
        Value::Num(num) => num.to_string(),
    }
}

fn text_of(binding: &Binding, resolver: &dyn Resolver, row: Option<Row<'_>>) -> String {
    match binding {
        Binding::Literal(text) => text.clone(),
        Binding::Localized(key) | Binding::Dynamic(key) => resolver.text(key),
        Binding::Field(path) => match row {
            Some(cell) => value_text(&cell.lists.value(cell.source, cell.index, path)),
            None => {
                log::debug!("L1 Field binding outside a row template is empty");
                String::new()
            }
        },
    }
}

fn flag_of(binding: &BoolBinding, resolver: &dyn Resolver, row: Option<Row<'_>>) -> bool {
    match binding {
        BoolBinding::Literal(flag) => *flag,
        BoolBinding::Flag(key) => resolver.flag(key),
        BoolBinding::Field(path) => match row {
            Some(cell) => matches!(
                cell.lists.value(cell.source, cell.index, path),
                Value::Bool(true)
            ),
            None => {
                log::debug!("L1 flag Field outside a row template is false");
                false
            }
        },
    }
}

fn num_of(binding: &NumBinding, resolver: &dyn Resolver, row: Option<Row<'_>>) -> u32 {
    match binding {
        NumBinding::Literal(num) => *num,
        NumBinding::Bound { source, path } => {
            let (source, index) = match row {
                Some(cell) if cell.source == source.as_str() => (cell.source, cell.index),
                _ => (source.as_str(), 0),
            };
            match resolver.value(source, index, path) {
                Value::Num(num) => num,
                _ => 0,
            }
        }
    }
}

/// Row projection. The only subunit that reads item data, and only through
/// [`Lists`]: ordering comes from the source, stability from `id_field`.
/// Row and cell keys are namespaced under the source and tracked in `live`.
#[allow(clippy::too_many_arguments)]
fn project_rows<K: Eq + Clone>(
    rows: &RowsDef,
    ids: &mut IdRegistry<K>,
    key: &dyn Fn(&str) -> K,
    resolver: &dyn Resolver,
    live: &mut Vec<K>,
) -> Vec<ItemRow> {
    let lists = resolver as &dyn Lists;
    let RowsDef::Bound { source, template } = rows;
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::new();
    for index in 0..lists.list_len(source) {
        let identity = value_text(&lists.value(source, index, &template.id_field));
        let occurrence = seen.get(&identity).copied().unwrap_or(0);
        seen.insert(identity.clone(), occurrence + 1);
        let namespace = row_key(source, &identity, occurrence);
        let row_key = key(&namespace);
        live.push(row_key.clone());
        let row_id = ids.id(row_key);
        let cell = Row {
            lists,
            source,
            index,
        };
        let cells = template
            .cells
            .iter()
            .map(|control| {
                let cell_key = key(&format!("{namespace}/{}", control.key));
                live.push(cell_key.clone());
                let id = ids.id(cell_key);
                build_control(control, ids, key, resolver, Some(cell), id, live)
            })
            .collect();
        out.push(ItemRow {
            id: row_id,
            depth: 0,
            expanded: None,
            cells,
        });
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn build_control<K: Eq + Clone>(
    def: &ControlDef,
    ids: &mut IdRegistry<K>,
    key: &dyn Fn(&str) -> K,
    resolver: &dyn Resolver,
    row: Option<Row<'_>>,
    id: ControlId,
    live: &mut Vec<K>,
) -> Control {
    let label = text_of(&def.label, resolver, row);
    let mut metadata = ControlMetadata::new(label.clone());
    metadata.enabled = flag_of(&def.enabled, resolver, row);
    metadata.visible = flag_of(&def.visible, resolver, row);
    metadata.hints = def.hints;
    Control {
        id,
        label,
        metadata,
        display: def.display,
        leaf: build_leaf(&def.leaf, ids, key, resolver, row, live),
    }
}

#[allow(clippy::too_many_arguments)]
fn build_leaf<K: Eq + Clone>(
    leaf: &LeafDef,
    ids: &mut IdRegistry<K>,
    key: &dyn Fn(&str) -> K,
    resolver: &dyn Resolver,
    row: Option<Row<'_>>,
    live: &mut Vec<K>,
) -> Box<dyn ControlLeaf> {
    match leaf {
        LeafDef::Note { text } => Box::new(NoteLeaf {
            text: text_of(text, resolver, row),
        }),
        LeafDef::Action => Box::new(ActionLeaf),
        LeafDef::Toggle { value } => Box::new(ToggleLeaf {
            value: flag_of(value, resolver, row),
        }),
        LeafDef::Choice { choices, selected } => {
            let items = match choices {
                ChoicesDef::Static(list) => list
                    .iter()
                    .map(|item| text_of(item, resolver, row))
                    .collect(),
                ChoicesDef::Bound(source) => resolver.choices(source),
            };
            let current = match selected {
                SelectedDef::Index(index) => Some(*index),
                SelectedDef::Bound(source) => resolver.selected(source),
            };
            Box::new(ChoiceLeaf {
                choices: items,
                selected: current,
            })
        }
        LeafDef::Text { value, placeholder } => Box::new(TextLeaf {
            value: text_of(value, resolver, row),
            placeholder: text_of(placeholder, resolver, row),
            browse_label: None,
        }),
        LeafDef::Number { value, min, max } => {
            let current = num_of(value, resolver, row);
            Box::new(NumberLeaf {
                value: current,
                min: *min,
                max: *max,
                step: 1,
                unit: String::new(),
                formatted: current.to_string(),
            })
        }
        LeafDef::List { columns, rows } => {
            use crate::scene::leaf::ColumnDef;
            Box::new(ListLeaf {
                columns: columns
                    .iter()
                    .map(|column| ColumnDef {
                        id: column.id.clone(),
                        header: text_of(&column.header, resolver, row),
                        width: column.width,
                    })
                    .collect(),
                rows: project_rows(rows, ids, key, resolver, live),
            })
        }
        LeafDef::Custom { kind, fields } => {
            let resolved: Vec<(String, Value)> = fields
                .iter()
                .map(|(name, binding)| match binding {
                    Binding::Literal(text) => (name.clone(), Value::Text(text.clone())),
                    Binding::Localized(key) | Binding::Dynamic(key) => {
                        (name.clone(), Value::Text(resolver.text(key)))
                    }
                    Binding::Field(path) => (
                        name.clone(),
                        match row {
                            Some(cell) => cell.lists.value(cell.source, cell.index, path),
                            None => Value::Text(String::new()),
                        },
                    ),
                })
                .collect();
            resolver
                .build(kind, &resolved)
                .unwrap_or_else(|| Box::new(NoteLeaf { text: kind.clone() }))
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_container<K: Eq + Clone>(
    def: &ContainerDef,
    ids: &mut IdRegistry<K>,
    key: &dyn Fn(&str) -> K,
    resolver: &dyn Resolver,
    live: &mut Vec<K>,
    row: Option<Row<'_>>,
) -> Container {
    let label = text_of(&def.label, resolver, row);
    let mut metadata = ControlMetadata::new(label.clone());
    metadata.hints = def.hints;
    let header = def.header.as_ref().map(|control| {
        let id = ids.id(key(&control.key));
        build_control(control, ids, key, resolver, row, id, live)
    });
    let children = def
        .children
        .iter()
        .map(|node| match node {
            NodeDef::Container(inner) => Node::Container(Box::new(build_container(
                inner, ids, key, resolver, live, row,
            ))),
            NodeDef::Tabs(tabs) => Node::Tabs(build_tabs(tabs, ids, key, resolver, live, row)),
            NodeDef::Control(control) => {
                let id = ids.id(key(&control.key));
                Node::Control(build_control(control, ids, key, resolver, row, id, live))
            }
        })
        .collect();
    Container {
        id: ids.id(key(&def.key)),
        label,
        metadata,
        header,
        policy: policy_of(&def.policy),
        children,
    }
}

#[allow(clippy::too_many_arguments)]
fn build_tabs<K: Eq + Clone>(
    def: &TabsDef,
    ids: &mut IdRegistry<K>,
    key: &dyn Fn(&str) -> K,
    resolver: &dyn Resolver,
    live: &mut Vec<K>,
    row: Option<Row<'_>>,
) -> Tabs {
    let tabs = def
        .tabs
        .iter()
        .enumerate()
        .map(|(index, tab)| build_tab(tab, index == def.selected, ids, key, resolver, live, row))
        .collect();
    Tabs {
        id: ids.id(key(&def.key)),
        metadata: ControlMetadata::new(String::new()),
        selected: Some(def.selected),
        tabs,
    }
}

#[allow(clippy::too_many_arguments)]
fn build_tab<K: Eq + Clone>(
    def: &TabDef,
    selected: bool,
    ids: &mut IdRegistry<K>,
    key: &dyn Fn(&str) -> K,
    resolver: &dyn Resolver,
    live: &mut Vec<K>,
    row: Option<Row<'_>>,
) -> Tab {
    let label = text_of(&def.label, resolver, row);
    Tab {
        id: ids.id(key(&def.key)),
        label: label.clone(),
        selected,
        select: Control {
            id: ids.id(key(&format!("{}/select", def.key))),
            label: label.clone(),
            metadata: ControlMetadata::new(label),
            display: ControlDisplay::Standard,
            leaf: Box::new(ActionLeaf),
        },
        body: build_container(&def.body, ids, key, resolver, live, row),
    }
}

fn build_page<K: Eq + Clone>(
    def: &PageDef,
    ids: &mut IdRegistry<K>,
    key: &dyn Fn(&str) -> K,
    resolver: &dyn Resolver,
    live: &mut Vec<K>,
) -> Page {
    let label = text_of(&def.label, resolver, None);
    Page {
        id: ids.id(key(&def.key)),
        label: label.clone(),
        selected: def.selected,
        select: Control {
            id: ids.id(key(&format!("{}/select", def.key))),
            label: label.clone(),
            metadata: ControlMetadata::new(label),
            display: ControlDisplay::Standard,
            leaf: Box::new(ActionLeaf),
        },
        body: build_container(&def.body, ids, key, resolver, live, None),
    }
}

fn policy_of(def: &PolicyDef) -> Box<dyn LayoutPolicy> {
    match def {
        PolicyDef::Row => Box::new(RowPolicy),
        PolicyDef::Column => Box::new(ColumnPolicy),
        PolicyDef::Group => Box::new(GroupPolicy),
        PolicyDef::Table => Box::new(TablePolicy),
        PolicyDef::Flex => Box::new(FlexPolicy),
        PolicyDef::Absolute => Box::new(AbsolutePolicy { bounds: None }),
        PolicyDef::Custom { kind } => {
            log::debug!("unknown L1 policy '{kind}' degrades to column placement");
            Box::new(ColumnPolicy)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::control::ControlVisitor;
    use crate::scene::def::ListColumnDef;
    use crate::scene::def::{
        Binding, BoolBinding, ChoicesDef, ContainerDef, ControlDef, LeafDef, NodeDef, PageDef,
        PolicyDef, RowTemplateDef, RowsDef, ScreenDef, SelectedDef,
    };
    use crate::scene::hints::LayoutHints;

    #[derive(Debug, Clone)]
    struct FakeItem {
        id: String,
        label: String,
        on: bool,
    }

    #[derive(Debug, Default)]
    struct Fake {
        texts: HashMap<String, String>,
        flags: HashMap<String, bool>,
        items: Vec<FakeItem>,
        choices: Vec<String>,
        selected: Option<usize>,
        custom_probe: bool,
    }

    impl Texts for Fake {
        fn text(&self, key: &str) -> String {
            self.texts.get(key).cloned().unwrap_or_default()
        }
    }

    impl Flags for Fake {
        fn flag(&self, key: &str) -> bool {
            self.flags.get(key).copied().unwrap_or(false)
        }
    }

    impl Lists for Fake {
        fn list_len(&self, source: &str) -> usize {
            assert_eq!(source, "options");
            self.items.len()
        }

        fn value(&self, source: &str, index: usize, path: &str) -> Value {
            assert_eq!(source, "options");
            let item = &self.items[index];
            match path {
                "id" => Value::Text(item.id.clone()),
                "label" => Value::Text(item.label.clone()),
                "on" => Value::Bool(item.on),
                _ => Value::Text(String::new()),
            }
        }
    }

    impl Choices for Fake {
        fn choices(&self, source: &str) -> Vec<String> {
            assert_eq!(source, "filter");
            self.choices.clone()
        }

        fn selected(&self, source: &str) -> Option<usize> {
            assert_eq!(source, "filter");
            self.selected
        }
    }

    impl CustomLeaves for Fake {
        fn build(&self, kind: &str, _fields: &[(String, Value)]) -> Option<Box<dyn ControlLeaf>> {
            if self.custom_probe && kind == "probe" {
                Some(Box::new(ActionLeaf))
            } else {
                None
            }
        }
    }

    fn note_cell(key: &str) -> ControlDef {
        ControlDef {
            key: key.into(),
            label: Binding::Field("label".into()),
            display: ControlDisplay::Standard,
            enabled: BoolBinding::Literal(true),
            visible: BoolBinding::Literal(true),
            hints: LayoutHints::default(),
            leaf: LeafDef::Note {
                text: Binding::Field("label".into()),
            },
        }
    }

    fn table_control() -> ControlDef {
        ControlDef {
            key: "table".into(),
            label: Binding::Localized("table".into()),
            display: ControlDisplay::Standard,
            enabled: BoolBinding::Literal(true),
            visible: BoolBinding::Literal(true),
            hints: LayoutHints::default(),
            leaf: LeafDef::List {
                columns: vec![
                    ListColumnDef {
                        id: "name".into(),
                        header: Binding::Literal("Name".into()),
                        width: None,
                    },
                    ListColumnDef {
                        id: "state".into(),
                        header: Binding::Literal("State".into()),
                        width: None,
                    },
                ],
                rows: RowsDef::Bound {
                    source: "options".into(),
                    template: RowTemplateDef {
                        id_field: "id".into(),
                        cells: vec![
                            note_cell("cell"),
                            ControlDef {
                                key: "toggle".into(),
                                label: Binding::Field("label".into()),
                                display: ControlDisplay::Standard,
                                enabled: BoolBinding::Literal(true),
                                visible: BoolBinding::Literal(true),
                                hints: LayoutHints::default(),
                                leaf: LeafDef::Toggle {
                                    value: BoolBinding::Flag("follow".into()),
                                },
                            },
                        ],
                    },
                },
            },
        }
    }

    fn screen_def() -> ScreenDef {
        ScreenDef {
            title: Binding::Localized("title".into()),
            pages: vec![PageDef {
                key: "page".into(),
                label: Binding::Localized("title".into()),
                selected: true,
                body: ContainerDef {
                    key: "body".into(),
                    label: Binding::Literal(String::new()),
                    hints: LayoutHints::default(),
                    header: None,
                    policy: PolicyDef::Column,
                    children: vec![
                        NodeDef::Control(Box::new(ControlDef {
                            key: "filter".into(),
                            label: Binding::Localized("filter".into()),
                            display: ControlDisplay::Standard,
                            enabled: BoolBinding::Literal(true),
                            visible: BoolBinding::Literal(true),
                            hints: LayoutHints::default(),
                            leaf: LeafDef::Choice {
                                choices: ChoicesDef::Bound("filter".into()),
                                selected: SelectedDef::Bound("filter".into()),
                            },
                        })),
                        NodeDef::Control(Box::new(table_control())),
                    ],
                },
            }],
            footer: ContainerDef {
                key: "footer".into(),
                label: Binding::Literal(String::new()),
                hints: LayoutHints::default(),
                header: None,
                policy: PolicyDef::Column,
                children: Vec::new(),
            },
        }
    }

    fn fake() -> Fake {
        Fake {
            texts: [
                ("title".to_string(), "Section".to_string()),
                ("filter".to_string(), "Option".to_string()),
                ("table".to_string(), "Section".to_string()),
            ]
            .into_iter()
            .collect(),
            flags: [("follow".to_string(), true)].into_iter().collect(),
            items: vec![
                FakeItem {
                    id: "a".into(),
                    label: "first".into(),
                    on: true,
                },
                FakeItem {
                    id: "b".into(),
                    label: "second".into(),
                    on: false,
                },
            ],
            choices: vec!["all".to_string(), "some".to_string()],
            selected: Some(1),
            custom_probe: false,
        }
    }

    #[derive(Default)]
    struct Order(Vec<String>);

    impl ControlVisitor for Order {
        fn control(&mut self, control: &Control) {
            self.0.push(control.label.clone());
        }
    }

    fn visit_labels(screen: &ControlScreen) -> Vec<String> {
        let mut order = Order::default();
        screen.visit(&mut order);
        order.0
    }

    fn row_ids(screen: &ControlScreen) -> Vec<ControlId> {
        let mut found = Vec::new();
        struct Rows(Vec<ControlId>);
        impl ControlVisitor for Rows {
            fn control(&mut self, control: &Control) {
                if control.downcast::<ListLeaf>().is_some() {
                    for row in &control.downcast::<ListLeaf>().unwrap().rows {
                        self.0.push(row.id);
                    }
                }
            }
        }
        let mut rows = Rows(Vec::new());
        screen.visit(&mut rows);
        found.append(&mut rows.0);
        found
    }

    #[test]
    fn stable_ids_across_frames_and_languages() {
        let def = screen_def();
        let resolver = fake();
        let mut ctx = InflateCtx::default();
        let key = |s: &str| s.to_string();
        let (first, live) = inflate(&def, &mut ctx, key, &resolver);
        ctx.retain_rows(&live);
        let (second, live) = inflate(&def, &mut ctx, key, &resolver);
        ctx.retain_rows(&live);
        assert_eq!(first, second);

        let mut relabeled = fake();
        relabeled
            .texts
            .insert("title".to_string(), "Renamed".to_string());
        let (third, live) = inflate(&def, &mut ctx, key, &relabeled);
        ctx.retain_rows(&live);
        assert_eq!(third.title, "Renamed");
        assert_eq!(visit_labels(&first).len(), visit_labels(&third).len());
        let mut first_ids = Order::default();
        first.visit(&mut first_ids);
        let mut third_ids = Order::default();
        third.visit(&mut third_ids);
        assert_eq!(first_ids.0.len(), third_ids.0.len());
    }

    #[test]
    fn language_switch_keeps_control_identity() {
        let def = screen_def();
        let mut ctx: InflateCtx<String> = InflateCtx::default();
        let key = |s: &str| s.to_string();
        let (first, live) = inflate(&def, &mut ctx, key, &fake());
        ctx.retain_rows(&live);
        let mut relabeled = fake();
        relabeled
            .texts
            .insert("title".to_string(), "Renamed".to_string());
        let (second, live) = inflate(&def, &mut ctx, key, &relabeled);
        ctx.retain_rows(&live);
        struct Ids(Vec<ControlId>);
        impl ControlVisitor for Ids {
            fn control(&mut self, control: &Control) {
                self.0.push(control.id);
            }
        }
        let mut a = Ids(Vec::new());
        first.visit(&mut a);
        let mut b = Ids(Vec::new());
        second.visit(&mut b);
        assert_eq!(a.0, b.0);
    }

    #[test]
    fn duplicate_rows_get_disjoint_ids_in_order() {
        let def = screen_def();
        let mut resolver = fake();
        resolver.items.push(FakeItem {
            id: "a".into(),
            label: "third".into(),
            on: false,
        });
        let mut ctx: InflateCtx<String> = InflateCtx::default();
        let key = |s: &str| s.to_string();
        let (screen, live) = inflate(&def, &mut ctx, key, &resolver);
        ctx.retain_rows(&live);
        let ids = row_ids(&screen);
        assert_eq!(ids.len(), 3);
        assert_ne!(ids[0], ids[2]);
        // ListLeaf cells surface only through explicit walk: read them off
        // the retained table instead of the flat visit order.
        struct Tables(Vec<Vec<String>>);
        impl ControlVisitor for Tables {
            fn control(&mut self, control: &Control) {
                if let Some(list) = control.downcast::<ListLeaf>() {
                    self.0.push(
                        list.rows
                            .iter()
                            .map(|row| row.cells[0].label.clone())
                            .collect(),
                    );
                }
            }
        }
        let mut tables = Tables(Vec::new());
        screen.visit(&mut tables);
        assert_eq!(tables.0, vec![vec!["first", "second", "third"]]);
    }

    #[test]
    fn removed_rows_expire_and_return_fresh() {
        let def = screen_def();
        let mut ctx: InflateCtx<String> = InflateCtx::default();
        let key = |s: &str| s.to_string();
        let (first, live) = inflate(&def, &mut ctx, key, &fake());
        ctx.retain_rows(&live);
        let before = row_ids(&first);
        assert_eq!(before.len(), 2);

        let mut pruned = fake();
        pruned.items.remove(0);
        let (second, live) = inflate(&def, &mut ctx, key, &pruned);
        ctx.retain_rows(&live);
        assert_eq!(row_ids(&second).len(), 1);

        let (third, live) = inflate(&def, &mut ctx, key, &fake());
        ctx.retain_rows(&live);
        let after = row_ids(&third);
        assert_eq!(after.len(), 2);
        assert!(
            !after.contains(&before[0]),
            "returning keys receive fresh IDs, never recycled ones"
        );
    }

    #[test]
    fn screen_level_field_binding_is_silent() {
        let mut def = screen_def();
        def.title = Binding::Field("nope".into());
        let mut ctx: InflateCtx<String> = InflateCtx::default();
        let key = |s: &str| s.to_string();
        let (screen, live) = inflate(&def, &mut ctx, key, &fake());
        ctx.retain_rows(&live);
        assert_eq!(screen.title, String::new());
    }

    #[test]
    fn unknown_custom_leaf_degrades_to_note_in_order() {
        let mut def = screen_def();
        def.pages[0]
            .body
            .children
            .push(NodeDef::Control(Box::new(ControlDef {
                key: "mystery".into(),
                label: Binding::Literal("mystery".into()),
                display: ControlDisplay::Standard,
                enabled: BoolBinding::Literal(true),
                visible: BoolBinding::Literal(true),
                hints: LayoutHints::default(),
                leaf: LeafDef::Custom {
                    kind: "unregistered".into(),
                    fields: Vec::new(),
                },
            })));
        let mut ctx: InflateCtx<String> = InflateCtx::default();
        let key = |s: &str| s.to_string();
        let (screen, live) = inflate(&def, &mut ctx, key, &fake());
        ctx.retain_rows(&live);
        let labels = visit_labels(&screen);
        assert!(labels.contains(&"mystery".to_string()));
        struct Fallback(Option<String>);
        impl ControlVisitor for Fallback {
            fn control(&mut self, control: &Control) {
                if control.label == "mystery" {
                    self.0 = control.downcast::<NoteLeaf>().map(|leaf| leaf.text.clone());
                }
            }
        }
        let mut fallback = Fallback(None);
        screen.visit(&mut fallback);
        assert_eq!(fallback.0.as_deref(), Some("unregistered"));
    }

    #[test]
    fn registered_custom_leaf_builds_through_resolver() {
        let mut def = screen_def();
        def.pages[0]
            .body
            .children
            .push(NodeDef::Control(Box::new(ControlDef {
                key: "probe".into(),
                label: Binding::Literal("probe".into()),
                display: ControlDisplay::Standard,
                enabled: BoolBinding::Literal(true),
                visible: BoolBinding::Literal(true),
                hints: LayoutHints::default(),
                leaf: LeafDef::Custom {
                    kind: "probe".into(),
                    fields: Vec::new(),
                },
            })));
        let mut resolver = fake();
        resolver.custom_probe = true;
        let mut ctx: InflateCtx<String> = InflateCtx::default();
        let key = |s: &str| s.to_string();
        let (screen, live) = inflate(&def, &mut ctx, key, &resolver);
        ctx.retain_rows(&live);
        struct Probe(bool);
        impl ControlVisitor for Probe {
            fn control(&mut self, control: &Control) {
                if control.label == "probe" && control.downcast::<ActionLeaf>().is_some() {
                    self.0 = true;
                }
            }
        }
        let mut probe = Probe(false);
        screen.visit(&mut probe);
        assert!(probe.0);
    }

    #[test]
    fn choice_and_toggle_resolve_from_outer_state() {
        let def = screen_def();
        let mut ctx: InflateCtx<String> = InflateCtx::default();
        let key = |s: &str| s.to_string();
        let (screen, live) = inflate(&def, &mut ctx, key, &fake());
        ctx.retain_rows(&live);
        struct Check {
            choices: usize,
            selected: Option<usize>,
        }
        impl ControlVisitor for Check {
            fn control(&mut self, control: &Control) {
                if let Some(choice) = control.downcast::<ChoiceLeaf>() {
                    self.choices = choice.choices.len();
                    self.selected = choice.selected;
                }
            }
        }
        let mut check = Check {
            choices: 0,
            selected: None,
        };
        screen.visit(&mut check);
        assert_eq!(check.choices, 2);
        assert_eq!(check.selected, Some(1));
    }

    #[test]
    fn custom_policy_degrades_without_panic() {
        let mut def = screen_def();
        def.pages[0].body.policy = PolicyDef::Custom {
            kind: "masonry".into(),
        };
        let mut ctx: InflateCtx<String> = InflateCtx::default();
        let key = |s: &str| s.to_string();
        let (screen, live) = inflate(&def, &mut ctx, key, &fake());
        ctx.retain_rows(&live);
        assert!(!visit_labels(&screen).is_empty());
    }
}
