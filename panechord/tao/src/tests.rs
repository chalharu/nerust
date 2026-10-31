#[test]
fn unloaded_surface_stays_finite_inside_scrollable_root() {
    use iced::{
        Size,
        advanced::{layout::Limits, widget::Tree},
    };
    let renderer = iced_tiny_skia::Renderer::new(iced::Font::default(), iced::Pixels(16.0));
    for (width, height) in [(660.0, 495.0), (548.0, 411.0)] {
        let mut element = super::surface::element(None, width, height);
        let mut tree = Tree::new(&element);
        let node = element.as_widget_mut().layout(
            &mut tree,
            &renderer,
            &Limits::new(Size::ZERO, Size::new(width, f32::INFINITY)),
        );
        assert!(node.size().width.is_finite());
        assert!(node.size().height.is_finite());
        assert_eq!(node.size(), Size::new(width, height));
    }
}

use super::*;

#[cfg(target_os = "linux")]
#[test]
fn horizontal_tabs_reserve_height_before_stack_footer() {
    use iced::{
        Size,
        advanced::{layout::Limits, widget::Tree},
    };
    let harness = Harness::new();
    let model = harness.store(String::from("general"));
    let view = View::new(Binding::constant("Settings".into()));
    let tab = view.tab(
        &view.root(),
        "tabs",
        model.bind(Clone::clone, Some(|state, value| *state = value)),
    );
    let page = view.tab_page(
        &tab,
        "general",
        Binding::constant("General".into()),
        "general",
    );
    for id in ["language", "storage", "directory"] {
        view.label(&page, id, Binding::constant(id.into()));
    }
    view.finish_tab(&tab).unwrap();
    view.label(
        &view.root(),
        "status",
        Binding::constant("Unsaved changes".into()),
    );
    view.button(
        &view.root(),
        "apply",
        Binding::constant("Apply".into()),
        model.command(|_| true, |_, ()| {}, Binding::constant(())),
    );
    let renderer = iced_tiny_skia::Renderer::new(iced::Font::default(), iced::Pixels(16.0));
    for width in [440.0, 660.0, 960.0] {
        let mut element = view.root().0.element();
        let mut tree = Tree::new(element.as_widget());
        let frame = element.as_widget_mut().layout(
            &mut tree,
            &renderer,
            &Limits::new(Size::ZERO, Size::new(width, 520.0)),
        );
        let children = frame.children()[0].children();
        let tabs = children[0].bounds();
        let status = children[1].bounds();
        let apply = children[2].bounds();
        assert!(
            tabs.height > 60.0,
            "tab body must contribute intrinsic height: {tabs:?}"
        );
        assert!(
            status.y >= tabs.y + tabs.height,
            "status overlaps tabs: {tabs:?} {status:?}"
        );
        assert!(apply.y >= status.y + status.height, "Apply overlaps status");
        assert!(
            apply.y + apply.height <= 520.0,
            "footer must remain in the viewport"
        );
    }
    view.close();
}

#[cfg(target_os = "linux")]
#[test]
fn vertical_tabs_keep_headers_disjoint_and_page_identity_stable() {
    use iced::{
        Size,
        advanced::{layout::Limits, widget::Tree},
    };
    use panechord_runtime::TabPosition;
    let harness = Harness::new();
    let model = harness.store(String::from("video"));
    let view = View::new(Binding::constant("Tabs".into()));
    let tab = view.tab(
        &view.root(),
        "tabs",
        model.bind(Clone::clone, Some(|state, value| *state = value)),
    );
    for key in ["video", "audio"] {
        let page = view.tab_page(&tab, key, Binding::constant(key.into()), key);
        view.label(
            &page,
            &format!("{key}_label"),
            Binding::constant(format!("{key} settings")),
        );
    }
    view.finish_tab(&tab).unwrap();
    let identities: Vec<_> = tab
        .0
        .children
        .borrow()
        .iter()
        .map(|page| page.identity)
        .collect();
    let renderer = iced_tiny_skia::Renderer::new(iced::Font::default(), iced::Pixels(16.0));
    for position in [
        TabPosition::Top,
        TabPosition::Bottom,
        TabPosition::Left,
        TabPosition::Right,
    ] {
        view.tab_position("tabs", position).unwrap();
        for width in [240.0, 440.0, 960.0] {
            let mut element = tab.0.element();
            let mut tree = Tree::new(element.as_widget());
            let frame = element.as_widget_mut().layout(
                &mut tree,
                &renderer,
                &Limits::new(Size::new(width, 140.0), Size::new(width, 140.0)),
            );
            let children = frame.children()[0].children();
            assert_eq!(children.len(), 2);
            let first = children[0].bounds();
            let second = children[1].bounds();
            assert!(!first.intersects(&second));
            assert!(first.x >= 0.0 && second.x + second.width <= width + 0.01);
            match position {
                TabPosition::Left => assert!((first.width - 120.0).abs() < 0.01),
                TabPosition::Right => {
                    assert!((second.width - 120.0).abs() < 0.01);
                    assert!((second.x + second.width - width).abs() < 0.01);
                }
                TabPosition::Top => assert!(first.y.abs() < 0.01),
                TabPosition::Bottom => assert!((second.y + second.height - 140.0).abs() < 0.01),
            }
        }
        view.select("tabs", "audio").unwrap();
        assert_eq!(model.snapshot(), "audio");
        view.select("tabs", "video").unwrap();
        assert_eq!(
            tab.0
                .children
                .borrow()
                .iter()
                .map(|page| page.identity)
                .collect::<Vec<_>>(),
            identities
        );
    }
    view.close();
}

