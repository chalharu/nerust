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
    AbsolutePolicy, Capture, ColumnPolicy, Container, Control, ControlDisplay, ControlMetadata,
    ControlScreen, FlexPolicy, GroupPolicy, IdRegistry, LayoutPolicy, Node, Page, RowPolicy, Tab,
    TablePolicy, Tabs,
};
use crate::scene::def::{
    Binding, BoolBinding, ChoicesDef, ContainerDef, ControlDef, LeafDef, NodeDef, NumBinding,
    PageDef, PolicyDef, RowsDef, ScreenDef, SelectedDef, TabDef, TabsDef,
};
use crate::scene::leaf::{
    ActionLeaf, BindingLeaf, ChoiceLeaf, ColumnDef, ControlLeaf, ImageLeaf, ItemRow, ListLeaf,
    NoteLeaf, NumberLeaf, TextLeaf, ToggleLeaf,
};

/// Framework-primitive resolved value. Domain types convert into this;
/// the engine never names them.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Text(String),
    Bool(bool),
    Num(u32),
    /// Raster picture data. Compared by value like the other variants;
    /// screens only project new pictures on generation change.
    Image(ImageData),
    /// Choice option labels for row-relative option sources.
    List(Vec<String>),
}

/// Raw picture bytes in framework vocabulary: the exact payload an
/// [`crate::scene::leaf::ImageLeaf`] carries.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageData {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
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
/// Nested templates additionally use `nested_len`/`nested_value`; defaults
/// degrade to empty so existing implementors never break (OCP).
pub trait Lists {
    fn list_len(&self, source: &str) -> usize {
        let _ = source;
        0
    }
    fn value(&self, source: &str, index: usize, path: &str) -> Value {
        let _ = (source, index, path);
        Value::Text(String::new())
    }
    fn nested_len(&self, source: &str, parent: &RowPath) -> usize {
        let _ = (source, parent);
        0
    }
    fn nested_value(&self, source: &str, parent: &RowPath, index: usize, path: &str) -> Value {
        let _ = (source, parent, index, path);
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
    let footer = build_container(
        &def.footer,
        &mut ctx.ids,
        key,
        resolver,
        &mut live,
        None,
        "",
    );
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

/// Ancestor chain for nested templates: `(source, index)` from the
/// outermost row down to the immediate parent (excluding self).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RowPath(pub Vec<(String, usize)>);

/// Row context: which item of which source a template cell reads.
/// `path` holds ancestors excluding self; `scoped` selects parent-relative
/// resolution (`nested_*`) over flat resolution (`list_len`/`value`).
#[derive(Clone, Copy)]
struct Row<'a> {
    lists: &'a dyn Lists,
    source: &'a str,
    index: usize,
    path: &'a RowPath,
    scoped: bool,
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
        Value::Image(_) | Value::List(_) => String::new(),
    }
}

/// Display formatting for numeric leaves: percent suffixes attach directly,
/// other units join with a space. View definitions pass the unit through.
fn format_number(value: u32, unit: &str) -> String {
    if unit == "%" {
        format!("{value}%")
    } else if unit.is_empty() {
        value.to_string()
    } else {
        format!("{value} {unit}")
    }
}

/// Single field read honoring the row's scope: flat or parent-relative.
fn field_value(cell: &Row<'_>, path: &str) -> Value {
    if cell.scoped {
        cell.lists
            .nested_value(cell.source, cell.path, cell.index, path)
    } else {
        cell.lists.value(cell.source, cell.index, path)
    }
}

