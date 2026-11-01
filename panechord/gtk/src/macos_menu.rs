//! System-global menubar for the macOS build: mirrors the view's gio menus
//! into an `NSMenu` installed as `NSApp.mainMenu`, hiding the in-window bar.
//! Dispatch reuses the registered gio actions (`activate()` fires the same
//! closures as the in-window items), so behavior stays identical.
use super::Inner;
use gtk::gio::prelude::{ActionExt as _, MenuModelExt as _};
use gtk::{gio, prelude::*};
use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::{ClassType, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSMenu, NSMenuItem};
use objc2_foundation::{MainThreadMarker, NSString};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

define_class!(
    /// Shared click target for every mirrored item. The gio action name rides
    /// `representedObject` (`{view-addr}/{action-name}`); firing looks up the
    /// live view and activates its action, so the same closures run as
    /// in-window. Runs on the main thread only.
    #[unsafe(super(NSObject))]
    #[name = "PanechordGtkMenuTarget"]
    struct MenuTarget;
    impl MenuTarget {
        #[unsafe(method(fire:))]
        fn fire(&self, sender: &NSMenuItem) {
            let Some(name) = sender
                .representedObject()
                .and_then(|object| object.downcast::<NSString>().ok())
                .map(|name| name.to_string())
            else {
                return;
            };
            let Some(slash) = name.find('/') else { return };
            let addr = usize::from_str_radix(&name[..slash], 16).unwrap_or(0);
            let after = name[slash + 1..].to_owned();
            VIEWS.with(|views| {
                let view = views
                    .borrow()
                    .get(&addr)
                    .and_then(|weak| weak.upgrade());
                let Some(view) = view else { return };
                if view.closed.get() {
                    return;
                }
                if let Some(action) = view
                    .actions
                    .borrow()
                    .values()
                    .find(|action| action.name == after)
                {
                    action.action.activate(None);
                }
            });
        }
    }
);

thread_local! {
    /// Views with an installed global menu, by `Inner` address. Main-thread
    /// only (all menu work runs there); dead entries are pruned on fire.
    static VIEWS: RefCell<HashMap<usize, std::rc::Weak<Inner>>> = RefCell::new(HashMap::new());
}

fn target() -> Retained<MenuTarget> {
    use std::sync::OnceLock;
    static TARGET: OnceLock<Retained<MenuTarget>> = OnceLock::new();
    TARGET
        .get_or_init(|| unsafe { msg_send![MenuTarget::class(), new] })
        .clone()
}

/// Remember the view for dispatch while its global menu is installed.
fn track(view: &Rc<Inner>) {
    VIEWS.with(|views| {
        views
            .borrow_mut()
            .insert(Rc::as_ptr(view) as usize, Rc::downgrade(view));
    });
}

/// Rebuild the global menubar from `view`'s menus. Views without menus
/// (settings, dialogs) leave the installed bar alone, so the main window's
/// menus stay global while secondary windows are key.
pub(super) fn sync_global_menu(view: &super::View, mtm: MainThreadMarker) {
    let mut menus: Vec<_> = view
        .0
        .menu_states
        .borrow()
        .iter()
        .map(|(id, state)| (id.clone(), state.clone()))
        .collect();
    menus.sort_by_key(|(_, state)| state.order);
    let addr = format!("{:x}", Rc::as_ptr(&view.0) as usize);
    let bar = NSMenu::new(mtm);
    let mut installed = 0;
    for (id, state) in &menus {
        if state.title.borrow().is_empty() {
            continue;
        }
        let submenu = build_menu(view, &addr, state.menu.upcast_ref(), mtm);
        if submenu.numberOfItems() == 0 {
            continue;
        }
        let title = NSString::from_str(&state.title.borrow());
        submenu.setTitle(&title);
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &title,
                None,
                &NSString::new(),
            )
        };
        item.setSubmenu(Some(&submenu));
        bar.addItem(&item);
        installed += 1;
        let _ = id;
    }
    if installed == 0 {
        return;
    }
    track(&view.0);
    NSApplication::sharedApplication(mtm).setMainMenu(Some(&bar));
}

fn build_menu(
    view: &super::View,
    addr: &str,
    model: &gio::MenuModel,
    mtm: MainThreadMarker,
) -> Retained<NSMenu> {
    let menu = NSMenu::new(mtm);
    let mut broke = false;
    append_model(view, addr, model, &menu, mtm, &mut broke);
    menu
}

/// Append a gio menu model's items; `broke` tracks whether content precedes
/// (section links become separators only between content).
fn append_model(
    view: &super::View,
    addr: &str,
    model: &gio::MenuModel,
    menu: &NSMenu,
    mtm: MainThreadMarker,
    broke: &mut bool,
) {
    for index in 0..model.n_items() {
        if let Some(section) = model.item_link(index, "section") {
            if *broke {
                menu.addItem(&NSMenuItem::separatorItem(mtm));
                *broke = false;
            }
            append_model(view, addr, &section, menu, mtm, broke);
            continue;
        }
        if let Some(sub) = model.item_link(index, "submenu") {
            let label = model
                .item_attribute_value(index, "label", None)
                .and_then(|value| value.get::<String>())
                .unwrap_or_default();
            let submenu = build_menu(view, addr, &sub, mtm);
            if submenu.numberOfItems() == 0 {
                continue;
            }
            let title = NSString::from_str(&label);
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &title,
                    None,
                    &NSString::new(),
                )
            };
            item.setSubmenu(Some(&submenu));
            submenu.setTitle(&title);
            menu.addItem(&item);
            *broke = true;
            continue;
        }
        let label = model
            .item_attribute_value(index, "label", None)
            .and_then(|value| value.get::<String>());
        let action = model
            .item_attribute_value(index, "action", None)
            .and_then(|value| value.get::<String>());
        let (Some(label), Some(action)) = (label, action) else {
            continue;
        };
        let name = action.strip_prefix("panechord.").unwrap_or(&action);
        let known = view
            .0
            .actions
            .borrow()
            .values()
            .find(|known| known.name == name)
            .map(|known| (known.action.is_enabled(), known.shortcut.clone()));
        let Some((enabled, shortcut)) = known else {
            continue;
        };
        let title = NSString::from_str(&label);
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &title,
                Some(sel!(fire:)),
                &NSString::new(),
            )
        };
        let tgt = target();
        unsafe { item.setTarget(Some(tgt.as_ref())) };
        let ident = NSString::from_str(&format!("{addr}/{name}"));
        unsafe { item.setRepresentedObject(Some(ident.as_ref())) };
        if !enabled {
            item.setEnabled(false);
        }
        if let Some(text) = shortcut.as_deref()
            && let Ok(parsed) = panechord_runtime::Shortcut::parse(text)
            && let Some((flags, key)) = parsed.to_appkit()
        {
            item.setKeyEquivalent(&NSString::from_str(&key));
            item.setKeyEquivalentModifierMask(NSEventModifierFlags(flags as usize));
        }
        menu.addItem(&item);
        *broke = true;
    }
}
