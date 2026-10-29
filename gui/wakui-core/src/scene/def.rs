//! L1 view definitions: domain-free declarative descriptions of screens.
//!
//! L1 is the template layer: structure plus leaf kinds plus [`Binding`]
//! references. It carries no row instances, no issued IDs, and no resolved
//! strings. [`crate::scene::inflate`] expands an L1 [`ScreenDef`] plus an
//! outer [`crate::scene::inflate::Resolver`] into an L2 [`crate::scene::control::ControlScreen`].
//! Serializing L2 snapshots is out of scope; serde derives live here only,
//! on concrete types (no trait objects, no typetag needed).

use crate::scene::control::ControlDisplay;
use crate::scene::hints::LayoutHints;

/// Unresolved string reference. Resolution is always outer-owned.
/// `Localized` reads the caption table, `Dynamic` reads live screen values
/// (input echoes, computed labels); both resolve through
/// [`crate::scene::inflate::Texts`].
/// `Field` reads one row item field inside a list template.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Binding {
    Literal(String),
    Localized(String),
    Dynamic(String),
    Field(String),
}

/// Unresolved flag reference. `Literal` passes through, `Flag` reads
/// outer boolean state through [`crate::scene::inflate::Flags`], and `Field`
/// reads a per-item boolean inside a list template (screen-level use
/// degrades to false, mirroring `Binding::Field`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum BoolBinding {
    Literal(bool),
    Flag(String),
    Field(String),
}

/// Choice list content: fixed literals or an outer-owned source.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ChoicesDef {
    Static(Vec<Binding>),
    Bound(String),
}

/// Choice selection: fixed index or an outer-owned source.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SelectedDef {
    Index(usize),
    Bound(String),
}

/// Numeric value: fixed literal or an outer-owned row field.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum NumBinding {
    Literal(u32),
    Bound { source: String, path: String },
}

/// Generic raster image bound to outer picture data. `path` resolves through
/// [`crate::scene::inflate::Lists`] to a [`crate::scene::inflate::Value::Image`];
/// anything else degrades to an empty image, mirroring `Field` silence.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ImageDef {
    pub path: String,
}

/// L1 column header: the id is stable, the header text resolves per frame
/// so definitions stay language-independent.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ListColumnDef {
    pub id: String,
    pub header: Binding,
    pub width: Option<f64>,
}
/// Framework-owned leaf vocabulary for L1. Domain-specific leaves use
/// [`LeafDef::Custom`]; the closed base set never grows an enum for them.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LeafDef {
    Note {
        text: Binding,
    },
    Action,
    Toggle {
        value: BoolBinding,
    },
    Choice {
        choices: ChoicesDef,
        selected: SelectedDef,
    },
    Text {
        value: Binding,
        placeholder: Binding,
    },
    Number {
        value: NumBinding,
        min: u32,
        max: u32,
    },
    Image {
        path: String,
    },
    List {
        columns: Vec<ListColumnDef>,
        rows: RowsDef,
    },
    /// External `impl Leaf` entry point. `kind` is the outer registration
    /// name; `fields` carry resolved values. Unknown kinds degrade to a
    /// note, preserving order and visibility.
    Custom {
        kind: String,
        fields: Vec<(String, Binding)>,
    },
}

/// Row content: bound lists only. Static rows belong in `children`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum RowsDef {
    Bound {
        source: String,
        template: RowTemplateDef,
    },
}

/// Per-row projection. `id_field` names the item field used for stable
/// identity; equal duplicates get occurrence-suffixed keys.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RowTemplateDef {
    pub id_field: String,
    pub cells: Vec<ControlDef>,
}

/// One declarative control. `key` is a stable string; the outer key mapper
/// turns it into the registry key. No issued IDs travel here.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ControlDef {
    pub key: String,
    pub label: Binding,
    pub display: ControlDisplay,
    pub enabled: BoolBinding,
    pub visible: BoolBinding,
    pub hints: LayoutHints,
    pub leaf: LeafDef,
}

/// Framework-owned arrangement vocabulary for L1. `Custom` degrades to
/// column placement; visibility, order, and enablement are preserved.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyDef {
    Row,
    Column,
    Group,
    Table,
    Flex,
    Absolute,
    Custom { kind: String },
}

/// One declarative container.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ContainerDef {
    pub key: String,
    pub label: Binding,
    pub hints: LayoutHints,
    pub header: Option<ControlDef>,
    pub policy: PolicyDef,
    pub children: Vec<NodeDef>,
}

/// One declarative tab. Selection is a static index; runtime tab switching
/// state stays in L2 (`Tabs.selected`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TabDef {
    pub key: String,
    pub label: Binding,
    pub body: ContainerDef,
}

/// Declarative tab set with a static selection index.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TabsDef {
    pub key: String,
    pub tabs: Vec<TabDef>,
    pub selected: usize,
}

/// One declarative page. `select` (the L2 tab-stop control) is derived from
/// `key` at expansion time, so L1 carries no second key for it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PageDef {
    pub key: String,
    pub label: Binding,
    pub selected: bool,
    pub body: ContainerDef,
}

/// Declarative node: the L1 analog of `Node`, without IDs.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum NodeDef {
    Container(Box<ContainerDef>),
    Tabs(TabsDef),
    Control(Box<ControlDef>),
}

/// Declarative screen: the L1 analog of `ControlScreen`.
/// Carries no `capture` and no `message`; expansion fills empty defaults
/// so runtime state never leaks into view definitions.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScreenDef {
    pub title: Binding,
    pub pages: Vec<PageDef>,
    pub footer: ContainerDef,
}
