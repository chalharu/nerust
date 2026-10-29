//! Main window: native chrome, event sink, and actions.
//! Menu, dialog, and aux-window realizations live in sibling modules;
//! behavior shared across them is covered by this file's tests.
use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::{Rc, Weak},
};

use crate::State;
use crate::chrome::aux_windows::NativeAux;
use crate::chrome::aux_windows::{drain_deferred_opens, open_auxiliary, pump_debug_widgets};
use crate::chrome::dialogs::{DialogRenderer, RenderedDialog, open_about};
use crate::chrome::menu::GTK_MENU_CAPABILITIES;
use crate::chrome::menu::{ActionRegistry, GtkMenuVisitor, MenubarModels};
use crate::chrome::surface::{Surface, SurfaceExtend};
use gtk::{glib, prelude::*};
use wakui_core::input::event::{CommandEffect, EventCommand, EventSink};
use wakui_core::input::session::SessionEvent;
use wakui_core::{
    chrome::aux::AuxWindows,
    chrome::menu::Menubar,
    chrome::menu_presenter::MenuPresenter,
    chrome::window::{MainContents, Window as HostWindow, WindowVisitor},
    input::focus::FocusEvent,
};

pub(crate) struct WindowCore<K: Copy + Eq + std::hash::Hash + 'static> {
    pub(crate) application: gtk::Application,
    pub(crate) window: gtk::ApplicationWindow,
    pub(crate) state: Rc<RefCell<State<K>>>,
    pub(crate) open_dialog: Option<gtk::FileChooserNative>,
    pub(crate) about_dialog: Option<gtk::AboutDialog>,
    pub(crate) about_opening: bool,
    pub(crate) aux: AuxWindows<K, u64, NativeAux>,
    pub(crate) actions: ActionRegistry,
    pub(crate) sink: Rc<dyn EventSink>,
    pub(crate) menubar: MenubarModels,
    pub(crate) menu_presenter: Rc<MenuPresenter>,
    pub(crate) main_delivery: Rc<MainWindowDelivery>,
}

/// A full window snapshot is latest-wins, including title-notify reentry before
/// visit_menu. This is a delivery mailbox, not another host window registry.
#[derive(Default)]
pub(crate) struct MainWindowDelivery {
    active: Cell<bool>,
    pending: RefCell<Option<HostWindow>>,
}

struct MainDeliveryGuard<'a>(&'a Cell<bool>);

impl Drop for MainDeliveryGuard<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

type MainSurface<K> = (gtk::Picture, Rc<RefCell<State<K>>>);

pub(crate) struct MainWindowRenderer<K: Copy + Eq + std::hash::Hash + 'static> {
    menu: GtkMenuVisitor,
    presenter: Rc<MenuPresenter>,
    surface: Option<MainSurface<K>>,
}

/// One outer traversal for main chrome and grouped native widgets. Builders
/// receive owned controllers; updates reuse retained controls without policy.
pub(crate) enum GtkWindowRenderer<'a, K: Copy + Eq + std::hash::Hash + 'static> {
    Main(MainWindowRenderer<K>),
    Build(Box<DialogRenderer<'a>>),
}

impl<K: Copy + Eq + std::hash::Hash + 'static> WindowVisitor for GtkWindowRenderer<'_, K> {
    fn begin_window(&mut self, model: &HostWindow) {
        let native: Option<gtk::Window> = match self {
            Self::Main(main) => main.menu.window.clone().map(Cast::upcast),
            Self::Build(builder) => {
                builder.model = Some(model.clone());
                None
            }
        };
        if let Some(window) = native
            && window.title().as_deref() != Some(model.title.as_str())
        {
            window.set_title(Some(&model.title));
        }
    }

    fn visit_menu(&mut self, menu: &Menubar) {
        let Self::Main(main) = self else { return };
        let mut next = main.presenter.submit(menu.clone());
        while let Some(plan) = next {
            plan.visit(&mut main.menu);
            next = main.presenter.complete(plan);
        }
    }

    fn visit_main(&mut self, _contents: &MainContents) {
        let Self::Main(main) = self else { return };
        if let Some((picture, state)) = main.surface.take() {
            let window = main.menu.window.as_ref().expect("main native window");
            window.set_child(Some(&picture));
            let _ = Surface::bind(window, &picture, state);
        }
    }

    fn visit_controls(&mut self, _screen: &wakui_core::scene::control::ControlScreen) {
        let Self::Build(builder) = self else {
            return;
        };
        let (Some(screen), Some(mailbox)) = (builder.screen.take(), builder.mailbox.take()) else {
            return;
        };
        builder.result = Some(RenderedDialog::Controls(crate::scene::binding::build(
            builder.model.as_ref().expect("window chrome"),
            builder.parent,
            screen,
            mailbox,
            builder.callbacks.take().expect("one control program"),
        )));
    }
}

fn present_main_model<K: Copy + Eq + std::hash::Hash + 'static>(
    window: &Window<K>,
    model: HostWindow,
    surface: Option<MainSurface<K>>,
) {
    let (delivery, mut visitor) = {
        let core = window.borrow();
        (
            core.main_delivery.clone(),
            GtkWindowRenderer::Main(MainWindowRenderer {
                menu: GtkMenuVisitor {
                    menubar: core.menubar.clone(),
                    window: Some(core.window.clone()),
                    application: Some(core.application.clone()),
                    actions: core.actions.clone(),
                    sink: Some(core.sink.clone()),
                    stack: Vec::new(),
                },
                presenter: core.menu_presenter.clone(),
                surface,
            }),
        )
    };
    *delivery.pending.borrow_mut() = Some(model);
    if delivery.active.replace(true) {
        return;
    }
    let _guard = MainDeliveryGuard(&delivery.active);
    loop {
        let model = delivery.pending.borrow_mut().take();
        let Some(model) = model else { break };
        model.visit(&mut visitor);
    }
}

pub(crate) type Window<K> = Rc<RefCell<WindowCore<K>>>;

struct WindowEventSink<K: Copy + Eq + std::hash::Hash + 'static>(Weak<RefCell<WindowCore<K>>>);

impl<K: Copy + Eq + std::hash::Hash + 'static> EventSink for WindowEventSink<K> {
    fn send(&self, event: EventCommand) -> bool {
        let Some(window) = self.0.upgrade() else {
            return false;
        };
        let state = {
            let core = window.borrow();
            core.state.clone()
        };
        let outcome = {
            let mut state = state.borrow_mut();
            // The modal guard lives view-side; flavors forward every event.
            state.host.handle_event(event)
        };
        // Native constructors, setters and effects may synchronously reenter.
        match outcome {
            Ok(outcome) => {
                if let Some(effect) = outcome.effect {
                    apply_command_effect(&window, effect);
                }
                if matches!(event, EventCommand::Command(_))
                    || outcome.session.command.is_some()
                    || outcome.effect.is_some()
                {
                    window.update_actions();
                }
            }
            Err(error) => {
                log::warn!("GTK event failed: {error}");
                window.update_actions();
            }
        }
        true
    }
}

fn apply_command_effect<K: Copy + Eq + std::hash::Hash + 'static>(
    window: &Window<K>,
    effect: CommandEffect<K>,
) {
    match effect {
        CommandEffect::OpenFile => window.open(),
        CommandEffect::OpenAux(kind) => {
            open_auxiliary(window, kind);
            drain_deferred_opens(window);
        }
        CommandEffect::About => open_about(window),
        CommandEffect::Quit => window.window().close(),
        CommandEffect::ToggleFullscreen => toggle_window_fullscreen(window),
        CommandEffect::LiveTick => (),
    }
}

pub(crate) trait WindowExtend<K: Copy + Eq + std::hash::Hash + 'static> {
    fn bind(
        application: gtk::Application,
        window: gtk::ApplicationWindow,
        picture: gtk::Picture,
        state: Rc<RefCell<State<K>>>,
        menubar: MenubarModels,
    ) -> Window<K>;
    fn window(&self) -> gtk::ApplicationWindow;
    fn application(&self) -> gtk::Application;
    fn state(&self) -> Rc<RefCell<State<K>>>;
    fn realize(&self);
    fn close_request(&self) -> bool;
    fn open(&self);
    fn load_path(&self, path: &Path);
    fn update_actions(&self);
    fn rebuild_menubar(&self);
    fn sync_fullscreen_from_settings(&self);
    fn key_event(&self, key: gdk::Key, enevt: KeyEventState) -> bool;
    fn event_sink(&self) -> Rc<dyn EventSink>;
}

#[derive(Clone, Copy)]
pub(crate) enum KeyEventState {
    Press,
    Release,
}

fn key_event_pressed(event: KeyEventState) -> bool {
    matches!(event, KeyEventState::Press)
}

