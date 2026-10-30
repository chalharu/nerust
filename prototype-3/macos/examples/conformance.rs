#[cfg(target_os = "macos")]
use std::{
    cell::{Cell, RefCell},
    num::NonZeroUsize,
    rc::Rc,
    task::Poll,
    time::Instant,
};
#[cfg(target_os = "macos")]
use wakui2_runtime::{Binding, Keyed, Layout, Limits, MenuItem, MenuRole, Store, Style, Viewport};
#[cfg(target_os = "macos")]
use wakui3_macos::View;

#[cfg(target_os = "macos")]
#[derive(Clone, Default)]
struct Model {
    text: String,
    second: String,
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("AppKit conformance requires macOS");
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), String> {
    let passed = Rc::new(Cell::new(false));
    let finished = passed.clone();
    let result = wakui3_macos::run(move |executor| {
        let model = Store::new(Model::default(), executor.clone());
        let view = View::new(Binding::constant("Wakui macOS conformance".into()));
        let parent = view.layout(
            &view.root(),
            "screen",
            Layout::Stack {
                horizontal: false,
                gap: 8.0,
            },
        );
        let style = Style {
            padding: wakui2_runtime::Edges {
                top: 20.0,
                right: 20.0,
                bottom: 20.0,
                left: 20.0,
            },
            ..Style::default()
        };
        view.style("screen", style);
        view.label(&parent, "name_caption", Binding::constant("Name".into()));
        view.entry(
            &parent,
            "name",
            model.bind(
                |state| state.text.clone(),
                Some(|state, value| state.text = value),
            ),
        );
        view.label(
            &parent,
            "second_caption",
            Binding::constant("Next field".into()),
        );
        view.entry(
            &parent,
            "second",
            model.bind(
                |state| state.second.clone(),
                Some(|state, value| state.second = value),
            ),
        );
        view.entry(
            &parent,
            "readonly",
            Binding::constant("Read-only summary".into()),
        );
        let range = Store::new(
            Viewport {
                count: 0,
                row_height: 32.0,
                ..Viewport::default()
            },
            executor.clone(),
        );
        let row_models: Vec<_> = (0..100)
            .map(|index| {
                Keyed::new(
                    index.to_string(),
                    Store::new(format!("File {index:03}.nes"), executor.clone()),
                )
            })
            .collect();
        let rows = Store::new(row_models, executor.clone());
        let row_builds = Rc::new(Cell::new(0));
        let builds = row_builds.clone();
        view.items(
            &parent,
            "rows",
            rows.bind(Clone::clone, None),
            range.bind(|state| *state, Some(|state, value| *state = value)),
            Rc::new(move |view, parent, scope, model: Store<String>| {
                builds.set(builds.get() + 1);
                view.label(
                    parent,
                    &format!("{scope}/label"),
                    model.bind(Clone::clone, None),
                );
            }),
        );
        assert_eq!(view.mounted_count("rows"), 0);
        assert!(
            view.items_error("rows")
                .unwrap()
                .contains("invalid viewport")
        );
        range.update(|viewport| viewport.count = 6);
        assert_eq!(view.mounted_count("rows"), 6);
        assert!(view.items_error("rows").is_none());
        let readonly_range = Store::new(
            Viewport {
                count: 2,
                row_height: 32.0,
                start: 0,
            },
            executor.clone(),
        );
        view.items(
            &parent,
            "readonly_rows",
            rows.bind(Clone::clone, None),
            readonly_range.bind(|state| *state, None),
            Rc::new(|view, parent, scope, model: Store<String>| {
                view.label(
                    parent,
                    &format!("{scope}/label"),
                    model.bind(Clone::clone, None),
                );
            }),
        );
        view.viewport("readonly_rows", 5);
        assert_eq!(view.item_keys("readonly_rows"), ["5", "6"]);
        assert!(view.text("readonly_rows/0/label").is_none());
        assert_eq!(readonly_range.snapshot().start, 0);
        readonly_range.update(|viewport| viewport.start = 9);
        assert_eq!(view.item_keys("readonly_rows"), ["9", "10"]);
        for id in ["__close", "rows@viewport", "__title"] {
            view.entry(
                &parent,
                id,
                model.bind(
                    |state| state.second.clone(),
                    Some(|state, value| state.second = value),
                ),
            );
            view.edit(id, "ordinary input").unwrap();
            assert!(view.downgrade().upgrade().is_some());
            assert_eq!(model.snapshot().second, "ordinary input");
            assert_eq!(view.text(id).as_deref(), Some("ordinary input"));
        }
        view.viewport("rows", 5);
        assert_eq!(view.item_keys("rows"), ["5", "6", "7", "8", "9", "10"]);
        rows.update(|items| items[99].key = items[98].key.clone());
        assert!(view.items_error("rows").is_some());
        view.viewport("rows", 10);
        assert_eq!(view.item_keys("rows"), ["10", "11", "12", "13", "14", "15"]);
        assert!(view.items_error("rows").is_some());
        rows.update(|items| items[99].key = "99".into());
        range.update(|viewport| viewport.start = 0);
        assert!(view.items_error("rows").is_none());
        println!("MACOS3 PASS event-namespace rejected-source-scroll");
        println!("MACOS3 PASS readonly-viewport local-scroll model-unchanged external-update");
        let typed_argument = Rc::new(Cell::new(0_i32));
        let output = typed_argument.clone();
        view.typed_menu(
            "typed",
            Binding::constant("Typed".into()),
            Binding::constant(vec![MenuItem {
                id: "integer".into(),
                label: "Integer".into(),
                argument: 42_i32,
                shortcut: None,
                role: MenuRole::Normal,
                enabled: true,
            }]),
            Rc::new(move |argument| {
                output.set(argument);
                true
            }),
        );
        view.activate_menu("typed/integer").unwrap();
        assert_eq!(typed_argument.get(), 42);
        println!("MACOS3 PASS typed-menu integer-argument");
        let valid_menu = MenuItem {
            id: "save".into(),
            label: "Save".into(),
            argument: "original".into(),
            shortcut: None,
            role: MenuRole::Normal,
            enabled: true,
        };
        let transaction = Store::new(vec![valid_menu.clone()], executor.clone());
        let arguments = Rc::new(RefCell::new(Vec::new()));
        let output = arguments.clone();
        view.menu(
            "transaction",
            Binding::constant("Transaction".into()),
            transaction.bind(Clone::clone, None),
            Rc::new(move |argument| {
                output.borrow_mut().push(argument);
                true
            }),
        );
        let mut invalid_id = valid_menu.clone();
        invalid_id.id.clear();
        let mut bad_shortcut = valid_menu.clone();
        bad_shortcut.id = "bad_shortcut".into();
        bad_shortcut.shortcut = Some("Primary+Unknown".into());
        let mut uncommitted = valid_menu.clone();
        uncommitted.argument = "uncommitted".into();
        for candidate in [
            vec![invalid_id],
            vec![uncommitted.clone(), valid_menu.clone()],
            vec![uncommitted, bad_shortcut],
        ] {
            transaction.update(move |items| *items = candidate);
            assert!(view.menu_error("transaction").is_some());
            view.activate_menu("transaction/save").unwrap();
            assert_eq!(
                arguments.borrow().last().map(String::as_str),
                Some("original")
            );
        }
        let mut replacement = valid_menu;
        replacement.argument = "updated".into();
        transaction.update(move |items| *items = vec![replacement]);
        assert!(view.menu_error("transaction").is_none());
        view.activate_menu("transaction/save").unwrap();
        assert_eq!(
            &*arguments.borrow(),
            &["original", "original", "original", "updated"]
        );
        println!(
            "MACOS3 PASS menu-transaction invalid-id duplicate-id invalid-shortcut preserve-handler recovery"
        );
        let nested_rows = Store::new(Vec::<Keyed<String>>::new(), executor.clone());
        let lifetimes = Rc::new(RefCell::new(Vec::new()));
        let output = lifetimes.clone();
        view.items(
            &parent,
            "nested",
            nested_rows.bind(Clone::clone, None),
            Binding::constant(Viewport::default()),
            Rc::new(move |view, parent, scope, model: Store<String>| {
                let lifetime = Rc::new(());
                output.borrow_mut().push(Rc::downgrade(&lifetime));
                view.items(
                    parent,
                    &format!("{scope}/children"),
                    Binding::constant(vec![Keyed::new("child", model)]),
                    Binding::constant(Viewport::default()),
                    Rc::new(move |view, parent, scope, model: Store<String>| {
                        let _retained = &lifetime;
                        view.label(
                            parent,
                            &format!("{scope}/label"),
                            model.bind(Clone::clone, None),
                        );
                    }),
                );
            }),
        );
        for _ in 0..3 {
            let row = Keyed::new("parent", Store::new("Child".to_owned(), executor.clone()));
            nested_rows.update(move |rows| rows.push(row));
            assert_eq!(view.item_keys("nested/parent/children"), ["child"]);
            nested_rows.update(Vec::clear);
            assert_eq!(view.mounted_count("nested/parent/children"), 0);
            assert!(view.item_keys("nested/parent/children").is_empty());
            assert!(view.text("nested/parent/children/child/label").is_none());
            view.viewport("nested/parent/children", 0);
            assert!(view.text("nested/parent/children/child/label").is_none());
            assert!(
                lifetimes
                    .borrow()
                    .iter()
                    .all(|lifetime| lifetime.upgrade().is_none())
            );
        }
        println!("MACOS3 PASS nested-cleanup registries handlers tokens released key-reuse");
        let enabled = Store::new(true, executor.clone());
        let clicks = Rc::new(Cell::new(0));
        let counter = clicks.clone();
        view.menu(
            "actions",
            Binding::constant("Actions".into()),
            enabled.bind(
                |enabled| {
                    vec![MenuItem {
                        id: "save".into(),
                        label: "Save".into(),
                        argument: "save".into(),
                        shortcut: Some("Primary+S".into()),
                        role: MenuRole::Normal,
                        enabled: *enabled,
                    }]
                },
                None,
            ),
            Rc::new(move |argument| {
                assert_eq!(argument, "save");
                counter.set(counter.get() + 1);
                true
            }),
        );
        let load = Store::with_dispatch_budget(
            0usize,
            executor.clone(),
            Limits::default(),
            NonZeroUsize::new(8).unwrap(),
        );
        view.label(
            &parent,
            "load",
            load.bind(|value| format!("Updates: {value}"), None),
        );
        let load_token = load.watch(load.bind(|value| *value, None), |target, value| {
            if value < 4000 {
                assert!(target.update(move |state| *state = value + 1));
            }
        });
        view.keep(load_token);
        let closed_model = model.clone();
        view.on_close(Rc::new(move || closed_model.close()));
        let owner = view.clone();
        executor.spawn(Box::pin(async move {
            let activated = Instant::now();
            while !owner.is_key_window() {
                assert!(activated.elapsed() < std::time::Duration::from_secs(5), "native window did not become active");
                futures_timer::Delay::new(std::time::Duration::from_millis(8)).await;
            }
            owner.probe_input("name").unwrap();
            assert_eq!(model.snapshot().text, "\u{65e5}\u{672c}\u{8a9e}");
            assert!(owner.edit("readonly", "forbidden").is_err());
            assert_eq!(owner.text("readonly").as_deref(), Some("Read-only summary"));
            println!("MACOS3 PASS injected-field-editor marked-text commit Japanese clipboard cut paste Tab accessibility-role readonly");
            assert!(owner.perform_shortcut("Primary+S").unwrap());
            assert_eq!(clicks.get(), 1);
            enabled.update(|value| *value = false);
            owner.perform_shortcut("Primary+S").unwrap();
            assert_eq!(clicks.get(), 1);
            assert!(owner.activate_menu("actions/save").is_err());
            enabled.update(|value| *value = true);
            assert!(owner.perform_shortcut("Cmd+S").unwrap());
            assert_eq!(clicks.get(), 2);
            println!("MACOS3 PASS native-menu Primary-Cmd shortcut enabled-state rebuild");
            assert_eq!(owner.item_keys("rows"), ["0", "1", "2", "3", "4", "5"]);
            assert_eq!(owner.mounted_count("rows"), 6);
            owner.viewport("rows", 90);
            assert_eq!(owner.item_keys("rows"), ["90", "91", "92", "93", "94", "95"]);
            assert!(owner.text("rows/0/label").is_none());
            assert_eq!(owner.text("rows/90/label").as_deref(), Some("File 090.nes"));
            assert_eq!(owner.mounted_count("rows"), 6);
            let valid = range.snapshot();
            for invalid in [
                Viewport { count: 0, ..valid },
                Viewport { count: 257, ..valid },
                Viewport { row_height: f32::NAN, ..valid },
                Viewport { row_height: 0.5, ..valid },
                Viewport { row_height: 4097.0, ..valid },
            ] {
                range.update(move |viewport| *viewport = invalid);
                assert!(owner.items_error("rows").unwrap().contains("invalid viewport"));
                assert_eq!(owner.mounted_count("rows"), 6);
                assert_eq!(owner.text("rows/90/label").as_deref(), Some("File 090.nes"));
            }
            range.update(move |viewport| *viewport = Viewport { start: usize::MAX, ..valid });
            assert!(owner.items_error("rows").is_none());
            assert_eq!(owner.text("rows/94/label").as_deref(), Some("File 094.nes"));
            range.update(move |viewport| *viewport = valid);
            println!("MACOS3 PASS viewport-contract initial-rejection retain-mounts last-page recovery");
            let before = row_builds.get();
            assert_eq!(owner.frame_height("rows/90"), Some(32.0));
            range.update(|viewport| viewport.row_height = 64.0);
            assert_eq!(owner.frame_height("rows/90"), Some(64.0));
            assert_eq!(row_builds.get(), before);
            assert_eq!(owner.item_keys("rows"), ["90", "91", "92", "93", "94", "95"]);
            range.update(|viewport| viewport.row_height = 32.0);
            assert_eq!(owner.frame_height("rows/90"), Some(32.0));
            assert_eq!(row_builds.get(), before);
            println!("MACOS3 PASS dynamic-row-height native-frame 32-64-32 no-remount");
            rows.update(|state| state.swap(90, 91));
            assert_eq!(owner.text("rows/90/label").as_deref(), Some("File 090.nes"));
            rows.update(|state| state[1].key = state[0].key.clone());
            assert!(owner.items_error("rows").is_some());
            assert_eq!(owner.mounted_count("rows"), 6);
            rows.update(|state| state[1].key = "1".into());
            assert!(owner.items_error("rows").is_none());
            println!("MACOS3 PASS keyed-rows viewport mount-bound reorder invalid-key recovery");
            let directory = std::env::temp_dir().join(format!("wakui3-macos-conformance-{}", std::process::id()));
            std::fs::create_dir(&directory).unwrap();
            owner.capture(&directory.join("native-640.png"), 640.0).unwrap();
            owner.capture(&directory.join("native-960.png"), 960.0).unwrap();
            println!("MACOS3 captures {}", directory.display());
            let start = Rc::new(Cell::new(None::<Instant>));
            let latencies = Rc::new(RefCell::new(Vec::new()));
            let mut renders = Vec::new();
            let timer = start.clone();
            let observations = latencies.clone();
            let workload = load.clone();
            let input = model.bind(|state| state.text.clone(), None).subscribe(Rc::new(move |text| {
                if text == "x" && let Some(started) = timer.take() {
                    assert!(workload.snapshot() < 4000, "input was starved by the reaction chain");
                    observations.borrow_mut().push(started.elapsed());
                }
            }));
            for _ in 0..20 {
                owner.edit("name", "").unwrap();
                load.update(|state| *state = 0);
                start.set(Some(Instant::now()));
                assert!(owner.post_test_key("name"));
                renders.push(owner.render());
                futures::future::poll_fn(|context| {
                    if load.snapshot() == 4000 && start.get().is_none() {
                        Poll::Ready(())
                    } else {
                        context.waker().wake_by_ref();
                        Poll::Pending
                    }
                }).await;
            }
            drop(input);
            let mut timings = latencies.borrow().clone();
            timings.sort_unstable();
            assert_eq!(timings.len(), 20);
            renders.sort_unstable();
            println!("MACOS3 PASS reaction-chain input-before-completion samples=20 updates=80000 synthetic-key-p50-us={} p95-us={} max-us={} (in-process AppKit events, not physical input or compositor latency)",
                timings[10].as_micros(), timings[19].as_micros(), timings[19].as_micros());
            println!("MACOS3 draw samples=20 p50-us={} p95-us={} max-us={} (AppKit CPU draw during queued reactions, not compositor frame time)",
                renders[10].as_micros(), renders[19].as_micros(), renders[19].as_micros());
            let weak = owner.downgrade();
            owner.request_close();
            assert!(weak.upgrade().is_none());
            assert!(model.is_closed());
            assert!(owner.edit("name", "closed").is_err());
            owner.close();
            finished.set(true);
            drop(range);
            println!("MACOS3 PASS weak-view close model-closed idempotent");
        }));
        view
    });
    result?;
    if passed.get() {
        Ok(())
    } else {
        Err("macOS conformance did not complete".into())
    }
}
