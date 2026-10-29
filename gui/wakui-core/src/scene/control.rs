//! Domain-free, owned control scenes. Render snapshots outside all owner borrows;
//! callbacks carry opaque IDs and are resolved against the latest scene.
//!
//! Layout is data-driven: `Container` carries a boxed `LayoutPolicy`
//! (`Row`/`Column`/`Group`/`Table`/`Flex`/`Absolute`). New arrangements are
//! added as `impl LayoutPolicy` outside this file; the `Node` enum is fixed
//! to `Control`/`Container`/`Tabs`. `Group` is a header-bearing container,
//! `Tabs` a selection-bearing container.

mod graph;
mod id;
mod policy;

pub use graph::{Capture, Container, ControlScreen, Node, Page, Tab, Tabs, policy};
pub use id::{ControlId, Id, IdRegistry, LayoutId, PageId, TabId, TabsId};
pub use policy::{
    AbsolutePolicy, ColumnPolicy, FlexPolicy, GroupPolicy, LayoutPolicy, RowPolicy, TablePolicy,
};

use crate::input::key::Key;
use crate::scene::hints::LayoutHints;
use crate::scene::leaf::{ControlLeaf, LeafCtx};

#[derive(Debug, Clone, PartialEq)]
pub struct ControlMetadata {
    pub enabled: bool,
    pub visible: bool,
    pub accessible_label: String,
    pub help: Option<String>,
    pub hints: LayoutHints,
}

impl ControlMetadata {
    pub fn new(label: impl Into<String>) -> Self {
        let label = label.into();
        Self {
            accessible_label: label,
            enabled: true,
            visible: true,
            help: None,
            hints: LayoutHints::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlDisplay {
    Standard,
    Primary,
    Secondary,
    Note,
    Heading,
}

#[derive(Debug)]
pub struct Control {
    pub id: ControlId,
    pub label: String,
    pub metadata: ControlMetadata,
    pub display: ControlDisplay,
    pub leaf: Box<dyn ControlLeaf>,
}

impl Clone for Control {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            label: self.label.clone(),
            metadata: self.metadata.clone(),
            display: self.display,
            leaf: self.leaf.clone_box(),
        }
    }
}

impl PartialEq for Control {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.label == other.label
            && self.metadata == other.metadata
            && self.display == other.display
            && self
                .leaf
                .equals(other.leaf.as_ref() as &dyn crate::scene::leaf::Leaf)
    }
}

impl Control {
    /// Structural validation only. Binding and effects remain in the host owner.
    pub fn accepts(&self, input: &ControlInput) -> bool {
        if !self.metadata.enabled || !self.metadata.visible {
            return false;
        }
        self.leaf.accepts(input)
    }

    /// Downcast the leaf to a concrete framework-provided or external type.
    /// Dispatch via `TypeId`, never via debug names or view-definition types.
    pub fn downcast<T: 'static>(&self) -> Option<&T> {
        self.leaf.as_any().downcast_ref::<T>()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlInput {
    Bool(bool),
    Select(usize),
    Number(u32),
    Text(String),
    Activate,
    CaptureClear,
    CaptureKey(Key),
    /// Pointer position over an image leaf, in image pixels.
    /// Integer to keep `Eq`; natives floor their float coordinates.
    Hover {
        x: u32,
        y: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlEvent {
    /// None routes a keyboard capture to the current screen's capture target.
    pub id: Option<ControlId>,
    pub input: ControlInput,
}

impl ControlEvent {
    pub fn new(id: ControlId, input: ControlInput) -> Self {
        Self {
            id: Some(id),
            input,
        }
    }
    pub fn capture_key(key: Key) -> Self {
        Self {
            id: None,
            input: ControlInput::CaptureKey(key),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlEffect {
    /// Ask the native side to open a folder picker. The result comes back
    /// through [`crate::app::binding::PickerSink`], never through [`ControlInput`].
    PickDirectory {
        title: String,
        target: ControlId,
        request: Id,
    },
}

/// Explicit child exposure for container-like leaves. Flavors call this
/// once after rendering the host control; base leaves no-op. Each control
/// is visited exactly once: the framework never walks implicitly, so this
/// helper is the single place where nested controls surface.
pub fn walk_control(control: &Control, visitor: &mut dyn ControlVisitor) {
    let ctx = LeafCtx {
        id: control.id,
        hints: control.metadata.hints,
    };
    control.leaf.walk(&ctx, visitor);
}

pub trait ControlVisitor {
    fn begin_screen(&mut self, _screen: &ControlScreen) {}
    fn end_screen(&mut self, _screen: &ControlScreen) {}
    fn begin_page(&mut self, _page: &Page) {}
    fn end_page(&mut self, _page: &Page) {}
    fn begin_container(&mut self, _container: &Container) {}
    fn end_container(&mut self, _container: &Container) {}
    fn begin_tabs(&mut self, _tabs: &Tabs) {}
    fn end_tabs(&mut self, _tabs: &Tabs) {}
    fn begin_tab(&mut self, _tab: &Tab) {}
    fn end_tab(&mut self, _tab: &Tab) {}
    fn control(&mut self, _control: &Control) {}
}
