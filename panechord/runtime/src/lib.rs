//! UI-local, closure-based MVVM. No application input/message contract is needed.
//!
//! Subscriptions synchronously deliver an initial value. Retain their RAII token
//! and capture native widgets weakly. Bindings retain frozen snapshots, not the
//! Store owner. User callbacks, destructors and future polls run outside runtime
//! RefCell borrows. Executors may poll immediately inside `spawn`.
mod key;
mod window;
pub use key::KeyCode;
#[doc(hidden)]
pub mod native_keys;
pub use window::{WindowEvent, WindowHost, WindowSurface};
mod blocking;
mod controls;
mod session;
mod shortcut;
mod store;

pub use blocking::{BlockingPool, TaskError};
pub use controls::{
    Choice, ImageFrame, SliderRange, TabPosition, Table, TableCell, TableColumn, TableRow,
    TableSelection, TextOverflow, validate_choices,
};
pub use session::{
    Session, ShutdownCoordinator, ShutdownFailure, ShutdownOutcome, ShutdownPhase, UiHost,
    WindowHooks, run_async,
};
pub use shortcut::{Shortcut, ShortcutError, ShortcutTarget};
pub use store::{
    Binding, Command, Executor, Limits, Policy, Store, StoreOwner, Subscription, WriteError,
    validation,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerPosition {
    pub x: f32,
    pub y: f32,
}

/// Whether an item key is nonempty and contains no hierarchical scope separator (`/`).
/// Backends must also validate snapshot keys because public fields bypass `Keyed::new`.
pub fn valid_item_key(key: &str) -> bool {
    !key.is_empty() && !key.contains('/')
}

/// A persistent item model, independent of the lifetime of mounted row widgets.
#[derive(Clone)]
pub struct Keyed<S: Clone + 'static> {
    pub key: String,
    pub model: Store<S>,
}

impl<S: Clone + 'static> Keyed<S> {
    /// # Panics
    /// Panics if `key` is empty or contains `/`.
    pub fn new(key: impl Into<String>, model: Store<S>) -> Self {
        let key = key.into();
        assert!(
            valid_item_key(&key),
            "item key must be nonempty and contain no '/'"
        );
        Self { key, model }
    }
}

impl<S: Clone + 'static> PartialEq for Keyed<S> {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.model.ptr_eq(&other.model)
    }
}

#[doc(hidden)]
pub fn validate_repeat_items<T: Clone + 'static>(items: &[Keyed<T>]) -> Result<(), &'static str> {
    let mut keys = std::collections::HashSet::new();
    if items.len() > 256
        || items.iter().any(|item| {
            !valid_item_key(&item.key) || item.key.len() > 256 || !keys.insert(item.key.as_str())
        })
    {
        return Err("invalid Repeat keys or item limit");
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub struct TabPageChanges {
    pub removed: Vec<usize>,
    pub added: Vec<usize>,
}

pub fn tab_page_selection<S: Clone + 'static>(selected: &str, pages: &[Keyed<S>]) -> String {
    if pages.iter().any(|page| page.key == selected) {
        selected.to_owned()
    } else {
        pages
            .first()
            .map(|page| page.key.clone())
            .unwrap_or_default()
    }
}

#[doc(hidden)]
pub struct ReconcileGuard<'a>(&'a std::cell::Cell<bool>);

impl<'a> ReconcileGuard<'a> {
    pub fn enter(busy: &'a std::cell::Cell<bool>) -> Option<Self> {
        if busy.replace(true) {
            None
        } else {
            Some(Self(busy))
        }
    }
}

impl Drop for ReconcileGuard<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