#[cfg(target_os = "linux")]
#[test]
fn combo_rejects_unknown_source_selection_and_accepts_recovery() {
    let harness = Harness::new();
    let model = harness.store(String::from("linear"));
    let view = View::new(Binding::constant("Controls".into()));
    let choices = Binding::constant(vec![panechord_runtime::Choice {
        id: "linear".into(),
        label: "Linear".into(),
    }]);
    view.combo(
        &view.root(),
        "combo",
        choices,
        model.bind(Clone::clone, Some(|state, value| *state = value)),
    );
    assert!(view.error().is_none());
    assert!(model.update(|state| *state = "missing".into()));
    assert!(view.error().unwrap().contains("selected"));
    assert!(model.update(|state| *state = "linear".into()));
    view.select("combo", "linear").unwrap();
    assert_eq!(model.snapshot(), "linear");
    view.close();
    assert!(view.select("combo", "linear").is_err());
}

#[cfg(target_os = "linux")]
#[derive(Clone, Default)]
struct LayoutDraft {
    name: String,
    volume: String,
    scale: String,
}
#[cfg(target_os = "linux")]
#[derive(Clone, Default)]
struct LayoutProjection {
    error: String,
    summary: String,
    status: String,
    warning: String,
    menu_items: Vec<panechord_runtime::MenuItem>,
}
#[cfg(target_os = "linux")]
#[derive(Clone, Default)]
struct SettingsLayout {
    draft: LayoutDraft,
    ui: LayoutProjection,
    stay: bool,
    leave: bool,
}
#[cfg(target_os = "linux")]
#[panechord_macros::commands]
impl SettingsLayout {
    #[command]
    fn save(&mut self, _value: bool) {}
    #[command]
    fn save_exit(&mut self, _value: bool) {}
    #[command]
    fn cancel(&mut self) {}
    #[command]
    fn discard_exit(&mut self) {}
    #[command]
    fn keep_editing(&mut self) {}
    #[command]
    fn choose(&mut self, _value: String) {}
}

#[cfg(target_os = "linux")]
#[test]
fn settings_yaml_layout_keeps_buttons_filled_and_controls_disjoint() {
    use iced::{
        Point, Rectangle, Size,
        advanced::{layout, widget::Tree},
    };
    fn collect(
        node: &Node,
        frame: &layout::Node,
        origin: Point,
        result: &mut Vec<(String, Rectangle)>,
    ) {
        let bounds = frame.bounds();
        let absolute = Rectangle {
            x: origin.x + bounds.x,
            y: origin.y + bounds.y,
            ..bounds
        };
        if matches!(&*node.handler.borrow(), Some(Handler::Button(_))) {
            assert!(
                (frame.children()[0].size().width - bounds.width).abs() < 0.01,
                "button body must fill its frame: {}",
                node.id
            );
        }
        if node.children.borrow().is_empty() {
            result.push((node.id.clone(), absolute));
        }
        let body = &frame.children()[0];
        let origin = Point::new(absolute.x + body.bounds().x, absolute.y + body.bounds().y);
        for (child, frame) in node.children.borrow().iter().zip(body.children()) {
            collect(child, frame, origin, result);
        }
    }
    let harness = Harness::new();
    let model = harness.store(SettingsLayout::default());
    let view = panechord_macros::view!("../settings/ui/settings.yaml", model, crate);
    let renderer = iced_tiny_skia::Renderer::new(iced::Font::default(), iced::Pixels(16.0));
    for (state, message) in [
        ("initial", ""),
        ("invalid", "Volume must be an integer from 0 to 100"),
        (
            "confirm",
            "Unsaved changes. Save and exit, discard, or keep editing.",
        ),
        (
            "long",
            "A very long validation message with an_unbroken_word_that_must_not_escape_the_window_bounds and multiple lines of detail. A very long validation message with additional detail.",
        ),
    ] {
        assert!(model.update(move |state| {
            state.ui.error = message.into();
            state.ui.warning = message.into();
            state.ui.status = "Unsaved changes".into();
            state.ui.summary = "Saved: Player | volume 42 | scale 1".into();
        }));
        for width in [440.0, 660.0, 960.0] {
            let mut element = view.root().0.element();
            let mut tree = Tree::new(element.as_widget());
            let frame = element.as_widget_mut().layout(
                &mut tree,
                &renderer,
                &layout::Limits::new(Size::ZERO, Size::new(width, f32::INFINITY)),
            );
            let mut bounds = Vec::new();
            collect(&view.root().0, &frame, Point::ORIGIN, &mut bounds);
            assert_eq!(bounds.len(), 16);
            for (id, rect) in &bounds {
                assert!(
                    rect.x >= 19.9 && rect.x + rect.width <= width - 19.9,
                    "{state}/{width}: {id}: {rect:?}"
                );
                assert!(
                    rect.y >= 19.9 && rect.y + rect.height <= frame.size().height - 19.9,
                    "{state}/{width}: {id} exceeds content height"
                );
            }
            for (index, (id, rect)) in bounds.iter().enumerate() {
                for (other, other_rect) in &bounds[index + 1..] {
                    assert!(
                        !rect.intersects(other_rect),
                        "{state}/{width}: {id} overlaps {other}"
                    );
                }
            }
        }
    }
}