impl<K: Copy + Eq + std::hash::Hash + 'static> WindowExtend<K> for Window<K> {
    fn state(&self) -> Rc<RefCell<State<K>>> {
        self.borrow().state.clone()
    }

    fn bind(
        application: gtk::Application,
        window: gtk::ApplicationWindow,
        picture: gtk::Picture,
        state: Rc<RefCell<State<K>>>,
        menubar: MenubarModels,
    ) -> Window<K> {
        let result = Rc::new_cyclic(|weak| {
            RefCell::new(WindowCore {
                application,
                window: window.clone(),
                state,
                open_dialog: None,
                about_dialog: None,
                about_opening: false,
                aux: AuxWindows::new(),
                actions: ActionRegistry::default(),
                sink: Rc::new(WindowEventSink(weak.clone())),
                menubar,
                menu_presenter: Rc::default(),
                main_delivery: Rc::default(),
            })
        });
        let state = result.state();
        let model = state.borrow().host.main_window().clone();
        present_main_model(&result, model, Some((picture, state)));
        {
            let weak = Rc::downgrade(&result);
            let _ = window.connect_realize(move |_window| {
                if let Some(result) = weak.upgrade() {
                    result.realize();
                }
            });
        }
        {
            let weak = Rc::downgrade(&result);
            let _ = window.connect_close_request(move |_| {
                glib::Propagation::from(weak.upgrade().is_some_and(|result| result.close_request()))
            });
        }
        {
            let sink = result.event_sink();
            let _ = window.connect_is_active_notify(move |window| {
                if !window.is_active() {
                    sink.send(EventCommand::Session(SessionEvent::ClearInput));
                }
                sink.send(EventCommand::Focus(FocusEvent::Main(window.is_active())));
            });
        }
        {
            let weak = Rc::downgrade(&result);
            let _ = window.connect_notify_local(Some("fullscreened"), move |window, _| {
                if let Some(result) = weak.upgrade() {
                    sync_persisted_window_fullscreen(&result, window.is_fullscreen());
                }
            });
        }

        let key_controller = gtk::EventControllerKey::new();
        {
            let weak = Rc::downgrade(&result);
            let _ = key_controller.connect_key_pressed(move |_, key, _, _| {
                glib::Propagation::from(
                    weak.upgrade()
                        .is_some_and(|result| result.key_event(key, KeyEventState::Press)),
                )
            });
        }
        {
            let weak = Rc::downgrade(&result);
            let _ = key_controller.connect_key_released(move |_, key, _, _| {
                if let Some(result) = weak.upgrade() {
                    result.key_event(key, KeyEventState::Release);
                }
            });
        }
        window.add_controller(key_controller);

        // Generic aux pump: deferred opens (e.g. one aux view from another gesture)
        // plus the polled-widget refresh/poll/live-tick without kinds.
        {
            let weak = Rc::downgrade(&result);
            let _ = glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
                let Some(window) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                drain_deferred_opens(&window);
                pump_debug_widgets(&window);
                glib::ControlFlow::Continue
            });
        }

        result.rebuild_menubar();
        window.present();
        result.sync_fullscreen_from_settings();
        let _ = window.grab_focus();
        result
    }

    fn open(&self) {
        let file_chooser_native = gtk::FileChooserNative::new(
            Some("Open File"),
            Some(&self.window()),
            gtk::FileChooserAction::Open,
            Some("_Open"),
            Some("_Cancel"),
        );
        // Unified "All ROMs" first: GTK pre-selects the first filter, so
        // every registered system is visible in the initial view while
        // per-factory entries stay selectable behind it.
        let filters = self.state().borrow().host.file_dialog_filters();
        let mut first_filter = None;
        for (name, extensions) in filters {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some(&name));
            for extension in extensions {
                filter.add_pattern(&format!("*.{extension}"));
            }
            if first_filter.is_none() {
                first_filter = Some(filter.clone());
            }
            file_chooser_native.add_filter(&filter);
        }
        if let Some(first) = first_filter {
            file_chooser_native.set_filter(&first);
        }
        let weak = Rc::downgrade(self);
        let _ = file_chooser_native.connect_response(move |file_chooser_native, response| {
            let Some(result) = weak.upgrade() else { return };
            if response == gtk::ResponseType::Accept
                && let Some(path) = file_chooser_native.file().and_then(|f| f.path())
            {
                result.load_path(&path);
            }

            file_chooser_native.hide();
            result.borrow_mut().open_dialog = None;
            result.update_actions();

            let window = result.window();
            if let Some(root) = gtk::prelude::GtkWindowExt::focus(&window) {
                let _ = root.grab_focus();
            } else {
                let _ = window.grab_focus();
            }
        });
        self.borrow_mut().open_dialog = Some(file_chooser_native.clone());
        file_chooser_native.show();
    }

    fn load_path(&self, path: &Path) {
        // End the state borrow before presenting: the dialog reads settings back
        // through the same RefCell, so holding the guard would double-borrow.
        let detail = self.state().borrow_mut().load_path(path).err();
        if let Some(detail) = detail {
            crate::chrome::dialogs::show_file_load_error(&self.window(), self.state(), &detail);
        }
        self.update_actions();
    }

    fn realize(&self) {}

    fn close_request(&self) -> bool {
        if !self.state().borrow_mut().begin_close() {
            return true;
        }
        let w = self.window().width();
        let h = self.window().height();
        if w > 0 && h > 0 {
            let state = self.state();
            let _ = state
                .borrow_mut()
                .host
                .update_window_size(w as u32, h as u32);
        }
        self.state().borrow_mut().finish_close();
        self.application().quit();
        false
    }

    fn window(&self) -> gtk::ApplicationWindow {
        self.borrow().window.clone()
    }

    fn application(&self) -> gtk::Application {
        self.borrow().application.clone()
    }

    fn update_actions(&self) {
        let state = self.state();
        let model = {
            let mut state = state.borrow_mut();
            let menu_changed = state.host.menubar_update(GTK_MENU_CAPABILITIES).is_some();
            let title_changed = state.title_update().is_some();
            (menu_changed || title_changed).then(|| state.host.main_window().clone())
        };
        if let Some(model) = model {
            present_main_model(self, model, None);
        }
    }

    fn rebuild_menubar(&self) {
        self.update_actions();
    }

    fn sync_fullscreen_from_settings(&self) {
        let native = self.window().is_fullscreen();
        // Single host decision: the request gate, the steady-state
        // restore, and the scaling policy all live in the host.
        let sync = {
            let state = self.state();
            state.borrow_mut().host.window_chrome_sync(native)
        };

        // Restore remembered window size whenever the window ends up
        // windowed, even with no fullscreen transition pending: the
        // request gate returns None in the steady state, which must not
        // skip the restore.
        if let Some((width, height)) = sync.restore_size {
            self.window().set_default_size(width, height);
        }

        if let Some(fullscreen) = sync.set_fullscreen {
            set_window_fullscreen(&self.window(), fullscreen);
        }
    }

    fn key_event(&self, key: gdk::Key, event: KeyEventState) -> bool {
        // No hardcoded F-keys: the aux open rides the shortcut schema
        // (default F4, rebindable in Settings), so a user binding always wins.
        if let Some(controller_input) = crate::input::keyboard::key_from_gdk(key) {
            self.event_sink()
                .send(EventCommand::Session(SessionEvent::Keyboard {
                    key: controller_input,
                    pressed: key_event_pressed(event),
                }));
        }
        false
    }

    fn event_sink(&self) -> Rc<dyn EventSink> {
        self.borrow().sink.clone()
    }
}

fn set_window_fullscreen(window: &gtk::ApplicationWindow, fullscreen: bool) {
    window.set_fullscreened(fullscreen);
}

fn persist_window_fullscreen_default<K: Copy + Eq + std::hash::Hash + 'static>(
    window: &Window<K>,
    fullscreen: bool,
) -> Result<bool, String> {
    let state = window.state();
    let mut state = state.borrow_mut();
    let changed = state.host.persist_fullscreen_default(fullscreen)?;
    if changed {
        state.renderer_reload_pending = true;
    }
    Ok(changed)
}

fn sync_persisted_window_fullscreen<K: Copy + Eq + std::hash::Hash + 'static>(
    window: &Window<K>,
    fullscreen: bool,
) {
    if !window.state().borrow_mut().observe_fullscreen(fullscreen) {
        return;
    }
    match persist_window_fullscreen_default(window, fullscreen) {
        Ok(true) => window.update_actions(),
        Ok(false) => (),
        Err(error) => log::warn!("failed to persist fullscreen setting: {error}"),
    }
}

fn apply_persisted_window_fullscreen<K: Copy + Eq + std::hash::Hash + 'static>(
    window: &Window<K>,
    fullscreen: bool,
) {
    match persist_window_fullscreen_default(window, fullscreen) {
        Ok(_) => window.sync_fullscreen_from_settings(),
        Err(error) => {
            log::warn!("failed to persist fullscreen setting: {error}");
            set_window_fullscreen(&window.window(), fullscreen);
        }
    }
}

fn toggle_window_fullscreen<K: Copy + Eq + std::hash::Hash + 'static>(window: &Window<K>) {
    let fullscreen = !window.window().is_fullscreen();
    apply_persisted_window_fullscreen(window, fullscreen);
}

/// Use the single modal queue, including its acknowledgement path.
#[cfg(test)]
pub(crate) mod tests {
    use crate::testkit::TestAux;
    use wakui_core::chrome::aux::AuxId;

    pub(crate) fn program(
        window: &super::Window<TestAux>,
        kind: TestAux,
    ) -> crate::scene::binding::WidgetDialog {
        crate::chrome::aux_windows::open_auxiliary(window, kind);
        let Some(crate::chrome::aux_windows::NativeAux::Controls(program)) =
            window.borrow().aux.get(kind).cloned()
        else {
            panic!("generic widget program");
        };
        program
    }

    pub(crate) fn route(
        window: &super::Window<TestAux>,
        kind: TestAux,
    ) -> Option<wakui_core::chrome::aux::AuxId> {
        window.borrow().aux.route_id(kind)
    }
    pub(crate) fn inert_callbacks() -> crate::scene::binding::Callbacks {
        crate::scene::binding::Callbacks {
            current: Box::new(|| true),
            refresh: Box::new(|| None),
            submit: Box::new(|_| Vec::new()),
            submit_picker: Box::new(|_, _, _| {}),
            close: Box::new(|| {}),
        }
    }
    use gio::prelude::ActionExt as _;
    use gtk::prelude::{
        ActionMapExt as _, ApplicationExt as _, Cast as _, CastNone as _, DialogExt as _,
        GtkApplicationExt as _, GtkWindowExt as _, ObjectExt as _,
    };

