//! Domain-free, owned control scenes. Render snapshots outside all owner borrows;
//! callbacks carry opaque IDs and are resolved against the latest scene.
//!
//! Layout is data-driven: `Container` carries a boxed `LayoutPolicy`
//! (`Row`/`Column`/`Group`/`Table`/`Flex`/`Absolute`). New arrangements are
//! added as `impl LayoutPolicy` outside this file; the `Node` enum is fixed
//! to `Control`/`Container`/`Tabs`. `Group` is a header-bearing container,
//! `Tabs` a selection-bearing container.

use std::any::Any;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::hints::LayoutHints;
use crate::key::Key;
use crate::leaf::{ControlLeaf, LeafCtx};

/// Process-unique opaque token. Neither position nor a hash of a domain key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Id(u64);
impl Id {
    /// Mint a fresh process-unique token. Always safe: issued IDs are never
    /// recycled. Prefer [`IdRegistry`] so surviving keys keep stable IDs.
    pub fn fresh() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let mut value = NEXT.load(Ordering::Relaxed);
        loop {
            let next = value.checked_add(1).expect("control identity exhausted");
            match NEXT.compare_exchange_weak(value, next, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => return Self(value),
                Err(current) => value = current,
            }
        }
    }
}
pub type ControlId = Id;
pub type LayoutId = Id;
pub type PageId = Id;
pub type TabsId = Id;
pub type TabId = Id;

/// Controller-owned equality registry. Issued IDs are never reassigned or recycled.
/// Keys keep their IDs while retained; a removed key gets a fresh ID if it returns.
/// The caller supplies semantic keys, not indices in a changing collection.
#[derive(Debug)]
pub struct IdRegistry<K>(Vec<(K, Id)>);

impl<K> Default for IdRegistry<K> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<K: Eq> IdRegistry<K> {
    /// Drop obsolete keys without changing surviving IDs or recycling issued IDs.
    /// Do not remove keys whose identity must survive temporarily hidden topology.
    pub fn retain(&mut self, mut keep: impl FnMut(&K, Id) -> bool) {
        self.0.retain(|(key, id)| keep(key, *id));
    }

    pub fn id(&mut self, key: K) -> Id {
        if let Some((_, id)) = self.0.iter().find(|(existing, _)| existing == &key) {
            return *id;
        }
        let id = Id::fresh();
        self.0.push((key, id));
        id
    }
}

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
                .equals(other.leaf.as_ref() as &dyn crate::leaf::Leaf)
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
    /// through [`crate::binding::PickerSink`], never through [`ControlInput`].
    PickDirectory {
        title: String,
        target: ControlId,
        request: Id,
    },
}

/// Open arrangement vocabulary. New arrangements implement this trait;
/// containers never grow new enum variants for them.
pub trait LayoutPolicy: std::fmt::Debug + Send + 'static {
    fn debug_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
    fn clone_box(&self) -> Box<dyn LayoutPolicy>;
    fn equals(&self, other: &dyn LayoutPolicy) -> bool;
    fn as_any(&self) -> &dyn Any;
}

macro_rules! unit_policy {
    ($t:ty) => {
        impl LayoutPolicy for $t {
            fn clone_box(&self) -> Box<dyn LayoutPolicy> {
                Box::new(*self)
            }
            fn equals(&self, other: &dyn LayoutPolicy) -> bool {
                other.as_any().downcast_ref::<Self>().is_some()
            }
            fn as_any(&self) -> &dyn Any {
                self
            }
        }
    };
}

/// Horizontal stack.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RowPolicy;
/// Vertical stack.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ColumnPolicy;
/// Framed group with an optional header control.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GroupPolicy;
/// Grid arrangement.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TablePolicy;
/// Flowing arrangement.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FlexPolicy;
/// Absolute arrangement. The only policy carrying a rectangle, and only
/// for exceptional overlay use; ordinary screens never set it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AbsolutePolicy {
    pub bounds: Option<crate::hints::Rect>,
}

unit_policy!(RowPolicy);
unit_policy!(ColumnPolicy);
unit_policy!(GroupPolicy);
unit_policy!(TablePolicy);
unit_policy!(FlexPolicy);