#[test]
#[cfg(target_os = "linux")]
fn style_late_close_and_viewport_writeback_contract() {
    let harness = Harness::new();
    let view = View::new(Binding::constant("contracts".into()));
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
    let viewport = harness.store(Viewport {
        count: 3,
        ..Viewport::default()
    });
    view.items(
        &view.root(),
        "rows",
        Binding::constant(
            (0..10)
                .map(|index| Keyed::new(index.to_string(), harness.store(index)))
                .collect(),
        ),
        viewport.bind(|state| *state, Some(|state, value| *state = value)),
        Rc::new(|_, _, _, _| {}),
    );
    view.viewport("rows", usize::MAX);
    assert_eq!(viewport.snapshot().start, 7);
    assert!(viewport.update(|state| state.start = usize::MAX));
    assert_eq!(viewport.snapshot().start, 7);
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
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

struct Harness {
    executor: Rc<scheduler::Scheduler>,
    ids: Arc<Mutex<VecDeque<usize>>>,
}
impl Harness {
    fn new() -> Self {
        let ids = Arc::new(Mutex::new(VecDeque::new()));
        let out = ids.clone();
        Self {
            executor: Rc::new(scheduler::Scheduler::new(Arc::new(move |id| {
                out.lock().unwrap().push_back(id)
            }))),
            ids,
        }
    }
    fn pump(&self) {
        for _ in 0..1000 {
            let ids: Vec<_> = self.ids.lock().unwrap().drain(..).collect();
            for id in ids {
                self.executor.ready(id);
            }
            let ready = self.executor.turn();
            if !ready && self.ids.lock().unwrap().is_empty() {
                return;
            }
        }
        panic!("UI work failed to settle");
    }
    fn store<T: Clone + 'static>(&self, state: T) -> Store<T> {
        Store::new(state, self.executor.clone())
    }
}
#[derive(Clone)]
struct Model {
    rows: Vec<Keyed<String>>,
    viewport: Viewport,
    selected: Option<Keyed<String>>,
}

#[test]
fn close_requests_are_weak_replaceable_reentrant_and_force_is_final() {
    let view = View::new(Binding::constant("close".into()));
    let weak = view.downgrade();
    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    let owner = weak.clone();
    view.on_close_request(Rc::new(move || {
        count.set(count.get() + 1);
        owner.upgrade().unwrap().request_close();
    }));
    view.request_close();
    view.request_close();
    assert_eq!(calls.get(), 2);
    assert!(weak.upgrade().is_some());
    let count = calls.clone();
    view.on_close_request(Rc::new(move || {
        count.set(count.get() + 10);
        panic!("test handler");
    }));
    view.request_close();
    view.request_close();
    assert_eq!(calls.get(), 22, "panic releases the request gate");
    assert!(view.error().unwrap().contains("test handler"));
    let closed = Rc::new(Cell::new(0));
    let count = closed.clone();
    let owner = weak.clone();
    view.on_close(Rc::new(move || {
        assert!(owner.upgrade().is_none());
        count.set(count.get() + 1);
    }));
    view.close();
    view.close();
    view.request_close();
    assert_eq!(closed.get(), 1);
    assert_eq!(calls.get(), 22);
    assert!(weak.upgrade().is_none());
    let dropped = View::new(Binding::constant("drop".into()));
    let weak = dropped.downgrade();
    let owner = weak.clone();
    dropped.on_close_request(Rc::new(move || {
        let _ = owner.upgrade();
    }));
    drop(dropped);
    assert!(weak.upgrade().is_none());
    let default = View::new(Binding::constant("default".into()));
    default.request_close();
    assert!(default.downgrade().upgrade().is_none());
}

#[test]
fn close_ready_subscription_is_kept_without_a_view_model_cycle() {
    let h = Harness::new();
    let model = h.store((0usize, false));
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
    view.request_close();
    h.pump();
    assert_eq!(model.snapshot(), (1, false));
    assert!(!model.is_closed());
    let _ = model.update(|s| s.1 = true);
    h.pump();
    assert_eq!(closes.get(), 1);
    assert!(model.is_closed());
    assert_eq!(view.subscription_count(), 0);
    let weak = Rc::downgrade(&view.0);
    drop(view);
    assert!(weak.upgrade().is_none());
}

#[test]
fn quit_role_requests_close_without_dispatching_or_recursing() {
    use panechord_runtime::{MenuItem, MenuRole};
    let view = View::new(Binding::constant("quit".into()));
    view.menu(
        "file",
        Binding::constant("File".into()),
        Binding::constant(vec![MenuItem {
            id: "quit".into(),
            label: "Quit".into(),
            argument: String::new(),
            shortcut: None,
            role: MenuRole::Quit,
            enabled: true,
        }]),
        Rc::new(|_| panic!("Quit must not dispatch")),
    );
    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    let weak = view.downgrade();
    view.on_close_request(Rc::new(move || {
        count.set(count.get() + 1);
        weak.upgrade().unwrap().activate_menu("quit").unwrap();
    }));
    view.activate_menu("quit").unwrap();
    assert_eq!(calls.get(), 1);
    assert!(!view.0.closed.get());
    view.close();
}

