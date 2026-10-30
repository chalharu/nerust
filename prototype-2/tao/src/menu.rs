use super::*;
use wakui2_runtime::{MenuItem, MenuRole};
use muda::{accelerator::Accelerator, IsMenuItem, PredefinedMenuItem};

#[derive(Clone)]
pub(super) struct Action { item: MenuItem, dispatch: Rc<dyn Fn(String) -> bool> }
pub fn parse_shortcut(value: &str) -> Result<Accelerator, String> {
    value.parse::<Accelerator>().map_err(|e| format!("invalid menu shortcut {value:?}: {e}"))
}
fn validate(items: &[MenuItem]) -> Result<(), String> {
    let mut ids = HashSet::new();
    for item in items {
        if !ids.insert(&item.id) { return Err(format!("duplicate menu item id: {}", item.id)); }
        if let Some(shortcut) = &item.shortcut {
            parse_shortcut(shortcut)?;
            if item.role != MenuRole::Normal {
                return Err(format!("custom shortcut on predefined native role is unsupported: {}", item.id));
            }
        }
        if item.role != MenuRole::Normal && item.role != MenuRole::Separator && !item.enabled {
            return Err(format!("muda cannot disable predefined role: {}", item.id));
        }
        #[cfg(target_os = "linux")]
        if item.role == MenuRole::Quit {
            return Err("muda GTK3 does not support the native Quit role; use a Normal close command".into());
        }
    }
    Ok(())
}
fn predefined(item: &MenuItem) -> PredefinedMenuItem {
    let text = Some(item.label.as_str());
    match item.role {
        MenuRole::Copy => PredefinedMenuItem::copy(text), MenuRole::Cut => PredefinedMenuItem::cut(text),
        MenuRole::Paste => PredefinedMenuItem::paste(text), MenuRole::SelectAll => PredefinedMenuItem::select_all(text),
        MenuRole::About => PredefinedMenuItem::about(text, Some(muda::AboutMetadata { name: Some(item.label.clone()), ..Default::default() })),
        MenuRole::Quit => PredefinedMenuItem::quit(text),
        MenuRole::Separator => PredefinedMenuItem::separator(), MenuRole::Normal => unreachable!(),
    }
}
fn native_ref(item: &muda::MenuItemKind) -> &dyn IsMenuItem {
    match item {
        muda::MenuItemKind::MenuItem(i) => i, muda::MenuItemKind::Submenu(i) => i,
        muda::MenuItemKind::Predefined(i) => i, muda::MenuItemKind::Check(i) => i,
        muda::MenuItemKind::Icon(i) => i,
    }
}
/// muda GTK3 unparents before removing accelerators; GTK may already remove them
/// on unparent. Clear via the supported API while still attached (also on updates).
pub(super) fn clear_accelerators(items: &[muda::MenuItemKind]) -> Result<(), String> {
    for item in items {
        match item {
            muda::MenuItemKind::MenuItem(item) => item.set_accelerator(None).map_err(|e| e.to_string())?,
            muda::MenuItemKind::Submenu(menu) => clear_accelerators(&menu.items())?,
            _ => {}
        }
    }
    Ok(())
}
impl View {
    pub fn menu(&self, id: &str, title: Binding<String>, items: Binding<Vec<MenuItem>>, dispatch: Rc<dyn Fn(String) -> bool>) {
        assert!(!self.0.closed.get(), "closed view");
        // A retained node owns the two dynamic subscriptions, just like widget mounts.
        let node = self.add(&self.root(), id, Kind::Content);
        // Menus are not software controls in the content layout.
        self.0.root.0.children.borrow_mut().retain(|n| !Rc::ptr_eq(n, &node));
        self.0.menu_nodes.borrow_mut().push(node);
        let submenu = muda::Submenu::new(title.get(), true);
        if let Err(e) = self.0.menus.append(&submenu) { self.0.report(e.to_string()); }
        let menu = submenu.clone();
        let weak = Rc::downgrade(&self.0);
        let token = title.subscribe(Rc::new(move |text| {
            if weak.upgrade().is_some_and(|i| !i.closed.get()) { menu.set_text(text); }
        }));
        self.keep(token);
        let weak = Rc::downgrade(&self.0); let menu_id = id.to_owned();
        let old_keys = RefCell::new(Vec::<String>::new());
        let old_native = RefCell::new(Vec::<muda::MenuId>::new());
        let token = items.subscribe(Rc::new(move |values| {
            let Some(inner) = weak.upgrade().filter(|i| !i.closed.get()) else { return };
            if let Err(error) = validate(&values) { inner.report(error); return; }
            // Validate and build the entire snapshot before changing the live native menu.
            let mut native: Vec<Box<dyn IsMenuItem>> = Vec::new();
            let mut actions = Vec::new(); let mut ids = Vec::new();
            for item in values {
                let key = format!("{menu_id}/{}", item.id);
                if item.role == MenuRole::Normal {
                    let accelerator = item.shortcut.as_deref().map(parse_shortcut).transpose().expect("validated shortcut");
                    let entry = muda::MenuItem::new(&item.label, item.enabled, accelerator);
                    ids.push((entry.id().clone(), key.clone()));
                    actions.push((key, Action { item, dispatch: dispatch.clone() })); native.push(Box::new(entry));
                } else {
                    let entry = predefined(&item);
                    // Predefined roles are OS actions, not application Command aliases.
                    native.push(Box::new(entry));
                    actions.push((key, Action { item, dispatch: dispatch.clone() }));
                }
            }
            let old = submenu.items();
            if let Err(e) = clear_accelerators(&old) { inner.report(e); return; }
            for old in old { if let Err(e) = submenu.remove(native_ref(&old)) { inner.report(e.to_string()); return; } }
            for key in old_keys.take() { inner.menu_actions.borrow_mut().remove(&key); }
            for id in old_native.take() { inner.menu_ids.borrow_mut().remove(&id); }
            for entry in &native { if let Err(e) = submenu.append(entry.as_ref()) { inner.report(e.to_string()); return; } }
            old_keys.replace(actions.iter().map(|(key, _)| key.clone()).collect());
            old_native.replace(ids.iter().map(|(id, _)| id.clone()).collect());
            inner.menu_actions.borrow_mut().extend(actions); inner.menu_ids.borrow_mut().extend(ids); inner.refresh();
        }));
        self.keep(token);
    }
    pub fn activate_menu(&self, id: &str) -> Result<(), String> {
        if self.0.closed.get() { return Err("view is closed".into()); }
        let actions = self.0.menu_actions.borrow();
        let action = actions.get(id).cloned().or_else(|| {
            let mut found = actions.values().filter(|a| a.item.id == id);
            let first = found.next().cloned(); if found.next().is_some() { None } else { first }
        });
        drop(actions);
        let action = action.ok_or_else(|| format!("missing or ambiguous menu item: {id}"))?;
        if !action.item.enabled { return Err(format!("disabled menu item: {id}")); }
        if action.item.role != MenuRole::Normal {
            return Err(format!("native predefined role requires OS activation, not automation: {id}"));
        }
        if (action.dispatch)(action.item.argument) { Ok(()) } else { Err(format!("menu command rejected: {id}")) }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_and_validate_before_snapshot_mutation() {
        assert!(parse_shortcut("Ctrl+KeyS").is_ok());
        assert!(parse_shortcut("NotAModifier+UnknownKey").is_err());
        let item = MenuItem { id: "save".into(), label: "Save".into(), argument: "now".into(),
            shortcut: Some("Ctrl+KeyS".into()), role: MenuRole::Normal, enabled: true };
        assert!(validate(&[item.clone()]).is_ok());
        assert!(validate(&[item.clone(), item.clone()]).is_err());
        let mut role = item; role.role = MenuRole::Copy;
        assert!(validate(&[role]).is_err());
    }
}