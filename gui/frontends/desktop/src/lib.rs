mod history;
mod render;
mod session;
mod settings;
mod view;

pub use session::SessionAdapter;

use nerust_gui_application::{Application, FramePort, PauseReason};
use nerust_gui_runtime::settings::{HostBackendCapabilities, SettingsPaths};
use nerust_gui_shell::context::FrontendContext;
use nerust_gui_shell::session::KeyboardShortcut;
use nerust_settings_core::model::input::ShortcutAction;
use panechord::{Store, WindowEvent};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::Duration,
};

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
    panechord::run_host(move |host| {
        let session = panechord::Session::new();
        host.attach(session.clone())?;
        let model = Store::new(
            view::Model::new(application.borrow().state()),
            host.executor(),
        );
        let actions = Rc::new(RefCell::new(None));
        let view = view::build(model.clone(), actions.clone());
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
        let settings_host = host.clone();
        let settings_session = session.clone();
        actions.replace(Some(Rc::new(move |intent| {
            if app.borrow().state().closed {
                return false;
            }
            match intent {
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
                        let selected = rfd::AsyncFileDialog::new().pick_file().await;
                        picker.set(false);
                        let mut app = app.borrow_mut();
                        let error = if weak_view.upgrade().is_some() {
                            selected.and_then(|file| app.load_media(file.path()).err())
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
                view::Intent::TogglePause => {
                    app.borrow_mut().toggle_pause();
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
        view.on_surface_viewport(Rc::new(move |width, height| {
            render.borrow_mut().set_logical_bounds(width, height);
        }));
        let app = application.clone();
        let render = renderer.clone();
        let output = model.clone();
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
                            Err("Debugger is not connected".into())
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
        view.on_close(Rc::new(move || {
            render.borrow_mut().close();
            app.borrow_mut().close();
            output.close();
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
        host.executor().spawn(Box::pin(async move {
            while weak.upgrade().is_some() && !application.borrow().state().closed {
                let (state, image) = {
                    let mut app = application.borrow_mut();
                    let profile = app.session().render_profile();
                    let vsync = app.settings().local.video.presentation.vsync;
                    app.advance_frame();
                    let image =
                        if let (Some(profile), Some(frame)) = (profile, app.session().frame()) {
                            let session = app.session();
                            let sync = nerust_gui_shell::session::osd::OsdSyncInput {
                                tick: session.metrics(),
                                chrome: session.chrome(),
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
                let _ = model.update(move |model| {
                    model.state = state;
                    match image {
                        Ok(image) => {
                            if !state.loaded {
                                model.screen = None;
                            } else {
                                if let Some(image) = image {
                                    model.screen = Some(image);
                                }
                            }
                        }
                        Err(error) => {
                            model.screen = None;
                            model.notice = Some(error);
                        }
                    }
                });
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