#[test]
fn virtual_ranges_are_bounded_and_saturating() {
    assert_eq!(virtual_range(0, Viewport::default()), 0..0);
    assert_eq!(
        virtual_range(
            100,
            Viewport {
                start: 99,
                count: 3,
                row_height: 10.0
            }
        ),
        97..100
    );
    assert_eq!(
        virtual_range(
            10000,
            Viewport {
                start: usize::MAX,
                count: usize::MAX,
                row_height: f32::NAN
            }
        ),
        0..0
    );
    assert_eq!(
        virtual_range(
            10,
            Viewport {
                start: 2,
                count: 0,
                row_height: 1.0
            }
        ),
        0..0
    );
}

#[test]
fn invalid_viewports_keep_mounts_and_initially_mount_nothing() {
    let harness = Harness::new();
    let viewport = harness.store(Viewport {
        count: 0,
        ..Viewport::default()
    });
    let rows = (0..10)
        .map(|index| Keyed::new(index.to_string(), harness.store(index.to_string())))
        .collect();
    let view = View::new(Binding::constant("viewport validation".into()));
    view.items(
        &view.root(),
        "rows",
        Binding::constant(rows),
        viewport.bind(|state| *state, Some(|state, value| *state = value)),
        Rc::new(|view, parent, scope, model: Store<String>| {
            view.label(
                parent,
                &format!("{scope}/text"),
                model.bind(Clone::clone, None),
            );
        }),
    );
    assert!(view.item_keys("rows").is_empty());
    assert!(
        view.0
            .error
            .borrow()
            .as_ref()
            .unwrap()
            .contains("invalid viewport")
    );
    let _ = viewport.update(|state| {
        *state = Viewport {
            count: 3,
            row_height: 24.0,
            start: usize::MAX,
        }
    });
    assert_eq!(view.item_keys("rows"), ["7", "8", "9"]);
    for invalid in [
        Viewport {
            count: 0,
            ..viewport.snapshot()
        },
        Viewport {
            count: 257,
            ..viewport.snapshot()
        },
        Viewport {
            row_height: f32::NAN,
            ..viewport.snapshot()
        },
        Viewport {
            row_height: 0.5,
            ..viewport.snapshot()
        },
        Viewport {
            row_height: 4097.0,
            ..viewport.snapshot()
        },
    ] {
        let _ = viewport.update(move |state| *state = invalid);
        assert_eq!(view.item_keys("rows"), ["7", "8", "9"]);
    }
    let _ = viewport.update(|state| {
        *state = Viewport {
            start: 1,
            count: 1,
            row_height: 1.0,
        }
    });
    assert_eq!(view.item_keys("rows"), ["1"]);
    view.close();
}

#[test]
fn recursive_typed_templates_keep_identity_evict_subscriptions_and_reject_duplicates() {
    let h = Harness::new();
    let rows: Vec<_> = (0..1000)
        .map(|i| Keyed::new(i.to_string(), h.store(format!("row {i}"))))
        .collect();
    let model = h.store(Model {
        rows: rows.clone(),
        viewport: Viewport {
            start: 0,
            count: 3,
            row_height: 24.0,
        },
        selected: None,
    });
    let view = View::new(Binding::constant("test".into()));
    let mounted = Rc::new(Cell::new(0usize));
    let released = Rc::new(Cell::new(0usize));
    let mounts = mounted.clone();
    let releases = released.clone();
    view.items(
        &view.root(),
        "rows",
        model.bind(|s| s.rows.clone(), Some(|s, v| s.rows = v)),
        model.bind(|s| s.viewport, Some(|s, v| s.viewport = v)),
        Rc::new(move |view, parent, scope, row| {
            mounts.set(mounts.get() + 1);
            let panel = view.layout(parent, "body", Layout::Flex { gap: 2.0 });
            view.entry(&panel, "entry", row.bind(Clone::clone, Some(|s, v| *s = v)));
            view.button(
                &panel,
                "button",
                Binding::constant("change".into()),
                row.command(
                    |_| true,
                    |store, argument| {
                        let _ = store.update(move |s| *s = argument);
                    },
                    Binding::constant("clicked".into()),
                ),
            );
            // Recursive content template uses the typed item Store, not string rendering.
            view.content(
                &panel,
                "detail",
                Binding::constant(Some(Keyed::new("nested", row.clone()))),
                Rc::new(|view, parent, _, model| {
                    view.label(parent, "name", model.bind(Clone::clone, None))
                }),
            );
            let out = releases.clone();
            view.mount(
                &panel,
                "primitive",
                Rc::new(|| widget::text("custom").into()),
                vec![Subscription::new(move || out.set(out.get() + 1))],
            );
            // Generated validators also use keep: its lifetime must be the row scope.
            view.keep(Subscription::new(|| {}));
            assert!(view.text(&format!("{scope}/entry")).is_some());
        }),
    );
    assert_eq!(view.item_keys("rows"), ["0", "1", "2"]);
    let identity = view.mount_identity("rows/1/entry");
    let baseline = view.subscription_count();
    view.viewport("rows", 1);
    h.pump();
    assert_eq!(view.item_keys("rows"), ["1", "2", "3"]);
    assert_eq!(view.mount_identity("rows/1/entry"), identity);
    assert_eq!(mounted.get(), 4);
    assert_eq!(released.get(), 1);
    assert_eq!(view.subscription_count(), baseline);
    assert!(view.text("rows/0/entry").is_none());
    let _ = model.update(|s| s.rows.swap(1, 2));
    h.pump();
    assert_eq!(view.item_keys("rows"), ["2", "1", "3"]);
    assert_eq!(view.mount_identity("rows/1/entry"), identity);
    let _ = model.update(|s| s.rows.swap(1, 2));
    h.pump();
    view.edit("rows/1/entry", "edited").unwrap();
    h.pump();
    assert_eq!(rows[1].model.snapshot(), "edited");
    assert_eq!(
        view.text("rows/1/detail/nested/name").as_deref(),
        Some("edited")
    );
    view.click("rows/1/button").unwrap();
    h.pump();
    assert_eq!(rows[1].model.snapshot(), "clicked");
    let before = view.item_keys("rows");
    let _ = model.update(|s| s.rows.push(s.rows[1].clone()));
    h.pump();
    assert_eq!(view.item_keys("rows"), before);
    assert!(view.error().unwrap().contains("duplicate key"));
    // Same key, new Store: the old handlers must be dropped, not rebound by text.
    let replacement = h.store("replacement".to_owned());
    let out = replacement.clone();
    let _ = model.update(move |s| {
        s.rows.pop();
        s.rows[1] = Keyed::new("1", out);
    });
    h.pump();
    assert_ne!(view.mount_identity("rows/1/entry"), identity);
    assert_eq!(view.text("rows/1/entry").as_deref(), Some("replacement"));
    for start in [20, 200, 500, 997, 0] {
        view.viewport("rows", start);
        h.pump();
        assert_eq!(view.mounted_count("rows"), 3);
    }
    assert_eq!(view.text("rows/1/entry").as_deref(), Some("replacement"));
    assert!(!rows[0].model.is_closed());
    view.close();
    assert_eq!(view.subscription_count(), 0);
    assert_eq!(mounted.get(), released.get());
    assert!(!replacement.is_closed());
}

