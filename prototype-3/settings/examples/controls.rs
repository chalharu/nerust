use std::{rc::Rc, sync::Arc};
#[cfg(feature = "backend-gtk")]
use wakui2_gtk as backend;
use wakui2_runtime::{Choice, ImageFrame, Store};
#[cfg(all(feature = "backend-tao", not(feature = "backend-gtk")))]
use wakui2_tao as backend;
#[cfg(all(
    feature = "backend-macos",
    not(any(feature = "backend-gtk", feature = "backend-tao"))
))]
use wakui3_macos as backend;

#[derive(Clone)]
struct Controls {
    shown: bool,
    ready: bool,
    volume_style: wakui2_runtime::Style,
    vsync: bool,
    region: String,
    volume: f64,
    renderer: String,
    renderers: Vec<Choice>,
    frame: Option<ImageFrame>,
    page: String,
    menu_tree: Vec<wakui2_runtime::MenuNode<String>>,
    menu_title: String,
    open: bool,
}
#[wakui2_macros::commands]
impl Controls {
    #[command]
    fn select_slot(&mut self, argument: String) {
        self.region = argument;
    }
    #[command]
    fn open_dialog(&mut self) {
        self.open = true;
    }
    #[command]
    fn dismiss(&mut self) {
        self.open = false;
    }
}
fn main() -> Result<(), String> {
    let mut pixels = Vec::new();
    for row in 0..120 {
        for column in 0..160 {
            let color = if (row / 15 + column / 20) % 2 == 0 {
                [28, 156, 116, 255]
            } else {
                [244, 192, 72, 255]
            };
            pixels.extend_from_slice(&color);
        }
    }
    backend::run(move |executor| {
        let rejected = Store::new(42.4, executor.clone());
        let sample =
            backend::View::new(wakui2_runtime::Binding::constant("rejected slider".into()));
        assert!(
            sample
                .try_tab_page(
                    &sample.root(),
                    "bad_page",
                    wakui2_runtime::Binding::constant("bad".into()),
                    "bad"
                )
                .is_err()
        );
        sample.slider(
            &sample.root(),
            "slider",
            rejected.bind(|value| *value, Some(|value, next| *value = next)),
            wakui2_runtime::SliderRange {
                min: 0.0,
                max: 100.0,
                step: 1.0,
            },
        );
        assert_eq!(sample.slider_value("slider"), Some(42.0));
        sample.enabled("slider", wakui2_runtime::Binding::constant(false));
        assert!(sample.slide("slider", 90.0).is_err());
        assert_eq!(rejected.snapshot(), 42.4);
        sample.enabled("slider", wakui2_runtime::Binding::constant(true));
        sample.visible("slider", wakui2_runtime::Binding::constant(false));
        assert!(sample.slide("slider", 90.0).is_err());
        assert_eq!(rejected.snapshot(), 42.4);
        sample.visible("slider", wakui2_runtime::Binding::constant(true));
        rejected.close();
        assert!(sample.slide("slider", 80.0).is_err());
        assert_eq!(rejected.snapshot(), 42.4);
        assert_eq!(sample.slider_value("slider"), Some(42.0));
        let capacity = Store::with_limits(
            42.4,
            executor.clone(),
            wakui2_runtime::Limits {
                ingress: 0,
                tasks: 1,
            },
        );
        let capacity_binding = capacity.bind(|value| *value, Some(|value, next| *value = next));
        assert!(matches!(
            capacity_binding.try_set(80.0),
            Err(wakui2_runtime::WriteError::Capacity)
        ));
        sample.slider(
            &sample.root(),
            "capacity",
            capacity_binding,
            wakui2_runtime::SliderRange {
                min: 0.0,
                max: 100.0,
                step: 1.0,
            },
        );
        assert!(sample.slide("capacity", 80.0).is_err());
        assert_eq!(sample.slider_value("capacity"), Some(42.0));
        assert_eq!(capacity.snapshot(), 42.4);
        let converted = Store::new(42.4, executor.clone());
        let converted_binding = converted
            .bind(|value| *value, Some(|value, next| *value = next))
            .convert(|value| value, |_| Err("rejected conversion".into()));
        assert!(matches!(
            converted_binding.try_set(80.0),
            Err(wakui2_runtime::WriteError::Conversion(_))
        ));
        sample.slider(
            &sample.root(),
            "conversion",
            converted_binding,
            wakui2_runtime::SliderRange {
                min: 0.0,
                max: 100.0,
                step: 1.0,
            },
        );
        assert!(sample.slide("conversion", 80.0).is_err());
        assert_eq!(sample.slider_value("conversion"), Some(42.0));
        assert_eq!(converted.snapshot(), 42.4);
        println!(
            "CONTRACT3 PASS slider closed capacity conversion readonly hidden disabled normalized-display"
        );
        sample.close();
        assert_eq!(sample.slider_value("slider"), None);
        let model = Store::new(
            Controls {
                shown: true,
                ready: true,
                volume_style: wakui2_runtime::Style::default(),
                vsync: false,
                region: "ntsc".into(),
                volume: 70.0,
                renderer: "nearest".into(),
                renderers: vec![
                    Choice {
                        id: "nearest".into(),
                        label: "Nearest".into(),
                    },
                    Choice {
                        id: "linear".into(),
                        label: "Linear".into(),
                    },
                ],
                frame: Some(ImageFrame::new(160, 120, Arc::from(pixels)).unwrap()),
                page: "video".into(),
                menu_tree: vec![wakui2_runtime::MenuNode::Submenu {
                    id: "states".into(),
                    label: "States".into(),
                    enabled: true,
                    children: vec![wakui2_runtime::MenuNode::Submenu {
                        id: "select".into(),
                        label: "Select Slot".into(),
                        enabled: true,
                        children: vec![wakui2_runtime::MenuNode::Item(wakui2_runtime::MenuItem {
                            id: "slot".into(),
                            label: "Slot 1".into(),
                            argument: "slot-1".into(),
                            shortcut: Some("Ctrl+S".into()),
                            role: wakui2_runtime::MenuRole::Normal,
                            enabled: true,
                        })],
                    }],
                }],
                menu_title: "Emulation".into(),
                open: false,
            },
            executor.clone(),
        );
        assert!(model.update(|state| state.page = "missing".into()));
        let rejected = wakui2_macros::try_view!("examples/controls.yaml", model, backend);
        assert!(rejected.is_err());
        assert!(!model.is_closed());
        let owner = Store::new(model.snapshot(), executor.clone())
            .try_into_owner()
            .ok()
            .unwrap();
        let shared = owner.store().clone();
        let rejected_owned =
            wakui2_macros::try_owned_view!("examples/controls.yaml", owner, backend);
        assert!(rejected_owned.is_err());
        assert!(shared.is_closed());
        assert!(model.update(|state| state.page = "video".into()));
        let view = wakui2_macros::try_view!("examples/controls.yaml", model, backend).unwrap();
        let probe = view.clone();
        let completed = Rc::new(std::cell::Cell::new(false));
        let observed = completed.clone();
        view.on_close(Rc::new(move || {
            assert!(observed.get(), "controls probe closed before completion")
        }));
        if std::env::var_os("WAKUI_CONTROLS_REVIEW").is_some() {
            completed.set(true);
            return view;
        }
        executor.spawn(Box::pin(async move {
            probe.activate_menu("emulation/slot").unwrap();
            assert_eq!(model.snapshot().region, "slot-1");
            assert!(model.update(|state| state.menu_title = "Emulation Updated".into()));
            probe.activate_menu("emulation/slot").unwrap();
            assert!(model.update(|state| {
                if let wakui2_runtime::MenuNode::Submenu { enabled, .. } = &mut state.menu_tree[0] {
                    *enabled = false;
                }
            }));
            assert!(probe.activate_menu("emulation/slot").is_err());
            assert_eq!(model.snapshot().region, "slot-1");
            assert!(model.update(|state| {
                if let wakui2_runtime::MenuNode::Submenu { enabled, .. } = &mut state.menu_tree[0] {
                    *enabled = true;
                }
            }));
            println!("MENUS3 PASS nested typed-argument parent-enablement");
            assert!(model.update(|state| state.shown = false));
            assert!(probe.slide("volume", 50.0).is_err());
            assert_eq!(probe.slider_value("volume"), Some(70.0));
            assert!(model.update(|state| {
                state.shown = true;
                state.ready = false;
            }));
            assert!(probe.slide("volume", 50.0).is_err());
            assert_eq!(probe.slider_value("volume"), Some(70.0));
            assert!(model.update(|state| state.ready = true));
            probe.enabled("readonly", wakui2_runtime::Binding::constant(true));
            probe.toggle("vsync", true).unwrap();
            assert!(model.snapshot().vsync);
            probe.click("vsync").unwrap();
            assert!(!model.snapshot().vsync);
            probe.toggle("vsync", true).unwrap();
            assert!(probe.toggle("readonly", true).is_err());
            assert!(probe.click("readonly").is_err());
            probe.dialog(
                &probe.root(),
                "readonly_dialog",
                wakui2_runtime::Binding::constant("Read Only".into()),
                wakui2_runtime::Binding::constant(false),
            );
            assert!(probe.set_open("readonly_dialog", true).is_err());
            probe.click("pal").unwrap();
            assert_eq!(model.snapshot().region, "pal");
            probe.slide("volume", 42.4).unwrap();
            assert_eq!(model.snapshot().volume, 42.0);
            assert!(probe.slide("volume", f64::NAN).is_err());
            probe.select("renderer", "linear").unwrap();
            assert_eq!(model.snapshot().renderer, "linear");
            assert!(probe.select("renderer", "missing").is_err());
            probe.select("pages", "audio").unwrap();
            assert_eq!(model.snapshot().page, "audio");
            probe.select("pages", "video").unwrap();
            for position in [
                wakui2_runtime::TabPosition::Top,
                wakui2_runtime::TabPosition::Bottom,
                wakui2_runtime::TabPosition::Left,
                wakui2_runtime::TabPosition::Right,
            ] {
                probe.tab_position("pages", position).unwrap();
                probe.select("pages", "audio").unwrap();
                assert_eq!(model.snapshot().page, "audio");
                probe.select("pages", "video").unwrap();
                assert_eq!(model.snapshot().page, "video");
                #[cfg(feature = "backend-macos")]
                if let Some(directory) = std::env::var_os("WAKUI_CONTROLS_CAPTURE") {
                    let directory = std::path::PathBuf::from(directory);
                    std::fs::create_dir_all(&directory).unwrap();
                    probe
                        .capture(&directory.join(format!("tabs-{position:?}.png")), 640.0)
                        .unwrap();
                }
            }
            probe
                .tab_position("pages", wakui2_runtime::TabPosition::Left)
                .unwrap();
            assert!(
                probe
                    .tab_position("missing", wakui2_runtime::TabPosition::Left)
                    .is_err()
            );
            assert!(
                probe
                    .tab_position("vsync", wakui2_runtime::TabPosition::Left)
                    .is_err()
            );
            #[cfg(feature = "backend-macos")]
            if let Some(directory) = std::env::var_os("WAKUI_CONTROLS_CAPTURE") {
                let directory = std::path::PathBuf::from(directory);
                std::fs::create_dir_all(&directory).unwrap();
                probe
                    .capture(&directory.join("controls-640.png"), 640.0)
                    .unwrap();
                probe
                    .capture(&directory.join("controls-960.png"), 960.0)
                    .unwrap();
            }
            probe.click("open_dialog").unwrap();
            assert!(model.snapshot().open);
            probe.click("dismiss").unwrap();
            assert!(!model.snapshot().open);
            probe.set_open("dialog", true).unwrap();
            assert!(model.snapshot().open);
            probe.set_open("dialog", false).unwrap();
            assert!(!model.snapshot().open);
            let dialog_parent = probe
                .try_layout(
                    &probe.root(),
                    "dialog_parent",
                    wakui2_runtime::Layout::Stack {
                        horizontal: false,
                        gap: 0.0,
                    },
                )
                .unwrap();
            probe.dialog(
                &dialog_parent,
                "nested_dialog",
                wakui2_runtime::Binding::constant("Nested".into()),
                model.bind(|state| state.open, Some(|state, next| state.open = next)),
            );
            probe.visible("dialog_parent", wakui2_runtime::Binding::constant(false));
            assert!(probe.set_open("nested_dialog", true).is_err());
            assert!(!model.snapshot().open);
            probe.visible("dialog_parent", wakui2_runtime::Binding::constant(true));
            probe.set_open("nested_dialog", true).unwrap();
            assert!(model.snapshot().open);
            probe.set_open("nested_dialog", false).unwrap();
            assert!(!model.snapshot().open);
            println!("DIALOG3 PASS readonly hidden-parent close-reopen");
            let stack = wakui2_runtime::Layout::Stack {
                horizontal: false,
                gap: 0.0,
            };
            probe.try_layout(&probe.root(), "boundary", stack).unwrap();
            assert!(probe.try_layout(&probe.root(), "boundary", stack).is_err());
            assert!(
                probe
                    .try_layout(
                        &probe.root(),
                        "invalid_grid",
                        wakui2_runtime::Layout::Grid {
                            columns: u32::MAX,
                            gap: 0.0
                        }
                    )
                    .is_err()
            );
            let foreign = backend::View::new(wakui2_runtime::Binding::constant("Foreign".into()));
            assert!(probe.try_layout(&foreign.root(), "foreign", stack).is_err());
            foreign.close();
            let tab = probe.tab(
                &probe.root(),
                "invalid_tab",
                wakui2_runtime::Binding::constant("missing".into()),
            );
            probe.tab_page(
                &tab,
                "invalid_page",
                wakui2_runtime::Binding::constant("Only".into()),
                "only",
            );
            assert!(probe.finish_tab(&tab).is_err());
            let readonly = probe.tab(
                &probe.root(),
                "readonly_tab",
                wakui2_runtime::Binding::constant("one".into()),
            );
            for key in ["one", "two"] {
                probe.tab_page(
                    &readonly,
                    &format!("readonly_{key}"),
                    wakui2_runtime::Binding::constant(key.into()),
                    key,
                );
            }
            probe.finish_tab(&readonly).unwrap();
            for position in [
                wakui2_runtime::TabPosition::Left,
                wakui2_runtime::TabPosition::Right,
            ] {
                probe.tab_position("readonly_tab", position).unwrap();
                assert!(probe.select("readonly_tab", "two").is_err());
            }
            println!("TABS3 PASS top bottom left right selection retained");
            completed.set(true);
            println!("CONTROLS3 PASS checkbox readonly radio slider combo image tab dialog");
            probe.close();
            assert!(probe.toggle("vsync", true).is_err());
            assert!(probe.select("pages", "video").is_err());
            assert!(
                probe
                    .tab_position("pages", wakui2_runtime::TabPosition::Left)
                    .is_err()
            );
            assert!(probe.set_open("dialog", true).is_err());
        }));
        view
    })
}
