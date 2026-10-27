//! Owned semantic menus. Native visitors supply controls, never menu policy.
//!
//! Menu IDs are unique in a model; item commands are unique within each parent
//! (including at most one action-less placeholder). Visitors reuse native
//! objects by those identities. Labels and enablement are not identities.

use nerust_gui_runtime::{menu_labels::shortcut_hint, slots::slot_label};
use nerust_gui_shell::session::commands::SessionCommand;
use nerust_settings_core::{
    i18n::{UiText, text},
    model::input::ShortcutAction,
};

use crate::application::MenuSig;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeRole {
    About,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuCommand {
    Open,
    Close,
    Settings,
    Debugger,
    NotificationHistory,
    Session(SessionCommand),
    Native(NativeRole),
}

impl MenuCommand {
    pub fn enabled(self, sig: &MenuSig) -> bool {
        use SessionCommand as S;
        if self == Self::Native(NativeRole::Quit) {
            return true;
        }
        !sig.settings_open
            && match self {
                Self::Open
                | Self::Settings
                | Self::Debugger
                | Self::NotificationHistory
                | Self::Native(_) => true,
                Self::Close => sig.loaded,
                Self::Session(command) => match command {
                    S::Pause => sig.loaded && !sig.paused,
                    S::Resume => sig.loaded && sig.paused,
                    S::LoadActiveSlot => sig.loaded && sig.active_slot.is_some(),
                    S::SelectActiveSlot(id)
                    | S::SaveSlot(id)
                    | S::LoadSlot(id)
                    | S::DeleteSlot(id) => {
                        sig.loaded && sig.slots.iter().any(|slot| slot.slot_id == id)
                    }
                    S::SelectNextSlot | S::SelectPreviousSlot => {
                        sig.loaded && !sig.slots.is_empty()
                    }
                    S::Reset | S::CreateSlot | S::SaveActiveSlotOrNew | S::TogglePause => {
                        sig.loaded
                    }
                },
            }
    }
}

/// Explicit frontend differences; no toolkit or target dependencies in host.
#[derive(Debug, Clone, Copy)]
pub struct MenuCapabilities {
    pub about: bool,
    pub dialog_ellipsis: bool,
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
pub struct MenuModel {
    pub menus: Vec<Submenu>,
}

impl MenuModel {
    pub fn build(sig: &MenuSig, capabilities: MenuCapabilities) -> Self {
        use MenuCommand as C;
        use SessionCommand as S;
        let label = |key| text(sig.language, key).to_owned();
        let item = |command: C, key| MenuItem {
            command: Some(command),
            label: label(key),
            enabled: command.enabled(sig),
        };
        let hinted = |command, key, shortcut| {
            let mut entry = item(command, key);
            entry.label = shortcut_hint(&entry.label, &sig.shortcuts, shortcut);
            MenuEntry::Item(entry)
        };
        let dialog = |command, key| {
            let mut entry = item(command, key);
            if capabilities.dialog_ellipsis {
                entry.label.push_str("...");
            }
            MenuEntry::Item(entry)
        };
        let submenu = |id, key, enabled, children| Submenu {
            id,
            label: label(key),
            enabled,
            children,
        };
        let mut states = vec![
            MenuEntry::Item(item(C::Session(S::CreateSlot), UiText::CreateSaveSlot)),
            hinted(
                C::Session(S::SaveActiveSlotOrNew),
                UiText::SaveActiveSlot,
                ShortcutAction::SaveActiveSlot,
            ),
            hinted(
                C::Session(S::LoadActiveSlot),
                UiText::LoadActiveSlot,
                ShortcutAction::LoadActiveSlot,
            ),
        ];
        for (id, key, command) in [
            (
                MenuId::SelectSlot,
                UiText::SelectActiveSlot,
                S::SelectActiveSlot as fn(u64) -> S,
            ),
            (MenuId::SaveSlot, UiText::SaveSlot, S::SaveSlot),
            (MenuId::LoadSlot, UiText::LoadSlot, S::LoadSlot),
            (MenuId::DeleteSlot, UiText::DeleteSlot, S::DeleteSlot),
        ] {
            let children = if sig.slots.is_empty() {
                vec![MenuEntry::Item(MenuItem {
                    command: None,
                    label: label(UiText::NoSaveSlots),
                    enabled: false,
                })]
            } else {
                sig.slots
                    .iter()
                    .map(|slot| {
                        let command = C::Session(command(slot.slot_id));
                        MenuEntry::Item(MenuItem {
                            command: Some(command),
                            label: slot_label(slot, sig.active_slot),
                            // This entry comes directly from the current slots;
                            // do not rescan the entire list for every item.
                            enabled: !sig.settings_open && sig.loaded,
                        })
                    })
                    .collect()
            };
            states.push(MenuEntry::Submenu(submenu(
                id,
                key,
                !sig.settings_open && sig.loaded && !sig.slots.is_empty(),
                children,
            )));
        }
        let mut menus = vec![
            submenu(
                MenuId::File,
                UiText::File,
                true,
                vec![
                    dialog(C::Open, UiText::Open),
                    MenuEntry::Item(item(C::Close, UiText::Close)),
                    dialog(C::Settings, UiText::Settings),
                    MenuEntry::Item(item(C::Native(NativeRole::Quit), UiText::Quit)),
                ],
            ),
            submenu(
                MenuId::Emulation,
                UiText::Emulation,
                true,
                vec![
                    MenuEntry::Item(item(C::Session(S::Pause), UiText::Pause)),
                    MenuEntry::Item(item(C::Session(S::Resume), UiText::Resume)),
                    MenuEntry::Item(item(C::Session(S::Reset), UiText::Reset)),
                    MenuEntry::Submenu(submenu(MenuId::States, UiText::SaveStates, true, states)),
                    hinted(
                        C::Debugger,
                        UiText::OpenDebugger,
                        ShortcutAction::OpenDebugger,
                    ),
                    MenuEntry::Item(item(C::NotificationHistory, UiText::NotificationHistory)),
                ],
            ),
        ];
        if capabilities.about {
            menus.push(submenu(
                MenuId::Help,
                UiText::Help,
                true,
                vec![MenuEntry::Item(item(
                    C::Native(NativeRole::About),
                    UiText::About,
                ))],
            ));
        }
        Self { menus }
    }

    /// Balanced preorder. Compare direct identities/labels only, not enabled
    /// state or descendants: slot changes do not rebuild their ancestors.
    pub fn visit(&self, previous: Option<&Self>, visitor: &mut impl MenuVisitor) {
        visitor.begin_model(previous.is_none_or(|old| !same_menus(&self.menus, &old.menus)));
        for menu in &self.menus {
            visit_menu(
                menu,
                previous.and_then(|old| find_menu(&old.menus, menu.id)),
                visitor,
            );
        }
        visitor.end_model();
    }
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

fn find_menu(menus: &[Submenu], id: MenuId) -> Option<&Submenu> {
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

fn visit_menu(menu: &Submenu, old: Option<&Submenu>, visitor: &mut impl MenuVisitor) {
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

pub trait MenuVisitor {
    fn begin_model(&mut self, rebuild: bool);
    fn begin_menu(&mut self, menu: &Submenu, rebuild: bool);
    fn item(&mut self, item: &MenuItem);
    fn end_menu(&mut self, id: MenuId);
    fn end_model(&mut self);
}

#[cfg(test)]
mod tests;
