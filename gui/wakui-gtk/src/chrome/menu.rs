//! Native menubar: models, registry, and the generic visitor.
//! Menu meaning resolves view-side; this builds native menus from the
//! [`wakui_core::chrome::menu`] snapshot and routes activations as opaque
//! [`wakui_core::input::commands::CommandRef`] events.
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

use gtk::{
    gio, glib,
    glib::variant::{StaticVariantType, ToVariant},
    prelude::*,
};
use wakui_core::input::event::{EventCommand, EventSink};
use wakui_core::{
    chrome::menu::{MenuCapabilities, MenuId, MenuItem, MenuVisitor, Menubar, Submenu},
    input::commands::{Command, CommandId, CommandRef, CommandVisitor},
};

/// Bound once via `set_menubar`; host IDs retain the same native submenus
/// across refills because replacing the root does not update the visible bar.
#[derive(Clone)]
pub(crate) struct MenubarModels {
    pub(crate) menu_model: gio::Menu,
    pub(crate) models: Rc<RefCell<HashMap<MenuId, gio::Menu>>>,
    pub(crate) attached: Rc<Cell<bool>>,
}

impl MenubarModels {
    pub(crate) fn new() -> Self {
        Self {
            menu_model: gio::Menu::new(),
            models: Rc::default(),
            attached: Rc::default(),
        }
    }

    pub(crate) fn submenu(&self, id: MenuId) -> gio::Menu {
        let existing = self.models.borrow().get(&id).cloned();
        if let Some(menu) = existing {
            return menu;
        }
        // No cache borrow across native construction, even on the first visit.
        let menu = gio::Menu::new();
        self.models.borrow_mut().entry(id).or_insert(menu).clone()
    }
}

/// Shared identity cache only; no borrow may survive a native call.
/// Names are derived from the opaque id alone, so menu-item references
/// (`win.<name>`) stay stable across rebuilds without domain knowledge.
#[derive(Clone, Default)]
pub(crate) struct ActionRegistry(pub(crate) Rc<RefCell<HashMap<CommandId, gio::SimpleAction>>>);

impl ActionRegistry {
    pub(crate) fn get(&self, id: CommandId) -> Option<gio::SimpleAction> {
        self.0.borrow().get(&id).cloned()
    }

    pub(crate) fn action_name(id: CommandId) -> String {
        format!("{:?}", id)
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
    }

    pub(crate) fn name(&self, id: CommandId) -> Option<String> {
        self.0
            .borrow()
            .contains_key(&id)
            .then(|| Self::action_name(id))
    }

    pub(crate) fn get_or_insert(
        &self,
        id: CommandId,
        parameter_type: Option<&glib::VariantTy>,
    ) -> gio::SimpleAction {
        if let Some(action) = self.get(id) {
            return action;
        }
        let action = gio::SimpleAction::new(&Self::action_name(id), parameter_type);
        self.0.borrow_mut().insert(id, action.clone());
        action
    }
}

pub(crate) const GTK_MENU_CAPABILITIES: MenuCapabilities = MenuCapabilities {
    about: true,
    dialog_ellipsis: false,
    application_menu: false,
};

/// A stack over persistent native objects, keyed by host semantic identity.
pub(crate) struct GtkMenuVisitor {
    pub(crate) menubar: MenubarModels,
    pub(crate) window: Option<gtk::ApplicationWindow>,
    pub(crate) application: Option<gtk::Application>,
    pub(crate) actions: ActionRegistry,
    pub(crate) sink: Option<Rc<dyn EventSink>>,
    pub(crate) stack: Vec<(gio::Menu, bool)>,
}

impl GtkMenuVisitor {
    #[cfg(test)]
    pub(crate) fn new(menubar: MenubarModels) -> Self {
        Self {
            menubar,
            window: None,
            application: None,
            actions: ActionRegistry::default(),
            sink: None,
            stack: Vec::new(),
        }
    }
}

/// Decode strictly before binding: a bad or missing item is never item zero.
/// Shape comes from the generic spec (`None` vs `Some`); domain meaning of
/// the index resolves view-side.
pub(crate) fn send_action(
    sink: &dyn EventSink,
    descriptor: &Command,
    value: Option<&glib::Variant>,
) {
    let indexed = descriptor.spec.argument().is_some();
    let argument = match (indexed, value) {
        (false, None) => None,
        (true, Some(value)) => {
            let Some(id) = value.get::<u64>() else { return };
            Some(id)
        }
        _ => return,
    };
    sink.send(EventCommand::Menu(CommandRef {
        id: descriptor.id,
        argument,
    }));
}

impl CommandVisitor for GtkMenuVisitor {
    fn command(&mut self, descriptor: &Command) {
        let action = if let Some(action) = self.actions.get(descriptor.id) {
            action
        } else {
            let parameter_type = descriptor
                .spec
                .argument()
                .is_some()
                .then(u64::static_variant_type);
            let action = self
                .actions
                .get_or_insert(descriptor.id, parameter_type.as_deref());
            if let Some(sink) = self.sink.clone() {
                let descriptor = descriptor.clone();
                action.connect_activate(move |_, parameter| {
                    send_action(sink.as_ref(), &descriptor, parameter);
                });
            }
            if let Some(window) = &self.window {
                window.add_action(&action);
            }
            action
        };
        // Once per declaration, never per item entry. No registry borrow here.
        action.set_enabled(descriptor.enabled);
    }
}

impl MenuVisitor for GtkMenuVisitor {
    fn begin_menubar(&mut self, _menubar: &Menubar, rebuild: bool) {
        if rebuild {
            self.menubar.menu_model.remove_all();
        }
        self.stack.push((self.menubar.menu_model.clone(), rebuild));
    }

    fn begin_menu(&mut self, menu: &Submenu, rebuild: bool) {
        let native = self.menubar.submenu(menu.id);
        let (parent, parent_rebuild) = self.stack.last().expect("host menu parent");
        if *parent_rebuild {
            parent.append_submenu(Some(&menu.label), &native);
        }
        if rebuild {
            native.remove_all();
        }
        self.stack.push((native, rebuild));
    }

    fn item(&mut self, item: &MenuItem) {
        let action = item.command.and_then(|activation| {
            self.actions
                .name(activation.id)
                .map(|name| (name, activation.argument))
        });
        let (parent, rebuild) = self.stack.last().expect("host item parent");
        if *rebuild {
            let native = gio::MenuItem::new(Some(&item.label), None);
            if let Some((name, target)) = action {
                native.set_action_and_target_value(
                    Some(&format!("win.{name}")),
                    target.map(|id| id.to_variant()).as_ref(),
                );
            }
            parent.append_item(&native);
        }
    }

    fn end_menu(&mut self, _id: MenuId) {
        self.stack.pop();
    }
    fn end_menubar(&mut self) {
        self.stack.pop();
        debug_assert!(self.stack.is_empty());
        if let (Some(application), Some(window)) = (&self.application, &self.window)
            && !self.menubar.attached.replace(true)
        {
            application.set_menubar(Some(&self.menubar.menu_model));
            window.set_show_menubar(true);
        }
    }
}