    use super::{KeyEventState, WindowExtend, key_event_pressed};
    use crate::chrome::menu::{GTK_MENU_CAPABILITIES, GtkMenuVisitor, MenubarModels};
    use crate::chrome::surface_state::surface_gone;
    use crate::testkit::{TestMenu, TestSession, menu_id};
    use wakui_core::input::commands::CommandId;

    /// Opaque id for a simulated command, found in the window's latest
    /// menubar by downcasting the generic spec. Native registries key on
    /// these, never on domain names or debug strings.
    pub(crate) fn command_id(window: &super::Window<TestAux>, command: TestMenu) -> CommandId {
        let state = window.state();
        let borrowed = state.borrow();
        let menu = borrowed
            .host
            .main_window()
            .menu
            .as_ref()
            .expect("menubar built");
        menu.commands
            .definitions
            .iter()
            .find(|descriptor| descriptor.downcast::<TestMenu>() == Some(&command.template()))
            .expect("command declaration")
            .id
    }

    #[test]
    fn test_aux_titles_and_resume_mirror_view_defs() {
        assert_eq!(TestAux::Gamma.title(), "Aux Gamma");
        assert_eq!(TestAux::Delta.title(), "Aux Delta");
        assert_eq!(TestAux::Beta.title(), "Aux Beta");
        assert_eq!(TestAux::Alpha.title(), "Aux Alpha");
        assert!(TestAux::Gamma.resume_on_arrival());
        assert!(TestAux::Delta.resume_on_arrival());
        assert!(!TestAux::Beta.resume_on_arrival());
        assert!(!TestAux::Alpha.resume_on_arrival());
    }

    #[test]
    fn submenu_registry_is_lazy_and_shared_across_clones() {
        let bars = MenubarModels::new();
        let shared = bars.clone();
        assert!(bars.models.borrow().is_empty());
        let file = bars.submenu(menu_id::GROUP_A);
        assert_eq!(bars.models.borrow().len(), 1);
        assert_eq!(shared.submenu(menu_id::GROUP_A), file);
        let states = shared.submenu(menu_id::GROUP_C);
        assert_ne!(states, file);
        assert_eq!(bars.submenu(menu_id::GROUP_C), states);
        assert_eq!(bars.models.borrow().len(), 2);
    }

    /// Externally destroyed dialogs keep their item (no signal fires):
    /// reopening must rebuild instead of panicking on the item borrow.
    /// Widget-only: needs a display, run under Xvfb (skips headless).
    #[test]
    fn open_history_rebuilds_dead_dialog_without_borrow_panic() {
        let Some(win) = bound_window() else {
            eprintln!("SKIP: window widget test needs a display (run under Xvfb)");
            return;
        };
        // Bypass the item wiring: a directly built dialog carries no
        // destroy hook, like an externally destroyed one whose
        // close-request never ran.
        crate::chrome::aux_windows::open_auxiliary(&win, TestAux::Beta);
        let built = win.borrow().aux.get(TestAux::Beta).cloned().unwrap();
        // External destroy disposes the widget without signals; the
        // item keeps the dead dialog.
        let dead = built.native_window();
        dead.destroy();
        assert!(surface_gone(&dead));
        // Must rebuild, not panic on the item borrow.
        crate::chrome::aux_windows::open_auxiliary(&win, TestAux::Beta);
        let item = win.borrow().aux.get(TestAux::Beta).cloned();
        let Some(crate::chrome::aux_windows::NativeAux::Controls(program)) = item else {
            panic!("rebuilt dialog");
        };
        let dialog = program.dialog;
        assert!(!surface_gone(&dialog));
        dialog.destroy();
    }

    pub(crate) fn bound_window() -> Option<super::Window<TestAux>> {
        let (app, window, state) = harness()?;
        Some(super::Window::<TestAux>::bind(
            app,
            window,
            gtk::Picture::new(),
            state,
            crate::chrome::menu::MenubarModels::new(),
        ))
    }

    /// Display-bound window sharing its headless app: hold assertions steer
    /// the fake directly; production code never sees these paths.
    pub(crate) fn bound_window_shared() -> Option<(
        super::Window<TestAux>,
        std::rc::Rc<std::cell::RefCell<crate::testkit::FakeApp>>,
    )> {
        let (app, window, state, fake) = harness_shared()?;
        Some((
            super::Window::<TestAux>::bind(
                app,
                window,
                gtk::Picture::new(),
                state,
                crate::chrome::menu::MenubarModels::new(),
            ),
            fake,
        ))
    }

    fn harness_shared() -> Option<(
        gtk::Application,
        gtk::ApplicationWindow,
        TestHarnessState,
        std::rc::Rc<std::cell::RefCell<crate::testkit::FakeApp>>,
    )> {
        if !crate::test_support::ui_test_available() {
            return None;
        }
        static NEXT_APP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let app_id = NEXT_APP.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let app = gtk::Application::builder()
            .application_id(format!("org.nerust.window-test.n{app_id}"))
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        // Menubar binding requires startup, just as in the real activate path.
        app.register(None::<&gio::Cancellable>)
            .expect("test application registers");
        let window = gtk::ApplicationWindow::builder().application(&app).build();
        let (state, fake) = model_state_shared();
        Some((app, window, state, fake))
    }

    #[test]
    #[ignore = "needs display; CI-only under xvfb"]
    fn common_main_title_reentry_delivers_latest_window_including_menu() {
        use gio::prelude::MenuModelExt;
        use std::{cell::Cell, rc::Rc};
        let win = bound_window().expect("run in an isolated Xvfb process");
        let mut outer = win.state().borrow().host.main_window().clone();
        outer.title = "outer title".into();
        outer.menu.as_mut().unwrap().menus[0].label = "outer menu".into();
        let mut newest = outer.clone();
        newest.title = "newest title".into();
        newest.menu.as_mut().unwrap().menus[0].label = "newest menu".into();
        let called = Rc::new(Cell::new(false));
        let signal = win.window().connect_title_notify({
            let weak = Rc::downgrade(&win);
            let called = called.clone();
            move |_| {
                let win = weak.upgrade().unwrap();
                assert!(win.try_borrow_mut().is_ok());
                assert!(win.state().try_borrow_mut().is_ok());
                assert!(win.borrow().main_delivery.pending.try_borrow_mut().is_ok());
                if !called.replace(true) {
                    super::present_main_model(&win, newest.clone(), None);
                }
            }
        });
        super::present_main_model(&win, outer, None);
        assert!(called.get());
        assert_eq!(win.window().title().as_deref(), Some("newest title"));
        let bars = win.borrow().menubar.clone();
        assert_eq!(
            bars.menu_model
                .item_attribute_value(0, "label", None)
                .unwrap()
                .str(),
            Some("newest menu")
        );
        assert!(!win.borrow().main_delivery.active.get());
        win.window().disconnect(signal);
        win.window().destroy();
    }

    #[test]
    #[ignore = "needs display; CI-only under xvfb"]
    fn common_main_window_menu_opens_debug_from_reserved_owned_blueprint() {
        use crate::chrome::aux_windows::NativeAux;
        use gio::prelude::MenuModelExt;
        use gtk::prelude::WidgetExt;
        use std::{cell::Cell, rc::Rc};
        use wakui_core::chrome::aux::OpenRequest;
        use wakui_core::chrome::window::Window as HostWindow;

        let win = bound_window().expect("run in an isolated Xvfb process");
        let mut model = win.state().borrow().host.main_window().clone();
        assert!(matches!(
            model.contents,
            wakui_core::chrome::window::WidgetContents::Main(_)
        ));
        model.title = "owned main chrome".into();
        model.menu.as_mut().unwrap().menus[0].label = "owned main menu".into();
        super::present_main_model(&win, model, None);
        assert_eq!(win.window().title().as_deref(), Some("owned main chrome"));
        assert!(win.window().child().unwrap().is::<gtk::Picture>());
        let bars = win.borrow().menubar.clone();
        assert_eq!(
            bars.menu_model
                .item_attribute_value(0, "label", None)
                .unwrap()
                .str(),
            Some("owned main menu")
        );

        let added = Rc::new(Cell::new(0));
        let mapped = Rc::new(Cell::new(0));
        let signal = win.application().connect_window_added({
            let weak = Rc::downgrade(&win);
            let added = added.clone();
            let mapped = mapped.clone();
            move |_, native| {
                let win = weak.upgrade().unwrap();
                assert!(win.try_borrow_mut().is_ok());
                assert!(win.state().try_borrow_mut().is_ok());
                let id = win.borrow().aux.route_id(TestAux::Gamma).unwrap();
                let model = win.borrow().aux.window(id).cloned().unwrap();
                assert!(matches!(
                    model.contents,
                    wakui_core::chrome::window::WidgetContents::Controls(_)
                ));
                assert_eq!(
                    win.borrow_mut()
                        .aux
                        .request_open(TestAux::Gamma, TestAux::Gamma.title()),
                    OpenRequest::Pending(id)
                );
                crate::chrome::aux_windows::open_auxiliary(&win, TestAux::Gamma);
                added.set(added.get() + 1);
                let weak = Rc::downgrade(&win);
                let mapped = mapped.clone();
                native.connect_map(move |native| {
                    let win = weak.upgrade().unwrap();
                    assert!(win.try_borrow_mut().is_ok());
                    assert!(win.state().try_borrow_mut().is_ok());
                    assert_eq!(native.title().as_deref(), Some(model.title.as_str()));
                    assert!(win.borrow().aux.get(TestAux::Gamma).is_none());
                    mapped.set(mapped.get() + 1);
                });
            }
        });
        // Resolve the real GMenu action rather than bypassing native dispatch.
        let group_b = bars.submenu(menu_id::GROUP_B);
        let gamma_action = (0..group_b.n_items()).find_map(|i| {
            group_b
                .item_attribute_value(i, "action", None)
                .and_then(|value| {
                    value
                        .str()
                        .filter(|name| *name == "win.aux_gamma")
                        .map(str::to_owned)
                })
        });
        let name = gamma_action.expect("host Gamma entry in main_window.menu");
        let action = win
            .window()
            .lookup_action(name.strip_prefix("win.").unwrap())
            .unwrap();
        action.activate(None);
        win.application().disconnect(signal);
        assert_eq!(added.get(), 1);
        assert_eq!(mapped.get(), 1);
        let id = win.borrow().aux.route_id(TestAux::Gamma).unwrap();
        let Some(NativeAux::Controls(program)) = win.borrow().aux.get(TestAux::Gamma).cloned()
        else {
            panic!("generic aux dialog constructed");
        };
        let native = program.dialog.clone();
        assert!(native.content_area().first_child().is_some());
        assert_eq!(
            native.title().as_deref(),
            Some(
                HostWindow::auxiliary(AuxId::from_raw(1), TestAux::Gamma.title())
                    .title
                    .as_str()
            )
        );
        crate::chrome::aux_windows::open_auxiliary(&win, TestAux::Gamma);
        assert_eq!(win.borrow().aux.route_id(TestAux::Gamma), Some(id));
        native.destroy();
        win.window().destroy();
    }

