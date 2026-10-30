//! UI-local, closure-based MVVM. No application input/message contract is needed.
//!
//! Subscriptions synchronously deliver an initial value. Retain their RAII token
//! and capture native widgets weakly. Bindings retain frozen snapshots, not the
//! Store owner. User callbacks, destructors and future polls run outside runtime
//! RefCell borrows. Executors may poll immediately inside `spawn`.
mod blocking;
mod session;
mod store;

pub use blocking::{BlockingPool, TaskError};
pub use session::{
    Session, ShutdownCoordinator, ShutdownOutcome, ShutdownPhase, UiHost, WindowHooks, run_async,
};
pub use store::{Binding, Command, Executor, Limits, Policy, Store, Subscription, validation};

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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub start: usize,
    pub count: usize,
    pub row_height: f32,
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
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Layout {
    Stack { horizontal: bool, gap: f32 },
    Grid { columns: u32, gap: f32 },
    Flex { gap: f32 },
    Absolute,
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
pub struct MenuItem {
    pub id: String,
    pub label: String,
    pub argument: String,
    pub shortcut: Option<String>,
    pub role: MenuRole,
    pub enabled: bool,
}

#[cfg(test)]
mod tests;