fn text_of(binding: &Binding, resolver: &dyn Resolver, row: Option<Row<'_>>) -> String {
    match binding {
        Binding::Literal(text) => text.clone(),
        Binding::Localized(key) | Binding::Dynamic(key) => resolver.text(key),
        Binding::Field(path) => match row {
            Some(cell) => value_text(&field_value(&cell, path)),
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
            Some(cell) => matches!(field_value(&cell, path), Value::Bool(true)),
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
            let num = match row {
                Some(cell) if cell.scoped || cell.source == source.as_str() => {
                    field_value(&cell, path)
                }
                _ => resolver.value(source, 0, path),
            };
            match num {
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
    row: Option<Row<'_>>,
    ns: &str,
) -> Vec<ItemRow> {
    let lists = resolver as &dyn Lists;
    let (source, template, parent) = match (rows, row) {
        (RowsDef::Bound { source, template }, _) => (source, template, None),
        (RowsDef::Nested { source, template }, Some(outer)) => {
            let mut base = outer.path.clone();
            base.0.push((outer.source.to_string(), outer.index));
            (source, template, Some(base))
        }
        (RowsDef::Nested { .. }, None) => {
            log::debug!("L1 nested list outside a row template is empty");
            return Vec::new();
        }
    };
    let len = match &parent {
        Some(base) => lists.nested_len(source, base),
        None => lists.list_len(source),
    };
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::new();
    for index in 0..len {
        let identity = value_text(&match &parent {
            Some(base) => lists.nested_value(source, base, index, &template.id_field),
            None => lists.value(source, index, &template.id_field),
        });
        let occurrence = seen.get(&identity).copied().unwrap_or(0);
        seen.insert(identity.clone(), occurrence + 1);
        let namespace = namespaced(ns, &row_key(source, &identity, occurrence));
        let row_key = key(&namespace);
        live.push(row_key.clone());
        let row_id = ids.id(row_key);
        let scoped = parent.is_some();
        // Ancestors excluding self: `parent` already is that chain.
        let path = parent.clone().unwrap_or_default();
        let cell = Row {
            lists,
            source,
            index,
            path: &path,
            scoped,
        };
        let cells = template
            .cells
            .iter()
            .map(|control| build_control(control, ids, key, resolver, Some(cell), live, &namespace))
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
    live: &mut Vec<K>,
    ns: &str,
) -> Control {
    let label = text_of(&def.label, resolver, row);
    let accessible = def
        .accessible
        .as_ref()
        .map(|binding| text_of(binding, resolver, row))
        .unwrap_or_else(|| label.clone());
    let mut metadata = ControlMetadata::new(accessible);
    metadata.enabled = flag_of(&def.enabled, resolver, row);
    metadata.visible = flag_of(&def.visible, resolver, row);
    metadata.help = def
        .help
        .as_ref()
        .map(|binding| text_of(binding, resolver, row));
    metadata.hints = def.hints;
    let name = key(&namespaced(ns, &def.key));
    live.push(name.clone());
    Control {
        id: ids.id(name),
        label,
        metadata,
        display: def.display,
        leaf: build_leaf(&def.leaf, ids, key, resolver, row, live, ns),
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
    ns: &str,
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
            let (items, current) = match choices {
                ChoicesDef::Static(list) => (
                    list.iter()
                        .map(|item| text_of(item, resolver, row))
                        .collect(),
                    match selected {
                        SelectedDef::Index(index) => Some(*index),
                        SelectedDef::Bound(source) => resolver.selected(source),
                    },
                ),
                ChoicesDef::Bound(source) => (
                    resolver.choices(source),
                    match selected {
                        SelectedDef::Index(index) => Some(*index),
                        SelectedDef::Bound(source) => resolver.selected(source),
                    },
                ),
                ChoicesDef::Row { labels, selected } => match row {
                    Some(cell) => (
                        match field_value(&cell, labels) {
                            Value::List(items) => items,
                            _ => Vec::new(),
                        },
                        match field_value(&cell, selected) {
                            Value::Num(index) => Some(index as usize),
                            _ => None,
                        },
                    ),
                    None => {
                        log::debug!("L1 row choices outside a row template are empty");
                        (Vec::new(), None)
                    }
                },
            };
            Box::new(ChoiceLeaf {
                choices: items,
                selected: current,
            })
        }
        LeafDef::Text {
            value,
            placeholder,
            browse_label,
        } => Box::new(TextLeaf {
            value: text_of(value, resolver, row),
            placeholder: text_of(placeholder, resolver, row),
            browse_label: browse_label
                .as_ref()
                .map(|binding| text_of(binding, resolver, row)),
        }),
        LeafDef::Number {
            value,
            min,
            max,
            step,
            unit,
        } => {
            let current = num_of(value, resolver, row);
            Box::new(NumberLeaf {
                value: current,
                min: *min,
                max: *max,
                step: *step,
                unit: unit.clone(),
                formatted: format_number(current, unit),
            })
        }
        LeafDef::Binding {
            text,
            capturing,
            change_label,
            clear_label,
        } => Box::new(BindingLeaf {
            text: text_of(text, resolver, row),
            capturing: flag_of(capturing, resolver, row),
            change_label: text_of(change_label, resolver, row),
            clear_label: text_of(clear_label, resolver, row),
        }),
        LeafDef::List { columns, rows } => Box::new(ListLeaf {
            columns: columns
                .iter()
                .map(|column| ColumnDef {
                    id: column.id.clone(),
                    header: text_of(&column.header, resolver, row),
                    width: column.width,
                })
                .collect(),
            rows: project_rows(rows, ids, key, resolver, live, row, ns),
        }),
        LeafDef::Image { path } => {
            let (width, height, rgba) = match row {
                Some(cell) => match field_value(&cell, path) {
                    Value::Image(data) => (data.width, data.height, data.rgba),
                    _ => (0, 0, Vec::new()),
                },
                None => {
                    log::debug!("L1 image outside a row template is empty");
                    (0, 0, Vec::new())
                }
            };
            Box::new(ImageLeaf {
                width,
                height,
                rgba,
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
                            Some(cell) => field_value(&cell, path),
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

/// Namespace join for static keys under a dynamic namespace.
/// Empty namespace passes the key through, keeping top-level chrome keys
/// (`filter`, `page/select`) stable. Dynamic levels nest their namespaces,
/// so equal identities under different parents never share an ID.
/// `cell_key` is the same join; view-defs recomputes keys with it.
fn namespaced(ns: &str, key: &str) -> String {
    if ns.is_empty() {
        key.to_string()
    } else {
        cell_key(ns, key)
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
    ns: &str,
) -> Container {
    let label = text_of(&def.label, resolver, row);
    let mut metadata = ControlMetadata::new(label.clone());
    metadata.hints = def.hints;
    let header = def
        .header
        .as_ref()
        .map(|control| build_control(control, ids, key, resolver, row, live, ns));
    let children = def
        .children
        .iter()
        .map(|node| match node {
            NodeDef::Container(inner) => Node::Container(Box::new(build_container(
                inner, ids, key, resolver, live, row, ns,
            ))),
            NodeDef::Tabs(tabs) => Node::Tabs(build_tabs(tabs, ids, key, resolver, live, row, ns)),
            NodeDef::Control(control) => {
                Node::Control(build_control(control, ids, key, resolver, row, live, ns))
            }
        })
        .collect();
    let name = key(&namespaced(ns, &def.key));
    live.push(name.clone());
    Container {
        id: ids.id(name),
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
    ns: &str,
) -> Tabs {
    match def {
        TabsDef::Static {
            key: tabs_key,
            tabs,
            selected,
        } => {
            let built = tabs
                .iter()
                .enumerate()
                .map(|(index, tab)| {
                    build_tab(tab, index == *selected, ids, key, resolver, live, row, ns)
                })
                .collect();
            let name = key(&namespaced(ns, tabs_key));
            live.push(name.clone());
            Tabs {
                id: ids.id(name),
                metadata: ControlMetadata::new(String::new()),
                selected: Some(*selected),
                tabs: built,
            }
        }
        TabsDef::Bound {
            key: tabs_key,
            source,
            template,
            selected,
        } => {
            let lists = resolver as &dyn Lists;
            // Positional rule: a bound set at screen level resolves flat;
            // inside a template it resolves parent-relatively.
            let parent = row.map(|outer| {
                let mut base = outer.path.clone();
                base.0.push((outer.source.to_string(), outer.index));
                base
            });
            let len = match &parent {
                Some(base) => lists.nested_len(source, base),
                None => lists.list_len(source),
            };
            let current = match selected {
                SelectedDef::Index(index) => Some(*index),
                SelectedDef::Bound(source) => resolver.selected(source),
            };
            let mut seen: HashMap<String, usize> = HashMap::new();
            let mut tabs = Vec::new();
            for index in 0..len {
                let identity = value_text(&match &parent {
                    Some(base) => lists.nested_value(source, base, index, &template.id_field),
                    None => lists.value(source, index, &template.id_field),
                });
                let occurrence = seen.get(&identity).copied().unwrap_or(0);
                seen.insert(identity.clone(), occurrence + 1);
                let tabns = namespaced(ns, &row_key(source, &identity, occurrence));
                let scoped = parent.is_some();
                let path = parent.clone().unwrap_or_default();
                let tab_row = Row {
                    lists,
                    source,
                    index,
                    path: &path,
                    scoped,
                };
                let label = text_of(&template.label, resolver, Some(tab_row));
                let tab_name = key(&tabns);
                live.push(tab_name.clone());
                let select_name = key(&cell_key(&tabns, "select"));
                live.push(select_name.clone());
                tabs.push(Tab {
                    id: ids.id(tab_name),
                    label: label.clone(),
                    selected: current == Some(index),
                    select: Control {
                        id: ids.id(select_name),
                        label: label.clone(),
                        metadata: ControlMetadata::new(label),
                        display: ControlDisplay::Standard,
                        leaf: Box::new(ActionLeaf),
                    },
                    body: build_container(
                        &template.body,
                        ids,
                        key,
                        resolver,
                        live,
                        Some(tab_row),
                        &tabns,
                    ),
                });
            }
            let name = key(&namespaced(ns, tabs_key));
            live.push(name.clone());
            Tabs {
                id: ids.id(name),
                metadata: ControlMetadata::new(String::new()),
                selected: current,
                tabs,
            }
        }
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
    ns: &str,
) -> Tab {
    let label = text_of(&def.label, resolver, row);
    let name = key(&namespaced(ns, &def.key));
    live.push(name.clone());
    let select_name = key(&namespaced(ns, &format!("{}/select", def.key)));
    live.push(select_name.clone());
    Tab {
        id: ids.id(name),
        label: label.clone(),
        selected,
        select: Control {
            id: ids.id(select_name),
            label: label.clone(),
            metadata: ControlMetadata::new(label),
            display: ControlDisplay::Standard,
            leaf: Box::new(ActionLeaf),
        },
        body: build_container(&def.body, ids, key, resolver, live, row, ns),
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
    let name = key(&def.key);
    live.push(name.clone());
    let select_name = key(&format!("{}/select", def.key));
    live.push(select_name.clone());
    Page {
        id: ids.id(name),
        label: label.clone(),
        selected: def.selected,
        select: Control {
            id: ids.id(select_name),
            label: label.clone(),
            metadata: ControlMetadata::new(label),
            display: ControlDisplay::Standard,
            leaf: Box::new(ActionLeaf),
        },
        body: build_container(&def.body, ids, key, resolver, live, None, ""),
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
    use crate::scene::control::ControlId;
    use crate::scene::control::ControlVisitor;
    use crate::scene::def::{
        Binding, BoolBinding, ChoicesDef, ContainerDef, ControlDef, LeafDef, NodeDef, PageDef,
        PolicyDef, RowTemplateDef, RowsDef, ScreenDef, SelectedDef,
    };
    use crate::scene::def::{ListColumnDef, TabTemplateDef};
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
            accessible: None,
            help: None,
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
            accessible: None,
            help: None,
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
                                accessible: None,
                                help: None,
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
                accessible: None,
                help: None,
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
                accessible: None,
                help: None,
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
    struct NestedLists {
        groups: Vec<(String, Vec<String>)>,
    }

    impl Texts for NestedLists {}
    impl Flags for NestedLists {}
    impl Choices for NestedLists {}
    impl CustomLeaves for NestedLists {}

    impl Lists for NestedLists {
        fn list_len(&self, source: &str) -> usize {
            usize::from(source == "groups") * self.groups.len()
        }

        fn value(&self, source: &str, index: usize, path: &str) -> Value {
            assert_eq!(source, "groups");
            match path {
                "id" => Value::Text(self.groups[index].0.clone()),
                _ => Value::Text(String::new()),
            }
        }

        fn nested_len(&self, source: &str, parent: &RowPath) -> usize {
            assert_eq!(source, "members");
            assert_eq!(parent.0.len(), 1);
            let (psource, pindex) = &parent.0[0];
            assert_eq!(psource, "groups");
            self.groups[*pindex].1.len()
        }

        fn nested_value(&self, source: &str, parent: &RowPath, index: usize, path: &str) -> Value {
            assert_eq!(source, "members");
            let (psource, pindex) = &parent.0[0];
            assert_eq!(psource, "groups");
            let member = &self.groups[*pindex].1[index];
            match path {
                "id" => Value::Text(member.clone()),
                "label" => Value::Text(format!("member {member}")),
                _ => Value::Text(String::new()),
            }
        }
    }

    #[test]
    fn nested_lists_resolve_through_parent_path() {
        let def = ScreenDef {
            title: Binding::Literal("Section".into()),
            pages: vec![PageDef {
                key: "page".into(),
                label: Binding::Literal("Section".into()),
                selected: true,
                body: ContainerDef {
                    key: "body".into(),
                    label: Binding::Literal(String::new()),
                    hints: LayoutHints::default(),
                    header: None,
                    policy: PolicyDef::Column,
                    children: vec![NodeDef::Control(Box::new(ControlDef {
                        key: "groups".into(),
                        label: Binding::Literal(String::new()),
                        accessible: None,
                        help: None,
                        display: ControlDisplay::Standard,
                        enabled: BoolBinding::Literal(true),
                        visible: BoolBinding::Literal(true),
                        hints: LayoutHints::default(),
                        leaf: LeafDef::List {
                            columns: Vec::new(),
                            rows: RowsDef::Bound {
                                source: "groups".into(),
                                template: RowTemplateDef {
                                    id_field: "id".into(),
                                    cells: vec![ControlDef {
                                        key: "members".into(),
                                        label: Binding::Literal(String::new()),
                                        accessible: None,
                                        help: None,
                                        display: ControlDisplay::Standard,
                                        enabled: BoolBinding::Literal(true),
                                        visible: BoolBinding::Literal(true),
                                        hints: LayoutHints::default(),
                                        leaf: LeafDef::List {
                                            columns: Vec::new(),
                                            rows: RowsDef::Nested {
                                                source: "members".into(),
                                                template: RowTemplateDef {
                                                    id_field: "id".into(),
                                                    cells: vec![ControlDef {
                                                        key: "cell".into(),
                                                        label: Binding::Field("label".into()),
                                                        accessible: None,
                                                        help: None,
                                                        display: ControlDisplay::Standard,
                                                        enabled: BoolBinding::Literal(true),
                                                        visible: BoolBinding::Literal(true),
                                                        hints: LayoutHints::default(),
                                                        leaf: LeafDef::Note {
                                                            text: Binding::Field("label".into()),
                                                        },
                                                    }],
                                                },
                                            },
                                        },
                                    }],
                                },
                            },
                        },
                    }))],
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
        };
        let resolver = NestedLists {
            groups: vec![
                ("g0".into(), vec!["a".into(), "b".into()]),
                ("g1".into(), vec!["c".into()]),
            ],
        };
        let mut ctx: InflateCtx<String> = InflateCtx::default();
        let key = |s: &str| s.to_string();
        let (screen, live) = inflate(&def, &mut ctx, key, &resolver);
        ctx.retain_rows(&live);
        struct Notes(Vec<String>);
        impl ControlVisitor for Notes {
            fn control(&mut self, control: &Control) {
                if let Some(note) = control.downcast::<NoteLeaf>() {
                    self.0.push(note.text.clone());
                }
                crate::scene::control::walk_control(control, self);
            }
        }
        let mut notes = Notes(Vec::new());
        screen.visit(&mut notes);
        assert_eq!(notes.0, vec!["member a", "member b", "member c"]);
        // Live keys nest under their parents: no cross-parent collisions.
        assert!(live.contains(&"groups/g0#0/members/a#0/cell".to_string()));
        assert!(live.contains(&"groups/g1#0/members/c#0/cell".to_string()));
    }

    #[test]
    fn row_choices_and_binding_leaf_resolve_per_row() {
        use crate::scene::leaf::{BindingLeaf, ChoiceLeaf};
        let def = ScreenDef {
            title: Binding::Literal("Section".into()),
            pages: vec![PageDef {
                key: "page".into(),
                label: Binding::Literal("Section".into()),
                selected: true,
                body: ContainerDef {
                    key: "body".into(),
                    label: Binding::Literal(String::new()),
                    hints: LayoutHints::default(),
                    header: None,
                    policy: PolicyDef::Column,
                    children: vec![NodeDef::Control(Box::new(ControlDef {
                        key: "rows".into(),
                        label: Binding::Literal(String::new()),
                        accessible: None,
                        help: None,
                        display: ControlDisplay::Standard,
                        enabled: BoolBinding::Literal(true),
                        visible: BoolBinding::Literal(true),
                        hints: LayoutHints::default(),
                        leaf: LeafDef::List {
                            columns: Vec::new(),
                            rows: RowsDef::Bound {
                                source: "rows".into(),
                                template: RowTemplateDef {
                                    id_field: "id".into(),
                                    cells: vec![
                                        ControlDef {
                                            key: "choice".into(),
                                            label: Binding::Field("label".into()),
                                            accessible: None,
                                            help: None,
                                            display: ControlDisplay::Standard,
                                            enabled: BoolBinding::Literal(true),
                                            visible: BoolBinding::Literal(true),
                                            hints: LayoutHints::default(),
                                            leaf: LeafDef::Choice {
                                                choices: ChoicesDef::Row {
                                                    labels: "options".into(),
                                                    selected: "selected".into(),
                                                },
                                                selected: SelectedDef::Index(0),
                                            },
                                        },
                                        ControlDef {
                                            key: "binding".into(),
                                            label: Binding::Field("label".into()),
                                            accessible: None,
                                            help: Some(Binding::Literal("tip".into())),
                                            display: ControlDisplay::Standard,
                                            enabled: BoolBinding::Literal(true),
                                            visible: BoolBinding::Literal(true),
                                            hints: LayoutHints::default(),
                                            leaf: LeafDef::Binding {
                                                text: Binding::Field("label".into()),
                                                capturing: BoolBinding::Field("capturing".into()),
                                                change_label: Binding::Literal("Change".into()),
                                                clear_label: Binding::Literal("Clear".into()),
                                            },
                                        },
                                    ],
                                },
                            },
                        },
                    }))],
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
        };
        struct RowResolver;
        impl Texts for RowResolver {}
        impl Flags for RowResolver {}
        impl Choices for RowResolver {}
        impl CustomLeaves for RowResolver {}
        impl Lists for RowResolver {
            fn list_len(&self, source: &str) -> usize {
                usize::from(source == "rows")
            }
            fn value(&self, source: &str, index: usize, path: &str) -> Value {
                assert_eq!(source, "rows");
                assert_eq!(index, 0);
                match path {
                    "id" => Value::Text("r0".into()),
                    "label" => Value::Text("Row".into()),
                    "options" => Value::List(vec!["a".into(), "b".into()]),
                    "selected" => Value::Num(1),
                    "capturing" => Value::Bool(true),
                    _ => Value::Text(String::new()),
                }
            }
        }
        let mut ctx: InflateCtx<String> = InflateCtx::default();
        let key = |s: &str| s.to_string();
        let (screen, live) = inflate(&def, &mut ctx, key, &RowResolver);
        ctx.retain_rows(&live);
        struct Found {
            choices: Vec<String>,
            selected: Option<usize>,
            capturing: bool,
            help: Option<String>,
        }
        impl ControlVisitor for Found {
            fn control(&mut self, control: &Control) {
                if let Some(choice) = control.downcast::<ChoiceLeaf>() {
                    self.choices = choice.choices.clone();
                    self.selected = choice.selected;
                }
                if let Some(binding) = control.downcast::<BindingLeaf>() {
                    self.capturing = binding.capturing;
                }
                if control.downcast::<BindingLeaf>().is_some() {
                    self.help = control.metadata.help.clone();
                }
                crate::scene::control::walk_control(control, self);
            }
        }
        let mut found = Found {
            choices: Vec::new(),
            selected: None,
            capturing: false,
            help: None,
        };
        screen.visit(&mut found);
        assert_eq!(found.choices, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(found.selected, Some(1));
        assert!(found.capturing);
        assert_eq!(found.help.as_deref(), Some("tip"));
    }

    #[test]
    fn bound_tabs_project_select_and_body_per_item() {
        use crate::scene::leaf::ActionLeaf;
        let def = ScreenDef {
            title: Binding::Literal("Section".into()),
            pages: vec![PageDef {
                key: "page".into(),
                label: Binding::Literal("Section".into()),
                selected: true,
                body: ContainerDef {
                    key: "body".into(),
                    label: Binding::Literal(String::new()),
                    hints: LayoutHints::default(),
                    header: None,
                    policy: PolicyDef::Column,
                    children: vec![NodeDef::Tabs(TabsDef::Bound {
                        key: "tabs".into(),
                        source: "tabs".into(),
                        template: Box::new(TabTemplateDef {
                            id_field: "id".into(),
                            label: Binding::Field("label".into()),
                            body: ContainerDef {
                                key: "pane".into(),
                                label: Binding::Literal(String::new()),
                                hints: LayoutHints::default(),
                                header: None,
                                policy: PolicyDef::Column,
                                children: vec![NodeDef::Control(Box::new(ControlDef {
                                    key: "pin".into(),
                                    label: Binding::Field("label".into()),
                                    accessible: None,
                                    help: None,
                                    display: ControlDisplay::Standard,
                                    enabled: BoolBinding::Literal(true),
                                    visible: BoolBinding::Literal(true),
                                    hints: LayoutHints::default(),
                                    leaf: LeafDef::Action,
                                }))],
                            },
                        }),
                        selected: SelectedDef::Bound("tabs".into()),
                    })],
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
        };
        struct TabsFake;
        impl Texts for TabsFake {}
        impl Flags for TabsFake {}
        impl CustomLeaves for TabsFake {}
        impl Lists for TabsFake {
            fn list_len(&self, source: &str) -> usize {
                usize::from(source == "tabs") * 2
            }
            fn value(&self, source: &str, index: usize, path: &str) -> Value {
                assert_eq!(source, "tabs");
                match (index, path) {
                    (0, "id") => Value::Text("alpha".into()),
                    (0, "label") => Value::Text("Alpha".into()),
                    (1, "id") => Value::Text("beta".into()),
                    (1, "label") => Value::Text("Beta".into()),
                    _ => Value::Text(String::new()),
                }
            }
        }
        impl Choices for TabsFake {
            fn selected(&self, source: &str) -> Option<usize> {
                assert_eq!(source, "tabs");
                Some(1)
            }
        }
        let mut ctx: InflateCtx<String> = InflateCtx::default();
        let key = |s: &str| s.to_string();
        let (screen, live) = inflate(&def, &mut ctx, key, &TabsFake);
        ctx.retain_rows(&live);
        // Reaches through the tabs container into both tab bodies.
        struct Pins(Vec<String>);
        impl ControlVisitor for Pins {
            fn control(&mut self, control: &Control) {
                if control.downcast::<ActionLeaf>().is_some() {
                    self.0.push(control.label.clone());
                }
                crate::scene::control::walk_control(control, self);
            }
        }
        let mut pins = Pins(Vec::new());
        screen.visit(&mut pins);
        assert!(pins.0.contains(&"Alpha".to_string()));
        assert!(pins.0.contains(&"Beta".to_string()));
        assert!(live.contains(&"tabs/alpha#0/pin".to_string()));
        assert!(live.contains(&"tabs/beta#0/select".to_string()));
    }

    #[test]
    fn image_leaf_resolves_bytes_and_degrades_empty() {
        use crate::scene::leaf::ImageLeaf;
        let mut def = screen_def();
        // Swap the table's note cell for an image cell reading a fake path.
        let NodeDef::Control(table) = &mut def.pages[0].body.children[1] else {
            panic!("table control");
        };
        let LeafDef::List { rows, .. } = &mut table.leaf else {
            panic!("list leaf");
        };
        let RowsDef::Bound { template, .. } = rows else {
            panic!("bound rows fixture");
        };
        template.cells[0].leaf = LeafDef::Image {
            path: "picture".into(),
        };
        struct Images;
        impl Texts for Images {}
        impl Flags for Images {}
        impl Lists for Images {
            fn list_len(&self, source: &str) -> usize {
                if source == "options" { 2 } else { 0 }
            }
            fn value(&self, source: &str, index: usize, path: &str) -> Value {
                assert_eq!(source, "options");
                match (index, path) {
                    (0, "picture") => Value::Image(ImageData {
                        width: 2,
                        height: 2,
                        rgba: vec![0xff; 16],
                    }),
                    _ => Value::Text(String::new()),
                }
            }
        }
        impl Choices for Images {}
        impl CustomLeaves for Images {}
        let mut ctx: InflateCtx<String> = InflateCtx::default();
        let key = |s: &str| s.to_string();
        let (screen, live) = inflate(&def, &mut ctx, key, &Images);
        ctx.retain_rows(&live);
        // Image cells hide inside the list: walk explicitly like flavors do.
        struct Walk(Vec<(u32, u32)>);
        impl ControlVisitor for Walk {
            fn control(&mut self, control: &Control) {
                if let Some(image) = control.downcast::<ImageLeaf>() {
                    self.0.push((image.width, image.height));
                }
                crate::scene::control::walk_control(control, self);
            }
        }
        let mut walk = Walk(Vec::new());
        screen.visit(&mut walk);
        assert_eq!(walk.0, vec![(2, 2), (0, 0)]);
    }
}