    #[test]
    #[ignore = "needs display; CI-only under xvfb"]
    fn common_auxiliary_contents_choose_native_groups_and_owned_titles() {
        use gtk::prelude::WidgetExt;
        use wakui_core::chrome::window::Window as HostWindow;
        use wakui_core::chrome::window::WindowId;
        let win = bound_window().expect("run in an isolated Xvfb process");
        let main = win.window();
        let state = win.state();
        // Owned titles survive: a model with an owned title renders verbatim.
        let mut debug = HostWindow::auxiliary(AuxId::from_raw(1), TestAux::Gamma.title());
        debug.id = WindowId::Auxiliary(AuxId::from_raw(2));
        debug.title = "owned debug title".into();
        let hid = crate::chrome::aux_windows::request_aux(&win, TestAux::Beta).unwrap();
        {
            let mut core = win.borrow_mut();
            let hstate = core.state.clone();
            hstate
                .borrow_mut()
                .host
                .prepare_widget(&mut core.aux, hid)
                .expect("prepare widget");
        }
        let (_hmodel, hscreen, hmailbox) = {
            let core = win.borrow();
            let hstate = core.state.clone();
            let borrowed = hstate.borrow();
            (
                core.aux.window(hid).cloned().expect("reserved window"),
                borrowed.host.widget_screen(hid).expect("screen"),
                borrowed.host.widget_mailbox(hid).expect("mailbox"),
            )
        };
        // Contents, not ID, select grouped controls: a History screen renders
        // even when the model id names another route.
        let mut history = _hmodel;
        history.id = WindowId::Auxiliary(AuxId::from_raw(2));
        let Some(crate::chrome::dialogs::RenderedDialog::Controls(built)) =
            crate::chrome::dialogs::render_dialog(
                &history,
                &main,
                hid,
                hscreen.clone(),
                hmailbox.clone(),
                crate::chrome::aux_windows::widget_callbacks(&win, hid),
            )
        else {
            panic!("History contents, not ID, select grouped controls")
        };
        assert!(built.dialog.content_area().first_child().is_some());
        let pending = HostWindow::auxiliary(AuxId::from_raw(1), TestAux::Alpha.title());
        assert!(
            crate::chrome::dialogs::render_dialog(
                &pending,
                &main,
                hid,
                hscreen.clone(),
                hmailbox.clone(),
                inert_callbacks()
            )
            .is_none()
        );
        crate::chrome::aux_windows::close_aux(&win, hid);

        // PPU is a generic widget window: the aux model starts pending,
        // the view-side aux screen supplies the ControlScreen.
        let pid = crate::chrome::aux_windows::request_aux(&win, TestAux::Delta).unwrap();
        {
            let mut core = win.borrow_mut();
            let pstate = core.state.clone();
            pstate
                .borrow_mut()
                .host
                .prepare_widget(&mut core.aux, pid)
                .expect("prepare PPU widget");
        }
        let (pmodel, pscreen2, pmailbox) = {
            let core = win.borrow();
            let pstate = core.state.clone();
            let borrowed = pstate.borrow();
            (
                core.aux.window(pid).cloned().expect("reserved PPU window"),
                borrowed.host.widget_screen(pid).expect("PPU screen"),
                borrowed.host.widget_mailbox(pid).expect("PPU mailbox"),
            )
        };
        assert_eq!(pmodel.title, TestAux::Delta.title());
        let Some(crate::chrome::dialogs::RenderedDialog::Controls(pbuilt)) =
            crate::chrome::dialogs::render_dialog(
                &pmodel,
                &main,
                pid,
                pscreen2,
                pmailbox,
                inert_callbacks(),
            )
        else {
            panic!("PPU widget renders as generic controls");
        };
        assert!(pbuilt.dialog.content_area().first_child().is_some());
        crate::chrome::aux_windows::close_aux(&win, pid);

        // Aux gamma is also a generic widget window now.
        let did = crate::chrome::aux_windows::request_aux(&win, TestAux::Gamma).unwrap();
        {
            let mut core = win.borrow_mut();
            let dstate = core.state.clone();
            dstate
                .borrow_mut()
                .host
                .prepare_widget(&mut core.aux, did)
                .expect("prepare Debug widget");
        }
        let (dmodel, dscreen, dmailbox) = {
            let core = win.borrow();
            let dstate = core.state.clone();
            let borrowed = dstate.borrow();
            (
                core.aux
                    .window(did)
                    .cloned()
                    .expect("reserved Debug window"),
                borrowed.host.widget_screen(did).expect("Debug screen"),
                borrowed.host.widget_mailbox(did).expect("Debug mailbox"),
            )
        };
        assert_eq!(dmodel.title, TestAux::Gamma.title());
        let Some(crate::chrome::dialogs::RenderedDialog::Controls(dbuilt)) =
            crate::chrome::dialogs::render_dialog(
                &dmodel,
                &main,
                did,
                dscreen,
                dmailbox,
                inert_callbacks(),
            )
        else {
            panic!("Debug widget renders as generic controls");
        };
        assert!(dbuilt.dialog.content_area().first_child().is_some());
        crate::chrome::aux_windows::close_aux(&win, did);

        let id = crate::chrome::aux_windows::request_aux(&win, TestAux::Beta).unwrap();
        {
            let mut core = win.borrow_mut();
            let state = core.state.clone();
            state
                .borrow_mut()
                .host
                .prepare_widget(&mut core.aux, id)
                .expect("prepare widget");
        }
        let (model, screen, mailbox) = {
            let core = win.borrow();
            let state = core.state.clone();
            let borrowed = state.borrow();
            (
                core.aux.window(id).cloned().expect("reserved window"),
                borrowed.host.widget_screen(id).expect("screen"),
                borrowed.host.widget_mailbox(id).expect("mailbox"),
            )
        };
        let mut history = model;
        history.id = WindowId::Auxiliary(AuxId::from_raw(2));
        let Some(crate::chrome::dialogs::RenderedDialog::Controls(built)) =
            crate::chrome::dialogs::render_dialog(
                &history,
                &main,
                id,
                screen.clone(),
                mailbox.clone(),
                crate::chrome::aux_windows::widget_callbacks(&win, id),
            )
        else {
            panic!("History contents, not ID, select grouped controls")
        };
        assert!(built.dialog.content_area().first_child().is_some());
        let pending = HostWindow::auxiliary(AuxId::from_raw(1), TestAux::Alpha.title());
        assert!(
            crate::chrome::dialogs::render_dialog(
                &pending,
                &main,
                id,
                screen.clone(),
                mailbox.clone(),
                inert_callbacks()
            )
            .is_none()
        );
        built.dialog.destroy();
        pbuilt.dialog.destroy();
        dbuilt.dialog.destroy();
        let _ = state;
        let _ = debug;
        main.destroy();
    }

    #[test]
    #[ignore = "needs display; CI-only under xvfb"]
    fn common_dialog_map_reentry_sees_committed_identity_without_borrows() {
        use gtk::prelude::{ListModelExt, WidgetExt};
        use std::{cell::Cell, rc::Rc};
        use wakui_core::chrome::aux::OpenRequest;
        let win = bound_window().expect("isolated Xvfb process");
        for kind in [TestAux::Beta, TestAux::Alpha] {
            let created = Rc::new(Cell::new(0));
            let mapped = Rc::new(Cell::new(0));
            let toplevels = gtk::Window::toplevels();
            let signal = toplevels.connect_items_changed({
                let weak = Rc::downgrade(&win);
                let created = created.clone();
                let mapped = mapped.clone();
                move |list, position, _, added| {
                    for i in position..position + added {
                        let Some(dialog) = list.item(i).and_downcast::<gtk::Dialog>() else {
                            continue;
                        };
                        let win = weak.upgrade().unwrap();
                        assert!(win.try_borrow_mut().is_ok());
                        assert!(win.state().try_borrow_mut().is_ok());
                        let id = win.borrow().aux.route_id(kind).unwrap();
                        assert_eq!(
                            win.borrow_mut().aux.request_open(kind, kind.title()),
                            OpenRequest::Pending(id)
                        );
                        crate::chrome::aux_windows::open_auxiliary(&win, kind);
                        created.set(created.get() + 1);
                        let weak = Rc::downgrade(&win);
                        let mapped = mapped.clone();
                        dialog.connect_map(move |dialog| {
                            let win = weak.upgrade().unwrap();
                            assert!(win.try_borrow_mut().is_ok());
                            assert!(win.state().try_borrow_mut().is_ok());
                            assert_eq!(win.borrow().aux.route_id(kind), Some(id));
                            assert_eq!(
                                win.borrow().aux.get(kind).unwrap().native_window(),
                                dialog.clone().upcast::<gtk::Window>()
                            );
                            // Count before re-presenting: GTK can emit map synchronously.
                            if mapped.replace(mapped.get() + 1) == 0 {
                                crate::chrome::aux_windows::open_auxiliary(&win, kind);
                            }
                        });
                    }
                }
            });
            crate::chrome::aux_windows::open_auxiliary(&win, kind);
            toplevels.disconnect(signal);
            assert_eq!(created.get(), 1);
            assert_eq!(mapped.get(), 1);
            let native = win.borrow().aux.get(kind).unwrap().native_window();
            native.destroy();
            assert!(!win.borrow().aux.contains(kind));
        }
        win.window().destroy();
    }