#[test]
fn content_replacement_and_nested_items_drop_mounts_but_not_item_models() {
    let h = Harness::new();
    let a = h.store("A".to_owned());
    let b = h.store("B".to_owned());
    let model = h.store(Model {
        rows: Vec::new(),
        viewport: Viewport::default(),
        selected: Some(Keyed::new("a", a.clone())),
    });
    let view = View::new(Binding::constant("content".into()));
    view.content(
        &view.root(),
        "selected",
        model.bind(|s| s.selected.clone(), Some(|s, v| s.selected = v)),
        Rc::new(|view, parent, _, row| {
            view.items(
                parent,
                "inner",
                Binding::constant(vec![Keyed::new("child", row)]),
                Binding::constant(Viewport::default()),
                Rc::new(|view, parent, _, row| {
                    view.entry(parent, "text", row.bind(Clone::clone, Some(|s, v| *s = v)))
                }),
            );
        }),
    );
    let id = "selected/a/inner/child/text";
    assert_eq!(view.text(id).as_deref(), Some("A"));
    let identity = view.mount_identity(id);
    let _ = model.update(|_| {});
    h.pump();
    assert_eq!(view.mount_identity(id), identity);
    let _ = model.update(move |s| s.selected = Some(Keyed::new("b", b)));
    h.pump();
    assert!(view.text(id).is_none());
    assert_eq!(
        view.text("selected/b/inner/child/text").as_deref(),
        Some("B")
    );
    let _ = model.update(|s| s.selected = None);
    h.pump();
    assert!(view.text("selected/b/inner/child/text").is_none());
    assert!(!a.is_closed());
}

