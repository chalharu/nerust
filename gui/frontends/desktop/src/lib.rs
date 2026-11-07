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

/// Shared handles for the main window and its auxiliary panes.
///
/// Verbatim home of the blocks that used to live inside the `runner.run`
/// closure: every method below moves code without logic, ordering, string,
/// or timing changes, so the entry point stays within cognitive-complexity
/// bounds. All fields are `Rc` clones, so the host itself moves freely into
/// window hooks and the owned async frame loop.
#[derive(Clone)]
struct MainHost {
    application: Rc<RefCell<Application<SessionAdapter>>>,
    model: Store<view::Model>,
    menus: Store<view::MenuModel>,
    renderer: Rc<RefCell<render::RenderAdapter>>,
    actions: view::Actions,
    weak_view: panechord::backend::WeakView,
    picker_active: Rc<Cell<bool>>,
    settings_window: Rc<RefCell<Option<panechord::backend::WeakView>>>,
    history_window: Rc<RefCell<Option<panechord::backend::WeakView>>>,
    history_updates: Rc<RefCell<Option<Rc<history::Pane>>>>,
    debugger_window: Rc<RefCell<Option<panechord::backend::WeakView>>>,
    debugger_updates: Rc<RefCell<Option<Rc<debugger::Pane>>>>,
    // Debugger/PPU/history panes poll below; aux windows own no
    // pause. Focus and the menu/shortcut User pause are the only
    // owners: opening the debugger defocuses Main (Focus pause),
    // returning clears it, and the pane mirrors both edges.
    ppu_window: Rc<RefCell<Option<panechord::backend::WeakView>>>,
    ppu_updates: Rc<RefCell<Option<Rc<ppu::Pane>>>>,
    host: Rc<dyn panechord::WindowHost<panechord::backend::View>>,
    session: panechord::Session,
    executor: Rc<dyn panechord::Executor>,
    error_window: Rc<RefCell<Option<panechord::backend::WeakView>>>,
    error_lease: Rc<RefCell<Option<PauseLease>>>,
    error_notice: panechord::Binding<Option<String>>,
    last_surface_size: Rc<Cell<Option<(u32, u32)>>>,
}

impl MainHost {
    fn handle_intent(&self, intent: view::Intent) -> bool {
        if self.application.borrow().state().closed {
            return false;
        }
        match intent {
            view::Intent::About => self.handle_about(),
            view::Intent::Ppu => self.open_ppu(),
            view::Intent::Debugger => self.open_debugger(),
            view::Intent::History => self.open_history(),
            view::Intent::Settings => self.open_settings(),
            view::Intent::Open => self.open_picker(),
            view::Intent::Pause(paused) => self.handle_pause(paused),
            view::Intent::Session(action) => self.handle_session(action),
            view::Intent::Fullscreen => self.handle_fullscreen(),
            view::Intent::Quit => self.handle_quit(),
        }
    }

    fn handle_about(&self) -> bool {
        // No About dialog: on macOS the native predefined item owns
        // its panel and never dispatches; automation is rejected.
        false
    }

    fn handle_pause(&self, paused: bool) -> bool {
        self.application
            .borrow_mut()
            .set_pause(PauseReason::User, paused);
        true
    }

    fn handle_session(&self, action: nerust_gui_application::SessionAction) -> bool {
        let result = self.application.borrow_mut().execute(action);
        let executed = result.as_ref().is_ok_and(|executed| *executed);
        let _ = self.model.update(move |model| model.notice = result.err());
        executed
    }

    fn handle_fullscreen(&self) -> bool {
        self.renderer.borrow().toggle_fullscreen();
        true
    }

    fn handle_quit(&self) -> bool {
        if let Some(view) = self.weak_view.upgrade() {
            view.request_quit();
        }
        true
    }