    #[test]
    #[ignore = "needs display; CI-only under xvfb"]
    fn common_dialog_stale_build_cannot_remove_replacement_or_release_settings_pause() {
        use crate::testkit::owners;
        use TestAux;
        use gtk::prelude::{ListModelExt, WidgetExt};
        use std::{cell::RefCell, rc::Rc};
        let Some((win, fake)) = bound_window_shared() else {
            eprintln!("SKIP: window widget test needs a display");
            return;
        };
        for kind in [TestAux::Beta, TestAux::Alpha] {
            let stale = Rc::new(RefCell::new(None));
            let toplevels = gtk::Window::toplevels();
            let signal = toplevels.connect_items_changed({
                let weak = Rc::downgrade(&win);
                let stale = stale.clone();
                move |list, position, _, added| {
                    if stale.borrow().is_some() {
                        return;
                    }
                    for i in position..position + added {
                        let Some(dialog) = list.item(i).and_downcast::<gtk::Dialog>() else {
                            continue;
                        };
                        *stale.borrow_mut() = Some(dialog);
                        let win = weak.upgrade().unwrap();
                        assert!(win.try_borrow_mut().is_ok());
                        assert!(win.state().try_borrow_mut().is_ok());
                        let old = win.borrow().aux.route_id(kind).unwrap();
                        assert!(crate::chrome::aux_windows::close_aux(&win, old));
                        crate::chrome::aux_windows::open_auxiliary(&win, kind);
                        assert!(!crate::chrome::aux_windows::close_aux(&win, old));
                    }
                }
            });
            crate::chrome::aux_windows::open_auxiliary(&win, kind);
            toplevels.disconnect(signal);
            let stale = stale.borrow().clone().unwrap();
            assert!(surface_gone(&stale));
            let current = win.borrow().aux.get(kind).unwrap().native_window();
            assert_ne!(current, stale.clone().upcast::<gtk::Window>());
            assert!(current.is_visible());
            if kind == TestAux::Alpha {
                assert!(fake.borrow().owns_pause(owners::GUARD));
                assert!(
                    !win.borrow()
                        .actions
                        .get(command_id(&win, TestMenu::AuxAlpha))
                        .unwrap()
                        .is_enabled()
                );
            }
            stale.destroy();
            assert!(win.borrow().aux.contains(kind));
            current.destroy();
            assert!(!win.borrow().aux.contains(kind));
            assert!(!fake.borrow().owns_pause(owners::GUARD));
        }
        win.window().destroy();
    }

    #[test]
    #[ignore = "needs display; CI-only under xvfb"]
    fn common_debug_stale_native_completion_preserves_replacement_without_borrows() {
        use TestAux;
        use std::{cell::RefCell, rc::Rc};
        let win = bound_window().expect("run in an isolated Xvfb process");
        let stale = Rc::new(RefCell::new(None));
        let signal = win.application().connect_window_added({
            let weak = Rc::downgrade(&win);
            let stale = stale.clone();
            move |_, native| {
                if stale.borrow().is_some() {
                    return;
                }
                *stale.borrow_mut() = Some(native.clone());
                let win = weak.upgrade().unwrap();
                let old = win.borrow().aux.route_id(TestAux::Gamma).unwrap();
                assert!(win.borrow_mut().aux.cancel_open(old));
                crate::chrome::aux_windows::open_auxiliary(&win, TestAux::Gamma);
                assert!(!crate::chrome::aux_windows::close_aux(&win, old));
            }
        });
        let removed = win.application().connect_window_removed({
            let weak = Rc::downgrade(&win);
            move |_, _| {
                let win = weak.upgrade().unwrap();
                assert!(win.try_borrow_mut().is_ok());
                assert!(win.state().try_borrow_mut().is_ok());
            }
        });
        crate::chrome::aux_windows::open_auxiliary(&win, TestAux::Gamma);
        win.application().disconnect(signal);
        win.application().disconnect(removed);
        let stale = stale.borrow().clone().unwrap();
        assert!(surface_gone(&stale));
        let current = win
            .borrow()
            .aux
            .get(TestAux::Gamma)
            .unwrap()
            .native_window();
        assert_ne!(current, stale);
        assert!(
            win.borrow()
                .aux
                .window(win.borrow().aux.route_id(TestAux::Gamma).unwrap())
                .is_some()
        );
        current.destroy();
        win.window().destroy();
    }

    #[test]
    fn action_enabled_notify_can_reenter_updates_without_controller_borrows() {
        use std::{cell::Cell, rc::Rc};
        let Some(win) = bound_window() else {
            eprintln!("SKIP: window widget test needs a display");
            return;
        };
        let notifications = Rc::new(Cell::new(0));
        let borrow_free = Rc::new(Cell::new(true));
        let action = win
            .borrow()
            .actions
            .get(command_id(&win, TestMenu::Open))
            .unwrap();
        let weak = Rc::downgrade(&win);
        let count = notifications.clone();
        let available = borrow_free.clone();
        action.connect_enabled_notify(move |_| {
            let win = weak.upgrade().unwrap();
            let state = win.state();
            let free = win.try_borrow_mut().is_ok()
                && state.try_borrow_mut().is_ok()
                && win.borrow().menubar.models.try_borrow_mut().is_ok()
                && win.borrow().actions.0.try_borrow_mut().is_ok();
            available.set(available.get() && free);
            count.set(count.get() + 1);
            if free {
                win.update_actions();
            }
        });
        // A real host input change, not a direct call to the native setter.
        let wakui_core::chrome::aux::OpenRequest::Create(id) = win
            .borrow_mut()
            .aux
            .request_open(TestAux::Alpha, TestAux::Alpha.title())
        else {
            panic!("expected settings reservation");
        };
        win.update_actions();
        assert_eq!(notifications.get(), 1);
        assert!(borrow_free.get());
        assert!(!action.is_enabled());
        assert!(
            !win.borrow()
                .actions
                .get(command_id(&win, TestMenu::AuxAlpha))
                .unwrap()
                .is_enabled()
        );
        assert!(crate::chrome::aux_windows::close_aux(&win, id));
        win.update_actions();
        assert_eq!(notifications.get(), 2);
        assert!(borrow_free.get());
        assert!(action.is_enabled());
        assert!(
            win.borrow()
                .actions
                .get(command_id(&win, TestMenu::AuxAlpha))
                .unwrap()
                .is_enabled()
        );
    }

