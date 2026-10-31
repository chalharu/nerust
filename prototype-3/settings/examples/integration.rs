use std::{
    cell::Cell,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};
#[cfg(feature = "backend-gtk")]
use wakui2_gtk as backend;
#[cfg(not(feature = "backend-gtk"))]
use wakui2_runtime::UiHost;
use wakui2_runtime::{Binding, ImageFrame, Session, Store, WindowHooks};
#[cfg(all(feature = "backend-tao", not(feature = "backend-gtk")))]
use wakui2_tao as backend;
#[cfg(all(
    feature = "backend-macos",
    not(any(feature = "backend-gtk", feature = "backend-tao"))
))]
use wakui3_macos as backend;

#[derive(Clone)]
struct Emulator {
    generation: u64,
    frame: ImageFrame,
    address: String,
    selection: String,
    tick: u64,
}

fn start(
    executor: Rc<dyn wakui2_runtime::Executor>,
    present: Rc<dyn Fn(&backend::View)>,
) -> backend::View {
    let pixels: Arc<[u8]> = (0..256 * 240)
        .flat_map(|pixel| {
            if (pixel / 256 / 16 + pixel % 256 / 16) % 2 == 0 {
                [24, 144, 96, 255]
            } else {
                [236, 192, 64, 255]
            }
        })
        .collect();
    let initial = ImageFrame::new(256, 240, pixels.clone()).unwrap();
    let model = Store::new(
        Emulator {
            generation: 1,
            frame: initial,
            address: "C000".into(),
            selection: "video".into(),
            tick: 0,
        },
        executor.clone(),
    );
    let session = Session::new();
    let image_updates = Rc::new(Cell::new(0_usize));
    let mut views = Vec::new();
    for (id, pictures) in [("main", 1), ("settings", 0), ("ppu", 6), ("debugger", 1)] {
        let view = backend::View::new(Binding::constant(format!("nerust {id}")));
        let root = view.root();
        view.label(
            &root,
            "generation",
            model.bind(|state| format!("Generation {}", state.generation), None),
        );
        view.entry(
            &root,
            "address",
            model.bind(
                |state| state.address.clone(),
                Some(|state, next| state.address = next),
            ),
        );
        for index in 0..pictures {
            let source =
                model.bind_scoped_explicit("frame", |state| Some(state.frame.clone()), None);
            let observed = image_updates.clone();
            view.keep(source.subscribe(Rc::new(move |_| observed.set(observed.get() + 1))));
            let key = format!("image-{index}");
            view.image(&root, &key, source);
            let mut style = wakui2_runtime::Style::default();
            style.placement.height = Some(128.0);
            view.style(&key, style);
        }
        let tab = view.tab(
            &root,
            "pages",
            model.bind(
                |state| state.selection.clone(),
                Some(|state, next| state.selection = next),
            ),
        );
        for key in ["video", "audio"] {
            let page = view
                .try_tab_page(&tab, key, Binding::constant(key.into()), key)
                .unwrap();
            view.label(
                &page,
                &format!("{key}-content"),
                Binding::constant(key.into()),
            );
        }
        view.finish_tab(&tab).unwrap();
        let weak = view.downgrade();
        view.keep(
            session
                .register_window(
                    id,
                    WindowHooks {
                        confirm: Rc::new(|| Box::pin(async { true })),
                        cancel: Rc::new(|| {}),
                        drain: Rc::new(|| Box::pin(async {})),
                        close: Rc::new(move || {
                            if let Some(view) = weak.upgrade() {
                                view.close();
                            }
                        }),
                    },
                )
                .unwrap(),
        );
        present(&view);
        views.push(view);
    }
    let primary = views[0].clone();
    executor.spawn(Box::pin(async move {
        futures_timer::Delay::new(Duration::from_millis(100)).await;
        assert_eq!(session.shutdown().window_count(), 4);
        let baseline = image_updates.get();
        let native_baseline: usize = views.iter().map(|view| view.image_update_count()).sum();
        assert_eq!(baseline, 8);
        assert_eq!(native_baseline, 8);
        let mut samples = Vec::new();
        for index in 0..144 {
            let started = Instant::now();
            assert!(model.update_scoped(&["tick"], |state| { let mut next = state.clone(); next.tick += 1; Some(next) }));
            views[3].edit("address", &format!("C{index:03X}")).unwrap();
            views[1].select("pages", if index % 2 == 0 { "audio" } else { "video" }).unwrap();
            assert_eq!(image_updates.get(), baseline);
            assert_eq!(views.iter().map(|view| view.image_update_count()).sum::<usize>(), native_baseline);
            assert!(Arc::ptr_eq(model.snapshot().frame.pixels(), &pixels));
            assert_eq!(views[0].text("address"), Some(format!("C{index:03X}")));
            if index >= 16 { samples.push(started.elapsed().as_nanos()); }
        }
        let old = image_updates.get();
        let replacement = ImageFrame::new(256, 240, Arc::from(vec![192_u8; 256 * 240 * 4])).unwrap();
        assert!(model.update_scoped(&["frame", "generation"], |state| { let mut next = state.clone(); next.frame = replacement; next.generation += 1; Some(next) }));
        assert_eq!(image_updates.get(), old + 8);
        assert_eq!(views.iter().map(|view| view.image_update_count()).sum::<usize>(), native_baseline + 8);
        let no_change = image_updates.get();
        for _ in 0..128 { assert!(model.update(|_| {})); }
        assert_eq!(image_updates.get(), no_change);
        assert_eq!(views.iter().map(|view| view.image_update_count()).sum::<usize>(), native_baseline + 8);
        views[1].close();
        assert_eq!(session.shutdown().window_count(), 3);
        assert!(!model.is_closed());
        let reopened = backend::View::new(Binding::constant("nerust settings reopened".into()));
        reopened.entry(&reopened.root(), "address", model.bind(|state| state.address.clone(), Some(|state, next| state.address = next)));
        let weak = reopened.downgrade();
        reopened.keep(session.register_window("settings", WindowHooks {
            confirm: Rc::new(|| Box::pin(async { true })), cancel: Rc::new(|| {}), drain: Rc::new(|| Box::pin(async {})),
            close: Rc::new(move || { if let Some(view) = weak.upgrade() { view.close(); } }),
        }).unwrap());
        present(&reopened);
        assert_eq!(reopened.text("address"), Some("C08F".into()));
        futures_timer::Delay::new(Duration::from_millis(100)).await;
        samples.sort_unstable();
        println!("INTEGRATION3 release={} samples={} p50_ns={} p95_ns={} windows=4 images=8 generation-gate no-change retained-edit-selection reopen", !cfg!(debug_assertions), samples.len(), samples[samples.len()/2], samples[samples.len()*95/100]);
        assert_eq!(session.shutdown().request_shutdown(Duration::from_secs(2)).await, wakui2_runtime::ShutdownOutcome::Closed);
        assert_eq!(session.shutdown().window_count(), 0);
        model.close();
    }));
    primary
}

#[cfg(feature = "backend-gtk")]
fn main() -> Result<(), String> {
    backend::run(|executor| start(executor, Rc::new(|view| view.present())))
}

#[cfg(not(feature = "backend-gtk"))]
fn main() -> Result<(), String> {
    backend::run_host(|host| {
        let presenter = host.clone();
        let primary = start(
            host.executor(),
            Rc::new(move |view| presenter.present(view)),
        );
        host.present(&primary);
        Ok(())
    })
}