#[test]
fn readonly_edit_never_echoes_and_close_is_idempotent() {
    let view = View::new(Binding::constant("readonly".into()));
    let style = Style {
        padding: panechord_runtime::Edges {
            left: 12.0,
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(view.try_style("generated-window", style).is_err());
    assert_eq!(view.root().0.style.get(), Style::default());
    view.style("__root", style);
    assert_eq!(view.root().0.style.get(), style);
    assert!(view.error().is_none());
    view.entry(&view.root(), "entry", Binding::constant("canonical".into()));
    assert!(view.edit("entry", "wrong").is_err());
    assert_eq!(view.text("entry").as_deref(), Some("canonical"));
    let calls = Rc::new(Cell::new(0));
    let out = calls.clone();
    view.on_close(Rc::new(move || out.set(out.get() + 1)));
    view.close();
    view.close();
    assert_eq!(calls.get(), 1);
    assert!(view.click("entry").is_err());
}

#[test]
fn invalid_public_keyed_fields_reject_items_and_content_before_mutation() {
    let h = Harness::new();
    let row = h.store("old".to_owned());
    let good = Keyed::new("good", row.clone());
    let state = h.store(Model {
        rows: vec![good.clone()],
        viewport: Viewport::default(),
        selected: Some(good.clone()),
    });
    let view = View::new(Binding::constant("keys".into()));
    let released = Rc::new(Cell::new(0));
    let releases = released.clone();
    let template: Template<String> = Rc::new(move |view, parent, _, model| {
        view.entry(
            parent,
            "entry",
            model.bind(Clone::clone, Some(|s, v| *s = v)),
        );
        let out = releases.clone();
        view.keep(Subscription::new(move || out.set(out.get() + 1)));
    });
    view.items(
        &view.root(),
        "rows",
        state.bind(|s| s.rows.clone(), None),
        Binding::constant(Viewport::default()),
        template.clone(),
    );
    view.content(
        &view.root(),
        "content",
        state.bind(|s| s.selected.clone(), None),
        template,
    );
    let ids = ["rows/good/entry", "content/good/entry"];
    let identities = ids.map(|id| view.mount_identity(id));
    let subscriptions = view.subscription_count();
    for key in ["", "/", "bad/key", "good/entry"] {
        let invalid = Keyed {
            key: key.into(),
            model: h.store("invalid".to_owned()),
        };
        let _ = state.update(move |s| {
            s.rows = vec![s.rows[0].clone(), invalid.clone()];
            s.selected = Some(invalid);
        });
        h.pump();
        assert_eq!(view.item_keys("rows"), ["good"]);
        assert_eq!(ids.map(|id| view.mount_identity(id)), identities);
        assert_eq!(view.subscription_count(), subscriptions);
        assert_eq!(released.get(), 0);
        assert!(view.error().unwrap().contains("invalid key"));
    }
    // Re-publishing the old content must still be a no-op (previous was not overwritten).
    let _ = state.update(move |s| {
        s.rows = vec![good.clone()];
        s.selected = Some(good);
    });
    h.pump();
    assert_eq!(ids.map(|id| view.mount_identity(id)), identities);
    view.edit(ids[0], "still bound").unwrap();
    h.pump();
    assert_eq!(view.text(ids[1]).as_deref(), Some("still bound"));
    let _ = state.update(|s| {
        s.rows.clear();
        s.selected = None;
    });
    h.pump();
    assert_eq!(released.get(), 2);
    assert!(!row.is_closed());
}

#[test]
fn native_scroll_message_moves_range_without_snapback_and_has_fractional_overscan() {
    let h = Harness::new();
    let rows = (0..100)
        .map(|i| Keyed::new(i.to_string(), h.store(i.to_string())))
        .collect();
    let model = h.store(Model {
        rows,
        viewport: Viewport {
            start: 0,
            count: 3,
            row_height: 20.0,
        },
        selected: None,
    });
    let view = View::new(Binding::constant("scroll".into()));
    view.items(
        &view.root(),
        "rows",
        model.bind(|s| s.rows.clone(), None),
        model.bind(|s| s.viewport, Some(|s, v| s.viewport = v)),
        Rc::new(|view, parent, _, model| {
            view.label(parent, "label", model.bind(Clone::clone, None))
        }),
    );
    view.0.scroll.borrow_mut().clear();
    view.dispatch(Message::Scroll("rows".into(), 45.0)).unwrap();
    h.pump();
    assert_eq!(view.item_keys("rows"), ["2", "3", "4", "5"]);
    assert_eq!(model.snapshot().viewport.start, 2);
    assert!(
        view.0.scroll.borrow().is_empty(),
        "native fractional offset must not snap back to a whole row"
    );
    view.viewport("rows", 8);
    h.pump();
    assert_eq!(view.item_keys("rows"), ["8", "9", "10"]);
    assert_eq!(view.0.scroll.borrow().get("rows"), Some(&160.0));
    assert!(view.text("rows/2/label").is_none());
}

#[test]
fn unsupported_host_does_not_construct_view() {
    let result = futures::executor::block_on(run_async(|_| panic!("must not construct")));
    assert!(result.unwrap_err().contains("external reactor host"));
}

#[test]
fn real_iced_wheel_event_reports_offset_and_virtualizes_the_visible_rows() {
    use iced_winit::runtime::user_interface::{Cache, UserInterface};
    struct Clipboard;
    impl iced::advanced::Clipboard for Clipboard {
        fn read(&self, _: iced::advanced::clipboard::Kind) -> Option<String> {
            None
        }
        fn write(&mut self, _: iced::advanced::clipboard::Kind, _: String) {}
    }
    let h = Harness::new();
    let view = View::new(Binding::constant("wheel".into()));
    view.items(
        &view.root(),
        "rows",
        Binding::constant(
            (0..100)
                .map(|i| Keyed::new(i.to_string(), h.store(i.to_string())))
                .collect(),
        ),
        Binding::constant(Viewport {
            start: 0,
            count: 3,
            row_height: 20.0,
        }),
        Rc::new(|view, parent, _, model| {
            view.label(parent, "label", model.bind(Clone::clone, None))
        }),
    );
    let mut renderer = iced_tiny_skia::Renderer::new(iced::Font::default(), iced::Pixels(16.0));
    let mut ui = UserInterface::build(
        view.root().0.element(),
        iced::Size::new(300.0, 200.0),
        Cache::default(),
        &mut renderer,
    );
    let mut messages = Vec::new();
    ui.update(
        &[iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
            delta: iced::mouse::ScrollDelta::Pixels { x: 0.0, y: -45.0 },
        })],
        iced::mouse::Cursor::Available(iced::Point::new(10.0, 10.0)),
        &mut renderer,
        &mut Clipboard,
        &mut messages,
    );
    assert!(messages.iter().any(|m| matches!(m, Message::Scroll(id, offset) if id == "rows" && (*offset - 45.0).abs() < 0.01)),
        "actual scrollable did not publish the real wheel offset: {messages:?}");
    drop(ui);
    for message in messages {
        view.dispatch(message).unwrap();
    }
    assert_eq!(view.item_keys("rows"), ["2", "3", "4", "5"]);
    assert!(view.text("rows/0/label").is_none());
}

