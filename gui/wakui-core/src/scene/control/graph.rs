//! Owned scene graph: containers, tabs, pages, and screens.
//! Traversal is explicit and single-pass; each control is visited once.

use super::policy::LayoutPolicy;
use super::{Control, ControlId, ControlMetadata, ControlVisitor, LayoutId, PageId, TabId, TabsId};

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
        Node::Control(control) => find_control(control, id),
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

/// Identity lookup reaching into list cells. Cells are real controls with
/// stable IDs; without this descent every list-row gesture would miss the
/// dispatch gate. External container-like leaves keep surfacing through
/// `walk_control`; only the framework-owned list degrades into lookup here.
fn find_control(control: &Control, id: ControlId) -> Option<&Control> {
    if control.id == id {
        return Some(control);
    }
    if let Some(list) = control.downcast::<crate::scene::leaf::ListLeaf>() {
        for row in &list.rows {
            for cell in &row.cells {
                if let Some(found) = find_control(cell, id) {
                    return Some(found);
                }
            }
        }
    }
    None
}

/// Downcast a container policy. Flavors branch on the static type,
/// never on view-definition types or debug names.
pub fn policy<T: 'static>(container: &Container) -> Option<&T> {
    container.policy.as_any().downcast_ref::<T>()
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