impl LayoutPolicy for AbsolutePolicy {
    fn clone_box(&self) -> Box<dyn LayoutPolicy> {
        Box::new(*self)
    }
    fn equals(&self, other: &dyn LayoutPolicy) -> bool {
        other.as_any().downcast_ref::<Self>() == Some(self)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Generic container: identity plus a boxed arrangement policy.
/// `Group` is a container with `GroupPolicy` and a header; selection
/// lives in `Tabs`, never in policies.
#[derive(Debug)]
pub struct Container {
    pub id: LayoutId,
    pub label: String,
    pub metadata: ControlMetadata,
    pub header: Option<Control>,
    pub policy: Box<dyn LayoutPolicy>,
    pub children: Vec<Node>,
}

impl Clone for Container {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            label: self.label.clone(),
            metadata: self.metadata.clone(),
            header: self.header.clone(),
            policy: self.policy.clone_box(),
            children: self.children.clone(),
        }
    }
}

impl PartialEq for Container {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.label == other.label
            && self.metadata == other.metadata
            && self.header == other.header
            && self.policy.equals(other.policy.as_ref())
            && self.children == other.children
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Container(Box<Container>),
    Tabs(Tabs),
    Control(Control),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tabs {
    pub id: TabsId,
    pub metadata: ControlMetadata,
    pub selected: Option<usize>,
    pub tabs: Vec<Tab>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    pub id: TabId,
    pub label: String,
    pub selected: bool,
    pub select: Control,
    pub body: Container,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub id: PageId,
    pub label: String,
    pub selected: bool,
    pub select: Control,
    pub body: Container,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Capture {
    pub target: Option<ControlId>,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ControlScreen {
    pub title: String,
    pub pages: Vec<Page>,
    pub footer: Container,
    pub message: Option<String>,
    pub capture: Capture,
}

impl ControlScreen {
    pub fn control(&self, id: ControlId) -> Option<&Control> {
        for page in &self.pages {
            if page.select.id == id {
                return Some(&page.select);
            }
            if let Some(control) = find(&page.body, id) {
                return Some(control);
            }
        }
        find(&self.footer, id)
    }

    pub fn visit(&self, visitor: &mut dyn ControlVisitor) {
        visitor.begin_screen(self);
        for page in &self.pages {
            visitor.begin_page(page);
            visit_container(&page.body, visitor);
            visitor.end_page(page);
        }
        visit_container(&self.footer, visitor);
        visitor.end_screen(self);
    }
}

fn find(container: &Container, id: ControlId) -> Option<&Control> {
    if let Some(control) = &container.header
        && control.id == id
    {
        return Some(control);
    }
    container.children.iter().find_map(|node| match node {
        Node::Control(control) => (control.id == id).then_some(control),
        Node::Container(inner) => find(inner, id),
        Node::Tabs(tabs) => tabs.tabs.iter().find_map(|tab| {
            if tab.select.id == id {
                Some(&tab.select)
            } else {
                find(&tab.body, id)
            }
        }),
    })
}

/// Downcast a container policy. Flavors branch on the static type,
/// never on view-definition types or debug names.
pub fn policy<T: 'static>(container: &Container) -> Option<&T> {
    container.policy.as_any().downcast_ref::<T>()
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

fn visit_container(container: &Container, visitor: &mut dyn ControlVisitor) {
    visitor.begin_container(container);
    if let Some(control) = &container.header {
        visitor.control(control);
    }
    for node in &container.children {
        match node {
            // Container-like leaves expose their children through an
            // explicit `walk` by the flavor (retained grouping) or the
            // immediate builder (flat degrade). The framework never walks
            // implicitly, so each control is visited exactly once.
            Node::Control(control) => visitor.control(control),
            Node::Container(inner) => visit_container(inner, visitor),
            Node::Tabs(tabs) => {
                visitor.begin_tabs(tabs);
                for tab in &tabs.tabs {
                    visitor.begin_tab(tab);
                    visit_container(&tab.body, visitor);
                    visitor.end_tab(tab);
                }
                visitor.end_tabs(tabs);
            }
        }
    }
    visitor.end_container(container);
}
