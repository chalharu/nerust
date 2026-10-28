//! Owned semantic menus: ids, capabilities, models, and visitors.
//! Labels arrive pre-localized; building from the signature stays view-side.

//! Owned semantic menus. Native visitors supply controls, never menu policy.
//!
//! Menu IDs are unique in a model; item commands are unique within each parent
//! (including at most one action-less placeholder). Visitors reuse native
//! objects by those identities. Labels and enablement are not identities.

pub use crate::commands::{CommandModel, CommandVisitor, MenuCommand};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MenuId {
    File,
    Emulation,
    States,
    SelectSlot,
    SaveSlot,
    LoadSlot,
    DeleteSlot,
    Help,
}

/// Explicit frontend differences; no toolkit or target dependencies in host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuCapabilities {
    pub about: bool,
    pub dialog_ellipsis: bool,
    pub application_menu: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    /// Also the stable item identity within its parent. None is a placeholder.
    pub command: Option<MenuCommand>,
    pub label: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuEntry {
    Submenu(Submenu),
    Item(MenuItem),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Submenu {
    pub id: MenuId,
    pub label: String,
    pub enabled: bool,
    pub children: Vec<MenuEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Menubar {
    pub commands: CommandModel,
    pub menus: Vec<Submenu>,
    pub application_menu: Option<NativeApplicationMenu>,
}

pub type MenuModel = Menubar;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeApplicationMenu {
    pub label: String,
    pub entries: Vec<NativeApplicationEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeApplicationEntry {
    Role(NativeApplicationRole),
    Separator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeApplicationRole {
    About,
    Services,
    Hide,
    HideOthers,
    ShowAll,
    Quit,
}

fn same_menus(left: &[Submenu], right: &[Submenu]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(a, b)| a.id == b.id && a.label == b.label)
}

fn same_children(left: &[MenuEntry], right: &[MenuEntry]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(a, b)| match (a, b) {
            (MenuEntry::Submenu(a), MenuEntry::Submenu(b)) => a.id == b.id && a.label == b.label,
            (MenuEntry::Item(a), MenuEntry::Item(b)) => {
                a.command == b.command && a.label == b.label
            }
            _ => false,
        })
}

/// Find a submenu by id, depth-first. View-side tests and native adapters
/// navigate built models without reimplementing the traversal.
pub fn find_menu(menus: &[Submenu], id: MenuId) -> Option<&Submenu> {
    for menu in menus {
        if menu.id == id {
            return Some(menu);
        }
        for child in &menu.children {
            if let MenuEntry::Submenu(child) = child
                && let Some(found) = find_menu(std::slice::from_ref(child), id)
            {
                return Some(found);
            }
        }
    }
    None
}

fn visit_menu(menu: &Submenu, old: Option<&Submenu>, visitor: &mut dyn MenuVisitor) {
    visitor.begin_menu(
        menu,
        old.is_none_or(|old| !same_children(&menu.children, &old.children)),
    );
    for child in &menu.children {
        match child {
            MenuEntry::Item(item) => visitor.item(item),
            MenuEntry::Submenu(child) => {
                let old = old.and_then(|old| find_menu(std::slice::from_ref(old), child.id));
                visit_menu(child, old, visitor);
            }
        }
    }
    visitor.end_menu(menu.id);
}

pub trait MenuVisitor: CommandVisitor {
    fn begin_menubar(&mut self, menubar: &Menubar, rebuild: bool);
    fn begin_menu(&mut self, menu: &Submenu, rebuild: bool);
    fn item(&mut self, item: &MenuItem);
    fn end_menu(&mut self, id: MenuId);
    fn end_menubar(&mut self);
}

impl Menubar {
    /// Balanced preorder. Compare direct identities/labels only, not enabled
    /// state or descendants: slot changes do not rebuild their ancestors.
    pub fn visit(&self, previous: Option<&Self>, visitor: &mut dyn MenuVisitor) {
        visitor.begin_menubar(
            self,
            previous.is_none_or(|old| {
                self.application_menu != old.application_menu
                    || !same_menus(&self.menus, &old.menus)
            }),
        );
        self.commands.visit(visitor);
        for menu in &self.menus {
            visit_menu(
                menu,
                previous.and_then(|old| find_menu(&old.menus, menu.id)),
                visitor,
            );
        }
        visitor.end_menubar();
    }
}
