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
    vsync: bool,
    region: String,
    volume: f64,
    renderer: String,
    renderers: Vec<Choice>,
    frame: Option<ImageFrame>,
    page: String,
    open: bool,
}
#[wakui2_macros::commands]
impl Controls {
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
        let model = Store::new(
            Controls {
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
                open: false,
            },
            executor.clone(),
        );
        let view = wakui2_macros::view!("examples/controls.yaml", model, backend);
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
            completed.set(true);
            println!("CONTROLS3 PASS checkbox readonly radio slider combo image tab dialog");
            probe.close();
            assert!(probe.toggle("vsync", true).is_err());
            assert!(probe.select("pages", "video").is_err());
            assert!(probe.set_open("dialog", true).is_err());
        }));
        view
    })
}