#[test]
fn dynamic_menu_snapshots_replace_arguments_enabled_hotkeys_and_native_items() {
    use panechord_runtime::{MenuItem, MenuRole};
    let h = Harness::new();
    let save = MenuItem {
        id: "save".into(),
        label: "Save".into(),
        argument: "first".into(),
        shortcut: Some("Ctrl+KeyS".into()),
        role: MenuRole::Normal,
        enabled: true,
    };
    let state = h.store(("File".to_owned(), vec![save.clone()]));
    let received = Rc::new(RefCell::new(Vec::new()));
    let out = received.clone();
    let view = View::new(Binding::constant("menus".into()));
    view.menu(
        "file",
        state.bind(|s| s.0.clone(), Some(|s, v| s.0 = v)),
        state.bind(|s| s.1.clone(), Some(|s, v| s.1 = v)),
        Rc::new(move |arg| {
            out.borrow_mut().push(arg);
            true
        }),
    );
    view.activate_menu("save").unwrap();
    assert_eq!(*received.borrow(), ["first"]);
    let submenu = view.0.menus.items()[0].as_submenu().unwrap().clone();
    let old_native = submenu.items()[0].id().clone();
    let _ = state.update(|s| {
        s.0 = "Changed".into();
        s.1[0].argument = "second".into();
        s.1[0].enabled = false;
    });
    h.pump();
    assert_eq!(submenu.text(), "Changed");
    assert!(view.activate_menu("file/save").is_err());
    assert!(!view.0.menu_ids.borrow().contains_key(&old_native));
    let _ = state.update(|s| {
        s.1[0].enabled = true;
        s.1[0].shortcut = Some("Ctrl+Shift+KeyS".into());
    });
    h.pump();
    view.activate_menu("file/save").unwrap();
    assert_eq!(*received.borrow(), ["first", "second"]);
    let before = submenu.items()[0].id().clone();
    let old_ids = view.0.menu_ids.borrow().clone();
    let _ = state.update(|s| {
        s.1[0].argument = "rejected argument".into();
        s.1[0].shortcut = Some("BadModifier+NoKey".into());
    });
    h.pump();
    assert_eq!(submenu.items()[0].id(), &before); // bad snapshots cannot tear down the live menu
    assert_eq!(*view.0.menu_ids.borrow(), old_ids);
    view.activate_menu("file/save").unwrap();
    assert_eq!(*received.borrow(), ["first", "second", "second"]);
    let _ = state.update(|s| {
        s.1 = vec![MenuItem {
            id: "about".into(),
            label: "About".into(),
            argument: String::new(),
            shortcut: None,
            role: MenuRole::About,
            enabled: true,
        }];
    });
    h.pump();
    assert!(submenu.items()[0].as_predefined_menuitem().is_some());
    assert!(
        view.activate_menu("file/about")
            .unwrap_err()
            .contains("OS activation")
    );
    assert!(view.activate_menu("save").is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn linux_quit_shim_closes_owner_without_dispatch_or_ownership_cycle() {
    use panechord_runtime::{MenuItem, MenuRole};
    let view = View::new(Binding::constant("quit".into()));
    let owner = Rc::downgrade(&view.0);
    let closed = Rc::new(Cell::new(0));
    let out = closed.clone();
    view.on_close(Rc::new(move || out.set(out.get() + 1)));
    view.menu(
        "file",
        Binding::constant("File".into()),
        Binding::constant(vec![MenuItem {
            id: "quit".into(),
            label: "Quit".into(),
            argument: "must not dispatch".into(),
            shortcut: None,
            role: MenuRole::Quit,
            enabled: true,
        }]),
        Rc::new(|_| panic!("Quit is not an application command alias")),
    );
    let item = view.0.menus.items()[0].as_submenu().unwrap().items()[0].clone();
    assert!(item.as_menuitem().is_some());
    let action = view.0.menu_actions.borrow()["file/quit"].clone();
    let id = view.0.menu_ids.borrow()[item.id()].clone();
    view.dispatch(Message::Menu(id)).unwrap();
    assert_eq!(closed.get(), 1);
    drop(view);
    assert!(
        owner.upgrade().is_none(),
        "stored Quit closure must only retain a weak owner"
    );
    (action.close.unwrap())();
    assert_eq!(closed.get(), 1);
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires Xvfb/display and an isolated GTK3 process; run with --exact --ignored"]
fn native_menu_append_failure_restores_accelerators_registry_and_quit() {
    gtk::init().unwrap();
    for hierarchical in [false, true] {
        native_menu_append_failure(hierarchical);
    }
}

#[cfg(target_os = "linux")]
fn native_menu_append_failure(hierarchical: bool) {
    use gtk::prelude::*;
    use panechord_runtime::{MenuItem, MenuNode, MenuRole};
    let h = Harness::new();
    let save = MenuItem {
        id: "save".into(),
        label: "Save".into(),
        argument: "old argument".into(),
        shortcut: Some("Ctrl+KeyS".into()),
        role: MenuRole::Normal,
        enabled: true,
    };
    let quit = MenuItem {
        id: "quit".into(),
        label: "Quit".into(),
        argument: "never dispatch".into(),
        shortcut: None,
        role: MenuRole::Quit,
        enabled: true,
    };
    let original = vec![save, quit];
    let state = h.store(("File".to_owned(), original.clone()));
    let view = View::new(Binding::constant("native menu".into()));
    let received = Rc::new(RefCell::new(Vec::new()));
    let out = received.clone();
    let dispatch = Rc::new(move |argument| {
        out.borrow_mut().push(argument);
        true
    });
    if hierarchical {
        view.hierarchical_menu(
            "file",
            state.bind(|s| s.0.clone(), None),
            state.bind(
                |s| {
                    vec![MenuNode::Submenu {
                        id: "nested".into(),
                        label: "Nested".into(),
                        enabled: true,
                        children: s.1.iter().cloned().map(MenuNode::Item).collect(),
                    }]
                },
                None,
            ),
            dispatch,
        );
    } else {
        view.menu(
            "file",
            state.bind(|s| s.0.clone(), None),
            state.bind(|s| s.1.clone(), None),
            dispatch,
        );
    }
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
    window.add(&vbox);
    view.0
        .menus
        .init_for_gtk_window(&window, Some(&vbox))
        .unwrap();
    window.show_all();
    let bar = view
        .0
        .menus
        .clone()
        .gtk_menubar_for_gtk_window(&window)
        .unwrap();
    let menu = bar.children()[0]
        .clone()
        .downcast::<gtk::MenuItem>()
        .unwrap()
        .submenu()
        .unwrap()
        .downcast::<gtk::Menu>()
        .unwrap();
    let submenu = view.0.menus.items()[0].as_submenu().unwrap().clone();
    let current_menu = || {
        if hierarchical {
            menu.children()[0]
                .clone()
                .downcast::<gtk::MenuItem>()
                .unwrap()
                .submenu()
                .unwrap()
                .downcast::<gtk::Menu>()
                .unwrap()
        } else {
            menu.clone()
        }
    };
    let old_ids = view.0.menu_ids.borrow().clone();
    let old_native: Vec<_> = submenu.items().iter().map(|i| i.id().clone()).collect();
    let deliver = || {
        let event = muda::MenuEvent::receiver()
            .try_recv()
            .expect("actual native activation event");
        let id = view.0.menu_ids.borrow()[&event.id].clone();
        view.dispatch(Message::Menu(id)).unwrap();
    };
    for shortcut in ["BadModifier+NoKey", "Ctrl+Fn"] {
        let _ = state.update(move |s| {
            s.0 = "Changed title".into();
            let mut first = s.1[0].clone();
            first.id = "new".into();
            first.label = "New".into();
            first.argument = "new argument".into();
            first.shortcut = Some("Ctrl+KeyN".into());
            let mut failing = first.clone();
            failing.id = "bad".into();
            failing.shortcut = Some(shortcut.into());
            s.1 = vec![first, failing];
        });
        h.pump();
        assert_eq!(submenu.text(), "Changed title");
        assert_eq!(*view.0.menu_ids.borrow(), old_ids);
        assert_eq!(
            submenu
                .items()
                .iter()
                .map(|i| i.id().clone())
                .collect::<Vec<_>>(),
            old_native
        );
        let labels: Vec<_> = current_menu()
            .children()
            .into_iter()
            .map(|child| child.downcast::<gtk::MenuItem>().unwrap().label().unwrap())
            .collect();
        assert_eq!(labels, ["Save", "Quit"]);
        assert!(gtk::accel_groups_activate(
            &window,
            *gtk::gdk::keys::constants::s,
            gtk::gdk::ModifierType::CONTROL_MASK
        ));
        deliver();
        assert_eq!(received.borrow().last().unwrap(), "old argument");
        assert!(!gtk::accel_groups_activate(
            &window,
            *gtk::gdk::keys::constants::n,
            gtk::gdk::ModifierType::CONTROL_MASK
        ));
    }
    assert!(view.error().unwrap().contains("previous snapshot restored"));
    // A valid update after rollback must still work, including native Quit enablement.
    let _ = state.update(move |s| {
        s.1 = original;
        s.1[1].enabled = false;
    });
    h.pump();
    assert!(!current_menu().children()[1].is_sensitive());
    assert!(view.activate_menu("file/quit").is_err());
    let _ = state.update(|s| s.1[1].enabled = true);
    h.pump();
    let closed = Rc::new(Cell::new(false));
    let out = closed.clone();
    view.on_close(Rc::new(move || out.set(true)));
    let quit = current_menu().children()[1]
        .clone()
        .downcast::<gtk::MenuItem>()
        .unwrap();
    assert!(quit.is_sensitive());
    quit.emit_by_name::<()>("activate", &[]);
    deliver();
    assert!(closed.get());
    assert_eq!(*received.borrow(), ["old argument", "old argument"]);
    menu::clear_accelerators(&view.0.menus.items()).unwrap();
    view.0.menus.remove_for_gtk_window(&window).unwrap();
    unsafe {
        window.destroy();
    }
}
