//! Run each GTK test in its own process to preserve initialization-thread affinity.
#[test]
#[ignore = "isolated Xvfb native contract"]
fn native_style_late_close_contract() {
    gtk::init().unwrap();
    let view = View::new(Binding::constant("style contract".into()));
    assert_eq!(
        view.try_style("missing", Style::default()),
        Err("style target not mounted: missing".into())
    );
    let mut invalid = Style::default();
    invalid.placement.x = 1.0;
    assert_eq!(
        view.try_style("__root", invalid),
        Err("x/y placement requires Absolute".into())
    );
    assert_eq!(view.try_style("__root", Style::default()), Ok(()));
    view.label(&view.root(), "fixed", Binding::constant("Label".into()));
    let mut fixed = Style::default();
    fixed.placement.width = Some(120.0);
    view.try_style("fixed", fixed).unwrap();
    let frame = view.0.nodes.borrow().get("fixed").unwrap().frame.clone();
    assert!(!frame.hexpands());
    assert_eq!(frame.width_request(), 120);
    view.try_style("fixed", Style::default()).unwrap();
    assert!(frame.hexpands());
    assert_eq!(frame.width_request(), -1);
    view.try_style(
        "fixed",
        Style {
            horizontal_alignment: Some(Alignment::End),
            vertical_alignment: Some(Alignment::Center),
            ..Style::default()
        },
    )
    .unwrap();
    assert_eq!(frame.halign(), gtk::Align::End);
    assert_eq!(frame.valign(), gtk::Align::Center);
    assert!(!frame.hexpands());
    assert!(!frame.vexpands());
    view.try_style(
        "fixed",
        Style {
            vertical_alignment: Some(Alignment::Stretch),
            ..Style::default()
        },
    )
    .unwrap();
    assert!(frame.vexpands());
    view.try_style("fixed", Style::default()).unwrap();
    assert_eq!(frame.halign(), gtk::Align::Fill);
    assert_eq!(frame.valign(), gtk::Align::Fill);
    assert!(!frame.is_vexpand_set());
    let label = view.widget::<gtk::Label>("fixed").unwrap();
    view.try_style(
        "fixed",
        Style {
            text: Some(panechord_runtime::TextStyle {
                role: panechord_runtime::TextRole::Code,
                scale: 5.0 / 6.0,
                foreground: Some([70, 70, 70]),
            }),
            ..Style::default()
        },
    )
    .unwrap();
    assert!(label.has_css_class("monospace"));
    let attributes = label.attributes().unwrap().to_string();
    assert!(attributes.contains("monospace"));
    assert!(attributes.contains("scale"));
    assert!(attributes.contains("foreground"));
    let invalid = Style {
        text: Some(panechord_runtime::TextStyle {
            scale: f32::NAN,
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(
        view.try_style("fixed", invalid),
        Err("text scale must be finite and within (0, 16]".into())
    );
    assert!(label.has_css_class("monospace"));
    view.try_style(
        "fixed",
        Style {
            text: Some(panechord_runtime::TextStyle {
                role: panechord_runtime::TextRole::Heading,
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(label.has_css_class("heading"));
    assert!(!label.has_css_class("monospace"));
    view.try_style("fixed", Style::default()).unwrap();
    assert!(!label.has_css_class("heading"));
    assert!(label.attributes().is_none());
    assert_eq!(label.text(), "Label");
    view.close();
    assert_eq!(
        view.try_style("__root", Style::default()),
        Err("view is closed".into())
    );
    let called = Rc::new(Cell::new(false));
    let observed = called.clone();
    view.on_close(Rc::new(move || observed.set(true)));
    assert!(called.get());
}

use super::*;
use futures::{FutureExt, channel::oneshot};
use panechord_runtime::{Edges, Placement, Policy, ShutdownOutcome, run_async};
use std::time::Duration;

#[test]
#[ignore = "isolated Xvfb native contract"]
fn native_item_rows_keep_configured_height_with_grid_entries() {
    gtk::init().unwrap();
    let host = GtkHost::attach();
    let view = View::new(Binding::constant("rows".into()));
    let rows: Vec<_> = (0..8)
        .map(|index| Keyed::new(index.to_string(), Store::new(index, host.executor())))
        .collect();
    view.items(
        &view.root(),
        "rows",
        Binding::constant(rows),
        Binding::constant(Viewport {
            count: 8,
            row_height: 34.0,
            ..Viewport::default()
        }),
        Rc::new(|view, parent, scope, _| {
            let grid = view.layout(
                parent,
                &format!("{scope}/grid"),
                Layout::Grid {
                    columns: 2,
                    gap: 12.0,
                },
            );
            view.label(
                &grid,
                &format!("{scope}/label"),
                Binding::constant("Toggle Pause".into()),
            );
            view.entry(
                &grid,
                &format!("{scope}/entry"),
                Binding::constant("Space".into()),
            );
        }),
    );
    let scroll = view.widget::<gtk::ScrolledWindow>("rows").unwrap();
    let container = scroll.child().unwrap().first_child().unwrap();
    let row = container.first_child().unwrap().next_sibling().unwrap();
    let grid = view.widget::<gtk::Grid>("rows/0/grid").unwrap();
    assert_eq!(grid.measure(gtk::Orientation::Vertical, 700).0, 34);
    assert_eq!(row.measure(gtk::Orientation::Vertical, 700).0, 34);
    assert_eq!(row.measure(gtk::Orientation::Vertical, 700).1, 34);
    view.close();
}

#[test]
#[ignore = "isolated Xvfb native contract"]
fn native_dynamic_tabs_preserve_reorder_and_release_removed_pages() {
    gtk::init().unwrap();
    let host = GtkHost::attach();
    let first = Keyed::new("first", Store::new("First".to_owned(), host.executor()));
    let second = Keyed::new("second", Store::new("Second".to_owned(), host.executor()));
    let model = Store::new(
        (vec![first.clone(), second.clone()], "second".to_owned()),
        host.executor(),
    );
    let view = View::new(Binding::constant("dynamic tabs".into()));
    view.tab_items(
        &view.root(),
        "pages",
        model.bind(|state| state.0.clone(), None),
        model.bind(
            |state| state.1.clone(),
            Some(|state, value| state.1 = value),
        ),
        Rc::new(|model| model.bind(Clone::clone, None)),
        Rc::new(|view, parent, scope, model| {
            view.label(
                parent,
                &format!("{scope}/label"),
                model.bind(Clone::clone, None),
            );
        }),
    );
    let notebook = view.widget::<gtk::Notebook>("pages").unwrap();
    let retained = view.widget::<gtk::Label>("pages/second/label").unwrap();
    assert_eq!(notebook.n_pages(), 2);
    assert_eq!(notebook.current_page(), Some(1));
    assert!(model.update(|state| state.0.reverse()));
    assert_eq!(notebook.current_page(), Some(0));
    assert_eq!(
        view.widget::<gtk::Label>("pages/second/label").unwrap(),
        retained
    );
    let duplicate = first.clone();
    assert!(model.update(move |state| state.0.push(duplicate)));
    assert_eq!(notebook.n_pages(), 2);
    let remaining = second.clone();
    assert!(model.update(move |state| state.0 = vec![remaining]));
    assert_eq!(notebook.n_pages(), 1);
    assert!(view.widget::<gtk::Label>("pages/first/label").is_err());
    assert!(first.model.update(|name| *name = "removed".into()));
    assert!(second.model.update(|name| *name = "Retained".into()));
    assert_eq!(retained.label(), "Retained");
    assert!(model.update(|state| state.0.clear()));
    assert_eq!(notebook.n_pages(), 0);
    assert!(model.update(move |state| state.0 = vec![first]));
    assert_eq!(notebook.n_pages(), 1);
    view.close();
    assert!(model.update(|state| state.0.clear()));

    let next_page = Keyed::new("next", Store::new("Next".to_owned(), host.executor()));
    let items = Store::new(vec![next_page.clone()], host.executor());
    let publisher = items.clone();
    let nested_view = View::new(Binding::constant("reentrant tabs".into()));
    nested_view.tab_items(
        &nested_view.root(),
        "nested",
        items.bind(Clone::clone, None),
        Binding::constant("".into()),
        Rc::new(|model| model.bind(Clone::clone, None)),
        Rc::new(move |_, _, _, _| {
            assert!(publisher.update(Vec::clear));
        }),
    );
    assert_eq!(
        nested_view
            .widget::<gtk::Notebook>("nested")
            .unwrap()
            .n_pages(),
        0
    );
    nested_view.close();
}

#[test]
#[ignore = "isolated Xvfb native contract"]
fn native_grid_default_styles_and_scroll_preserve_allocation() {
    gtk::init().unwrap();
    let view = View::new(Binding::constant("Grid and Scroll".into()));
    let viewport = view.try_scroll(&view.root(), "viewport", false).unwrap();
    let grid = view
        .try_layout(
            &viewport,
            "grid",
            Layout::Grid {
                columns: 2,
                gap: 12.0,
            },
        )
        .unwrap();
    let native = grid.widget.downcast_ref::<gtk::Grid>().unwrap();
    for index in 0..5 {
        let id = format!("child-{index}");
        view.label(&grid, &id, Binding::constant(id.clone()));
        for padding in [0.0, 12.0, 0.0] {
            view.try_style(
                &id,
                Style {
                    padding: Edges {
                        top: padding,
                        right: padding,
                        bottom: padding,
                        left: padding,
                    },
                    ..Style::default()
                },
            )
            .unwrap();
            let frame = view.0.nodes.borrow().get(&id).unwrap().frame.clone();
            assert_eq!(native.query_child(&frame), (index % 2, index / 2, 1, 1));
        }
    }
    let scroll = view.widget::<gtk::ScrolledWindow>("viewport").unwrap();
    assert_eq!(scroll.hscrollbar_policy(), gtk::PolicyType::Never);
    assert_eq!(scroll.vscrollbar_policy(), gtk::PolicyType::Automatic);
    assert!(view.try_scroll(&viewport, "viewport", false).is_err());
    view.tab(&view.root(), "tabs", Binding::constant("general".into()));
    view.tab_position("tabs", panechord_runtime::TabPosition::Left)
        .unwrap();
    let notebook = view.widget::<gtk::Notebook>("tabs").unwrap();
    assert_eq!(notebook.first_child().unwrap().width_request(), 120);
    view.tab_position("tabs", panechord_runtime::TabPosition::Top)
        .unwrap();
    assert_eq!(notebook.first_child().unwrap().width_request(), -1);
    view.close();
    assert!(view.try_scroll(&view.root(), "closed", false).is_err());
}

#[test]
#[ignore = "isolated Xvfb native contract"]
fn native_menu_declaration_order_survives_rebuild() {
    gtk::init().unwrap();
    let view = View::new(Binding::constant("Menus".into()));
    for (id, title) in [
        ("file", "File"),
        ("emulation", "Emulation"),
        ("help", "Help"),
    ] {
        view.typed_menu(
            id,
            Binding::constant(title.into()),
            Binding::constant(Vec::<MenuItem>::new()),
            Rc::new(|_| true),
        );
    }
    let titles = || {
        (0..view.0.menus.n_items())
            .map(|index| {
                view.0
                    .menus
                    .item_attribute_value(index, "label", None)
                    .unwrap()
                    .get::<String>()
                    .unwrap()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(titles(), ["File", "Emulation", "Help"]);
    view.0
        .menu_states
        .borrow()
        .get("file")
        .unwrap()
        .title
        .replace("Files".into());
    view.rebuild_menubar();
    assert_eq!(titles(), ["Files", "Emulation", "Help"]);
    view.close();
}

#[cfg(target_os = "linux")]
#[path = "x11_test.rs"]
mod x11;

/// Actual X server key delivery through GTK's own IMContextSimple. No emitted
/// commit signal, direct Editable mutation, ibus/fcitx or Japanese preedit IME.
#[cfg(target_os = "linux")]
#[test]
#[ignore = "isolated Xvfb, GTK_IM_MODULE=simple; optional libXtst (PANECHORD_REQUIRE_XTEST=1 makes absence fail)"]
fn native_x11_japanese_compose_focus_clipboard_and_rejection() {
    let Some(mut keys) = x11::X11::optional() else {
        return;
    };
    gtk::init().expect("requires Xvfb and GDK_BACKEND=x11");
    assert_eq!(std::env::var("GTK_IM_MODULE").as_deref(), Ok("simple"));
    let host = GtkHost::attach();
    let model = Store::new((String::new(), String::new(), 0usize), host.executor());
    let view = View::new(Binding::constant("native X11 input".into()));
    view.entry(
        &view.root(),
        "first",
        model.bind(
            |s| s.0.clone(),
            Some(|s, v| {
                s.0 = v;
                s.2 += 1;
            }),
        ),
    );
    view.entry(
        &view.root(),
        "second",
        model.bind(
            |s| s.1.clone(),
            Some(|s, v| {
                s.1 = v;
                s.2 += 1;
            }),
        ),
    );
    view.entry(
        &view.root(),
        "readonly",
        Binding::constant("読取専用".into()),
    );
    let number = Store::new(12u32, host.executor());
    view.entry(
        &view.root(),
        "number",
        number.bind(|s| *s, Some(|s, v| *s = v)).convert(
            |v| v.to_string(),
            |v: String| v.parse::<u32>().map_err(|e| e.to_string()),
        ),
    );
    let first = view.widget::<gtk::Entry>("first").unwrap();
    let second = view.widget::<gtk::Entry>("second").unwrap();
    let readonly = view.widget::<gtk::Entry>("readonly").unwrap();
    let numeric = view.widget::<gtk::Entry>("number").unwrap();
    view.present();
    settle();
    let surface = view.native_window().surface().unwrap();
    assert_eq!(surface.display().type_().name(), "GdkX11Display");
    unsafe extern "C" {
        fn gdk_x11_surface_get_xid(surface: *mut gtk::gdk::ffi::GdkSurface) -> std::ffi::c_ulong;
    }
    // SAFETY: checked X11 backend and live mapped GTK surface.
    keys.focus(unsafe { gdk_x11_surface_get_xid(surface.as_ptr()) });
    assert!(first.grab_focus());
    settle();
    let focused = |entry: &gtk::Entry| {
        let focus = GtkWindowExt::focus(&view.native_window()).unwrap();
        assert!(focus == *entry.upcast_ref::<gtk::Widget>() || focus.is_ancestor(entry));
    };
    focused(&first);
    keys.compose("日本語");
    drive_until(|| model.snapshot().0 == "日本語");
    assert_eq!(first.text(), "日本語");
    assert_eq!(model.snapshot().2, 3, "one model write per Unicode commit");
    keys.chord(&[0xffe3], b'a'.into());
    settle();
    assert_eq!(
        first.selection_bounds(),
        Some((0, 3)),
        "character, not UTF-8 byte offsets"
    );
    keys.chord(&[0xffe3], b'c'.into());
    settle();
    let clipboard = Rc::new(RefCell::new(None));
    let result = clipboard.clone();
    first
        .clipboard()
        .read_text_async(None::<&gio::Cancellable>, move |value| {
            result.replace(Some(value.unwrap().map(|s| s.to_string())));
        });
    drive_until(|| clipboard.borrow().is_some());
    assert_eq!(
        clipboard.borrow().as_ref().unwrap().as_deref(),
        Some("日本語")
    );
    keys.tap(0xff09); // real Tab traversal, not grab_focus(second)
    settle();
    focused(&second);
    keys.chord(&[0xffe3], b'v'.into());
    drive_until(|| model.snapshot().1 == "日本語");
    keys.tap(0xff08); // Backspace removes one Japanese character.
    drive_until(|| model.snapshot().1 == "日本");
    keys.chord(&[0xffe1], 0xff51); // Shift+Left selects 本.
    settle();
    assert_eq!(second.selection_bounds(), Some((1, 2)));
    keys.tap(0xffff); // Delete selected character.
    drive_until(|| model.snapshot().1 == "日");
    assert_eq!(
        model.snapshot().0,
        "日本語",
        "focus routes edits to second only"
    );
    keys.tap(0xff09);
    settle();
    focused(&readonly);
    keys.chord(&[0xffe3], b'a'.into());
    keys.chord(&[0xffe3], b'v'.into());
    keys.tap(0xff08);
    keys.compose("無効");
    settle();
    assert_eq!(readonly.text(), "読取専用");
    assert_eq!(
        (model.snapshot().0, model.snapshot().1),
        ("日本語".into(), "日".into())
    );
    keys.tap(0xff09);
    settle();
    focused(&numeric);
    keys.chord(&[0xffe3], b'a'.into());
    keys.chord(&[0xffe3], b'v'.into());
    settle();
    assert_eq!(
        numeric.text(),
        "12",
        "Japanese clipboard fails numeric conversion and rolls back"
    );
    assert_eq!(number.snapshot(), 12);
    view.close();
    host.close();
}

#[test]
fn native_close_requests_weak_and_reentrant() {
    gtk::init().expect("native_close_requests_weak_and_reentrant requires Xvfb");
    let view = View::new(Binding::constant("close requests".into()));
    view.menu(
        "file",
        Binding::constant("File".into()),
        Binding::constant(vec![menu_item("quit", MenuRole::Quit, None)]),
        Rc::new(|_| panic!("Quit must not dispatch an app argument")),
    );
    let weak = view.downgrade();
    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    let owner = weak.clone();
    view.on_close_request(Rc::new(move || {
        count.set(count.get() + 1);
        let view = owner.upgrade().unwrap();
        view.request_close();
        view.activate_menu("quit").unwrap();
        view.native_window().close();
    }));
    let closed = Rc::new(Cell::new(0));
    let count = closed.clone();
    let owner = weak.clone();
    view.on_close(Rc::new(move || {
        assert!(owner.upgrade().is_none());
        count.set(count.get() + 1);
    }));
    view.present();
    settle();
    view.native_window().close();
    view.activate_menu("quit").unwrap();
    assert_eq!(
        calls.get(),
        2,
        "nested requests are ignored, later ones retry"
    );
    assert_eq!(closed.get(), 0);
    assert!(view.native_window().is_mapped());
    let count = calls.clone();
    view.on_close_request(Rc::new(move || {
        count.set(count.get() + 10);
        panic!("test handler");
    }));
    view.request_close();
    view.request_close();
    assert_eq!(calls.get(), 22, "panic must release request gate");
    assert!(
        view.0
            .error
            .borrow()
            .as_ref()
            .unwrap()
            .contains("test handler")
    );
    view.close();
    view.close();
    view.request_close();
    assert_eq!(closed.get(), 1);
    assert_eq!(calls.get(), 22);
    assert!(weak.upgrade().is_none());
    assert!(!view.native_window().is_mapped());
    drop(view);
    let dropped = View::new(Binding::constant("drop".into()));
    let weak = dropped.downgrade();
    let owner = weak.clone();
    dropped.on_close_request(Rc::new(move || {
        let _ = owner.upgrade();
    }));
    drop(dropped);
    assert!(weak.upgrade().is_none());
    let default = View::new(Binding::constant("default".into()));
    default.present();
    default.request_close();
    assert!(default.0.closed.get());
    assert!(!default.native_window().is_mapped());
}

#[test]
fn native_close_ready_subscription_retains_model_without_view_cycle() {
    gtk::init()
        .expect("native_close_ready_subscription_retains_model_without_view_cycle requires Xvfb");
    let host = GtkHost::attach();
    let model = Store::new((0usize, false), host.executor());
    let view = View::new(Binding::constant("settings".into()));
    let owner = view.downgrade();
    view.keep(model.bind(|s| s.1, None).subscribe(Rc::new(move |ready| {
        if ready && let Some(view) = owner.upgrade() {
            view.close();
        }
    })));
    let command = model.command(
        |_| true,
        |store, ()| {
            let _ = store.update(|s| s.0 += 1);
        },
        Binding::constant(()),
    );
    view.on_close_request(Rc::new(move || {
        let _ = command.invoke();
    }));
    let closes = Rc::new(Cell::new(0));
    let count = closes.clone();
    let store = model.clone();
    view.on_close(Rc::new(move || {
        count.set(count.get() + 1);
        store.close();
    }));
    view.present();
    view.native_window().close();
    assert_eq!(model.snapshot(), (1, false));
    assert!(!model.is_closed());
    assert!(view.native_window().is_mapped());
    let _ = model.update(|s| s.1 = true);
    assert_eq!(closes.get(), 1);
    assert!(model.is_closed());
    assert!(view.0.resources.borrow().is_empty());
    assert!(!view.native_window().is_mapped());
    let weak = Rc::downgrade(&view.0);
    drop(view);
    assert!(weak.upgrade().is_none());
    host.close();
}

#[test]
fn native_close_request_handler_precedes_coordinator_and_menu_falls_back() {
    gtk::init().expect(
        "native_close_request_handler_precedes_coordinator_and_menu_falls_back requires Xvfb",
    );
    let host = GtkHost::attach();
    let session = Session::new();
    let view = View::new(Binding::constant("app handler".into()));
    host.coordinate_close(&view, &session, Duration::from_secs(30))
        .unwrap();
    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    view.on_close_request(Rc::new(move || count.set(count.get() + 1)));
    view.present();
    view.native_window().close();
    assert_eq!(calls.get(), 1);
    assert!(host.executor.tasks.borrow().is_empty());
    assert_eq!(session.shutdown().phase(), ShutdownPhase::Running);
    view.close();

    let probe = Rc::new(CloseProbe::default());
    let _work = session.register_window("work", probe.hooks()).unwrap();
    let view = View::new(Binding::constant("coordinator fallback".into()));
    host.coordinate_close(&view, &session, Duration::from_secs(30))
        .unwrap();
    view.menu(
        "file",
        Binding::constant("File".into()),
        Binding::constant(vec![menu_item("quit", MenuRole::Quit, None)]),
        Rc::new(|_| false),
    );
    view.present();
    view.activate_menu("quit").unwrap();
    assert_eq!(host.executor.tasks.borrow().len(), 1);
    reap_all(&host.executor);
    assert_eq!(probe.confirms.get(), 1);
    assert!(
        !view.0.closed.get(),
        "denial leaves menu-requested View open"
    );
    view.close();
    host.close();
}

#[test]
fn native_canonical_shortcut_semantics() {
    gtk::init().expect("native_canonical_shortcut_semantics requires Xvfb");
    use panechord_runtime::ShortcutTarget;
    for (input, key, extra) in [
        (
            "Primary+S",
            gtk::gdk::Key::s,
            gtk::gdk::ModifierType::empty(),
        ),
        (
            "Primary+Q",
            gtk::gdk::Key::q,
            gtk::gdk::ModifierType::empty(),
        ),
        (
            "Primary+Shift+S",
            gtk::gdk::Key::s,
            gtk::gdk::ModifierType::SHIFT_MASK,
        ),
    ] {
        for target in [ShortcutTarget::MacOS, ShortcutTarget::Other] {
            let shortcut = Shortcut::parse(input).unwrap();
            let trigger = gtk::ShortcutTrigger::parse_string(&shortcut.to_gtk_for(target))
                .unwrap()
                .downcast::<gtk::KeyvalTrigger>()
                .unwrap();
            let primary = if target == ShortcutTarget::MacOS {
                gtk::gdk::ModifierType::META_MASK
            } else {
                gtk::gdk::ModifierType::CONTROL_MASK
            };
            assert_eq!(trigger.keyval(), key);
            assert_eq!(trigger.modifiers(), primary | extra);
        }
        assert_eq!(
            parse_shortcut(input).unwrap().to_string(),
            gtk::ShortcutTrigger::parse_string(&Shortcut::parse(input).unwrap().to_gtk())
                .unwrap()
                .to_string()
        );
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
            parse_shortcut(&format!("Ctrl+Shift+{key}")).is_some(),
            "{key}"
        );
    }
    assert_eq!(
        parse_shortcut("Ctrl+Shift+S").unwrap().to_string(),
        parse_shortcut("<Control><Shift>s").unwrap().to_string()
    );
    // Install and activate real canonical menu shortcuts, not just string output.
    let view = View::new(Binding::constant("shortcuts".into()));
    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    view.menu(
        "file",
        Binding::constant("File".into()),
        Binding::constant(vec![
            menu_item("save", MenuRole::Normal, Some("Primary+S")),
            menu_item("quit", MenuRole::Quit, Some("Primary+Q")),
        ]),
        Rc::new(move |_| {
            count.set(count.get() + 1);
            true
        }),
    );
    let requests = Rc::new(Cell::new(0));
    let count = requests.clone();
    view.on_close_request(Rc::new(move || count.set(count.get() + 1)));
    let state = view.0.menu_states.borrow()["file"].clone();
    let shortcuts = state.shortcuts.borrow().clone();
    assert_eq!(shortcuts.len(), 2);
    for shortcut in shortcuts {
        assert!(shortcut.action().unwrap().activate(
            gtk::ShortcutActionFlags::empty(),
            &view.0.window,
            None
        ));
    }
    assert_eq!(calls.get(), 1);
    assert_eq!(requests.get(), 1);
    assert!(!view.0.closed.get());
    view.close();
}

#[test]
fn native_grid_equal_columns_geometry_and_spans() {
    gtk::init().expect("native_grid_equal_columns_geometry_and_spans requires Xvfb");
    for width in [420, 660] {
        let view = View::new(Binding::constant("equal columns".into()));
        view.native_window().set_default_size(width, 300);
        let parent = view.layout(
            &view.root(),
            "grid",
            Layout::Grid {
                columns: 3,
                gap: 7.0,
            },
        );
        for (i, text) in ["a", "longer label", "c", "spanning"]
            .into_iter()
            .enumerate()
        {
            let id = format!("g{i}");
            view.label(&parent, &id, Binding::constant(text.into()));
            view.style(
                &id,
                Style {
                    placement: Placement {
                        row: if i == 3 { 2 } else { 0 },
                        column: if i == 3 { 0 } else { i as i32 },
                        column_span: if i == 3 { 2 } else { 1 },
                        row_span: if i == 3 { 2 } else { 1 },
                        ..Placement::default()
                    },
                    ..Style::default()
                },
            );
        }
        view.present();
        settle();
        let grid = parent.widget.downcast_ref::<gtk::Grid>().unwrap();
        assert!(grid.is_mapped());
        assert!(grid.is_column_homogeneous());
        assert!(grid.hexpands());
        if let Ok(scale) = std::env::var("GDK_SCALE") {
            assert_eq!(grid.scale_factor(), scale.parse::<i32>().unwrap());
        }
        let nodes = view.0.nodes.borrow();
        let frames: Vec<_> = (0..4)
            .map(|i| nodes[&format!("g{i}")].frame.clone())
            .collect();
        let bounds: Vec<_> = frames
            .iter()
            .map(|frame| frame.compute_bounds(grid).unwrap())
            .collect();
        let cell = (grid.width() as f32 - 14.0) / 3.0;
        for i in 0..3 {
            assert!(
                (bounds[i].width() - cell).abs() <= 1.0,
                "{width}: {bounds:?}"
            );
            if i > 0 {
                assert!(
                    (bounds[i].x() - bounds[i - 1].x() - bounds[i - 1].width() - 7.0).abs() <= 1.0
                );
            }
        }
        assert_eq!(grid.query_child(&frames[3]), (0, 2, 2, 2));
        assert!((bounds[3].width() - (2.0 * cell + 7.0)).abs() <= 1.0);
        assert!(bounds[3].y() > bounds[0].y());
        assert!(
            (grid.width() - view.root().widget.width()).abs() <= 1,
            "fills root width"
        );
        drop(nodes);
        view.close();
    }
    // Unused declared tracks must still take their equal share of available width.
    let view = View::new(Binding::constant("empty tracks".into()));
    let grid = view.layout(
        &view.root(),
        "grid",
        Layout::Grid {
            columns: 3,
            gap: 7.0,
        },
    );
    view.label(&grid, "only", Binding::constant("one".into()));
    view.present();
    settle();
    let frame = view.0.nodes.borrow()["only"].frame.clone();
    assert!((frame.width() as f32 - (grid.widget.width() as f32 - 14.0) / 3.0).abs() <= 1.0);
    view.close();
}

// Drive real native sources, not a blocking futures executor. The watchdog is
// only a failure bound; test sequencing uses phases, oneshots and task reaping.
fn drive_until(done: impl Fn() -> bool) {
    let expired = Rc::new(Cell::new(false));
    let flag = expired.clone();
    let watchdog = glib::timeout_add_local_once(Duration::from_secs(5), move || flag.set(true));
    let context = glib::MainContext::default();
    while !done() && !expired.get() {
        context.iteration(true);
    }
    if !expired.get() {
        watchdog.remove();
    }
    assert!(!expired.get(), "native shutdown did not make progress");
}

#[derive(Default)]
struct CloseProbe {
    allow: Cell<bool>,
    confirms: Cell<usize>,
    cancels: Cell<usize>,
    drains: Cell<usize>,
    closes: Cell<usize>,
    confirm: RefCell<Option<LocalBoxFuture<'static, bool>>>,
    drain: RefCell<Option<LocalBoxFuture<'static, ()>>>,
}

impl CloseProbe {
    fn hooks(self: &Rc<Self>) -> WindowHooks {
        let confirm = self.clone();
        let cancel = self.clone();
        let drain = self.clone();
        let close = self.clone();
        WindowHooks {
            confirm: Rc::new(move || {
                confirm.confirms.set(confirm.confirms.get() + 1);
                confirm.confirm.take().unwrap_or_else(|| {
                    let allow = confirm.allow.get();
                    Box::pin(async move { allow })
                })
            }),
            cancel: Rc::new(move || cancel.cancels.set(cancel.cancels.get() + 1)),
            drain: Rc::new(move || {
                drain.drains.set(drain.drains.get() + 1);
                drain.drain.take().unwrap_or_else(|| Box::pin(async {}))
            }),
            close: Rc::new(move || close.closes.set(close.closes.get() + 1)),
        }
    }
}

fn coordinated_pair(host: &GtkHost, session: &Session, timeout: Duration) -> [View; 2] {
    let views = ["one", "two"].map(|id| {
        let view = View::new(Binding::constant(id.into()));
        host.register_view(session, id, &view).unwrap();
        host.coordinate_close(&view, session, timeout).unwrap();
        view
    });
    UiHost::attach(host, session.clone()).unwrap();
    assert!(views.iter().all(|v| v.native_window().is_mapped()));
    views
}

#[test]
fn native_coordinated_multiwindow_close() {
    gtk::init().expect("native_coordinated_multiwindow_close requires Xvfb");
    let host = GtkHost::attach();
    let session = Session::new();
    let probe = Rc::new(CloseProbe::default());
    let _work = session.register_window("0-work", probe.hooks()).unwrap();
    let views = coordinated_pair(&host, &session, Duration::from_secs(30));
    assert_eq!(session.window_count(), 3, "custom hooks were not replaced");
    let closed = Rc::new(Cell::new(0));
    for view in &views {
        let count = closed.clone();
        view.on_close(Rc::new(move || count.set(count.get() + 1)));
    }

    views[0].native_window().close();
    views[0].native_window().close();
    assert_eq!(host.executor.tasks.borrow().len(), 1, "one queued request");
    assert_eq!(probe.confirms.get(), 0, "confirmation is asynchronous");
    reap_all(&host.executor);
    assert_eq!(probe.confirms.get(), 1);
    assert_eq!(probe.cancels.get(), 0);
    assert_eq!(closed.get(), 0);
    assert_eq!(session.shutdown().phase(), ShutdownPhase::Running);
    assert!(
        views
            .iter()
            .all(|v| !v.0.closed.get() && v.native_window().is_mapped())
    );

    probe.allow.set(true);
    let (release, drain) = oneshot::channel();
    probe
        .drain
        .replace(Some(Box::pin(async move { drain.await.unwrap() })));
    views[1].native_window().close();
    drive_until(|| session.shutdown().phase() == ShutdownPhase::Draining);
    assert_eq!(probe.confirms.get(), 2);
    assert_eq!(probe.cancels.get(), 1);
    assert_eq!(probe.drains.get(), 1);
    assert_eq!(probe.closes.get(), 0);
    for view in &views {
        view.native_window().close();
    }
    assert_eq!(
        host.executor.tasks.borrow().len(),
        1,
        "busy native requests ignored"
    );

    // A native idle callback executes while drain is pending and both windows
    // remain mapped. Only that UI turn releases the application's oneshot.
    let live = views.each_ref().map(|v| Rc::downgrade(&v.0));
    let ui_turn = Rc::new(Cell::new(false));
    let flag = ui_turn.clone();
    glib::idle_add_local_once(move || {
        for view in live {
            let view = view.upgrade().unwrap();
            assert!(!view.closed.get());
            assert!(view.window.is_mapped());
        }
        flag.set(true);
        release.send(()).unwrap();
    });
    drive_until(|| session.shutdown().phase() == ShutdownPhase::Closed);
    reap_all(&host.executor);
    assert!(ui_turn.get());
    assert_eq!(probe.confirms.get(), 2);
    assert_eq!(
        (probe.cancels.get(), probe.drains.get(), probe.closes.get()),
        (1, 1, 1)
    );
    assert_eq!(closed.get(), 2);
    assert_eq!(session.window_count(), 0);
    assert!(host.windows.borrow().is_empty());
    assert!(host.sessions.borrow().is_empty());
    for view in &views {
        assert!(view.0.closed.get());
        assert!(!view.native_window().is_mapped());
        view.close();
    }
    session.shutdown().close_now();
    assert_eq!(
        closed.get(),
        2,
        "force close never recurses or repeats callbacks"
    );
}

#[test]
fn native_coordinated_close_timeout() {
    gtk::init().expect("native_coordinated_close_timeout requires Xvfb");
    let host = GtkHost::attach();
    let session = Session::new();
    let probe = Rc::new(CloseProbe::default());
    let (answer, response) = oneshot::channel::<()>();
    probe.confirm.replace(Some(Box::pin(async move {
        response.await.unwrap();
        true
    })));
    let (send, receive) = oneshot::channel::<()>();
    probe.drain.replace(Some(Box::pin(async move {
        receive.await.unwrap();
    })));
    let _work = session.register_window("0-work", probe.hooks()).unwrap();
    let views = coordinated_pair(&host, &session, Duration::ZERO);
    let closed = Rc::new(Cell::new(0));
    for view in &views {
        let count = closed.clone();
        view.on_close(Rc::new(move || count.set(count.get() + 1)));
    }
    views[0].native_window().close();
    assert_eq!(closed.get(), 0, "no synchronous teardown");
    drive_until(|| session.shutdown().phase() == ShutdownPhase::Confirming);
    let waited = Rc::new(Cell::new(false));
    let elapsed = waited.clone();
    glib::timeout_add_local_once(Duration::from_millis(10), move || elapsed.set(true));
    drive_until(|| waited.get());
    assert_eq!(session.shutdown().phase(), ShutdownPhase::Confirming);
    assert_eq!((probe.cancels.get(), probe.closes.get()), (0, 0));
    assert_eq!(closed.get(), 0);
    assert!(views.iter().all(|view| view.native_window().is_mapped()));
    answer.send(()).unwrap();
    drive_until(|| session.shutdown().phase() == ShutdownPhase::Closed);
    reap_all(&host.executor);
    assert!(send.is_canceled(), "deadline dropped pending drain work");
    assert_eq!((probe.confirms.get(), probe.drains.get()), (1, 1));
    assert_eq!((probe.cancels.get(), probe.closes.get()), (1, 1));
    assert_eq!(closed.get(), 2);
    assert!(
        views
            .iter()
            .all(|view| view.0.closed.get() && !view.native_window().is_mapped())
    );
    assert_eq!(session.window_count(), 0);
    session.shutdown().close_now();
    assert_eq!(closed.get(), 2);
}

#[test]
fn native_coordinated_close_cancel() {
    gtk::init().expect("native_coordinated_close_cancel requires Xvfb");
    let host = GtkHost::attach();
    let session = Session::new();
    let probe = Rc::new(CloseProbe::default());
    probe.allow.set(true);
    let _work = session.register_window("0-work", probe.hooks()).unwrap();
    let views = coordinated_pair(&host, &session, Duration::from_secs(30));
    let (confirm, confirmed) = oneshot::channel();
    probe
        .confirm
        .replace(Some(Box::pin(async move { confirmed.await.unwrap() })));

    // An external request can win after native close queues its task. Runtime
    // Busy must release that task's gate, not force-close or cancel the winner.
    views[0].native_window().close();
    let shutdown = session.shutdown();
    let mut external = Box::pin(shutdown.request_shutdown(Duration::from_secs(30)));
    assert!(external.as_mut().now_or_never().is_none());
    reap_all(&host.executor);
    assert_eq!(probe.confirms.get(), 1);
    assert_eq!(shutdown.phase(), ShutdownPhase::Confirming);
    assert!(shutdown.cancel_shutdown());
    assert_eq!(external.now_or_never(), Some(ShutdownOutcome::Cancelled));
    assert!(confirm.is_canceled());
    assert_eq!(probe.cancels.get(), 0);
    assert!(
        views
            .iter()
            .all(|v| !v.0.closed.get() && v.native_window().is_mapped())
    );

    let (release, drained) = oneshot::channel::<()>();
    probe
        .drain
        .replace(Some(Box::pin(async move { drained.await.unwrap() })));
    views[0].native_window().close();
    drive_until(|| shutdown.phase() == ShutdownPhase::Draining);
    assert!(shutdown.cancel_shutdown());
    reap_all(&host.executor);
    assert!(release.is_canceled());
    assert_eq!(shutdown.phase(), ShutdownPhase::Running);
    assert_eq!(probe.closes.get(), 0);
    assert!(
        views
            .iter()
            .all(|v| !v.0.closed.get() && v.native_window().is_mapped())
    );
    views[0].native_window().close();
    drive_until(|| shutdown.phase() == ShutdownPhase::Closed);
    reap_all(&host.executor);
    assert_eq!(probe.confirms.get(), 3);
    assert_eq!((probe.cancels.get(), probe.closes.get()), (2, 1));
    assert!(views.iter().all(|v| v.0.closed.get()));
}

#[test]
fn native_coordinated_close_weak_teardown() {
    gtk::init().expect("native_coordinated_close_weak_teardown requires Xvfb");
    for drop_host in [false, true] {
        let host = GtkHost::attach();
        let session = Session::new();
        let probe = Rc::new(CloseProbe::default());
        let (send, receive) = oneshot::channel::<bool>();
        probe
            .confirm
            .replace(Some(Box::pin(async move { receive.await.unwrap() })));
        let _work = session.register_window("work", probe.hooks()).unwrap();
        // Deliberately not register_view: only the caller owns this View.
        let view = View::new(Binding::constant("weak close".into()));
        host.coordinate_close(&view, &session, Duration::from_secs(30))
            .unwrap();
        assert!(
            host.coordinate_close(&view, &session, Duration::ZERO)
                .is_err()
        );
        view.present();
        let native = view.native_window();
        let weak_view = Rc::downgrade(&view.0);
        let weak_executor = Rc::downgrade(&host.executor);
        native.close();
        drive_until(|| session.shutdown().phase() == ShutdownPhase::Confirming);
        if drop_host {
            drop(host);
            assert!(weak_executor.upgrade().is_none());
            assert_eq!(session.shutdown().phase(), ShutdownPhase::Running);
            assert!(send.is_canceled());
            native.close(); // A closed executor rejects gracefully, not synchronously.
            assert!(!view.0.closed.get());
            assert!(native.is_mapped());
            view.close();
            drop(view);
        } else {
            drop(view);
            assert!(
                weak_view.upgrade().is_none(),
                "request must not retain View"
            );
            reap_all(&host.executor);
            assert_eq!(session.shutdown().phase(), ShutdownPhase::Running);
            assert!(send.is_canceled());
        }
        assert!(weak_view.upgrade().is_none());
        assert_eq!((probe.cancels.get(), probe.closes.get()), (0, 0));
    }
}

fn reap_all(executor: &GtkExecutor) {
    for _ in 0..256 {
        if executor.tasks.borrow().is_empty() && executor.reap.borrow().is_none() {
            return;
        }
        executor.context.iteration(false);
    }
    panic!("completion cleanup did not finish without another spawn or a timer");
}

#[test]
fn native_executor_completion() {
    gtk::init().expect("native_executor_completion requires a display: run under Xvfb");
    let host = GtkHost::attach();
    assert!(
        host.executor.reap.borrow().is_none(),
        "idle host has no cleanup source"
    );
    let drops = Rc::new(Cell::new(0));
    for _ in 0..128 {
        let count = drops.clone();
        let owner = Rc::downgrade(&host.executor);
        let token = Subscription::new(move || {
            let executor = owner.upgrade().unwrap();
            assert!(
                executor.tasks.try_borrow_mut().is_ok(),
                "Source destructor outside task borrow"
            );
            assert!(
                executor.reap.try_borrow_mut().is_ok(),
                "Source destructor outside callback borrow"
            );
            count.set(count.get() + 1);
        });
        // Keep the capture in the completed future until GLib finalizes its Source.
        host.executor.spawn(Box::pin(std::future::poll_fn(move |_| {
            let _ = &token;
            std::task::Poll::Ready(())
        })));
    }
    reap_all(&host.executor);
    assert_eq!(drops.get(), 128);
    assert!(host.executor.completed.borrow().is_empty());

    // An externally woken task must be reaped without any subsequent spawn.
    let (send, receive) = futures::channel::oneshot::channel::<()>();
    host.executor.spawn(Box::pin(async move {
        receive.await.unwrap();
    }));
    host.executor.context.iteration(false);
    assert_eq!(host.executor.tasks.borrow().len(), 1);
    assert!(
        host.executor.reap.borrow().is_none(),
        "pending work must not schedule polling"
    );
    send.send(()).unwrap();
    reap_all(&host.executor);

    // GLib can drop application captures during its Ready poll, before the idle
    // callback. Destructors may spawn more work without borrowing bookkeeping.
    let owner = Rc::downgrade(&host.executor);
    let count = drops.clone();
    let token = Subscription::new(move || {
        let executor = owner.upgrade().unwrap();
        assert!(executor.tasks.try_borrow_mut().is_ok());
        assert!(executor.reap.try_borrow_mut().is_ok());
        executor.spawn(Box::pin(async move {
            count.set(count.get() + 1);
        }));
    });
    host.executor.spawn(Box::pin(std::future::poll_fn(move |_| {
        let _ = &token;
        std::task::Poll::Ready(())
    })));
    reap_all(&host.executor);
    assert_eq!(drops.get(), 129);

    // Closing inside a completed future's destructor cancels pending work safely.
    let marker = Rc::new(());
    let weak_marker = Rc::downgrade(&marker);
    host.executor.spawn(Box::pin(async move {
        let _marker = marker;
        std::future::pending::<()>().await;
    }));
    let owner = Rc::downgrade(&host.executor);
    let token = Subscription::new(move || owner.upgrade().unwrap().close());
    host.executor.spawn(Box::pin(std::future::poll_fn(move |_| {
        let _ = &token;
        std::task::Poll::Ready(())
    })));
    reap_all(&host.executor);
    assert!(host.executor.closed.get());
    assert!(weak_marker.upgrade().is_none());

    // Drop while a one-shot cleanup is queued: no stale SourceId or strong cycle.
    let host = GtkHost::attach();
    host.executor.spawn(Box::pin(async {}));
    for _ in 0..256 {
        if host.executor.reap.borrow().is_some() {
            break;
        }
        host.executor.context.iteration(false);
    }
    assert!(host.executor.reap.borrow().is_some());
    let weak = Rc::downgrade(&host.executor);
    drop(host);
    assert!(
        weak.upgrade().is_none(),
        "tasks and callback must not retain executor"
    );
}

fn settle() {
    let main = glib::MainLoop::new(None, false);
    let done = main.clone();
    glib::timeout_add_local_once(Duration::from_millis(40), move || done.quit());
    main.run();
}

#[derive(Clone)]
struct Leaf {
    text: String,
}

#[test]
fn native_entry_converted_replacement() {
    gtk::init().expect("native_entry_converted_replacement requires Xvfb");
    // GTK replaces text by deleting and inserting, but coalesces text notify.
    let probe = gtk::Entry::new();
    probe.set_text("0.5");
    let changed = Rc::new(RefCell::new(Vec::new()));
    let seen = changed.clone();
    probe.connect_changed(move |entry| seen.borrow_mut().push(entry.text().to_string()));
    let notified = Rc::new(RefCell::new(Vec::new()));
    let seen = notified.clone();
    probe.connect_text_notify(move |entry| seen.borrow_mut().push(entry.text().to_string()));
    probe.set_text("0.75");
    assert_eq!(&*changed.borrow(), &["", "0.75"]);
    assert_eq!(&*notified.borrow(), &["0.75"]);

    let host = GtkHost::attach();
    let model = Store::new((0.5_f64, 0_usize), host.executor());
    let conversions = Rc::new(RefCell::new(Vec::new()));
    let seen = conversions.clone();
    let text = model
        .bind(
            |s| s.0,
            Some(|s, value| {
                s.0 = value;
                s.1 += 1;
            }),
        )
        .convert(
            |value| value.to_string(),
            move |text: String| {
                seen.borrow_mut().push(text.clone());
                text.parse::<f64>().map_err(|error| error.to_string())
            },
        );
    let view = View::new(Binding::constant("Converted entry".into()));
    view.entry(&view.root(), "number", text);
    let entry = view.widget::<gtk::Entry>("number").unwrap();
    let editable = entry.upcast_ref::<gtk::Editable>();
    assert_eq!(editable.text(), "0.5");
    assert_eq!(model.snapshot(), (0.5, 0));
    assert!(
        conversions.borrow().is_empty(),
        "initial value must not echo"
    );

    // No main-loop iteration: replacement and rollback must finish synchronously.
    editable.set_text("0.75");
    assert_eq!(model.snapshot(), (0.75, 1));
    assert_eq!(editable.text(), "0.75");
    assert_eq!(&*conversions.borrow(), &["0.75"]);
    editable.set_text("not a number");
    assert_eq!(model.snapshot(), (0.75, 1));
    assert_eq!(editable.text(), "0.75");
    editable.set_text("");
    assert_eq!(model.snapshot(), (0.75, 1));
    assert_eq!(editable.text(), "0.75");
    // Accepted, but not already canonical; normalization must not write twice.
    editable.set_text("0.2500");
    assert_eq!(model.snapshot(), (0.25, 2));
    assert_eq!(editable.text(), "0.25");
    // Same typed value produces no source notification, but still normalizes.
    editable.set_text("0.250");
    assert_eq!(model.snapshot(), (0.25, 3));
    assert_eq!(editable.text(), "0.25");
    let _ = model.update(|s| s.0 = 0.5);
    assert_eq!(model.snapshot(), (0.5, 3));
    assert_eq!(editable.text(), "0.5");
    assert_eq!(
        &*conversions.borrow(),
        &["0.75", "not a number", "", "0.2500", "0.250"]
    );
    view.edit("number", "0.75").unwrap();
    assert_eq!(model.snapshot(), (0.75, 4));
    assert_eq!(editable.text(), "0.75");
    editable.insert_text("0", &mut 4);
    assert_eq!(model.snapshot(), (0.75, 5));
    assert_eq!(editable.text(), "0.75");
    editable.delete_text(3, 4);
    assert_eq!(model.snapshot(), (0.7, 6));
    assert_eq!(editable.text(), "0.7");
    editable.insert_text("x", &mut 3);
    assert_eq!(model.snapshot(), (0.7, 6));
    assert_eq!(editable.text(), "0.7");
    editable.delete_text(0, -1);
    assert_eq!(model.snapshot(), (0.7, 6));
    assert_eq!(editable.text(), "0.7");
    let _ = model.update(|s| s.0 = 0.75);
    assert_eq!(model.snapshot(), (0.75, 6));
    assert_eq!(editable.text(), "0.75");
    model.close();
    editable.set_text("0.9");
    assert_eq!(model.snapshot(), (0.75, 6));
    assert_eq!(editable.text(), "0.75", "closed writer also rolls back");
    view.close();
    editable.set_text("detached");
    assert_eq!(editable.text(), "detached", "shutdown disconnects input");
    assert_eq!(model.snapshot(), (0.75, 6));
    host.close();
}

#[test]
fn native_entry_incremental_edits_and_readonly() {
    gtk::init().expect("native_entry_incremental_edits_and_readonly requires Xvfb");
    let host = GtkHost::attach();
    let model = Store::new(("ab".to_owned(), 0_usize), host.executor());
    let view = View::new(Binding::constant("Entry edits".into()));
    view.entry(
        &view.root(),
        "text",
        model.bind(
            |s| s.0.clone(),
            Some(|s, value| {
                s.0 = value;
                s.1 += 1;
            }),
        ),
    );
    let entry = view.widget::<gtk::Entry>("text").unwrap();
    assert_eq!(model.snapshot().1, 0);
    let mut position = 2;
    entry.insert_text("c", &mut position);
    assert_eq!(position, 3);
    assert_eq!(model.snapshot(), ("abc".into(), 1));
    entry.delete_text(1, 2);
    assert_eq!(model.snapshot(), ("ac".into(), 2));
    entry.delete_text(0, -1);
    assert_eq!(model.snapshot(), ("".into(), 3), "empty String is valid");
    entry.insert_text("typed", &mut 0);
    assert_eq!(model.snapshot(), ("typed".into(), 4));
    view.edit("text", "replacement").unwrap();
    assert_eq!(model.snapshot(), ("replacement".into(), 5));
    let _ = model.update(|s| s.0 = "model update".into());
    assert_eq!(entry.text(), "model update");
    assert_eq!(model.snapshot().1, 5, "model updates must not echo");

    view.entry(&view.root(), "readonly", Binding::constant("fixed".into()));
    let readonly = view.widget::<gtk::Entry>("readonly").unwrap();
    assert!(!readonly.is_editable());
    assert!(view.edit("readonly", "invalid").is_err());
    readonly.set_text("native replacement");
    assert_eq!(readonly.text(), "fixed");
    readonly.delete_text(0, -1);
    assert_eq!(readonly.text(), "fixed");
    view.cleanup("text");
    entry.set_text("evicted");
    assert_eq!(model.snapshot(), ("model update".into(), 5));
    view.close();
    host.close();
}

#[test]
fn native_invalid_keyed_snapshots() {
    gtk::init().expect("native_invalid_keyed_snapshots requires a display: run under Xvfb");
    let host = GtkHost::attach();
    let first = Keyed::new(
        "first",
        Store::new(
            Leaf {
                text: "first".into(),
            },
            host.executor(),
        ),
    );
    let second = Keyed::new(
        "second",
        Store::new(
            Leaf {
                text: "second".into(),
            },
            host.executor(),
        ),
    );
    let model = Store::new(
        (vec![first.clone(), second.clone()], Some(first.clone())),
        host.executor(),
    );
    let view = View::new(Binding::constant("key validation".into()));
    let builds = Rc::new(Cell::new(0));
    let count = builds.clone();
    let template: Template<Leaf> = Rc::new(move |view, parent, prefix, leaf| {
        count.set(count.get() + 1);
        view.label(
            parent,
            &format!("{prefix}/text"),
            leaf.bind(|s| s.text.clone(), None),
        );
    });
    let viewport = Store::new(
        Viewport {
            count: 0,
            row_height: 32.0,
            start: 0,
        },
        host.executor(),
    );
    view.items(
        &view.root(),
        "rows",
        model.bind(|s| s.0.clone(), None),
        viewport.bind(|state| *state, Some(|state, value| *state = value)),
        template.clone(),
    );
    assert!(view.item_keys("rows").is_empty());
    assert!(
        view.items_error("rows")
            .unwrap()
            .contains("invalid viewport")
    );
    let _ = viewport.update(|state| state.count = 1);
    assert_eq!(view.item_keys("rows"), ["first"]);
    assert!(view.items_error("rows").is_none());
    let first_widget = view.widget::<gtk::Label>("rows/first/text").unwrap();
    let valid = viewport.snapshot();
    for invalid in [
        Viewport { count: 0, ..valid },
        Viewport {
            count: 257,
            ..valid
        },
        Viewport {
            row_height: f32::NAN,
            ..valid
        },
        Viewport {
            row_height: 0.5,
            ..valid
        },
        Viewport {
            row_height: 4097.0,
            ..valid
        },
    ] {
        let _ = viewport.update(move |state| *state = invalid);
        assert!(
            view.items_error("rows")
                .unwrap()
                .contains("invalid viewport")
        );
        assert_eq!(
            view.widget::<gtk::Label>("rows/first/text").unwrap(),
            first_widget
        );
    }
    let _ = viewport.update(move |state| {
        *state = Viewport {
            start: usize::MAX,
            ..valid
        }
    });
    assert_eq!(view.item_keys("rows"), ["second"]);
    assert!(view.items_error("rows").is_none());
    let _ = viewport.update(move |state| *state = valid);
    view.content(
        &view.root(),
        "content",
        model.bind(|s| s.1.clone(), None),
        template,
    );
    let row = view.widget::<gtk::Label>("rows/first/text").unwrap();
    let content = view.widget::<gtk::Label>("content/first/text").unwrap();
    let scroll = view.widget::<gtk::ScrolledWindow>("rows").unwrap();
    let container = view.widget::<gtk::Box>("content").unwrap();

    for key in ["", "bad/key"] {
        // Public fields bypass Keyed::new. Even an offscreen invalid key rejects
        // the whole snapshot before the valid first entry can replace a mount.
        let invalid = Keyed {
            key: key.into(),
            model: second.model.clone(),
        };
        let replacement = second.clone();
        let before = builds.get();
        let _ = model.update(move |s| s.0 = vec![replacement, invalid]);
        assert!(
            view.items_error("rows")
                .unwrap()
                .contains("invalid item key")
        );
        assert_eq!(
            scroll.tooltip_text().as_deref(),
            view.items_error("rows").as_deref()
        );
        assert_eq!(view.item_keys("rows"), ["first"]);
        assert_eq!(view.widget::<gtk::Label>("rows/first/text").unwrap(), row);
        assert!(row.parent().is_some());
        assert_eq!(builds.get(), before);
    }
    // Scrolling still uses the previous valid source, not the rejected snapshot.
    view.viewport("rows", 1);
    assert_eq!(view.item_keys("rows"), ["second"]);
    assert!(view.items_error("rows").is_some());
    view.viewport("rows", 0);
    assert_eq!(view.item_keys("rows"), ["first"]);
    let replacement = second.clone();
    let _ = model.update(move |s| s.0 = vec![replacement]);
    assert_eq!(view.item_keys("rows"), ["second"]);
    assert!(view.items_error("rows").is_none());
    assert!(scroll.tooltip_text().is_none());

    for key in ["", "bad/key"] {
        let invalid = Keyed {
            key: key.into(),
            model: second.model.clone(),
        };
        let before = builds.get();
        let _ = model.update(move |s| s.1 = Some(invalid));
        assert!(
            view.0
                .error
                .borrow()
                .as_ref()
                .unwrap()
                .contains("invalid content key")
        );
        assert!(
            container
                .tooltip_text()
                .unwrap()
                .contains("invalid content key")
        );
        assert_eq!(
            view.widget::<gtk::Label>("content/first/text").unwrap(),
            content
        );
        assert!(content.parent().is_some());
        assert_eq!(builds.get(), before);
    }
    let _ = first.model.update(|s| s.text = "still bound".into());
    assert_eq!(
        content.text(),
        "still bound",
        "last valid content keeps its subscription"
    );
    let replacement = second.clone();
    let _ = model.update(move |s| s.1 = Some(replacement));
    assert!(view.text("content/first/text").is_none());
    assert!(content.parent().is_none());
    assert_eq!(view.text("content/second/text").as_deref(), Some("second"));
    assert!(container.tooltip_text().is_none());
    let _ = model.update(|s| s.1 = None);
    assert!(view.text("content/second/text").is_none());
    view.close();
    host.close();
}

#[derive(Clone)]
struct Row {
    name: String,
    enabled: bool,
    calls: usize,
    writes: usize,
    children: Vec<Keyed<Leaf>>,
}

#[derive(Clone)]
struct State {
    rows: Vec<Keyed<Row>>,
    viewport: Viewport,
    selected: Option<Keyed<Row>>,
    menus: Vec<MenuItem>,
    title: String,
    progress: f64,
    clipboard: String,
}

fn menu_item(id: &str, role: MenuRole, shortcut: Option<&str>) -> MenuItem {
    MenuItem {
        id: id.into(),
        label: format!("Label {id}"),
        argument: format!("arg {id}"),
        shortcut: shortcut.map(Into::into),
        role,
        enabled: true,
    }
}

#[test]
fn native_root_style() {
    gtk::init().expect("native_root_style requires a display: run under Xvfb");
    let view = View::new(Binding::constant("Root style".into()));
    let root = view.root().widget;
    let outer = view.native_window().child().unwrap();
    let canvas = root.parent().unwrap();
    assert!(canvas.is::<gtk::Box>());
    assert!(canvas.parent().unwrap().is::<gtk::Viewport>());
    assert_eq!(
        view.widget::<gtk::Box>("__root")
            .unwrap()
            .upcast::<gtk::Widget>(),
        root
    );
    {
        let nodes = view.0.nodes.borrow();
        assert_eq!(nodes["__root"].frame.upcast_ref::<gtk::Widget>(), &root);
        assert_eq!(nodes["__root"].parent.widget, canvas);
    }
    view.label(&view.root(), "child", Binding::constant("content".into()));
    view.style(
        "__root",
        Style {
            margin: Edges {
                top: 2.0,
                right: 3.0,
                bottom: 4.0,
                left: 5.0,
            },
            padding: Edges {
                top: 6.0,
                right: 7.0,
                bottom: 8.0,
                left: 9.0,
            },
            ..Style::default()
        },
    );
    view.present();
    settle();
    assert!(root.is_mapped());
    assert_eq!((root.margin_top(), root.margin_end()), (2, 3));
    assert_eq!((root.margin_bottom(), root.margin_start()), (4, 5));
    let padding = root.style_context().padding();
    assert_eq!((padding.top(), padding.right()), (6, 7));
    assert_eq!((padding.bottom(), padding.left()), (8, 9));
    assert_eq!(root.parent().as_ref(), Some(&canvas));
    assert!(outer.first_child().unwrap().is::<gtk::PopoverMenuBar>());
    assert!(outer.last_child().unwrap().is::<gtk::ScrolledWindow>());
    assert_eq!(root.first_child(), root.last_child());
    assert_eq!(
        view.0.nodes.borrow()["child"].frame.parent().as_ref(),
        Some(&root)
    );
    view.style("__root", Style::default());
    settle();
    assert_eq!(root.margin_start(), 0);
    assert_eq!(root.style_context().padding().left(), 0);
    view.close();
}

#[test]
fn native_menu_namespaces() {
    gtk::init().expect("native_menu_namespaces requires a display: run under Xvfb");
    let host = GtkHost::attach();
    let view = View::new(Binding::constant("Menu namespaces".into()));
    let items = Store::new(
        vec![menu_item("open", MenuRole::Normal, None)],
        host.executor(),
    );
    let dispatched = Rc::new(RefCell::new(Vec::new()));
    for id in ["file", "tools"] {
        let seen = dispatched.clone();
        view.menu(
            id,
            Binding::constant(id.into()),
            items.bind(Clone::clone, None),
            Rc::new(move |argument| {
                seen.borrow_mut().push((id, argument));
                true
            }),
        );
    }
    assert!(view.0.error.borrow().is_none());
    assert_eq!(view.0.actions.borrow().len(), 2);
    for id in ["file", "tools"] {
        assert_eq!(
            &*view.0.menu_states.borrow()[id].actions.borrow(),
            &[format!("{id}/open")]
        );
        view.activate_menu(&format!("{id}/open")).unwrap();
    }
    assert_eq!(
        &*dispatched.borrow(),
        &[("file", "arg open".into()), ("tools", "arg open".into())]
    );
    assert_eq!(
        view.activate_menu("open").unwrap_err(),
        "ambiguous menu item: open; use menu/item"
    );
    assert!(
        view.activate_menu("missing/open")
            .unwrap_err()
            .contains("missing")
    );
    assert_eq!(dispatched.borrow().len(), 2);
    let stale = view.0.actions.borrow()["file/open"].action.clone();
    let _ = items.update(|items| items.push(items[0].clone()));
    assert!(
        view.0
            .error
            .borrow()
            .as_ref()
            .unwrap()
            .contains("duplicate menu item id")
    );
    assert_eq!(
        view.0.actions.borrow().len(),
        2,
        "invalid snapshot keeps both menus"
    );
    view.activate_menu("file/open").unwrap();
    let _ = items.update(|items| {
        items.pop();
        items[0].argument = "updated".into();
    });
    stale.activate(None);
    assert_eq!(dispatched.borrow().len(), 3, "old signal disconnected");
    view.activate_menu("tools/open").unwrap();
    assert_eq!(
        dispatched.borrow().last().unwrap(),
        &("tools", "updated".into())
    );
    view.cleanup("file");
    assert!(!view.0.actions.borrow().contains_key("file/open"));
    assert!(view.0.actions.borrow().contains_key("tools/open"));
    view.activate_menu("open").unwrap();
    let _ = items.update(|items| items[0].enabled = false);
    assert!(view.activate_menu("open").unwrap_err().contains("disabled"));
    assert!(
        view.activate_menu("tools/open")
            .unwrap_err()
            .contains("disabled")
    );
    let _ = items.update(Vec::clear);
    assert!(view.activate_menu("open").unwrap_err().contains("missing"));
    assert!(view.0.actions.borrow().is_empty());
    view.hierarchical_menu(
        "tree",
        Binding::constant("Tree".into()),
        Binding::constant(vec![
            panechord_runtime::MenuNode::Item(menu_item("first", MenuRole::Normal, None)),
            panechord_runtime::MenuNode::Item(menu_item("separator", MenuRole::Separator, None)),
            panechord_runtime::MenuNode::Submenu {
                id: "group".into(),
                label: "Group".into(),
                enabled: true,
                children: vec![panechord_runtime::MenuNode::Item(menu_item(
                    "last",
                    MenuRole::Normal,
                    None,
                ))],
            },
        ]),
        Rc::new(|_| true),
    );
    let native = view.0.menu_states.borrow()["tree"]
        .menu
        .item_link(0, gio::MENU_LINK_SECTION)
        .unwrap();
    assert_eq!(
        native.n_items(),
        2,
        "separator splits two nonempty sections"
    );
    for index in 0..2 {
        assert_eq!(
            native
                .item_link(index, gio::MENU_LINK_SECTION)
                .unwrap()
                .n_items(),
            1
        );
    }
    view.activate_menu("tree/last").unwrap();
    view.close();
    assert!(
        view.activate_menu("tools/open")
            .unwrap_err()
            .contains("closed")
    );
    host.close();
}

#[test]
fn native_contract() {
    gtk::init().expect("native_contract requires a display: run under Xvfb");
    let host = GtkHost::attach();
    let executor = host.executor();
    let rows: Vec<_> = (0..100)
        .map(|i| {
            let leaf = Store::new(
                Leaf {
                    text: format!("child {i}"),
                },
                executor.clone(),
            );
            Keyed::new(
                format!("k{i}"),
                Store::new(
                    Row {
                        name: format!("row {i}"),
                        enabled: true,
                        calls: 0,
                        writes: 0,
                        children: vec![Keyed::new("child", leaf)],
                    },
                    executor.clone(),
                ),
            )
        })
        .collect();
    let model = Store::new(
        State {
            rows: rows.clone(),
            viewport: Viewport {
                start: 0,
                count: 3,
                row_height: 64.0,
            },
            selected: Some(rows[0].clone()),
            title: "Native contract".into(),
            progress: 0.25,
            clipboard: "clipboard text".into(),
            menus: vec![
                menu_item("choose", MenuRole::Normal, Some("<Control>r")),
                menu_item("sep", MenuRole::Separator, None),
                menu_item("select", MenuRole::SelectAll, None),
                menu_item("copy", MenuRole::Copy, None),
                menu_item("cut", MenuRole::Cut, None),
                menu_item("paste", MenuRole::Paste, None),
                menu_item("about", MenuRole::About, None),
            ],
        },
        executor.clone(),
    );
    let view = View::new(model.bind(|s| s.title.clone(), None));
    let root = view.layout(
        &view.root(),
        "stack",
        Layout::Stack {
            horizontal: false,
            gap: 5.0,
        },
    );
    view.style(
        "stack",
        Style {
            margin: Edges {
                top: 2.0,
                right: 3.0,
                bottom: 4.0,
                left: 5.0,
            },
            padding: Edges {
                top: 6.0,
                right: 7.0,
                bottom: 8.0,
                left: 9.0,
            },
            ..Style::default()
        },
    );
    assert_eq!(root.widget.downcast_ref::<gtk::Box>().unwrap().spacing(), 5);
    let grid = view.layout(
        &root,
        "grid",
        Layout::Grid {
            columns: 2,
            gap: 7.0,
        },
    );
    for i in 0..3 {
        view.label(
            &grid,
            &format!("g{i}"),
            Binding::constant(format!("grid {i}")),
        );
    }
    view.style(
        "g2",
        Style {
            placement: Placement {
                row: 1,
                column: 0,
                column_span: 2,
                ..Placement::default()
            },
            ..Style::default()
        },
    );
    let flex = view.layout(&root, "flex", Layout::Flex { gap: 4.0 });
    for i in 0..6 {
        view.label(
            &flex,
            &format!("f{i}"),
            Binding::constant(format!("flex {i}")),
        );
        view.style(
            &format!("f{i}"),
            Style {
                placement: Placement {
                    width: Some(200.0),
                    ..Placement::default()
                },
                ..Style::default()
            },
        );
    }
    let absolute = view.layout(&root, "absolute", Layout::Absolute);
    view.label(
        &absolute,
        "positioned",
        Binding::constant("absolute".into()),
    );
    view.style(
        "positioned",
        Style {
            placement: Placement {
                x: 31.0,
                y: 17.0,
                width: Some(90.0),
                height: Some(25.0),
                ..Placement::default()
            },
            ..Style::default()
        },
    );
    view.style(
        "absolute",
        Style {
            placement: Placement {
                height: Some(64.0),
                ..Placement::default()
            },
            ..Style::default()
        },
    );
    view.meter(&root, "meter", model.bind(|s| s.progress, None));
    // An application factory can mount an actual primitive, not a string descriptor.
    let custom = gtk::Switch::new();
    view.mount_native(&root, "native-switch", &custom);
    view.bind_native(
        "native-switch",
        model.bind(|s| s.progress >= 0.5, None),
        &custom,
        |widget, value| widget.set_active(value),
    );
    view.entry(
        &root,
        "clipboard",
        model.bind(
            |s| s.clipboard.clone(),
            Some(|s, value| s.clipboard = value),
        ),
    );
    view.entry(&root, "readonly", Binding::constant("fixed".into()));
    assert!(view.edit("readonly", "invalid").is_err());
    let live = Rc::new(Cell::new(0usize));
    let live_rows = live.clone();
    let template: Template<Row> = Rc::new(move |view, parent, prefix, row| {
        live_rows.set(live_rows.get() + 1);
        let live = live_rows.clone();
        view.keep(Subscription::new(move || live.set(live.get() - 1)));
        let body = view.layout(
            parent,
            &format!("{prefix}/body"),
            Layout::Stack {
                horizontal: true,
                gap: 2.0,
            },
        );
        view.entry(
            &body,
            &format!("{prefix}/name"),
            row.bind(
                |s| s.name.clone(),
                Some(|s, value| {
                    s.name = value;
                    s.writes += 1;
                }),
            ),
        );
        let command = row.command(
            |s| s.enabled,
            |model, argument| {
                let _ = model.update(move |s| {
                    s.calls += 1;
                    s.name = argument;
                });
            },
            row.bind(|s| format!("{}!", s.name), None),
        );
        view.button(
            &body,
            &format!("{prefix}/invoke"),
            Binding::constant("Invoke".into()),
            command,
        );
        view.items(
            &body,
            &format!("{prefix}/children"),
            row.bind(|s| s.children.clone(), None),
            Binding::constant(Viewport {
                start: 0,
                count: 1,
                row_height: 32.0,
            }),
            Rc::new(|view, parent, prefix, leaf: Store<Leaf>| {
                view.label(
                    parent,
                    &format!("{prefix}/text"),
                    leaf.bind(|s| s.text.clone(), None),
                );
            }),
        );
    });
    view.items(
        &root,
        "rows",
        model.bind(|s| s.rows.clone(), Some(|s, rows| s.rows = rows)),
        model.bind(|s| s.viewport, Some(|s, value| s.viewport = value)),
        template.clone(),
    );
    view.content(
        &root,
        "content",
        model.bind(|s| s.selected.clone(), None),
        template.clone(),
    );
    let dispatched = Rc::new(RefCell::new(Vec::new()));
    let seen = dispatched.clone();
    view.menu(
        "menu",
        model.bind(|s| s.title.clone(), None),
        model.bind(|s| s.menus.clone(), None),
        Rc::new(move |argument| {
            seen.borrow_mut().push(argument);
            true
        }),
    );
    view.present();
    settle();
    assert!(view.native_window().is_mapped());
    assert_eq!(view.item_keys("rows"), ["k0", "k1", "k2"]);
    assert_eq!(
        view.text("rows/k0/children/child/text").as_deref(),
        Some("child 0")
    );
    assert_eq!(live.get(), 4);
    assert_eq!(
        rows[0].model.snapshot().writes,
        0,
        "initial binding must not echo"
    );
    {
        let nodes = view.0.nodes.borrow();
        let stack = &nodes["stack"].frame;
        assert_eq!(stack.margin_top(), 2);
        assert_eq!(stack.margin_start(), 5);
        assert_eq!(stack.style_context().padding().left(), 9);
        assert_eq!(stack.style_context().padding().top(), 6);
        let grid = grid.widget.downcast_ref::<gtk::Grid>().unwrap();
        assert_eq!(grid.column_spacing(), 7);
        assert_eq!(grid.query_child(&nodes["g2"].frame), (0, 1, 2, 1));
        assert!(nodes["g1"].frame.allocation().x() > nodes["g0"].frame.allocation().x());
        assert!(nodes["g2"].frame.allocation().y() > nodes["g0"].frame.allocation().y());
        assert!(flex.widget.is::<gtk::FlowBox>());
        let first = nodes["f0"].frame.compute_bounds(&flex.widget).unwrap();
        let last = nodes["f5"].frame.compute_bounds(&flex.widget).unwrap();
        assert!(last.y() > first.y(), "FlowBox must actually wrap");
        let position = absolute
            .widget
            .downcast_ref::<gtk::Fixed>()
            .unwrap()
            .child_position(&nodes["positioned"].frame);
        assert_eq!(position, (31.0, 17.0));
        assert_eq!(nodes["positioned"].frame.width_request(), 90);
    }
    assert_eq!(
        view.widget::<gtk::ProgressBar>("meter").unwrap().fraction(),
        0.25
    );
    let _ = model.update(|s| {
        s.progress = 0.75;
        s.title = "Updated title".into();
    });
    assert!(custom.is_active());
    assert_eq!(view.title(), "Updated title");
    assert_eq!(
        view.widget::<gtk::ProgressBar>("meter").unwrap().fraction(),
        0.75
    );

    let old_entry = view.widget::<gtk::Entry>("rows/k0/name").unwrap();
    let old_button = view.widget::<gtk::Button>("rows/k0/invoke").unwrap();
    let old_weak = old_entry.downgrade();
    view.edit("rows/k0/name", "edited").unwrap();
    assert_eq!(rows[0].model.snapshot().name, "edited");
    assert_eq!(view.text("content/k0/name").as_deref(), Some("edited"));
    let writes = rows[0].model.snapshot().writes;
    assert!(writes > 0, "native changed signal must write back");
    view.click("rows/k0/invoke").unwrap();
    assert_eq!(rows[0].model.snapshot().name, "edited!");
    assert_eq!(rows[0].model.snapshot().calls, 1);
    assert_eq!(
        rows[0].model.snapshot().writes,
        writes,
        "model updates must not echo via changed"
    );
    view.viewport("rows", 50);
    assert_eq!(view.item_keys("rows"), ["k50", "k51", "k52"]);
    assert_eq!(live.get(), 4, "evicted keep tokens must be released");
    assert!(view.text("rows/k0/name").is_none());
    assert!(
        !view
            .0
            .nodes
            .borrow()
            .keys()
            .any(|id| id.starts_with("rows/k0/"))
    );
    assert!(
        !view
            .0
            .resources
            .borrow()
            .keys()
            .any(|id| scoped(id, "rows/k0"))
    );
    old_entry.set_text("detached input");
    old_button.emit_clicked();
    assert_eq!(rows[0].model.snapshot().name, "edited!");
    assert_eq!(rows[0].model.snapshot().calls, 1);
    drop(old_entry);
    drop(old_button);
    assert!(
        old_weak.upgrade().is_none(),
        "all per-prefix native refs must be gone"
    );
    let _ = rows[0].model.update(|s| {
        s.name = "offscreen".into();
        s.enabled = false;
    });
    view.viewport("rows", 0);
    assert_eq!(view.text("rows/k0/name").as_deref(), Some("offscreen"));
    assert!(view.click("rows/k0/invoke").is_err());
    let _ = rows[0].model.update(|s| s.enabled = true);
    view.click("rows/k0/invoke").unwrap();
    assert_eq!(rows[0].model.snapshot().calls, 2);
    let retained_entry = view.widget::<gtk::Entry>("rows/k0/name").unwrap();
    let _ = model.update(|s| s.rows.swap(0, 1));
    assert_eq!(view.item_keys("rows"), ["k1", "k0", "k2"]);
    assert_eq!(
        view.widget::<gtk::Entry>("rows/k0/name").unwrap(),
        retained_entry
    );
    let valid_keys = view.item_keys("rows");
    let _ = model.update(|s| s.rows = vec![s.rows[0].clone(), s.rows[0].clone()]);
    assert!(view.items_error("rows").unwrap().contains("duplicate"));
    assert_eq!(
        view.item_keys("rows"),
        valid_keys,
        "duplicate rejection must be pre-mutation"
    );
    let reset_rows = rows.clone();
    let _ = model.update(move |s| s.rows = reset_rows);
    assert!(view.items_error("rows").is_none());
    settle();
    let scroll = view.widget::<gtk::ScrolledWindow>("rows").unwrap();
    assert_eq!(
        scroll.vadjustment().upper(),
        6400.0,
        "spacers preserve total extent"
    );
    assert_eq!(scroll.vadjustment().page_size(), 192.0);
    scroll.vadjustment().set_value(64.0 * 20.0);
    assert_eq!(view.item_keys("rows"), ["k20", "k21", "k22"]);
    assert_eq!(
        model.snapshot().viewport.start,
        20,
        "actual native adjustment updates model range"
    );
    for start in [99, 60, 2, 0] {
        view.viewport("rows", start);
        assert!(view.mounted_count("rows") <= model.snapshot().viewport.count);
    }
    // Same key with a different Store must clean up before mounting that ID again.
    let replacement = Store::new(
        Row {
            name: "replacement".into(),
            calls: 0,
            writes: 0,
            enabled: true,
            children: vec![],
        },
        executor.clone(),
    );
    let _ = model.update(move |s| s.rows[0] = Keyed::new("k0", replacement));
    assert_eq!(view.text("rows/k0/name").as_deref(), Some("replacement"));
    let selected = rows[1].clone();
    let _ = model.update(move |s| s.selected = Some(selected));
    assert!(view.text("content/k0/name").is_none());
    assert_eq!(view.text("content/k1/name").as_deref(), Some("row 1"));
    let _ = model.update(|s| s.selected = None);
    assert!(view.text("content/k1/name").is_none());
    assert_eq!(live.get(), 3);

    view.activate_menu("choose").unwrap();
    assert_eq!(&*dispatched.borrow(), &["arg choose"]);
    let state = view.0.menu_states.borrow()["menu"].clone();
    assert_eq!(
        state.menu.n_items(),
        2,
        "separator creates native menu sections"
    );
    let shortcut = state.shortcuts.borrow()[0].clone();
    assert!(shortcut.action().unwrap().is::<gtk::NamedAction>());
    assert!(shortcut.action().unwrap().activate(
        gtk::ShortcutActionFlags::empty(),
        &view.0.window,
        None
    ));
    assert_eq!(dispatched.borrow().len(), 2);
    let stale_action = view.0.actions.borrow()["menu/choose"].action.clone();
    let _ = model.update(|s| {
        s.menus[0].argument = "new argument".into();
        s.menus[0].label = "new label".into();
        s.menus[0].shortcut = Some("<Control>t".into());
    });
    assert!(!shortcut.action().unwrap().activate(
        gtk::ShortcutActionFlags::empty(),
        &view.0.window,
        None
    ));
    stale_action.activate(None);
    assert_eq!(dispatched.borrow().len(), 2, "old menu signal disconnected");
    let shortcut = state.shortcuts.borrow()[0].clone();
    assert_eq!(shortcut.trigger().unwrap().to_string(), "<Control>t");
    assert!(shortcut.action().unwrap().activate(
        gtk::ShortcutActionFlags::empty(),
        &view.0.window,
        None
    ));
    assert_eq!(dispatched.borrow().last().unwrap(), "new argument");
    let _ = model.update(|s| s.menus[0].enabled = false);
    assert!(view.activate_menu("choose").is_err());
    let _ = model.update(|s| s.menus[0].enabled = true);
    let clipboard = view.widget::<gtk::Entry>("clipboard").unwrap();
    clipboard.grab_focus();
    settle();
    view.activate_menu("select").unwrap();
    assert_eq!(clipboard.selection_bounds(), Some((0, 14)));
    view.activate_menu("copy").unwrap();
    let clip_text = Rc::new(RefCell::new(None));
    let result = clip_text.clone();
    clipboard
        .clipboard()
        .read_text_async(None::<&gio::Cancellable>, move |value| {
            result.replace(Some(value.unwrap().map(|text| text.to_string())));
        });
    settle();
    assert_eq!(
        clip_text.borrow().as_ref().unwrap().as_deref(),
        Some("clipboard text")
    );
    view.activate_menu("cut").unwrap();
    assert_eq!(model.snapshot().clipboard, "");
    view.activate_menu("paste").unwrap();
    settle();
    assert_eq!(model.snapshot().clipboard, "clipboard text");
    view.activate_menu("about").unwrap();
    settle();
    let about: Vec<_> = gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::AboutDialog>().ok())
        .collect();
    assert_eq!(about.len(), 1);
    assert_eq!(about[0].program_name().as_deref(), Some("Updated title"));
    about[0].destroy();
    drop(about);

    // Completion handles are pruned while the loop is running, even if no new task arrives.
    for _ in 0..128 {
        executor.spawn(Box::pin(async {}));
    }
    settle();
    assert!(host.executor.tasks.borrow().is_empty());
    let witness = Rc::new(());
    let weak_task = Rc::downgrade(&witness);
    executor.spawn(Box::pin(async move {
        let _witness = witness;
        std::future::pending::<()>().await;
    }));
    settle();
    assert!(weak_task.upgrade().is_some());
    let closed = Rc::new(Cell::new(0));
    let count = closed.clone();
    view.on_close(Rc::new(move || count.set(count.get() + 1)));
    let weak = Rc::downgrade(&view.0);
    view.0.window.close();
    assert_eq!(closed.get(), 1, "native close-request path");
    assert_eq!(live.get(), 0);
    assert!(view.0.resources.borrow().is_empty());
    view.close();
    assert_eq!(closed.get(), 1);
    let frozen = rows[0].model.snapshot().calls;
    retained_entry.set_text("after close");
    assert_eq!(rows[0].model.snapshot().calls, frozen);
    drop(view);
    assert!(
        weak.upgrade().is_none(),
        "rows, menus and native signals must not own View"
    );
    host.close();
    assert!(
        weak_task.upgrade().is_none(),
        "abort drops pending futures synchronously"
    );

    // Formal runtime UiHost attachment is nonblocking and owns actual native windows.
    let attached = GtkHost::attach();
    let session = Session::new();
    let window = View::new(Binding::constant("attached".into()));
    attached
        .register_view(&session, "attached", &window)
        .unwrap();
    let returned = Rc::new(Cell::new(false));
    let flag = returned.clone();
    let result = Rc::new(RefCell::new(None));
    let done = result.clone();
    let attached = Rc::new(attached);
    let owner = attached.clone();
    attached.executor().spawn(Box::pin(async move {
        done.replace(Some(run_async(&*owner, |_| session).await.unwrap()));
        flag.set(true);
    }));
    settle();
    assert!(returned.get());
    assert!(window.native_window().is_mapped());
    let session = result.take().unwrap();
    assert_eq!(session.window_count(), 1);
    session.shutdown().close_now();
    assert!(window.0.closed.get());
    assert!(attached.windows.borrow().is_empty());
    settle();
    assert!(attached.sessions.borrow().is_empty());
    attached.close();

    // Destruction without explicit close also releases signals and application work.
    let host = GtkHost::attach();
    let model = Store::new("drop".to_owned(), host.executor());
    let dropped = View::new(Binding::constant("drop".into()));
    dropped.entry(
        &dropped.root(),
        "entry",
        model.bind(Clone::clone, Some(|s, v| *s = v)),
    );
    let native = dropped.widget::<gtk::Entry>("entry").unwrap();
    let weak = Rc::downgrade(&dropped.0);
    let marker = Rc::new(());
    let task = Rc::downgrade(&marker);
    let _ = model.effect(
        Policy::Parallel,
        "drop",
        async move {
            let _marker = marker;
            std::future::pending::<()>().await
        },
        |_, _| {},
    );
    let owner = model.clone();
    dropped.on_close(Rc::new(move || owner.close()));
    drop(dropped);
    assert!(weak.upgrade().is_none());
    native.set_text("detached");
    assert_eq!(model.snapshot(), "drop");
    settle();
    assert!(task.upgrade().is_none());
    host.close();

    // Standalone run drives a real Application loop, aborting its executor on close.
    let ran = Rc::new(Cell::new(false));
    let flag = ran.clone();
    run(move |executor| {
        let view = View::new(Binding::constant("standalone".into()));
        let weak = Rc::downgrade(&view.0);
        executor.spawn(Box::pin(async move {
            glib::timeout_future(Duration::from_millis(20)).await;
            let view = View(weak.upgrade().unwrap());
            view.menu(
                "quit-menu",
                Binding::constant("Quit".into()),
                Binding::constant(vec![menu_item("quit", MenuRole::Quit, None)]),
                Rc::new(|_| false),
            );
            flag.set(view.native_window().is_mapped());
            view.activate_menu("quit").unwrap();
        }));
        view
    })
    .unwrap();
    assert!(ran.get());
}
