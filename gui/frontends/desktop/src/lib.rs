mod debugger;
mod error;
mod history;
mod ppu;
mod render;
mod session;
mod settings;
mod view;

pub use session::SessionAdapter;

use nerust_gui_application::{Application, FramePort, PauseLease, PauseReason};
use nerust_gui_runtime::settings::{HostBackendCapabilities, SettingsPaths};
use nerust_gui_shell::context::FrontendContext;
use nerust_gui_shell::session::KeyboardShortcut;
use nerust_settings_core::model::input::ShortcutAction;
#[cfg(target_os = "macos")]
use panechord::RunnerBuilderExtMacOS;
use panechord::{Store, WindowEvent};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::Duration,
};

/// Window-close drain shared by panes: spin the frame-loop poll until the
/// in-flight operation settles. The shutdown timeout bounds the wait;
/// dropping here would leave an accepted mutation without its outcome (see
/// `DrainFlight::resolve_mutation_unknown`, which surfaces OutcomeUnknown
/// instead of retrying).
fn settle_flight(
    is_idle: Rc<dyn Fn() -> bool>,
) -> Rc<dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + 'static>>> {
    Rc::new(move || {
        let is_idle = is_idle.clone();
        Box::pin(async move {
            while !is_idle() {
                futures_timer::Delay::new(Duration::from_millis(5)).await;
            }
        })
    })
}