    fn open_ppu(&self) -> bool {
        if let Some(view) = self
            .ppu_window
            .borrow()
            .as_ref()
            .and_then(panechord::backend::WeakView::upgrade)
        {
            view.present();
            return true;
        }
        let language = self.application.borrow().settings().shared.general.language;
        let pane = ppu::Pane::new(language, self.executor.clone());
        let images = self.application.clone();
        pane.connect(
            self.application.borrow().session().media_revision(),
            Rc::new(move |generation| images.borrow().session().ppu_live_begin(generation)),
        );
        let view = match pane.build() {
            Ok(view) => view,
            Err(error) => {
                let _ = self.model.update(move |model| model.notice = Some(error));
                return false;
            }
        };
        let weak = view.downgrade();
        let close = weak.clone();
        let cancelled = pane.clone();
        if let Err(error) = self.host.register_view_with_hooks(
            &self.session,
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
            let _ = self.model.update(move |model| model.notice = Some(error));
            return false;
        }
        view.on_close_request(Rc::new(move || {
            if let Some(view) = weak.upgrade() {
                view.close();
            }
        }));
        let closed_window = self.ppu_window.clone();
        let closed_pane = self.ppu_updates.clone();
        view.on_close(Rc::new(move || {
            closed_window.replace(None);
            closed_pane.replace(None);
        }));
        let flight = self.application.borrow().session().ppu_layout_begin();
        pane.start(flight);
        self.ppu_window.replace(Some(view.downgrade()));
        self.ppu_updates.replace(Some(pane));
        true
    }

    fn open_debugger(&self) -> bool {
        if let Some(view) = self
            .debugger_window
            .borrow()
            .as_ref()
            .and_then(panechord::backend::WeakView::upgrade)
        {
            view.present();
            return true;
        }
        let language = self.application.borrow().settings().shared.general.language;
        let pane = debugger::Pane::new(language, self.executor.clone());
        let debug = self.application.clone();
        let dispatch_source = Rc::downgrade(&self.actions);
        let Some(dispatch) = dispatch_source
            .upgrade()
            .and_then(|actions| actions.borrow().clone())
        else {
            return false;
        };
        let spaces = self.application.clone();
        pane.connect(
            self.application.borrow().session().debug_spaces(),
            Rc::new(move |actions, input| debug.borrow().session().debug_begin(actions, input)),
            dispatch,
            self.application.borrow().session().media_revision(),
            Rc::new(move || spaces.borrow().session().debug_spaces()),
        );
        // Aux windows open at the backend default (660x520 on
        // tao) unless a size is staged: the debugger needs its
        // full width or rows overflow their columns. GTK grows
        // to content regardless and Swift honors the frame, so
        // one staged size serves every backend. Stage before
        // build: the Host is created by build and consumes the
        // pending size at construction.
        panechord::backend::set_initial_window_size(1100, 946);
        let view = match pane.build() {
            Ok(view) => view,
            Err(error) => {
                let _ = self.model.update(move |model| model.notice = Some(error));
                return false;
            }
        };
        let weak = view.downgrade();
        let close = weak.clone();
        let cancelled = pane.clone();
        if let Err(error) = self.host.register_view_with_hooks(
            &self.session,
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
            let _ = self.model.update(move |model| model.notice = Some(error));
            return false;
        }
        view.on_close_request(Rc::new(move || {
            if let Some(view) = weak.upgrade() {
                view.close();
            }
        }));
        let closed_window = self.debugger_window.clone();
        let closed_pane = self.debugger_updates.clone();
        view.on_close(Rc::new(move || {
            closed_window.replace(None);
            closed_pane.replace(None);
        }));
        let (space, flight) = self.application.borrow().session().debugger_layout_begin();
        pane.start(space, flight);
        self.debugger_window.replace(Some(view.downgrade()));
        self.debugger_updates.replace(Some(pane));
        true
    }

    fn open_history(&self) -> bool {
        if let Some(view) = self
            .history_window
            .borrow()
            .as_ref()
            .and_then(panechord::backend::WeakView::upgrade)
        {
            view.present();
            return true;
        }
        let pane = history::Pane::new(
            self.application.borrow().settings().shared.general.language,
            self.executor.clone(),
        );
        let snapshot = pane.snapshot(&self.application.borrow());
        if let Some((revision, language, lines)) = snapshot {
            pane.refresh(revision, language, &lines);
        }
        let cleared = self.application.clone();
        let view = match pane.build(Rc::new(move || {
            cleared.borrow_mut().clear_history();
        })) {
            Ok(view) => view,
            Err(error) => {
                let _ = self.model.update(move |model| model.notice = Some(error));
                return false;
            }
        };
        let weak = view.downgrade();
        let close = weak.clone();
        if let Err(error) = self.host.register_view_with_hooks(
            &self.session,
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
            let _ = self.model.update(move |model| model.notice = Some(error));
            return false;
        }
        view.on_close_request(Rc::new(move || {
            if let Some(view) = weak.upgrade() {
                view.close();
            }
        }));
        let closed_window = self.history_window.clone();
        let closed_pane = self.history_updates.clone();
        view.on_close(Rc::new(move || {
            closed_window.replace(None);
            closed_pane.replace(None);
        }));
        self.history_window.replace(Some(view.downgrade()));
        self.history_updates.replace(Some(pane));
        true
    }

