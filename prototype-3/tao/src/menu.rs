use super::*;
use muda::{IsMenuItem, PredefinedMenuItem, accelerator::Accelerator};
use wakui2_runtime::{MenuItem, MenuRole, Shortcut};

#[derive(Clone)]
pub(super) struct Action {
    item: MenuItem,
    dispatch: Rc<dyn Fn(String) -> bool>,
    pub(super) close: Option<Rc<dyn Fn()>>,
}
fn quit_shim(role: &MenuRole) -> bool {
    *role == MenuRole::Quit
}
pub fn parse_shortcut(value: &str) -> Result<Accelerator, String> {
    // Canonical first; historical muda Code syntax remains a legacy fallback.
    let canonical = Shortcut::parse(value)
        .ok()
        .map(|shortcut| shortcut.to_muda());
    canonical
        .as_deref()
        .unwrap_or(value)
        .parse::<Accelerator>()
        .map_err(|e| format!("invalid menu shortcut {value:?}: {e}"))
}
fn validate<A>(items: &[MenuItem<A>]) -> Result<(), String> {
    let mut ids = HashSet::new();
    for item in items {
        if !ids.insert(&item.id) {
            return Err(format!("duplicate menu item id: {}", item.id));
        }
        if let Some(shortcut) = &item.shortcut {
            parse_shortcut(shortcut)?;
            if item.role != MenuRole::Normal && !quit_shim(&item.role) {
                return Err(format!(
                    "custom shortcut on predefined native role is unsupported: {}",
                    item.id
                ));
            }
        }
        if item.role != MenuRole::Normal
            && item.role != MenuRole::Separator
            && !quit_shim(&item.role)
            && !item.enabled
        {
            return Err(format!("muda cannot disable predefined role: {}", item.id));
        }
    }
    Ok(())
}
fn predefined<A>(item: &MenuItem<A>) -> PredefinedMenuItem {
    let text = Some(item.label.as_str());
    match item.role {
        MenuRole::Copy => PredefinedMenuItem::copy(text),
        MenuRole::Cut => PredefinedMenuItem::cut(text),
        MenuRole::Paste => PredefinedMenuItem::paste(text),
        MenuRole::SelectAll => PredefinedMenuItem::select_all(text),
        MenuRole::About => PredefinedMenuItem::about(
            text,
            Some(muda::AboutMetadata {
                name: Some(item.label.clone()),
                ..Default::default()
            }),
        ),
        MenuRole::Quit => unreachable!("Quit must use the vetoable ordinary-item shim"),
        MenuRole::Separator => PredefinedMenuItem::separator(),
        MenuRole::Normal => unreachable!(),
    }
}
fn native_ref(item: &muda::MenuItemKind) -> &dyn IsMenuItem {
    match item {
        muda::MenuItemKind::MenuItem(i) => i,
        muda::MenuItemKind::Submenu(i) => i,
        muda::MenuItemKind::Predefined(i) => i,
        muda::MenuItemKind::Check(i) => i,
        muda::MenuItemKind::Icon(i) => i,
    }
}
/// muda GTK3 unparents before removing accelerators; GTK may already remove them
/// on unparent. Clear via the supported API while still attached (also on updates).
pub(super) fn clear_accelerators(items: &[muda::MenuItemKind]) -> Result<(), String> {
    for item in items {
        match item {
            muda::MenuItemKind::MenuItem(item) => {
                item.set_accelerator(None).map_err(|e| e.to_string())?
            }
            muda::MenuItemKind::Submenu(menu) => clear_accelerators(&menu.items())?,
            _ => {}
        }
    }
    Ok(())
}
/// No registry changes until this succeeds. Roll back even a failed remove:
/// a backend may have already applied part of the requested operation.
fn replace_snapshot<T>(
    old: &[T],
    next: &[T],
    prepare: impl FnOnce() -> Result<(), String>,
    mut remove: impl FnMut(&T) -> Result<(), String>,
    mut append: impl FnMut(&T) -> Result<(), String>,
    rollback: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let result: Result<(), String> = (|| {
        prepare()?;
        for item in old {
            remove(item)?;
        }
        for item in next {
            append(item)?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        return match rollback() {
            Ok(()) => Err(format!(
                "menu snapshot rejected (previous snapshot restored): {error}"
            )),
            Err(restore) => Err(format!(
                "menu snapshot failed: {error}; rollback failed: {restore}"
            )),
        };
    }
    Ok(())
}
impl View {
    pub fn menu(
        &self,
        id: &str,
        title: Binding<String>,
        items: Binding<Vec<MenuItem>>,
        dispatch: Rc<dyn Fn(String) -> bool>,
    ) {
        self.typed_menu(id, title, items, dispatch);
    }
    pub fn typed_menu<A: Clone + 'static>(
        &self,
        id: &str,
        title: Binding<String>,
        items: Binding<Vec<MenuItem<A>>>,
        dispatch: Rc<dyn Fn(A) -> bool>,
    ) {
        assert!(!self.0.closed.get(), "closed view");
        // A retained node owns the two dynamic subscriptions, just like widget mounts.
        let node = self.add(&self.root(), id, Kind::Content);
        // Menus are not software controls in the content layout.
        self.0
            .root
            .0
            .children
            .borrow_mut()
            .retain(|n| !Rc::ptr_eq(n, &node));
        self.0.menu_nodes.borrow_mut().push(node);
        let submenu = muda::Submenu::new(title.get(), true);
        if let Err(e) = self.0.menus.append(&submenu) {
            self.0.report(e.to_string());
        }
        let menu = submenu.clone();
        let weak = Rc::downgrade(&self.0);
        let token = title.subscribe(Rc::new(move |text| {
            if weak.upgrade().is_some_and(|i| !i.closed.get()) {
                menu.set_text(text);
            }
        }));
        self.keep(token);
        let weak = Rc::downgrade(&self.0);
        let menu_id = id.to_owned();
        let old_keys = RefCell::new(Vec::<String>::new());
        let old_native = RefCell::new(Vec::<muda::MenuId>::new());
        let old_accelerators = RefCell::new(Vec::<(muda::MenuItem, Option<Accelerator>)>::new());
        let token = items.subscribe(Rc::new(move |values| {
            let Some(inner) = weak.upgrade().filter(|i| !i.closed.get()) else {
                return;
            };
            if let Err(error) = validate(&values) {
                inner.report(error);
                return;
            }
            // Validate and build the entire snapshot before changing the live native menu.
            let mut native = Vec::new();
            let mut accelerators = Vec::new();
            let mut actions = Vec::new();
            let mut ids = Vec::new();
            for item in values {
                let argument = item.argument;
                let invoke = dispatch.clone();
                let dispatch: Rc<dyn Fn(String) -> bool> =
                    Rc::new(move |_| invoke(argument.clone()));
                let item = MenuItem {
                    id: item.id,
                    label: item.label,
                    argument: String::new(),
                    shortcut: item.shortcut,
                    role: item.role,
                    enabled: item.enabled,
                };
                let key = format!("{menu_id}/{}", item.id);
                if item.role == MenuRole::Normal || quit_shim(&item.role) {
                    let accelerator = item
                        .shortcut
                        .as_deref()
                        .map(parse_shortcut)
                        .transpose()
                        .expect("validated shortcut");
                    let entry = muda::MenuItem::new(&item.label, item.enabled, accelerator);
                    ids.push((entry.id().clone(), key.clone()));
                    // Predefined Quit directly terminates the native loop/app on
                    // Windows/macOS (not vetoable). Always use an ordinary item.
                    let close = if quit_shim(&item.role) {
                        let owner = weak.clone();
                        Some(Rc::new(move || {
                            if let Some(inner) = owner.upgrade() {
                                View(inner).request_close();
                            }
                        }) as Rc<dyn Fn()>)
                    } else {
                        None
                    };
                    accelerators.push((entry.clone(), accelerator));
                    actions.push((
                        key,
                        Action {
                            item,
                            dispatch: dispatch.clone(),
                            close,
                        },
                    ));
                    native.push(entry.kind());
                } else {
                    let entry = predefined(&item);
                    // Predefined roles are OS actions, not application Command aliases.
                    native.push(entry.kind());
                    actions.push((
                        key,
                        Action {
                            item,
                            dispatch: dispatch.clone(),
                            close: None,
                        },
                    ));
                }
            }
            let old = submenu.items();
            // Offscreen Submenu + root insert is not a strong-guarantee swap:
            // muda platform attach can fail while creating descendants, before its
            // public children registry is updated. Restore the existing submenu instead.
            let update = replace_snapshot(
                &old,
                &native,
                || clear_accelerators(&old),
                |item| submenu.remove(native_ref(item)).map_err(|e| e.to_string()),
                |item| submenu.append(native_ref(item)).map_err(|e| e.to_string()),
                || {
                    let mut errors = Vec::new();
                    // Also clear candidates whose platform attach failed part-way.
                    if let Err(e) = clear_accelerators(&native) {
                        errors.push(e);
                    }
                    if let Err(e) = clear_accelerators(&old) {
                        errors.push(e);
                    }
                    while submenu.remove_at(0).is_some() {}
                    for item in &old {
                        if let Err(e) = submenu.append(native_ref(item)) {
                            errors.push(e.to_string());
                        }
                    }
                    for (item, accelerator) in old_accelerators.borrow().iter() {
                        if let Err(e) = item.set_accelerator(*accelerator) {
                            errors.push(e.to_string());
                        }
                    }
                    if errors.is_empty() {
                        Ok(())
                    } else {
                        Err(errors.join("; "))
                    }
                },
            );
            if let Err(error) = update {
                inner.report(error);
                return;
            }
            for key in old_keys.take() {
                inner.menu_actions.borrow_mut().remove(&key);
            }
            for id in old_native.take() {
                inner.menu_ids.borrow_mut().remove(&id);
            }
            old_accelerators.replace(accelerators);
            old_keys.replace(actions.iter().map(|(key, _)| key.clone()).collect());
            old_native.replace(ids.iter().map(|(id, _)| id.clone()).collect());
            inner.menu_actions.borrow_mut().extend(actions);
            inner.menu_ids.borrow_mut().extend(ids);
            inner.refresh();
        }));
        self.keep(token);
    }
    pub fn activate_menu(&self, id: &str) -> Result<(), String> {
        if self.0.closed.get() {
            return Err("view is closed".into());
        }
        let actions = self.0.menu_actions.borrow();
        let action = actions.get(id).cloned().or_else(|| {
            let mut found = actions.values().filter(|a| a.item.id == id);
            let first = found.next().cloned();
            if found.next().is_some() { None } else { first }
        });
        drop(actions);
        let action = action.ok_or_else(|| format!("missing or ambiguous menu item: {id}"))?;
        if !action.item.enabled {
            return Err(format!("disabled menu item: {id}"));
        }
        if let Some(close) = action.close {
            close();
            return Ok(());
        }
        if action.item.role != MenuRole::Normal {
            return Err(format!(
                "native predefined role requires OS activation, not automation: {id}"
            ));
        }
        if (action.dispatch)(action.item.argument) {
            Ok(())
        } else {
            Err(format!("menu command rejected: {id}"))
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_shortcuts_match_native_modifier_and_key_semantics() {
        use muda::accelerator::{Code, Modifiers};
        use wakui2_runtime::ShortcutTarget;
        for (input, key, extra) in [
            ("Primary+S", Code::KeyS, Modifiers::empty()),
            ("Primary+Q", Code::KeyQ, Modifiers::empty()),
            ("Primary+Shift+S", Code::KeyS, Modifiers::SHIFT),
        ] {
            let actual = parse_shortcut(input).unwrap();
            let primary = if cfg!(target_os = "macos") {
                Modifiers::META
            } else {
                Modifiers::CONTROL
            };
            assert_eq!(actual.modifiers(), primary | extra);
            assert_eq!(actual.key(), key);
            for target in [ShortcutTarget::MacOS, ShortcutTarget::Other] {
                let shortcut = Shortcut::parse(input).unwrap();
                let native: Accelerator = shortcut.to_muda_for(target).parse().unwrap();
                let primary = if target == ShortcutTarget::MacOS {
                    Modifiers::META
                } else {
                    Modifiers::CONTROL
                };
                assert_eq!(native.modifiers(), primary | extra);
                assert_eq!(native.key(), key);
            }
        }
        for key in [
            "Enter",
            "Tab",
            "Space",
            "Escape",
            "Backspace",
            "Delete",
            "Insert",
            "Home",
            "End",
            "PageUp",
            "PageDown",
            "Left",
            "Right",
            "Up",
            "Down",
            "F1",
            "F12",
            "0",
            "9",
        ] {
            assert!(
                parse_shortcut(&format!("Ctrl+Shift+{key}")).is_ok(),
                "{key}"
            );
        }
        assert_eq!(
            parse_shortcut("Ctrl+Shift+S"),
            parse_shortcut("Ctrl+Shift+KeyS")
        );
    }
    #[test]
    fn parse_and_validate_before_snapshot_mutation() {
        assert!(parse_shortcut("Ctrl+KeyS").is_ok());
        assert!(parse_shortcut("NotAModifier+UnknownKey").is_err());
        let item: MenuItem = MenuItem {
            id: "save".into(),
            label: "Save".into(),
            argument: "now".into(),
            shortcut: Some("Ctrl+KeyS".into()),
            role: MenuRole::Normal,
            enabled: true,
        };
        assert!(validate(std::slice::from_ref(&item)).is_ok());
        assert!(validate(&[item.clone(), item.clone()]).is_err());
        let mut role = item;
        role.role = MenuRole::Copy;
        assert!(validate(&[role]).is_err());
    }

    #[test]
    fn every_native_failure_restores_snapshot_before_registry_commit() {
        // Fail before AND after each native operation to model partial backend mutation.
        for fail in 0..5 {
            for partial in [false, true] {
                let old = vec![1, 2];
                let next = vec![3, 4];
                let live = RefCell::new(old.clone());
                let registry = RefCell::new(old.clone());
                let step = Cell::new(0);
                let operation = |apply: &dyn Fn()| {
                    let current = step.replace(step.get() + 1);
                    if current == fail && !partial {
                        return Err("injected native failure".into());
                    }
                    apply();
                    if current == fail {
                        Err("injected native failure".into())
                    } else {
                        Ok(())
                    }
                };
                let result = replace_snapshot(
                    &old,
                    &next,
                    || operation(&|| {}),
                    |item| operation(&|| live.borrow_mut().retain(|v| v != item)),
                    |item| operation(&|| live.borrow_mut().push(*item)),
                    || {
                        live.replace(old.clone());
                        Ok(())
                    },
                );
                if result.is_ok() {
                    registry.replace(next.clone());
                }
                assert!(result.unwrap_err().contains("previous snapshot restored"));
                assert_eq!(*live.borrow(), old);
                assert_eq!(*registry.borrow(), old);
            }
        }
        let live = RefCell::new(vec![1, 2]);
        replace_snapshot(
            &[1, 2],
            &[3, 4],
            || Ok(()),
            |item| {
                live.borrow_mut().retain(|v| v != item);
                Ok(())
            },
            |item| {
                live.borrow_mut().push(*item);
                Ok(())
            },
            || panic!("successful transaction must not roll back"),
        )
        .unwrap();
        assert_eq!(*live.borrow(), [3, 4]);
        let error = replace_snapshot::<i32>(
            &[],
            &[],
            || Err("apply".into()),
            |_| Ok(()),
            |_| Ok(()),
            || Err("restore".into()),
        )
        .unwrap_err();
        assert!(error.contains("apply"));
        assert!(error.contains("rollback failed: restore"));
    }

    #[test]
    fn predefined_capabilities_remain_explicit() {
        for role in [
            MenuRole::Copy,
            MenuRole::Cut,
            MenuRole::Paste,
            MenuRole::SelectAll,
            MenuRole::About,
            MenuRole::Quit,
            MenuRole::Separator,
        ] {
            let mut item = MenuItem {
                id: "role".into(),
                label: "Role".into(),
                argument: String::new(),
                shortcut: None,
                role: role.clone(),
                enabled: true,
            };
            assert!(validate(&[item.clone()]).is_ok());
            item.enabled = false;
            assert_eq!(
                validate(&[item.clone()]).is_ok(),
                role == MenuRole::Separator || quit_shim(&role)
            );
            item.enabled = true;
            item.shortcut = Some("Ctrl+KeyS".into());
            assert_eq!(validate(&[item]).is_ok(), quit_shim(&role));
        }
    }
}