pub fn run(
    context: FrontendContext,
    capabilities: HostBackendCapabilities,
    settings_paths: Option<SettingsPaths>,
    rom: Option<PathBuf>,
) -> Result<(), String> {
    let FrontendContext {
        gpu_factory,
        registry,
        rom_loader,
        audio_registry,
    } = context;
    let mut application = Application::new(SessionAdapter::new(
        registry,
        audio_registry,
        rom_loader,
        capabilities,
        settings_paths,
    )?);
    if let Some(path) = rom {
        application.load_media(&path)?;
    }
    let application = Rc::new(RefCell::new(application));
    let renderer = Rc::new(RefCell::new(render::RenderAdapter::new(gpu_factory)));
    // Restore the size persisted at the previous exit; backends consume it
    // once for the first (main) window they build.
    let remembered_size = application
        .borrow()
        .settings()
        .app_state
        .window_size("main");
    let mut runner = panechord::Runner::builder();
    // Separate windows (main + settings/debugger/history) must never merge
    // into the system tab bar: backends apply this to windows created after
    // the call, so it flows through the runner before any view is built.
    #[cfg(target_os = "macos")]
    {
        runner = runner.allows_automatic_window_tabbing(false);
    }
    if let Some(size) = remembered_size {
        runner = runner.initial_window_size(size.width, size.height);
    }
    // Platform menu convention: macOS owns an application menu, every other
    // platform keeps File with Quit inside it. Named so the frontend policy
    // gate recognizes this sanctioned platform boundary (not a backend leak);
    // both branches stay unit-tested through MenuModel::new.
    runner.run(move |host| {
        let session = panechord::Session::new();
        host.attach(session.clone())?;
        let model = Store::new(
            view::Model::new(application.borrow().state()),
            host.executor(),
        );
        let actions = Rc::new(RefCell::new(None));
        let menus = Store::new(view::MenuModel::new(&application.borrow()), host.executor());
        let view = view::build(model.clone(), menus.clone(), actions.clone());
        let app = application.clone();
        let render = renderer.clone();
        let output = model.clone();
        let executor = host.executor();
        let weak_view = view.downgrade();
        let picker_active = Rc::new(Cell::new(false));
        let settings_window = Rc::new(RefCell::new(None::<panechord::WeakView>));
        let history_window = Rc::new(RefCell::new(None::<panechord::WeakView>));
        let history_updates = Rc::new(RefCell::new(None::<Rc<history::Pane>>));
        let history_refresh = history_updates.clone();
        let debugger_window = Rc::new(RefCell::new(None::<panechord::WeakView>));
        let debugger_updates = Rc::new(RefCell::new(None::<Rc<debugger::Pane>>));
        let debugger_refresh = debugger_updates.clone();
        let ppu_window = Rc::new(RefCell::new(None::<panechord::WeakView>));
        let ppu_updates = Rc::new(RefCell::new(None::<Rc<ppu::Pane>>));
        let ppu_refresh = ppu_updates.clone();
        let settings_host = host.clone();
        let settings_session = session.clone();
        let error_poll_host = settings_host.clone();
        let error_poll_session = settings_session.clone();
        let error_poll_executor = settings_host.executor();
        let dispatch_source = Rc::downgrade(&actions);
        actions.replace(Some(Rc::new(move |intent| {
            if app.borrow().state().closed {
                return false;
            }
            match intent {
                view::Intent::About => {
                    // No About dialog: on macOS the native predefined item owns
                    // its panel and never dispatches; automation is rejected.
                    false
                }
                view::Intent::Ppu => {
                    if let Some(view) = ppu_window
                        .borrow()
                        .as_ref()
                        .and_then(panechord::WeakView::upgrade)
                    {
                        view.present();
                        return true;
                    }
                    let language = app.borrow().settings().shared.general.language;
                    let pane = ppu::Pane::new(language, executor.clone());
                    let images = app.clone();
                    pane.connect(
                        app.borrow().session().media_revision(),
                        Rc::new(move |generation| {
                            images.borrow().session().ppu_live_begin(generation)
                        }),
                    );
                    let view = match pane.build() {
                        Ok(view) => view,
                        Err(error) => {
                            let _ = output.update(move |model| model.notice = Some(error));
                            return false;
                        }
                    };
                    let weak = view.downgrade();
                    let close = weak.clone();
                    let cancelled = pane.clone();
                    if let Err(error) = settings_host.register_view(
                        &settings_session,
                        "ppu",
                        &view,
                        panechord::WindowHooks {
                            confirm: Rc::new(|| Box::pin(async { true })),
                            cancel: Rc::new(move || cancelled.cancel()),
                            drain: settle_flight({
                                let pane = pane.clone();
                                Rc::new(move || pane.is_idle())
                            }),
                            close: Rc::new(move || {
                                if let Some(view) = close.upgrade() {
                                    view.close();
                                }
                            }),
                        },
                    ) {
                        view.close();
                        let _ = output.update(move |model| model.notice = Some(error));
                        return false;
                    }
                    view.on_close_request(Rc::new(move || {
                        if let Some(view) = weak.upgrade() {
                            view.close();
                        }
                    }));
                    let closed_window = ppu_window.clone();
                    let closed_pane = ppu_updates.clone();
                    view.on_close(Rc::new(move || {
                        closed_window.replace(None);
                        closed_pane.replace(None);
                    }));
                    let flight = app.borrow().session().ppu_layout_begin();
                    pane.start(flight);
                    ppu_window.replace(Some(view.downgrade()));
                    ppu_updates.replace(Some(pane));
                    true
                }
                view::Intent::Debugger => {
                    if let Some(view) = debugger_window
                        .borrow()
                        .as_ref()
                        .and_then(panechord::WeakView::upgrade)
                    {
                        view.present();
                        return true;
                    }
                    let language = app.borrow().settings().shared.general.language;
                    let pane = debugger::Pane::new(language, executor.clone());
                    let debug = app.clone();
                    let Some(dispatch) = dispatch_source
                        .upgrade()
                        .and_then(|actions| actions.borrow().clone())
                    else {
                        return false;
                    };
                    let spaces = app.clone();
                    pane.connect(
                        app.borrow().session().debug_spaces(),
                        Rc::new(move |actions, input| {
                            debug.borrow().session().debug_begin(actions, input)
                        }),
                        dispatch,
                        app.borrow().session().media_revision(),
                        Rc::new(move || spaces.borrow().session().debug_spaces()),
                    );
                    let view = match pane.build() {
                        Ok(view) => view,
                        Err(error) => {
                            let _ = output.update(move |model| model.notice = Some(error));
                            return false;
                        }
                    };
                    let weak = view.downgrade();
                    let close = weak.clone();
                    let cancelled = pane.clone();
                    if let Err(error) = settings_host.register_view(
                        &settings_session,
                        "debugger",
                        &view,
                        panechord::WindowHooks {
                            confirm: Rc::new(|| Box::pin(async { true })),
                            cancel: Rc::new(move || cancelled.cancel()),
                            drain: settle_flight({
                                let pane = pane.clone();
                                Rc::new(move || pane.is_idle())
                            }),
                            close: Rc::new(move || {
                                if let Some(view) = close.upgrade() {
                                    view.close();
                                }
                            }),
                        },
                    ) {
                        view.close();
                        let _ = output.update(move |model| model.notice = Some(error));
                        return false;
                    }
                    view.on_close_request(Rc::new(move || {
                        if let Some(view) = weak.upgrade() {
                            view.close();
                        }
                    }));
                    let closed_window = debugger_window.clone();
                    let closed_pane = debugger_updates.clone();
                    view.on_close(Rc::new(move || {
                        closed_window.replace(None);
                        closed_pane.replace(None);
                    }));
                    let (space, flight) = app.borrow().session().debugger_layout_begin();
                    pane.start(space, flight);
                    debugger_window.replace(Some(view.downgrade()));
                    debugger_updates.replace(Some(pane));
                    true
                }
                view::Intent::History => {
                    if let Some(view) = history_window
                        .borrow()
                        .as_ref()
                        .and_then(panechord::WeakView::upgrade)
                    {
                        view.present();
                        return true;
                    }
                    let pane = history::Pane::new(
                        app.borrow().settings().shared.general.language,
                        executor.clone(),
                    );
                    let snapshot = pane.snapshot(&app.borrow());
                    if let Some((revision, language, lines)) = snapshot {
                        pane.refresh(revision, language, &lines);
                    }
                    let cleared = app.clone();
                    let view = match pane.build(Rc::new(move || {
                        cleared.borrow_mut().clear_history();
                    })) {
                        Ok(view) => view,
                        Err(error) => {
                            let _ = output.update(move |model| model.notice = Some(error));
                            return false;
                        }
                    };
                    let weak = view.downgrade();
                    let close = weak.clone();
                    if let Err(error) = settings_host.register_view(
                        &settings_session,
                        "history",
                        &view,
                        panechord::WindowHooks {
                            confirm: Rc::new(|| Box::pin(async { true })),
                            cancel: Rc::new(|| {}),
                            drain: Rc::new(|| Box::pin(async {})),
                            close: Rc::new(move || {
                                if let Some(view) = close.upgrade() {
                                    view.close();
                                }
                            }),
                        },
                    ) {
                        view.close();
                        let _ = output.update(move |model| model.notice = Some(error));
                        return false;
                    }
                    view.on_close_request(Rc::new(move || {
                        if let Some(view) = weak.upgrade() {
                            view.close();
                        }
                    }));
                    let closed_window = history_window.clone();
                    let closed_pane = history_updates.clone();
                    view.on_close(Rc::new(move || {
                        closed_window.replace(None);
                        closed_pane.replace(None);
                    }));
                    history_window.replace(Some(view.downgrade()));
                    history_updates.replace(Some(pane));
                    true
                }
                view::Intent::Settings => {
                    if let Some(view) = settings_window
                        .borrow()
                        .as_ref()
                        .and_then(|view| view.upgrade())
                    {
                        view.present();
                        return true;
                    }
                    let editor = match settings::Editor::new(
                        app.borrow().settings().clone(),
                        app.borrow().session(),
                        executor.clone(),
                    ) {
                        Ok(editor) => editor,
                        Err(error) => {
                            let _ = output.update(move |model| model.notice = Some(error));
                            return false;
                        }
                    };
                    let applied = app.clone();
                    let quit = weak_view.clone();
                    let view = match editor.build(
                        Rc::new(move |snapshot| applied.borrow_mut().apply_settings(snapshot)),
                        Rc::new(move || {
                            if let Some(view) = quit.upgrade() {
                                view.request_quit();
                            }
                        }),
                    ) {
                        Ok(view) => view,
                        Err(error) => {
                            let _ = output.update(move |model| model.notice = Some(error));
                            return false;
                        }
                    };
                    let lease = Rc::new(RefCell::new(
                        app.borrow_mut().acquire_pause(PauseReason::Settings),
                    ));
                    let released = app.clone();
                    let closed = editor.clone();
                    view.on_close(Rc::new(move || {
                        closed.close();
                        if let Some(lease) = lease.borrow_mut().take() {
                            released.borrow_mut().release_pause(lease);
                        }
                    }));
                    let weak = view.downgrade();
                    let close = weak.clone();
                    let confirm = editor.clone();
                    let cancel = editor.clone();
                    if let Err(error) = settings_host.register_view(
                        &settings_session,
                        "settings",
                        &view,
                        panechord::WindowHooks {
                            confirm: Rc::new(move || {
                                let allowed = confirm.confirm_close(true);
                                Box::pin(async move { allowed })
                            }),
                            cancel: Rc::new(move || cancel.cancel_close()),
                            drain: Rc::new(|| Box::pin(async {})),
                            close: Rc::new(move || {
                                if let Some(view) = close.upgrade() {
                                    view.close();
                                }
                            }),
                        },
                    ) {
                        view.close();
                        let _ = output.update(move |model| model.notice = Some(error));
                        return false;
                    }
                    view.on_close_request(Rc::new(move || {
                        if editor.confirm_close(false)
                            && let Some(view) = weak.upgrade()
                        {
                            view.close();
                        }
                    }));
                    settings_window.replace(Some(view.downgrade()));
                    true
                }
                view::Intent::Open => {
                    if picker_active.replace(true) {
                        return false;
                    }
                    let lease = app.borrow_mut().acquire_pause(PauseReason::Modal);
                    let app = app.clone();
                    let output = output.clone();
                    let picker = picker_active.clone();
                    let weak_view = weak_view.clone();
                    executor.spawn(Box::pin(async move {
                        let selected = panechord::pick_file().await;
                        picker.set(false);
                        let mut app = app.borrow_mut();
                        let error = if weak_view.upgrade().is_some() {
                            selected.and_then(|path| app.load_media(&path).err())
                        } else {
                            None
                        };
                        if let Some(lease) = lease {
                            app.release_pause(lease);
                        }
                        drop(app);
                        let _ = output.update(move |model| model.notice = error);
                    }));
                    true
                }
                view::Intent::Pause(paused) => {
                    app.borrow_mut().set_pause(PauseReason::User, paused);
                    true
                }
                view::Intent::Session(action) => {
                    let result = app.borrow_mut().execute(action);
                    let executed = result.as_ref().is_ok_and(|executed| *executed);
                    let _ = output.update(move |model| model.notice = result.err());
                    executed
                }
                view::Intent::Fullscreen => {
                    render.borrow().toggle_fullscreen();
                    true
                }
                view::Intent::Quit => {
                    if let Some(view) = weak_view.upgrade() {
                        view.request_quit();
                    }
                    true
                }
            }
        })));
        let render = renderer.clone();
        view.on_surface_ready(move |window| {
            render.borrow_mut().set_window(window);
            Ok(())
        })?;
        let render = renderer.clone();
        // Last known main-window content size (logical points) for the
        // exit-time settings write; updated on every viewport change.
        let last_surface_size = Rc::new(Cell::new(None::<(u32, u32)>));
        let track_surface_size = last_surface_size.clone();
        view.on_surface_viewport(Rc::new(move |width, height| {
            render.borrow_mut().set_logical_bounds(width, height);
            if width > 0.0 && height > 0.0 {
                track_surface_size.set(Some((width.round() as u32, height.round() as u32)));
            }
        }));
        let app = application.clone();
        let render = renderer.clone();
        let output = model.clone();
        let keyboard_actions = actions.clone();
        view.on_surface_event(Rc::new(move |event| {
            let mut app = app.borrow_mut();
            match event {
                WindowEvent::Focused(focused) => {
                    if !focused {
                        app.clear_input();
                    }
                    app.set_pause(PauseReason::Focus, !focused);
                }
                WindowEvent::KeyboardInput {
                    key,
                    pressed,
                    repeat: false,
                    synthetic: false,
                } => {
                    let Some(key) = SessionAdapter::key(key) else {
                        return;
                    };
                    let result = match app.key_event(key, *pressed) {
                        Some(KeyboardShortcut::Session(ShortcutAction::TogglePause)) => {
                            app.toggle_pause();
                            Ok(())
                        }
                        Some(KeyboardShortcut::Session(action)) => {
                            SessionAdapter::session_action(action)
                                .map_or(Ok(()), |action| app.execute(action).map(|_| ()))
                        }
                        Some(KeyboardShortcut::ToggleFullscreen) => {
                            render.borrow().toggle_fullscreen();
                            Ok(())
                        }
                        Some(KeyboardShortcut::OpenDebugger) => {
                            drop(app);
                            let dispatch = keyboard_actions.borrow().clone();
                            if let Some(dispatch) = dispatch {
                                dispatch(view::Intent::Debugger);
                            }
                            return;
                        }
                        None => return,
                    };
                    drop(app);
                    let _ = output.update(move |model| model.notice = result.err());
                }
                _ => {}
            }
        }));
        let render = renderer.clone();
        let app = application.clone();
        let output = model.clone();
        let closed_menus = menus.clone();
        let closed_surface_size = last_surface_size.clone();
        view.on_close(Rc::new(move || {
            // Exit-time persist: remember the main window size for the next
            // launch before tearing down the session that owns settings.
            if let Some((width, height)) = closed_surface_size.get() {
                let _ = app
                    .borrow_mut()
                    .session_mut()
                    .remember_window_size(width, height);
            }
            render.borrow_mut().close();
            app.borrow_mut().close();
            output.close();
            closed_menus.close();
        }));
        let weak = view.downgrade();
        host.register_view(
            &session,
            "main",
            &view,
            panechord::WindowHooks {
                confirm: Rc::new(|| Box::pin(async { true })),
                cancel: Rc::new(|| {}),
                drain: Rc::new(|| Box::pin(async {})),
                close: Rc::new(move || {
                    if let Some(view) = weak.upgrade() {
                        view.close();
                    }
                }),
            },
        )?;
        let shutdown = session.shutdown();
        let executor = host.executor();
        view.on_close_request(Rc::new(move || {
            let shutdown = shutdown.clone();
            executor.spawn(Box::pin(async move {
                shutdown.request_shutdown(Duration::from_secs(5)).await;
            }));
        }));
        let weak = view.downgrade();
        // Error-dialog coordinator (desktop owner of the Shell ModalQueue gate).
        // The Shell pump (`advance_frame` -> `pump_osd` -> `pump_modal`) queues
        // batches; this loop is the only taker, mirroring Android's
        // `ErrorDialogController::poll_show`/`on_dismissed`. One error window at
        // a time: the window's close runs the ACK (`modal_closed`) and releases
        // the Modal pause lease (Application owns pause, like the settings and
        // file-picker leases). A separate window keeps the main View's content
        // (the Native screen surface) and Model untouched.
        let error_window = Rc::new(RefCell::new(None::<panechord::WeakView>));
        let error_lease: Rc<RefCell<Option<PauseLease>>> = Rc::new(RefCell::new(None));
        let error_poll_window = error_window.clone();
        let error_poll_lease = error_lease.clone();
        let error_poll_app = application.clone();
        // Projected binding: `snapshot()` would clone the screen image every
        // tick; this only clones the notice text.
        let error_poll_notice = model.bind(|model| model.notice.clone(), None);
        host.executor().spawn(Box::pin(async move {
            // Same platform flag the first MenuModel was built from, reused
            // for every menu refresh.
            while weak.upgrade().is_some() && !application.borrow().state().closed {
                let changed_menus = {
                    let app = application.borrow();
                    (!menus.with(|model| model.matches(&app))).then(|| view::MenuModel::new(&app))
                };
                if let Some(changed) = changed_menus {
                    let _ = menus.update(move |model| *model = changed);
                }
                let (state, image) = {
                    let mut app = application.borrow_mut();
                    let profile = app.session().render_profile();
                    let vsync = app.settings().local.video.presentation.vsync;
                    app.advance_frame();
                    let image =
                        if let (Some(profile), Some(frame)) = (profile, app.session().frame()) {
                            let session = app.session();
                            let metrics = session.metrics();
                            let content = crate::render::ScreenContentKey {
                                frame: metrics.frame_counter,
                                media: app.session().media_revision(),
                                osd: session.osd_state().generation(),
                                chrome: session.chrome(),
                            };
                            let sync = nerust_gui_shell::session::osd::OsdSyncInput {
                                tick: metrics,
                                chrome: content.chrome,
                                language: app.settings().shared.general.language,
                                now: std::time::Instant::now(),
                                width: 0,
                                height: 0,
                                ui_scale: 1.0,
                                safe_insets: nerust_gui_shell::session::osd::ViewInsets::default(),
                            };
                            renderer.borrow_mut().capture(
                                frame,
                                &profile,
                                vsync,
                                session.osd_state(),
                                sync,
                                content,
                            )
                        } else {
                            Ok(None)
                        };
                    (app.state(), image)
                };
                let pane = history_refresh.borrow().clone();
                if let Some(pane) = pane {
                    let snapshot = pane.snapshot(&application.borrow());
                    if let Some((revision, language, lines)) = snapshot {
                        pane.refresh(revision, language, &lines);
                    }
                }
                let pane = debugger_refresh.borrow().clone();
                if let Some(pane) = pane {
                    let app = application.borrow();
                    let language = app.settings().shared.general.language;
                    let loaded = app.state().loaded;
                    let revision = app.session().media_revision();
                    drop(app);
                    pane.poll(language, loaded, revision);
                }
                let pane = ppu_refresh.borrow().clone();
                if let Some(pane) = pane {
                    let app = application.borrow();
                    let language = app.settings().shared.general.language;
                    let loaded = app.state().loaded;
                    let revision = app.session().media_revision();
                    drop(app);
                    pane.poll(language, loaded, revision);
                }
                // Skip the whole update (and its full-state clone) when neither
                // the app state nor the screen changed: the capture above
                // already reports unchanged pixels as `Ok(None)`, so a quiet
                // frame touches nothing.
                let touch = match &image {
                    Err(_) => true,
                    Ok(image) => model.with(|model| {
                        model.state != state
                            || image.is_some()
                            || (!state.loaded && model.screen.is_some())
                    }),
                };
                if touch {
                    let _ = model.update(move |model| {
                        model.state = state;
                        match image {
                            Ok(image) => {
                                if !state.loaded {
                                    model.screen = None;
                                } else if let Some(image) = image {
                                    model.screen = Some(image);
                                }
                            }
                            Err(error) => {
                                model.screen = None;
                                model.notice = Some(error);
                            }
                        }
                    });
                }
                // Error dialog: one window at a time. A live window owns its text;
                // the close hook below ACKs the queue gate and releases pause.
                // Batches already taken but unshowable (shutdown, build or
                // registration failure) fall back to `notice` so no batch is
                // lost with the gate left open.
                let window_live = error_poll_window
                    .borrow()
                    .as_ref()
                    .and_then(panechord::WeakView::upgrade)
                    .is_some();
                if !window_live && !error_poll_app.borrow().state().closed {
                    // Borrow-gated: `get()` clones the notice String, so only
                    // project it when a notice is actually pending.
                    let next = error_poll_app
                        .borrow_mut()
                        .session_mut()
                        .take_modal_batch()
                        .map(|batch| (batch, true))
                        .or_else(|| {
                            model
                                .with(|model| model.notice.is_some())
                                .then(|| error_poll_notice.get())
                                .flatten()
                                .map(|notice| (notice, false))
                        });
                    if let Some((detail, from_modal)) = next {
                        let shown = detail.clone();
                        let opened = (|| {
                            let lease = error_poll_app
                                .borrow_mut()
                                .acquire_pause(PauseReason::Modal)?;
                            error_poll_lease.borrow_mut().replace(lease);
                            let language =
                                error_poll_app.borrow().settings().shared.general.language;
                            let pane = error::Pane::new(
                                language,
                                detail.clone(),
                                error_poll_executor.clone(),
                            );
                            let open_view = match pane.build() {
                                Ok(view) => view,
                                Err(failure) => {
                                    if from_modal {
                                        error_poll_app.borrow_mut().session_mut().modal_closed();
                                    }
                                    if let Some(lease) = error_poll_lease.borrow_mut().take() {
                                        error_poll_app.borrow_mut().release_pause(lease);
                                    }
                                    let _ = model.update(move |model| {
                                        model.notice = Some(failure);
                                    });
                                    return None;
                                }
                            };
                            let weak = open_view.downgrade();
                            let close = weak.clone();
                            let closed_app = error_poll_app.clone();
                            let closed_lease = error_poll_lease.clone();
                            let closed_window = error_poll_window.clone();
                            if let Err(failure) = error_poll_host.register_view(
                                &error_poll_session,
                                "error",
                                &open_view,
                                panechord::WindowHooks {
                                    confirm: Rc::new(|| Box::pin(async { true })),
                                    cancel: Rc::new(|| {}),
                                    // No in-flight work: the batch text is already
                                    // taken, so shutdown drain returns at once.
                                    drain: Rc::new(|| Box::pin(async {})),
                                    close: Rc::new(move || {
                                        if let Some(view) = close.upgrade() {
                                            view.close();
                                        }
                                    }),
                                },
                            ) {
                                open_view.close();
                                if from_modal {
                                    error_poll_app.borrow_mut().session_mut().modal_closed();
                                }
                                if let Some(lease) = error_poll_lease.borrow_mut().take() {
                                    error_poll_app.borrow_mut().release_pause(lease);
                                }
                                let _ = model.update(move |model| {
                                    model.notice = Some(failure);
                                });
                                return None;
                            }
                            open_view.on_close_request(Rc::new(move || {
                                if let Some(view) = weak.upgrade() {
                                    view.close();
                                }
                            }));
                            open_view.on_close(Rc::new(move || {
                                if from_modal {
                                    closed_app.borrow_mut().session_mut().modal_closed();
                                }
                                if let Some(lease) = closed_lease.borrow_mut().take() {
                                    closed_app.borrow_mut().release_pause(lease);
                                }
                                closed_window.replace(None);
                            }));
                            error_poll_window.replace(Some(open_view.downgrade()));
                            if !from_modal {
                                let _ = model.update(move |model| {
                                    if model.notice.as_deref() == Some(&shown) {
                                        model.notice = None;
                                    }
                                });
                            }
                            Some(())
                        })();
                        // Shutdown raced the take: the gate is already released
                        // above and the text survives through `notice`.
                        if opened.is_none() && error_poll_app.borrow().state().closed {
                            if from_modal {
                                error_poll_app.borrow_mut().session_mut().modal_closed();
                            }
                            if let Some(lease) = error_poll_lease.borrow_mut().take() {
                                error_poll_app.borrow_mut().release_pause(lease);
                            }
                            let _ = model.update(move |model| {
                                model.notice = Some(detail);
                            });
                        }
                    }
                }
                futures_timer::Delay::new(Duration::from_millis(16)).await;
            }
        }));
        Ok(())
    })
}

pub fn review(context: FrontendContext, rom: Option<PathBuf>) -> Result<(), String> {
    let paths = std::env::var_os("NERUST_SETTINGS_ROOT")
        .map(|root| SettingsPaths::from_root(PathBuf::from(root)));
    run(
        context,
        HostBackendCapabilities {
            window: nerust_gui_runtime::settings::HostWindowCapabilities {
                remembers_window_size: true,
                supports_fullscreen_default: true,
                supports_scaling: true,
            },
            presentation: None,
        },
        paths,
        rom,
    )
}
