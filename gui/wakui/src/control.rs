//! Domain-free, owned control scenes. Render snapshots outside all owner borrows;
//! callbacks carry opaque IDs and are resolved against the latest scene.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::key::Key;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlMetadata {
    pub enabled: bool,
    pub visible: bool,
    pub accessible_label: String,
    pub help: Option<String>,
}

impl ControlMetadata {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            enabled: true,
            visible: true,
            accessible_label: label.into(),
            help: None,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlKind {
    Toggle {
        value: bool,
    },
    Choice {
        choices: Vec<String>,
        selected: Option<usize>,
    },
    Number {
        value: u32,
        min: u32,
        max: u32,
        step: u32,
        unit: String,
        formatted: String,
    },
    Text {
        value: String,
        placeholder: String,
        browse_label: Option<String>,
    },
    Binding {
        text: String,
        capturing: bool,
        change_label: String,
        clear_label: String,
    },
    Action,
    Note {
        text: String,
    },
    /// Generic raster image (RGBA8, row-major). View definitions supply
    /// the bytes; backends render verbatim and report pointer positions
    /// as [`ControlInput::Hover`]. Any view may show images.
    Image {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Control {
    pub id: ControlId,
    pub label: String,
    pub metadata: ControlMetadata,
    pub display: ControlDisplay,
    pub kind: ControlKind,
}

impl Control {
    /// Structural validation only. Binding and effects remain in the host owner.
    pub fn accepts(&self, input: &ControlInput) -> bool {
        if !self.metadata.enabled || !self.metadata.visible {
            return false;
        }
        match (&self.kind, input) {
            (ControlKind::Toggle { .. }, ControlInput::Bool(_)) => true,
            (ControlKind::Choice { choices, .. }, ControlInput::Select(index)) => {
                *index < choices.len()
            }
            (ControlKind::Number { min, max, .. }, ControlInput::Number(n)) => {
                (*min..=*max).contains(n)
            }
            (ControlKind::Text { .. }, ControlInput::Text(_)) => true,
            (
                ControlKind::Text {
                    browse_label: Some(_),
                    ..
                },
                ControlInput::Activate,
            ) => true,
            (ControlKind::Action | ControlKind::Binding { .. }, ControlInput::Activate) => true,
            (ControlKind::Binding { .. }, ControlInput::CaptureClear) => true,
            (
                ControlKind::Binding {
                    capturing: true, ..
                },
                ControlInput::CaptureKey(_),
            ) => true,
            (ControlKind::Image { .. }, ControlInput::Hover { .. }) => true,
            _ => false,
        }
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
    /// Pointer position over an [`ControlKind::Image`], in image pixels.
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub id: LayoutId,
    pub label: String,
    pub metadata: ControlMetadata,
    pub spacing: u16,
    pub indent: u16,
    pub header: Option<Control>,
    pub children: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Row(Layout),
    Column(Layout),
    Group(Layout),
    Table(Layout),
    Tabs(Tabs),
    Control(Control),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tabs {
    pub id: TabsId,
    pub metadata: ControlMetadata,
    pub selected: Option<usize>,
    pub tabs: Vec<Tab>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    pub id: TabId,
    pub label: String,
    pub selected: bool,
    pub select: Control,
    pub body: Layout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub id: PageId,
    pub label: String,
    pub selected: bool,
    pub select: Control,
    pub body: Layout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capture {
    pub target: Option<ControlId>,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlScreen {
    pub title: String,
    pub pages: Vec<Page>,
    pub footer: Layout,
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
            visit_layout(&page.body, LayoutKind::Column, visitor);
            visitor.end_page(page);
        }
        visit_layout(&self.footer, LayoutKind::Row, visitor);
        visitor.end_screen(self);
    }
}

fn find(layout: &Layout, id: ControlId) -> Option<&Control> {
    if let Some(control) = &layout.header
        && control.id == id
    {
        return Some(control);
    }
    layout.children.iter().find_map(|node| match node {
        Node::Control(control) => (control.id == id).then_some(control),
        Node::Row(l) | Node::Column(l) | Node::Group(l) | Node::Table(l) => find(l, id),
        Node::Tabs(tabs) => tabs.tabs.iter().find_map(|tab| {
            if tab.select.id == id {
                Some(&tab.select)
            } else {
                find(&tab.body, id)
            }
        }),
    })
}

#[derive(Debug, Clone, Copy)]
pub enum LayoutKind {
    Row,
    Column,
    Group,
    Table,
}

pub trait ControlVisitor {
    fn begin_screen(&mut self, _screen: &ControlScreen) {}
    fn end_screen(&mut self, _screen: &ControlScreen) {}
    fn begin_page(&mut self, _page: &Page) {}
    fn end_page(&mut self, _page: &Page) {}
    fn begin_layout(&mut self, _layout: &Layout, _kind: LayoutKind) {}
    fn end_layout(&mut self, _layout: &Layout, _kind: LayoutKind) {}
    fn begin_tabs(&mut self, _tabs: &Tabs) {}
    fn end_tabs(&mut self, _tabs: &Tabs) {}
    fn begin_tab(&mut self, _tab: &Tab) {}
    fn end_tab(&mut self, _tab: &Tab) {}
    fn control(&mut self, _control: &Control) {}
}

fn visit_layout(layout: &Layout, kind: LayoutKind, visitor: &mut dyn ControlVisitor) {
    visitor.begin_layout(layout, kind);
    if let Some(control) = &layout.header {
        visitor.control(control);
    }
    for node in &layout.children {
        match node {
            Node::Control(c) => visitor.control(c),
            Node::Row(l) => visit_layout(l, LayoutKind::Row, visitor),
            Node::Column(l) => visit_layout(l, LayoutKind::Column, visitor),
            Node::Group(l) => visit_layout(l, LayoutKind::Group, visitor),
            Node::Table(l) => visit_layout(l, LayoutKind::Table, visitor),
            Node::Tabs(tabs) => {
                visitor.begin_tabs(tabs);
                for tab in &tabs.tabs {
                    visitor.begin_tab(tab);
                    visit_layout(&tab.body, LayoutKind::Column, visitor);
                    visitor.end_tab(tab);
                }
                visitor.end_tabs(tabs);
            }
        }
    }
    visitor.end_layout(layout, kind);
}