    fn open_settings(&self) -> bool {
        if let Some(view) = self
            .settings_window
            .borrow()
            .as_ref()
            .and_then(|view| view.upgrade())
        {
            view.present();
            return true;
        }
        let editor = match settings::Editor::new(
            self.application.borrow().settings().clone(),
            self.application.borrow().session(),
            self.executor.clone(),
        ) {
            Ok(editor) => editor,
            Err(error) => {
                let _ = self.model.update(move |model| model.notice = Some(error));
                return false;
            }
        };
        let applied = self.application.clone();
        let quit = self.weak_view.clone();
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
                let _ = self.model.update(move |model| model.notice = Some(error));
                return false;
            }
        };
        let lease = Rc::new(RefCell::new(
            self.application
                .borrow_mut()
                .acquire_pause(PauseReason::Settings),
        ));
        let released = self.application.clone();
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
        if let Err(error) = self.host.register_view_with_hooks(
            &self.session,
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
            let _ = self.model.update(move |model| model.notice = Some(error));
            return false;
        }
        view.on_close_request(Rc::new(move || {
            if editor.confirm_close(false)
                && let Some(view) = weak.upgrade()
            {
                view.close();
            }
        }));
        self.settings_window.replace(Some(view.downgrade()));
        true
    }

    fn open_picker(&self) -> bool {
        if self.picker_active.replace(true) {
            return false;
        }
        let lease = self
            .application
            .borrow_mut()
            .acquire_pause(PauseReason::Modal);
        let app = self.application.clone();
        let output = self.model.clone();
        let picker = self.picker_active.clone();
        let weak_view = self.weak_view.clone();
        self.executor.spawn(Box::pin(async move {
            let selected = panechord::backend::pick_file().await;
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

    fn on_viewport(&self, width: f32, height: f32) {
        // Last known main-window content size (logical points) for the
        // exit-time settings write; updated on every viewport change.
        self.renderer.borrow_mut().set_logical_bounds(width, height);
        if width > 0.0 && height > 0.0 {
            self.last_surface_size
                .set(Some((width.round() as u32, height.round() as u32)));
        }
    }

    fn on_surface_event(&self, event: &WindowEvent) {
        let mut app = self.application.borrow_mut();
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
                        self.renderer.borrow().toggle_fullscreen();
                        Ok(())
                    }
                    Some(KeyboardShortcut::OpenDebugger) => {
                        drop(app);
                        let dispatch = self.actions.borrow().clone();
                        if let Some(dispatch) = dispatch {
                            dispatch(view::Intent::Debugger);
                        }
                        return;
                    }
                    None => return,
                };
                drop(app);
                let _ = self.model.update(move |model| model.notice = result.err());
            }
            _ => {}
        }
    }

    fn on_close(&self) {
        // Exit-time persist: remember the main window size for the next
        // launch before tearing down the session that owns settings.
        if let Some((width, height)) = self.last_surface_size.get() {
            let _ = self
                .application
                .borrow_mut()
                .session_mut()
                .remember_window_size(width, height);
        }
        self.renderer.borrow_mut().close();
        self.application.borrow_mut().close();
        self.model.close();
        self.menus.close();
    }

    fn register_main(&self, view: &panechord::backend::View) -> Result<(), String> {
        let weak = view.downgrade();
        self.host.register_view_with_hooks(
            &self.session,
            "main",
            view,
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
        let shutdown = self.session.shutdown();
        let executor = self.host.executor();
        view.on_close_request(Rc::new(move || {
            let shutdown = shutdown.clone();
            executor.spawn(Box::pin(async move {
                shutdown.request_shutdown(Duration::from_secs(5)).await;
            }));
        }));
        Ok(())
    }

    fn refresh_menus(&self) {
        // Same platform flag the first MenuModel was built from, reused
        // for every menu refresh.
        let changed_menus = {
            let app = self.application.borrow();
            (!self.menus.with(|model| model.matches(&app))).then(|| view::MenuModel::new(&app))
        };
        if let Some(changed) = changed_menus {
            let _ = self.menus.update(move |model| *model = changed);
        }
    }

    fn capture_frame(
        &self,
    ) -> (
        nerust_gui_application::ApplicationState,
        Result<Option<panechord::ImageFrame>, String>,
    ) {
        let mut app = self.application.borrow_mut();
        let profile = app.session().render_profile();
        let vsync = app.settings().local.video.presentation.vsync;
        app.advance_frame();
        let image = if let (Some(profile), Some(frame)) = (profile, app.session().frame()) {
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
                density: self.renderer.borrow().density(),
            };
            self.renderer.borrow_mut().capture(
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
    }

    fn present_frame(
        &self,
        state: nerust_gui_application::ApplicationState,
        image: Result<Option<panechord::ImageFrame>, String>,
    ) {
        // Skip the whole update (and its full-state clone) when neither
        // the app state nor the screen changed: the capture above
        // already reports unchanged pixels as `Ok(None)`, so a quiet
        // frame touches nothing.
        let touch = match &image {
            Err(_) => true,
            Ok(image) => self.model.with(|model| {
                model.state != state || image.is_some() || (!state.loaded && model.screen.is_some())
            }),
        };
        if touch {
            let _ = self.model.update(move |model| {
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
    }

    fn poll_history(&self) {
        let pane = self.history_updates.borrow().clone();
        if let Some(pane) = pane {
            let snapshot = pane.snapshot(&self.application.borrow());
            if let Some((revision, language, lines)) = snapshot {
                pane.refresh(revision, language, &lines);
            }
        }
    }

    fn poll_debugger(&self) {
        let pane = self.debugger_updates.borrow().clone();
        if let Some(pane) = pane {
            let app = self.application.borrow();
            let language = app.settings().shared.general.language;
            let loaded = app.state().loaded;
            let revision = app.session().media_revision();
            let paused = app.state().paused;
            drop(app);
            pane.poll(language, loaded, revision, paused);
        }
    }

    fn poll_ppu(&self) {
        let pane = self.ppu_updates.borrow().clone();
        if let Some(pane) = pane {
            let app = self.application.borrow();
            let language = app.settings().shared.general.language;
            let loaded = app.state().loaded;
            let revision = app.session().media_revision();
            drop(app);
            pane.poll(language, loaded, revision);
        }
    }

    // Error-dialog coordinator (desktop owner of the Shell ModalQueue gate).
    // The Shell pump (`advance_frame` -> `pump_osd` -> `pump_modal`) queues
    // batches; this loop is the only taker, mirroring Android's
    // `ErrorDialogController::poll_show`/`on_dismissed`. One error window at
    // a time: the window's close runs the ACK (`modal_closed`) and releases
    // the Modal pause lease (Application owns pause, like the settings and
    // file-picker leases). A separate window keeps the main View's content
    // (the Native screen surface) and Model untouched.
    fn poll_error_dialog(&self) {
        // Error dialog: one window at a time. A live window owns its text;
        // the close hook below ACKs the queue gate and releases pause.
        // Batches already taken but unshowable (shutdown, build or
        // registration failure) fall back to `notice` so no batch is
        // lost with the gate left open.
        let window_live = self
            .error_window
            .borrow()
            .as_ref()
            .and_then(panechord::backend::WeakView::upgrade)
            .is_some();
        if !window_live
            && !self.application.borrow().state().closed
            && let Some((detail, from_modal)) = self.next_error_batch()
        {
            let opened = self.open_error_window(detail.clone(), from_modal);
            // Shutdown raced the take: the gate is already released
            // above and the text survives through `notice`.
            if opened.is_none() && self.application.borrow().state().closed {
                if from_modal {
                    self.application.borrow_mut().session_mut().modal_closed();
                }
                if let Some(lease) = self.error_lease.borrow_mut().take() {
                    self.application.borrow_mut().release_pause(lease);
                }
                let _ = self.model.update(move |model| {
                    model.notice = Some(detail);
                });
            }
        }
    }

    fn next_error_batch(&self) -> Option<(String, bool)> {
        // Borrow-gated: `get()` clones the notice String, so only
        // project it when a notice is actually pending.
        self.application
            .borrow_mut()
            .session_mut()
            .take_modal_batch()
            .map(|batch| (batch, true))
            .or_else(|| {
                self.model
                    .with(|model| model.notice.is_some())
                    .then(|| self.error_notice.get())
                    .flatten()
                    .map(|notice| (notice, false))
            })
    }

    fn open_error_window(&self, detail: String, from_modal: bool) -> Option<()> {
        let shown = detail.clone();
        let lease = self
            .application
            .borrow_mut()
            .acquire_pause(PauseReason::Modal)?;
        self.error_lease.borrow_mut().replace(lease);
        let language = self.application.borrow().settings().shared.general.language;
        let pane = error::Pane::new(language, detail.clone(), self.executor.clone());
        let open_view = match pane.build() {
            Ok(view) => view,
            Err(failure) => {
                if from_modal {
                    self.application.borrow_mut().session_mut().modal_closed();
                }
                if let Some(lease) = self.error_lease.borrow_mut().take() {
                    self.application.borrow_mut().release_pause(lease);
                }
                let _ = self.model.update(move |model| {
                    model.notice = Some(failure);
                });
                return None;
            }
        };
        let weak = open_view.downgrade();
        let close = weak.clone();
        let closed_app = self.application.clone();
        let closed_lease = self.error_lease.clone();
        let closed_window = self.error_window.clone();
        if let Err(failure) = self.host.register_view_with_hooks(
            &self.session,
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
                self.application.borrow_mut().session_mut().modal_closed();
            }
            if let Some(lease) = self.error_lease.borrow_mut().take() {
                self.application.borrow_mut().release_pause(lease);
            }
            let _ = self.model.update(move |model| {
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
        self.error_window.replace(Some(open_view.downgrade()));
        if !from_modal {
            let _ = self.model.update(move |model| {
                if model.notice.as_deref() == Some(&shown) {
                    model.notice = None;
                }
            });
        }
        Some(())
    }
}

/// Owned frame loop: polls menus, frames, aux panes, and the error dialog
/// once per tick until the main view or the application closes.
async fn frame_loop(host: MainHost) {
    while host.weak_view.upgrade().is_some() && !host.application.borrow().state().closed {
        host.refresh_menus();
        let (state, image) = host.capture_frame();
        host.poll_history();
        host.poll_debugger();
        host.poll_ppu();
        host.present_frame(state, image);
        host.poll_error_dialog();
        futures_timer::Delay::new(Duration::from_millis(16)).await;
    }
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
        // Projected binding: `snapshot()` would clone the screen image every
        // tick; this only clones the notice text.
        let error_notice = model.bind(|model| model.notice.clone(), None);
        let main = MainHost {
            application: application.clone(),
            model: model.clone(),
            menus: menus.clone(),
            renderer: renderer.clone(),
            actions: actions.clone(),
            weak_view: view.downgrade(),
            picker_active: Rc::new(Cell::new(false)),
            settings_window: Rc::new(RefCell::new(None)),
            history_window: Rc::new(RefCell::new(None)),
            history_updates: Rc::new(RefCell::new(None)),
            debugger_window: Rc::new(RefCell::new(None)),
            debugger_updates: Rc::new(RefCell::new(None)),
            ppu_window: Rc::new(RefCell::new(None)),
            ppu_updates: Rc::new(RefCell::new(None)),
            executor: host.executor(),
            host: host.clone(),
            session: session.clone(),
            error_window: Rc::new(RefCell::new(None)),
            error_lease: Rc::new(RefCell::new(None)),
            error_notice,
            last_surface_size: Rc::new(Cell::new(None)),
        };
        let dispatch_main = main.clone();
        actions.replace(Some(Rc::new(move |intent| {
            dispatch_main.handle_intent(intent)
        })));
        let render = renderer.clone();
        view.on_surface_ready(move |window| {
            render.borrow_mut().set_window(window);
            Ok(())
        })?;
        let viewport_main = main.clone();
        view.on_surface_viewport(Rc::new(move |width, height| {
            viewport_main.on_viewport(width, height);
        }));
        let event_main = main.clone();
        view.on_surface_event(Rc::new(move |event| {
            event_main.on_surface_event(event);
        }));
        let close_main = main.clone();
        view.on_close(Rc::new(move || {
            close_main.on_close();
        }));
        main.register_main(&view)?;
        let frame_main = main.clone();
        host.executor().spawn(Box::pin(frame_loop(frame_main)));
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