    /// Drive the settings widget without native dialogs: reserve, prepare,
    /// dispatch a single choice by visible label, submit, close.
    fn apply_settings_choice(win: &super::Window<TestAux>, label: &str, needle: &str) {
        use wakui_core::scene::control::{
            Control, ControlDisplay, ControlEvent, ControlId, ControlInput, ControlVisitor,
        };
        struct Find<'a>(&'a str, Option<ControlId>);
        impl ControlVisitor for Find<'_> {
            fn control(&mut self, control: &Control) {
                if control.label == self.0 {
                    self.1 = Some(control.id);
                }
            }
        }
        fn find(screen: &wakui_core::scene::control::ControlScreen, label: &str) -> ControlId {
            let mut find = Find(label, None);
            screen.visit(&mut find);
            find.1.unwrap_or_else(|| panic!("control {label:?}"))
        }
        let wakui_core::chrome::aux::OpenRequest::Create(id) = win
            .borrow_mut()
            .aux
            .request_open(TestAux::Alpha, TestAux::Alpha.title())
        else {
            panic!("settings reservation");
        };
        {
            let mut core = win.borrow_mut();
            let state = core.state.clone();
            state
                .borrow_mut()
                .host
                .prepare_widget(&mut core.aux, id)
                .expect("prepare widget");
        }
        let screen = win
            .state()
            .borrow()
            .host
            .widget_screen(id)
            .expect("settings screen");
        let control = find(&screen, label);
        let choices = screen
            .control(control)
            .unwrap()
            .downcast::<wakui_core::scene::leaf::ChoiceLeaf>()
            .unwrap()
            .choices
            .clone();
        let index = choices
            .iter()
            .position(|choice| choice.contains(needle))
            .unwrap_or_else(|| panic!("choice containing {needle:?}"));
        win.state()
            .borrow_mut()
            .host
            .widget_dispatch(id, ControlEvent::new(control, ControlInput::Select(index)));
        struct Primary(Option<ControlId>);
        impl ControlVisitor for Primary {
            fn control(&mut self, control: &Control) {
                if control.display == ControlDisplay::Primary {
                    self.0 = Some(control.id);
                }
            }
        }
        let screen = win
            .state()
            .borrow()
            .host
            .widget_screen(id)
            .expect("settings screen");
        let mut primary = Primary(None);
        screen.visit(&mut primary);
        let submit = primary.0.expect("settings submit");
        win.state()
            .borrow_mut()
            .host
            .widget_dispatch(id, ControlEvent::new(submit, ControlInput::Activate));
        {
            let mut core = win.borrow_mut();
            let state = core.state.clone();
            let _ = state
                .borrow_mut()
                .host
                .close_widget(&mut core.aux, id, false);
        }
        assert!(crate::chrome::aux_windows::close_aux(win, id));
    }

    #[test]
    fn unchanged_menu_models_emit_no_items_changed_and_relabels_allow_reentry() {
        use gio::prelude::MenuModelExt as _;
        use std::{cell::Cell, rc::Rc};
        let Some(win) = bound_window() else {
            eprintln!("SKIP: window widget test needs a display");
            return;
        };
        // The default language is English; the later change must change
        // the visible model.
        win.update_actions();
        let menubar = win.borrow().menubar.clone();
        assert!(menubar.attached.get());
        assert_eq!(
            win.application().menubar().as_ref(),
            Some(menubar.menu_model.upcast_ref())
        );
        let models = [
            menubar.menu_model.clone(),
            menubar.submenu(menu_id::GROUP_A),
            menubar.submenu(menu_id::GROUP_C),
            menubar.submenu(menu_id::GROUP_B),
            menubar.submenu(menu_id::GROUP_D),
            menubar.submenu(menu_id::SUB_A),
            menubar.submenu(menu_id::SUB_B),
            menubar.submenu(menu_id::SUB_C),
            menubar.submenu(menu_id::SUB_D),
        ];
        let changes = Rc::new(Cell::new(0));
        let borrow_free = Rc::new(Cell::new(true));
        for model in &models {
            let weak = Rc::downgrade(&win);
            let changes = changes.clone();
            let available = borrow_free.clone();
            model.connect_items_changed(move |_, _, _, _| {
                let win = weak.upgrade().unwrap();
                let state = win.state();
                let free = win.try_borrow_mut().is_ok()
                    && state.try_borrow_mut().is_ok()
                    && win.borrow().menubar.models.try_borrow_mut().is_ok()
                    && win.borrow().actions.0.try_borrow_mut().is_ok();
                available.set(available.get() && free);
                changes.set(changes.get() + 1);
                if free {
                    win.update_actions();
                }
            });
        }
        win.update_actions();
        win.rebuild_menubar();
        let wakui_core::chrome::aux::OpenRequest::Create(id) = win
            .borrow_mut()
            .aux
            .request_open(TestAux::Alpha, TestAux::Alpha.title())
        else {
            panic!("expected settings reservation");
        };
        win.update_actions();
        assert!(crate::chrome::aux_windows::close_aux(&win, id));
        win.update_actions();
        assert_eq!(
            changes.get(),
            0,
            "action-only changes must not rebuild models"
        );

        apply_settings_choice(&win, "Choice A", "日本語");
        win.update_actions();
        assert!(changes.get() > 0);
        assert!(borrow_free.get());
        assert_eq!(
            win.application().menubar().as_ref(),
            Some(menubar.menu_model.upcast_ref())
        );
        for (model, id) in models[1..].iter().zip([
            menu_id::GROUP_A,
            menu_id::GROUP_C,
            menu_id::GROUP_B,
            menu_id::GROUP_D,
            menu_id::SUB_A,
            menu_id::SUB_B,
            menu_id::SUB_C,
            menu_id::SUB_D,
        ]) {
            assert_eq!(model, &menubar.submenu(id));
        }
        changes.set(0);
        win.update_actions();
        win.rebuild_menubar();
        assert_eq!(changes.get(), 0);
    }

    #[test]
    fn action_notify_newer_settings_guard_wins_over_outer_render() {
        use std::{cell::Cell, rc::Rc};
        let Some(win) = bound_window() else {
            eprintln!("SKIP: window widget test needs a display");
            return;
        };
        let action = win
            .borrow()
            .actions
            .get(command_id(&win, TestMenu::Open))
            .unwrap();
        let quit = win
            .borrow()
            .actions
            .get(command_id(&win, TestMenu::Quit))
            .unwrap();
        let wakui_core::chrome::aux::OpenRequest::Create(id) = win
            .borrow_mut()
            .aux
            .request_open(TestAux::Alpha, TestAux::Alpha.title())
        else {
            panic!("expected settings reservation");
        };
        let called = Rc::new(Cell::new(false));
        let once = called.clone();
        let weak = Rc::downgrade(&win);
        action.connect_enabled_notify(move |_| {
            if once.replace(true) {
                return;
            }
            let win = weak.upgrade().unwrap();
            assert!(win.try_borrow_mut().is_ok());
            assert!(win.state().try_borrow_mut().is_ok());
            assert!(crate::chrome::aux_windows::close_aux(&win, id));
            win.update_actions();
        });
        win.update_actions();
        assert!(called.get());
        assert!(action.is_enabled());
        assert!(
            win.borrow()
                .actions
                .get(command_id(&win, TestMenu::AuxAlpha))
                .unwrap()
                .is_enabled()
        );
        assert!(
            win.borrow()
                .actions
                .get(command_id(&win, TestMenu::AuxGamma))
                .unwrap()
                .is_enabled()
        );
        assert!(
            win.borrow()
                .actions
                .get(command_id(&win, TestMenu::AuxBeta))
                .unwrap()
                .is_enabled()
        );
        assert!(
            quit.is_enabled(),
            "old outer traversal must not leave Quit disabled"
        );
        assert!(
            !win.borrow()
                .actions
                .get(command_id(&win, TestMenu::Close))
                .unwrap()
                .is_enabled()
        );
    }

    #[test]
    fn items_changed_callback_mutates_settings_and_newest_labels_win() {
        use gio::prelude::MenuModelExt;
        use std::{cell::Cell, rc::Rc};
        let Some(win) = bound_window() else {
            eprintln!("SKIP: window widget test needs a display");
            return;
        };
        win.update_actions();
        let called = Rc::new(Cell::new(false));
        let once = called.clone();
        let weak = Rc::downgrade(&win);
        let menubar = win.borrow().menubar.clone();
        let model = menubar.submenu(menu_id::GROUP_C);
        model.connect_items_changed(move |_, _, _, _| {
            if once.replace(true) {
                return;
            }
            let win = weak.upgrade().unwrap();
            assert!(win.try_borrow_mut().is_ok());
            apply_settings_choice(&win, "Choice A", "English");
            win.update_actions();
        });
        apply_settings_choice(&win, "Choice A", "日本語");
        win.update_actions();
        assert!(called.get(), "must exercise real items-changed reentry");
        fn labels(model: &gio::MenuModel, result: &mut Vec<String>) {
            for index in 0..model.n_items() {
                if let Some(label) = model
                    .item_attribute_value(index, "label", None)
                    .and_then(|value| value.str().map(str::to_owned))
                {
                    result.push(label);
                }
                if let Some(child) = model.item_link(index, gio::MENU_LINK_SUBMENU) {
                    labels(&child, result);
                }
            }
        }
        let root = win.borrow().menubar.menu_model.clone();
        let mut rendered = Vec::new();
        labels(root.upcast_ref(), &mut rendered);
        assert!(
            rendered.iter().any(|label| label == "Group A"),
            "{rendered:?}"
        );
        assert!(
            !rendered.iter().any(|label| label == "ファイル"),
            "{rendered:?}"
        );
        win.update_actions();
    }

    #[test]
    fn auxiliary_open_reservations_block_reentrant_native_creation() {
        use wakui_core::chrome::aux::OpenRequest;
        let Some(win) = bound_window() else {
            eprintln!("SKIP: window widget test needs a display");
            return;
        };
        for kind in [TestAux::Gamma, TestAux::Beta, TestAux::Alpha] {
            let OpenRequest::Create(id) = win.borrow_mut().aux.request_open(kind, kind.title())
            else {
                panic!("expected reservation");
            };
            match kind {
                TestAux::Gamma => crate::chrome::aux_windows::open_auxiliary(&win, TestAux::Gamma),
                TestAux::Beta => crate::chrome::aux_windows::open_auxiliary(&win, TestAux::Beta),
                TestAux::Alpha => {
                    let action = win
                        .borrow()
                        .actions
                        .get(command_id(&win, TestMenu::AuxAlpha))
                        .unwrap();
                    crate::chrome::aux_windows::open_auxiliary(&win, TestAux::Alpha);
                    assert!(action.is_enabled());
                }
                TestAux::Delta => unreachable!(),
            }
            assert_eq!(win.borrow().aux.route_id(kind), Some(id));
            assert!(win.borrow().aux.get(kind).is_none());
            assert!(win.borrow_mut().aux.cancel_open(id));
        }
    }

    #[test]
    fn history_reuses_identity_and_old_destroy_cannot_close_replacement() {
        use crate::chrome::aux_windows::NativeAux;
        let Some(win) = bound_window() else {
            eprintln!("SKIP: window widget test needs a display");
            return;
        };
        crate::chrome::aux_windows::open_auxiliary(&win, TestAux::Beta);
        let old_id = win.borrow().aux.route_id(TestAux::Beta).unwrap();
        let Some(NativeAux::Controls(program)) = win.borrow().aux.get(TestAux::Beta).cloned()
        else {
            panic!("history dialog");
        };
        let old = program.dialog;
        crate::chrome::aux_windows::open_auxiliary(&win, TestAux::Beta);
        assert_eq!(win.borrow().aux.route_id(TestAux::Beta), Some(old_id));
        assert!(crate::chrome::aux_windows::close_aux(&win, old_id));
        crate::chrome::aux_windows::open_auxiliary(&win, TestAux::Beta);
        let new_id = win.borrow().aux.route_id(TestAux::Beta).unwrap();
        assert_ne!(old_id, new_id);
        old.destroy();
        assert!(!crate::chrome::aux_windows::close_aux(&win, old_id));
        assert_eq!(win.borrow().aux.route_id(TestAux::Beta), Some(new_id));
        let current = win.borrow().aux.get(TestAux::Beta).unwrap().native_window();
        current.destroy();
        assert!(!win.borrow().aux.contains(TestAux::Beta));
    }

    #[test]
    fn settings_identity_guards_pause_owner_and_native_finish() {
        use crate::chrome::aux_windows::NativeAux;
        use crate::testkit::owners;
        let Some((win, fake)) = bound_window_shared() else {
            eprintln!("SKIP: window widget test needs a display");
            return;
        };
        let action = win
            .borrow()
            .actions
            .get(command_id(&win, TestMenu::AuxAlpha))
            .unwrap();
        action.activate(None);
        let old_id = win.borrow().aux.route_id(TestAux::Alpha).unwrap();
        let Some(NativeAux::Controls(program)) = win.borrow().aux.get(TestAux::Alpha).cloned()
        else {
            panic!("settings dialog");
        };
        let old = program.dialog;
        crate::chrome::aux_windows::open_auxiliary(&win, TestAux::Alpha);
        assert_eq!(win.borrow().aux.route_id(TestAux::Alpha), Some(old_id));
        win.update_actions();
        assert!(!action.is_enabled());
        old.response(gtk::ResponseType::Cancel);
        assert!(action.is_enabled());
        assert!(!win.borrow().aux.contains(TestAux::Alpha));
        assert!(!fake.borrow().owns_pause(owners::GUARD));
        crate::chrome::aux_windows::open_auxiliary(&win, TestAux::Alpha);
        let new_id = win.borrow().aux.route_id(TestAux::Alpha).unwrap();
        assert_ne!(old_id, new_id);
        assert!(!crate::chrome::aux_windows::close_aux(&win, old_id));
        old.destroy();
        assert!(fake.borrow().owns_pause(owners::GUARD));
        assert!(!action.is_enabled());
        fake.borrow_mut().set_pause_owner(owners::MODAL, true);
        let current = win
            .borrow()
            .aux
            .get(TestAux::Alpha)
            .unwrap()
            .native_window();
        current.destroy();
        assert!(action.is_enabled());
        assert!(!win.borrow().aux.contains(TestAux::Alpha));
        assert!(!fake.borrow().owns_pause(owners::GUARD));
        assert!(fake.borrow().owns_pause(owners::MODAL));
    }

    /// Headless-capable state: session plus settings only, no renderer.
    type TestHarnessState = std::rc::Rc<std::cell::RefCell<crate::State<TestAux>>>;

    fn harness() -> Option<(gtk::Application, gtk::ApplicationWindow, TestHarnessState)> {
        if !crate::test_support::ui_test_available() {
            return None;
        }
        static NEXT_APP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let app_id = NEXT_APP.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let app = gtk::Application::builder()
            .application_id(format!("org.nerust.window-test.n{app_id}"))
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        // Menubar binding requires startup, just as in the real activate path.
        app.register(None::<&gio::Cancellable>)
            .expect("test application registers");
        let window = gtk::ApplicationWindow::builder().application(&app).build();
        Some((app, window, model_state()))
    }

    pub(crate) fn model_state() -> TestHarnessState {
        std::rc::Rc::new(std::cell::RefCell::new(crate::State::new_for_tests()))
    }

    pub(crate) fn model_state_shared() -> (
        TestHarnessState,
        std::rc::Rc<std::cell::RefCell<crate::testkit::FakeApp>>,
    ) {
        let (state, fake) = crate::State::new_for_tests_shared();
        (std::rc::Rc::new(std::cell::RefCell::new(state)), fake)
    }

    #[test]
    fn key_press_state_maps_to_pressed_flag() {
        assert!(key_event_pressed(KeyEventState::Press));
        assert!(!key_event_pressed(KeyEventState::Release));
    }

    #[test]
    fn command_declarations_generate_and_reuse_every_action() {
        use gio::prelude::MenuModelExt;
        use gtk::glib::variant::StaticVariantType;
        let state = model_state();
        let model = {
            let mut state = state.borrow_mut();
            state.host.menubar_update(GTK_MENU_CAPABILITIES).unwrap()
        };
        let mut visitor = GtkMenuVisitor::new(MenubarModels::new());
        model.visit(None, &mut visitor);
        assert_eq!(
            visitor.actions.0.borrow().len(),
            model.commands.definitions.len()
        );
        let actions: Vec<_> = model
            .commands
            .definitions
            .iter()
            .map(|descriptor| {
                // Names derive from the opaque id alone, never domain vocabulary.
                let expected = crate::chrome::menu::ActionRegistry::action_name(descriptor.id);
                let action = visitor.actions.get(descriptor.id).unwrap();
                assert_eq!(action.name().as_str(), expected);
                assert_eq!(action.is_enabled(), descriptor.enabled);
                assert_eq!(
                    action.parameter_type(),
                    descriptor
                        .spec
                        .argument()
                        .is_some()
                        .then(u64::static_variant_type)
                        .map(|variant| variant.into_owned())
                );
                (descriptor.id, action)
            })
            .collect();
        model.visit(Some(&model), &mut visitor);
        for (id, action) in actions {
            assert_eq!(visitor.actions.get(id).unwrap(), action);
        }
        let quit = model
            .commands
            .definitions
            .iter()
            .find(|descriptor| descriptor.downcast::<TestMenu>() == Some(&TestMenu::Quit))
            .expect("quit declaration")
            .id;
        let about = model
            .commands
            .definitions
            .iter()
            .find(|descriptor| descriptor.downcast::<TestMenu>() == Some(&TestMenu::About))
            .expect("about declaration")
            .id;
        let file = visitor.menubar.submenu(menu_id::GROUP_A);
        assert_eq!(
            file.item_attribute_value(file.n_items() - 1, "action", None)
                .unwrap()
                .str(),
            Some(
                format!(
                    "win.{}",
                    crate::chrome::menu::ActionRegistry::action_name(quit)
                )
                .as_str()
            )
        );
        assert_eq!(
            visitor
                .menubar
                .submenu(menu_id::GROUP_D)
                .item_attribute_value(0, "action", None)
                .unwrap()
                .str(),
            Some(
                format!(
                    "win.{}",
                    crate::chrome::menu::ActionRegistry::action_name(about)
                )
                .as_str()
            )
        );
    }

    #[test]
    fn generated_actions_send_typed_commands_to_dyn_sink_and_reject_bad_arguments() {
        use gtk::glib::variant::ToVariant;
        use std::{cell::RefCell, rc::Rc};
        use wakui_core::input::commands::CommandRef;
        use wakui_core::input::commands::CommandVisitor;
        use wakui_core::input::event::{EventCommand, EventSink};
        #[derive(Default)]
        struct RecordingSink(RefCell<Vec<EventCommand>>);
        impl EventSink for RecordingSink {
            fn send(&self, event: EventCommand) -> bool {
                self.0.borrow_mut().push(event);
                true
            }
        }
        let state = model_state();
        let model = {
            let mut state = state.borrow_mut();
            state.host.menubar_update(GTK_MENU_CAPABILITIES).unwrap()
        };
        let recording = Rc::new(RecordingSink::default());
        let mut visitor = GtkMenuVisitor::new(MenubarModels::new());
        visitor.sink = Some(recording.clone());
        for descriptor in &model.commands.definitions {
            let mut descriptor = descriptor.clone();
            descriptor.enabled = true;
            visitor.command(&descriptor);
        }
        let find = |command: TestMenu| {
            model
                .commands
                .definitions
                .iter()
                .find(|descriptor| descriptor.downcast::<TestMenu>() == Some(&command))
                .expect("declaration")
                .id
        };
        let reset = find(TestMenu::Session(TestSession::Restart));
        let load_item = find(TestMenu::Session(TestSession::Load(0)));
        let reset_ref = CommandRef {
            id: reset,
            argument: None,
        };
        let load_ref = |index: u64| CommandRef {
            id: load_item,
            argument: Some(index),
        };
        visitor.actions.get(reset).unwrap().activate(None);
        visitor
            .actions
            .get(load_item)
            .unwrap()
            .activate(Some(&42_u64.to_variant()));
        assert!(matches!(
            recording.0.borrow().as_slice(),
            [EventCommand::Command(a), EventCommand::Command(b)]
                if *a == reset_ref && *b == load_ref(42)
        ));
        let descriptor = model.commands.get(load_item).unwrap();
        let sink: Rc<dyn EventSink> = recording.clone();
        // Exercise the callback decoder directly: GAction itself also rejects
        // these before emitting activate, which would otherwise mask a bug here.
        crate::chrome::menu::send_action(sink.as_ref(), descriptor, None);
        crate::chrome::menu::send_action(sink.as_ref(), descriptor, Some(&"42".to_variant()));
        crate::chrome::menu::send_action(sink.as_ref(), descriptor, Some(&42_i32.to_variant()));
        crate::chrome::menu::send_action(
            sink.as_ref(),
            model.commands.get(reset).unwrap(),
            Some(&42_u64.to_variant()),
        );
        assert_eq!(recording.0.borrow().len(), 2);
        visitor
            .actions
            .get(load_item)
            .unwrap()
            .activate(Some(&0_u64.to_variant()));
        assert!(
            matches!(
                recording.0.borrow().last(),
                Some(EventCommand::Command(last)) if *last == load_ref(0)
            ),
            "explicit zero is valid, never a missing-argument fallback"
        );
    }

    #[test]
    fn native_roles_go_through_host_settings_guard_but_quit_remains_available() {
        use crate::testkit::owners;
        use wakui_core::chrome::aux::OpenRequest;
        use wakui_core::input::event::EventCommand;
        let Some((win, fake)) = bound_window_shared() else {
            eprintln!("SKIP: window widget test needs a display");
            return;
        };
        let about_name =
            crate::chrome::menu::ActionRegistry::action_name(command_id(&win, TestMenu::About));
        let quit_name =
            crate::chrome::menu::ActionRegistry::action_name(command_id(&win, TestMenu::Quit));
        assert!(win.application().lookup_action(&about_name).is_none());
        assert!(win.application().lookup_action(&quit_name).is_none());
        let about = win
            .borrow()
            .actions
            .get(command_id(&win, TestMenu::About))
            .unwrap();
        let quit = win
            .borrow()
            .actions
            .get(command_id(&win, TestMenu::Quit))
            .unwrap();
        assert_eq!(
            win.window().lookup_action(&about_name).unwrap(),
            about.clone().upcast::<gio::Action>()
        );
        assert_eq!(
            win.window().lookup_action(&quit_name).unwrap(),
            quit.clone().upcast::<gio::Action>()
        );
        let sink = win.event_sink();
        let about_event = EventCommand::Command(wakui_core::input::commands::CommandRef {
            id: command_id(&win, TestMenu::About),
            argument: None,
        });
        let OpenRequest::Create(id) = win
            .borrow_mut()
            .aux
            .request_open(TestAux::Alpha, TestAux::Alpha.title())
        else {
            panic!("settings reservation");
        };
        // Bypass native enablement to emulate an already queued activation.
        assert!(sink.send(about_event));
        assert!(win.borrow().about_dialog.is_none());
        assert!(!about.is_enabled());
        assert!(quit.is_enabled());
        assert!(crate::chrome::aux_windows::close_aux(&win, id));
        fake.borrow_mut().set_pause_owner(owners::GUARD, true);
        assert!(sink.send(about_event));
        assert!(
            win.borrow().about_dialog.is_none(),
            "host pause guard also applies without aux"
        );
        fake.borrow_mut().set_pause_owner(owners::GUARD, false);
        win.update_actions();
        about.activate(None);
        let dialog = win
            .borrow()
            .about_dialog
            .clone()
            .expect("Host About effect");
        assert_eq!(dialog.transient_for(), Some(win.window().upcast()));
        about.activate(None);
        assert_eq!(win.borrow().about_dialog.as_ref(), Some(&dialog));
        dialog.close();
        assert!(win.borrow().about_dialog.is_none());
        let OpenRequest::Create(_) = win
            .borrow_mut()
            .aux
            .request_open(TestAux::Alpha, TestAux::Alpha.title())
        else {
            panic!("settings reservation");
        };
        win.update_actions();
        quit.activate(None);
        assert!(
            win.state().borrow().host.close_exited(),
            "Quit crosses Host guard and closes"
        );
    }

    #[test]
    fn about_map_effect_is_borrow_free_and_can_reenter_event_sink() {
        use gtk::prelude::{ListModelExt, WidgetExt};
        use std::{cell::Cell, rc::Rc};
        let Some(win) = bound_window() else {
            eprintln!("SKIP: window widget test needs a display");
            return;
        };
        let mapped = Rc::new(Cell::new(0));
        let toplevels = gtk::Window::toplevels();
        let signal = toplevels.connect_items_changed({
            let weak = Rc::downgrade(&win);
            let mapped = mapped.clone();
            move |list, position, _, added| {
                for i in position..position + added {
                    let Some(dialog) = list.item(i).and_downcast::<gtk::AboutDialog>() else {
                        continue;
                    };
                    dialog.connect_map({
                        let weak = weak.clone();
                        let mapped = mapped.clone();
                        move |_| {
                            let win = weak.upgrade().unwrap();
                            assert!(win.try_borrow_mut().is_ok());
                            assert!(win.state().try_borrow_mut().is_ok());
                            assert!(win.borrow().actions.0.try_borrow_mut().is_ok());
                            mapped.set(mapped.get() + 1);
                            win.event_sink()
                                .send(wakui_core::input::event::EventCommand::LiveTick);
                        }
                    });
                }
            }
        });
        let about = win
            .borrow()
            .actions
            .get(command_id(&win, TestMenu::About))
            .unwrap();
        about.activate(None);
        assert_eq!(mapped.get(), 1);
        let dialog = win.borrow().about_dialog.clone().unwrap();
        dialog.close();
        toplevels.disconnect(signal);
    }

    #[test]
    fn event_sink_and_action_callbacks_do_not_keep_window_alive() {
        use std::rc::Rc;
        use wakui_core::input::event::EventCommand;
        use wakui_core::input::session::SessionEvent;
        let Some(win) = bound_window() else {
            eprintln!("SKIP: window widget test needs a display");
            return;
        };
        let weak = Rc::downgrade(&win);
        let sink = win.event_sink();
        let action = win
            .borrow()
            .actions
            .get(command_id(&win, TestMenu::About))
            .unwrap();
        let native = win.window();
        drop(win);
        assert!(weak.upgrade().is_none());
        assert!(!sink.send(EventCommand::Session(SessionEvent::ClearInput)));
        action.activate(None);
        native.destroy();
    }

    #[test]
    fn keys_and_shortcuts_use_the_same_dyn_event_boundary() {
        use std::{cell::RefCell, rc::Rc};
        use wakui_core::input::event::{EventCommand, EventSink};
        use wakui_core::input::session::SessionEvent;
        struct RecordingSink(RefCell<Vec<EventCommand>>);
        impl EventSink for RecordingSink {
            fn send(&self, event: EventCommand) -> bool {
                self.0.borrow_mut().push(event);
                true
            }
        }
        let Some(win) = bound_window() else {
            eprintln!("SKIP: window widget test needs a display");
            return;
        };
        let sink = Rc::new(RecordingSink(RefCell::new(Vec::new())));
        win.borrow_mut().sink = sink.clone();
        win.key_event(gdk::Key::F4, KeyEventState::Press);
        win.key_event(gdk::Key::F4, KeyEventState::Release);
        assert!(matches!(
            sink.0.borrow().as_slice(),
            [
                EventCommand::Session(SessionEvent::Keyboard { pressed: true, .. }),
                EventCommand::Session(SessionEvent::Keyboard { pressed: false, .. }),
            ]
        ));
        win.window().destroy();
    }

    #[test]
    fn slot_menu_explains_empty_state() {
        use crate::testkit::FakeCommands;
        use gio::prelude::MenuModelExt;
        let bars = MenubarModels::new();
        let commands = FakeCommands::new();
        let model = crate::testkit::canned_menubar(&commands, &[], None, false, true);
        let mut visitor = GtkMenuVisitor::new(bars.clone());
        model.visit(None, &mut visitor);
        for descriptor in &model.commands.definitions {
            if descriptor.spec.argument().is_some() {
                assert!(!visitor.actions.get(descriptor.id).unwrap().is_enabled());
            }
        }
        // One disabled item names the empty state, never a blank list.
        for id in [
            menu_id::SUB_A,
            menu_id::SUB_B,
            menu_id::SUB_C,
            menu_id::SUB_D,
        ] {
            let menu = bars.submenu(id);
            assert_eq!(menu.n_items(), 1);
            assert!(menu.item_attribute_value(0, "action", None).is_none());
            assert!(menu.item_attribute_value(0, "target", None).is_none());
            assert_eq!(
                menu.item_attribute_value(0, "label", None).unwrap().str(),
                Some("No items yet")
            );
        }
    }

    #[test]
    fn slot_menu_lists_slots_with_action_targets() {
        use crate::testkit::FakeCommands;
        use gio::prelude::MenuModelExt;
        let bars = MenubarModels::new();
        let commands = FakeCommands::new();
        let mut model = crate::testkit::canned_menubar(&commands, &[1, 2], Some(2), false, true);
        fn disable_items(menu: &mut wakui_core::chrome::menu::Submenu) {
            use wakui_core::chrome::menu::MenuEntry;
            for entry in &mut menu.children {
                match entry {
                    MenuEntry::Item(item) => item.enabled = false,
                    MenuEntry::Submenu(child) => disable_items(child),
                }
            }
        }
        for menu in &mut model.menus {
            disable_items(menu);
        }
        let mut visitor = GtkMenuVisitor::new(bars.clone());
        model.visit(None, &mut visitor);
        let load_item = model
            .commands
            .definitions
            .iter()
            .find(|descriptor| {
                descriptor.downcast::<TestMenu>() == Some(&TestMenu::Session(TestSession::Load(0)))
            })
            .expect("load item declaration")
            .id;
        assert!(
            visitor.actions.get(load_item).unwrap().is_enabled(),
            "item entries cannot overwrite shared declaration enablement"
        );
        let menu = bars.submenu(menu_id::SUB_C);
        assert_eq!(menu.n_items(), 2);
        for (index, id) in [(0, 1_u64), (1, 2_u64)] {
            assert_eq!(
                menu.item_attribute_value(index, "target", None)
                    .unwrap()
                    .get::<u64>(),
                Some(id)
            );
            assert_eq!(
                menu.item_attribute_value(index, "action", None)
                    .unwrap()
                    .str(),
                Some(
                    format!(
                        "win.{}",
                        crate::chrome::menu::ActionRegistry::action_name(load_item)
                    )
                    .as_str()
                )
            );
        }
    }
}