pub fn tab_page_changes<S: Clone + 'static>(
    previous: &[Keyed<S>],
    next: &[Keyed<S>],
) -> Result<TabPageChanges, &'static str> {
    use std::collections::HashMap;
    if next.len() > 256 {
        return Err("Tab allows at most 256 pages");
    }
    let mut next_by_key = HashMap::with_capacity(next.len());
    for item in next {
        if !valid_item_key(&item.key) {
            return Err("invalid Tab page key");
        }
        if next_by_key.insert(item.key.as_str(), item).is_some() {
            return Err("duplicate Tab page key");
        }
    }
    let previous_by_key: HashMap<_, _> = previous
        .iter()
        .map(|item| (item.key.as_str(), item))
        .collect();
    Ok(TabPageChanges {
        removed: previous
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                (next_by_key.get(item.key.as_str()).copied() != Some(item)).then_some(index)
            })
            .collect(),
        added: next
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                (previous_by_key.get(item.key.as_str()).copied() != Some(item)).then_some(index)
            })
            .collect(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub start: usize,
    pub count: usize,
    pub row_height: f32,
}
impl Viewport {
    pub fn validate(self) -> Result<Self, &'static str> {
        if !(1..=256).contains(&self.count)
            || !self.row_height.is_finite()
            || !(1.0..=4096.0).contains(&self.row_height)
        {
            return Err(
                "invalid viewport: count must be 1..=256 and row_height finite and 1..=4096",
            );
        }
        Ok(self)
    }

    pub fn range(self, length: usize) -> Result<std::ops::Range<usize>, &'static str> {
        self.validate()?;
        let start = self.start.min(length.saturating_sub(self.count));
        Ok(start..start.saturating_add(self.count).min(length))
    }
}
impl Default for Viewport {
    fn default() -> Self {
        Self {
            start: 0,
            count: 20,
            row_height: 48.0,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Edges {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub row: i32,
    pub column: i32,
    pub row_span: i32,
    pub column_span: i32,
    pub x: f32,
    pub y: f32,
    pub width: Option<f32>,
    pub height: Option<f32>,
}
impl Default for Placement {
    fn default() -> Self {
        Self {
            row: 0,
            column: 0,
            row_span: 1,
            column_span: 1,
            x: 0.0,
            y: 0.0,
            width: None,
            height: None,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Style {
    pub margin: Edges,
    pub padding: Edges,
    pub placement: Placement,
    pub horizontal_alignment: Option<Alignment>,
    pub vertical_alignment: Option<Alignment>,
    pub text: Option<TextStyle>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum TextRole {
    #[default]
    Body,
    Heading,
    Code,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    pub role: TextRole,
    pub scale: f32,
    pub foreground: Option<[u8; 3]>,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            role: TextRole::Body,
            scale: 1.0,
            foreground: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    Start,
    Center,
    End,
    Stretch,
}

impl Alignment {
    pub fn offset(self, available: f32, extent: f32) -> f32 {
        if !available.is_finite() || !extent.is_finite() {
            return 0.0;
        }
        let surplus = (available.max(0.0) - extent.max(0.0)).max(0.0);
        match self {
            Self::Start | Self::Stretch => 0.0,
            Self::Center => surplus / 2.0,
            Self::End => surplus,
        }
    }
}

impl Placement {
    pub fn grid_position(self, index: usize, columns: u32) -> (i32, i32) {
        if self.row == 0 && self.column == 0 && self.row_span == 1 && self.column_span == 1 {
            let columns = columns.max(1) as usize;
            ((index / columns) as i32, (index % columns) as i32)
        } else {
            (self.row, self.column)
        }
    }
}

impl Style {
    pub fn validate(self, parent: Layout) -> Result<(), &'static str> {
        parent.validate()?;
        if self
            .text
            .is_some_and(|text| !text.scale.is_finite() || text.scale <= 0.0 || text.scale > 16.0)
        {
            return Err("text scale must be finite and within (0, 16]");
        }
        for edges in [self.margin, self.padding] {
            if [edges.top, edges.right, edges.bottom, edges.left]
                .iter()
                .any(|value| !value.is_finite() || *value < 0.0)
            {
                return Err("spacing must be finite and nonnegative");
            }
            if [edges.top, edges.right, edges.bottom, edges.left]
                .into_iter()
                .any(|value| value > MAX_LAYOUT_SIZE)
            {
                return Err("spacing exceeds native layout limit");
            }
        }
        let placement = self.placement;
        if [placement.width, placement.height]
            .into_iter()
            .flatten()
            .any(|value| !value.is_finite() || value < 0.0)
        {
            return Err("size must be finite and nonnegative");
        }
        if [placement.width, placement.height]
            .into_iter()
            .flatten()
            .any(|value| value > MAX_LAYOUT_SIZE)
            || [
                (self.margin.top, self.padding.top),
                (self.margin.right, self.padding.right),
                (self.margin.bottom, self.padding.bottom),
                (self.margin.left, self.padding.left),
            ]
            .into_iter()
            .any(|(margin, padding)| margin + padding > MAX_LAYOUT_SIZE)
        {
            return Err("size or combined inset exceeds native layout limit");
        }
        if !placement.x.is_finite() || !placement.y.is_finite() {
            return Err("coordinates must be finite");
        }
        if placement.x.abs() > MAX_LAYOUT_SIZE || placement.y.abs() > MAX_LAYOUT_SIZE {
            return Err("coordinates exceed native layout limit");
        }
        if matches!(parent, Layout::Grid { .. }) {
            if placement.row < 0
                || placement.column < 0
                || placement.row_span < 1
                || placement.column_span < 1
                || placement
                    .row
                    .checked_add(placement.row_span)
                    .is_none_or(|end| end > 4096)
                || placement
                    .column
                    .checked_add(placement.column_span)
                    .is_none_or(|end| end > 4096)
            {
                return Err("invalid Grid placement");
            }
        } else if placement.row != 0
            || placement.column != 0
            || placement.row_span != 1
            || placement.column_span != 1
        {
            return Err("grid placement requires Grid");
        }
        if !matches!(parent, Layout::Absolute) && (placement.x != 0.0 || placement.y != 0.0) {
            return Err("x/y placement requires Absolute");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Layout {
    Stack { horizontal: bool, gap: f32 },
    Grid { columns: u32, gap: f32 },
    Flex { gap: f32 },
    Absolute,
}

pub const MAX_LAYOUT_SIZE: f32 = 1_000_000.0;
pub const MAX_GRID_COLUMNS: u32 = 4096;

impl Layout {
    pub fn validate(self) -> Result<Self, &'static str> {
        let gap = match self {
            Self::Grid { columns, gap } => {
                if !(1..=MAX_GRID_COLUMNS).contains(&columns) {
                    return Err("Grid columns must be 1..=4096");
                }
                gap
            }
            Self::Stack { gap, .. } | Self::Flex { gap } => gap,
            Self::Absolute => 0.0,
        };
        if !gap.is_finite() || !(0.0..=MAX_LAYOUT_SIZE).contains(&gap) {
            return Err("layout gap must be finite and within native layout limit");
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MenuRole {
    Normal,
    Copy,
    Cut,
    Paste,
    SelectAll,
    About,
    Quit,
    Separator,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem<A = String> {
    pub id: String,
    pub label: String,
    pub argument: A,
    pub shortcut: Option<String>,
    pub role: MenuRole,
    pub enabled: bool,
}

#[diagnostic::on_unimplemented(
    message = "command argument is required; add an argument property to this Button"
)]
pub trait OmittedCommandArgument: Clone + 'static {
    fn omitted() -> Self;
}
impl OmittedCommandArgument for String {
    fn omitted() -> Self {
        String::new()
    }
}
pub fn omitted_command_argument<T: OmittedCommandArgument>() -> Binding<T> {
    Binding::constant(T::omitted())
}

#[derive(Debug, Clone, PartialEq)]
pub enum MenuNode<A = String> {
    Item(MenuItem<A>),
    Submenu {
        id: String,
        label: String,
        enabled: bool,
        children: Vec<MenuNode<A>>,
    },
}

pub fn flatten_menu<A: Clone>(nodes: &[MenuNode<A>]) -> Result<Vec<MenuItem<A>>, String> {
    fn walk<A: Clone>(
        nodes: &[MenuNode<A>],
        enabled: bool,
        depth: usize,
        ids: &mut std::collections::HashSet<String>,
        result: &mut Vec<MenuItem<A>>,
    ) -> Result<(), String> {
        if depth > 64 {
            return Err("menu depth limit exceeded".into());
        }
        for node in nodes {
            let id = match node {
                MenuNode::Item(item) => &item.id,
                MenuNode::Submenu { id, .. } => id,
            };
            if !valid_item_key(id) || !ids.insert(id.clone()) || ids.len() > 4096 {
                return Err("invalid/duplicate menu id or menu node limit".into());
            }
            match node {
                MenuNode::Item(item) => {
                    let mut item = item.clone();
                    item.enabled &= enabled;
                    result.push(item);
                }
                MenuNode::Submenu {
                    enabled: local,
                    children,
                    ..
                } => walk(children, enabled && *local, depth + 1, ids, result)?,
            }
        }
        Ok(())
    }
    let mut items = Vec::new();
    walk(
        nodes,
        true,
        0,
        &mut std::collections::HashSet::new(),
        &mut items,
    )?;
    Ok(items)
}

pub fn checked_menu<A: Clone + PartialEq + 'static>(
    source: Binding<Vec<MenuNode<A>>>,
    validate: impl Fn(&[MenuItem<A>]) -> Result<(), String> + 'static,
    report: impl Fn(String) + 'static,
) -> Binding<Vec<MenuNode<A>>> {
    let accepted = std::cell::RefCell::new(Vec::new());
    source
        .map(move |nodes| {
            match flatten_menu(&nodes).and_then(|items| validate(&items)) {
                Ok(()) => {
                    accepted.replace(nodes);
                }
                Err(error) => report(error),
            }
            accepted.borrow().clone()
        })
        .distinct()
}

#[cfg(test)]
mod tests;
