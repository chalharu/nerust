use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::Path,
    rc::{Rc, Weak},
};

use super::{
    State, TITLE_UPDATE_INTERVAL,
    surface::{Surface, SurfaceExtend},
    surface_state::surface_gone,
};
use crate::widgets::controls::{Callbacks, WidgetBinding, WidgetDialog};
use gtk::{
    gio, glib,
    glib::variant::{StaticVariantType, ToVariant},
    prelude::*,
};
use nerust_frontend_host::{
    aux::{AuxId, AuxWindows, OpenRequest},
    commands::{CommandDescriptor, CommandId, CommandParameter, CommandVisitor},
    event::{CommandEffect, EventCommand, EventHandler, EventSink},
    focus::{AuxKind, FocusEvent},
    menu::{MenuCapabilities, MenuId, MenuItem, MenuVisitor, Menubar, Submenu},
    session::SessionEvent,
    widget_session::WidgetSession,
    widgets::MenuPresenter,
    window::{MainContents, Window as HostWindow, WindowEffects, WindowVisitor},
};

/// Bound once via `set_menubar`; host IDs retain the same native submenus
/// across refills because replacing the root does not update the visible bar.
#[derive(Clone)]
pub(crate) struct MenubarModels {
    pub(crate) menu_model: gio::Menu,
    models: Rc<RefCell<HashMap<MenuId, gio::Menu>>>,
    attached: Rc<Cell<bool>>,
}

impl MenubarModels {
    pub(crate) fn new() -> Self {
        Self {
            menu_model: gio::Menu::new(),
            models: Rc::default(),
            attached: Rc::default(),
        }
    }

    pub(crate) fn submenu(&self, id: MenuId) -> gio::Menu {
        let existing = self.models.borrow().get(&id).cloned();
        if let Some(menu) = existing {
            return menu;
        }
        // No cache borrow across native construction, even on the first visit.
        let menu = gio::Menu::new();
        self.models.borrow_mut().entry(id).or_insert(menu).clone()
    }
}

pub(crate) struct WindowCore {
    application: gtk::Application,
    window: gtk::ApplicationWindow,
    state: Rc<RefCell<State>>,
    open_dialog: Option<gtk::FileChooserNative>,
    about_dialog: Option<gtk::AboutDialog>,
    about_opening: bool,
    aux: AuxWindows<u64, NativeAux>,
    actions: ActionRegistry,
    sink: Rc<dyn EventSink>,
    menubar: MenubarModels,
    menu_presenter: Rc<MenuPresenter>,
    main_delivery: Rc<MainWindowDelivery>,
}

/// A full window snapshot is latest-wins, including title-notify reentry before
/// visit_menu. This is a delivery mailbox, not another host window registry.
#[derive(Default)]
struct MainWindowDelivery {
    active: Cell<bool>,
    pending: RefCell<Option<HostWindow>>,
}

struct MainDeliveryGuard<'a>(&'a Cell<bool>);

impl Drop for MainDeliveryGuard<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

type MainSurface = (gtk::Picture, Rc<RefCell<State>>);

pub(crate) struct MainWindowRenderer {
    menu: GtkMenuVisitor,
    presenter: Rc<MenuPresenter>,
    surface: Option<MainSurface>,
}

/// One outer traversal for main chrome and grouped native widgets. Builders
/// receive owned controllers; updates reuse retained controls without policy.
pub(crate) enum GtkWindowRenderer<'a> {
    Main(MainWindowRenderer),
    Build(Box<DialogRenderer<'a>>),
    Controls(&'a WidgetBinding),
}

pub(crate) struct DialogRenderer<'a> {
    parent: &'a gtk::ApplicationWindow,
    session: Option<WidgetSession>,
    callbacks: Option<Callbacks>,
    model: Option<HostWindow>,
    result: Option<RenderedDialog>,
}

pub(crate) enum RenderedDialog {
    Controls(WidgetDialog),
}

pub(crate) fn render_dialog(
    model: &HostWindow,
    parent: &gtk::ApplicationWindow,
    session: Option<WidgetSession>,
    callbacks: Callbacks,
) -> Option<RenderedDialog> {
    let mut renderer = GtkWindowRenderer::Build(Box::new(DialogRenderer {
        parent,
        session,
        callbacks: Some(callbacks),
        model: None,
        result: None,
    }));
    model.visit(&mut renderer);
    let GtkWindowRenderer::Build(builder) = renderer else {
        unreachable!()
    };
    builder.result
}

impl WindowVisitor for GtkWindowRenderer<'_> {
    fn begin_window(&mut self, model: &HostWindow) {
        let native: Option<gtk::Window> = match self {
            Self::Main(main) => main.menu.window.clone().map(Cast::upcast),
            Self::Build(builder) => {
                builder.model = Some(model.clone());
                None
            }
            Self::Controls(binding) => binding.dialog().map(Cast::upcast),
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

    fn visit_controls(&mut self, screen: &wakui::control::ControlScreen) {
        match self {
            Self::Controls(binding) => binding.render(screen),
            Self::Build(builder) => {
                let Some(session) = builder.session.take() else {
                    return;
                };
                builder.result = Some(RenderedDialog::Controls(crate::widgets::controls::build(
                    builder.model.as_ref().expect("window chrome"),
                    builder.parent,
                    session,
                    builder.callbacks.take().expect("one control program"),
                )));
            }
            _ => (),
        }
    }
}

fn present_main_model(window: &Window, model: HostWindow, surface: Option<MainSurface>) {
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

/// Native resources only. Opening, presence, identity and routing belong to host.
#[derive(Clone)]
enum NativeAux {
    Debug(crate::debugger::DebuggerWindow),
    Controls(WidgetDialog),
}

impl NativeAux {
    fn is_gone(&self) -> bool {
        match self {
            Self::Debug(debugger) => debugger.borrow().is_gone(),
            Self::Controls(program) => surface_gone(&program.dialog),
        }
    }

    fn present(&self) {
        match self {
            Self::Debug(debugger) => {
                crate::debugger::DebuggerWindowCore::refresh(debugger);
                debugger.borrow().present();
            }
            Self::Controls(program) => {
                program.dialog.present();
                program.binding.sync();
            }
        }
    }

    fn native_window(&self) -> gtk::Window {
        match self {
            Self::Debug(debugger) => debugger.borrow().native_window().upcast(),
            Self::Controls(program) => program.dialog.clone().upcast(),
        }
    }
}

/// Native liveness is an input; the shared registry decides create/present/pending.
fn request_aux(window: &Window, kind: AuxKind) -> Option<AuxId> {
    let existing = {
        let window = window.borrow();
        window.aux.route_id(kind).zip(window.aux.get(kind).cloned())
    };
    if let Some((id, handle)) = existing
        && handle.is_gone()
    {
        close_aux(window, id);
    }
    let request = {
        let mut core = window.borrow_mut();
        let state = core.state.clone();
        let request = state.borrow_mut().host.reserve_window(&mut core.aux, kind);
        match request {
            Ok(Some(request)) => request,
            Ok(None) => return None,
            Err(error) => {
                log::warn!("GTK auxiliary reservation failed: {error}");
                return None;
            }
        }
    };
    match request {
        OpenRequest::Create(id) => Some(id),
        OpenRequest::Present(_) => {
            let existing = window.borrow().aux.get(kind).cloned();
            if let Some(existing) = existing {
                existing.present();
            }
            None
        }
        OpenRequest::Pending(_) => None,
    }
}

fn complete_aux(window: &Window, id: AuxId, handle: NativeAux) -> bool {
    let native = handle.native_window();
    let controls = matches!(&handle, NativeAux::Controls(_));
    let result = window.borrow_mut().aux.complete_open(id, None, handle);
    if let Err(rejected) = result {
        rejected.native_window().destroy();
        close_aux(window, id);
        return false;
    }
    let weak = Rc::downgrade(window);
    native.connect_destroy(move |_| {
        if let Some(window) = weak.upgrade() {
            close_aux(&window, id);
        }
    });
    if controls {
        let weak = Rc::downgrade(window);
        native.connect_unrealize(move |_| {
            if let Some(window) = weak.upgrade() {
                close_aux(&window, id);
            }
        });
    }
    true
}

/// Remove before any native drop or session callback: both can synchronously reenter.
fn close_aux(window: &Window, id: AuxId) -> bool {
    let closed = {
        let mut core = window.borrow_mut();
        let state = core.state.clone();
        state
            .borrow_mut()
            .host
            .close_widget_window(&mut core.aux, id, None)
    };
    let Some((handle, result)) = closed else {
        return false;
    };
    if let Err(error) = result {
        log::warn!("GTK auxiliary close failed: {error}");
    }
    drop(handle);
    true
}

fn open_auxiliary(window: &Window, kind: AuxKind) {
    let Some(id) = request_aux(window, kind) else {
        return;
    };
    window.update_actions();
    let state = window.state();
    // Host prepares the only draft and publishes its Window into the reserved
    // route. Validation/rollback and SettingsOpened/Closed are not native policy.
    let session = if matches!(kind, AuxKind::Settings | AuxKind::History) {
        let prepared = {
            let mut core = window.borrow_mut();
            let mut state = state.borrow_mut();
            state.host.prepare_widget_window(&mut core.aux, id)
        };
        match prepared {
            Ok(Some(session)) => Some(session),
            Ok(None) => return,
            Err(error) => {
                state.borrow_mut().host.report_host_error(error);
                window.update_actions();
                return;
            }
        }
    } else {
        None
    };
    let model = window.borrow().aux.window(id).cloned();
    let Some(model) = model else {
        close_aux(window, id);
        return;
    };
    let built = render_dialog(
        &model,
        &window.window(),
        session,
        widget_callbacks(window, id),
    );
    let handle = match built {
        Some(RenderedDialog::Controls(built)) => NativeAux::Controls(built),
        None => {
            close_aux(window, id);
            window.update_actions();
            return;
        }
    };
    let native = handle.native_window();
    if complete_aux(window, id, handle) {
        native.present();
    }
}

fn widget_callbacks(window: &Window, id: AuxId) -> Callbacks {
    let current = Rc::downgrade(window);
    let refresh = current.clone();
    let publish = current.clone();
    let submit = current.clone();
    let close = current.clone();
    Callbacks {
        current: Box::new(move || {
            current.upgrade().is_some_and(|window| {
                let core = window.borrow();
                core.aux.kind_of_id(id).is_some() && !core.state.borrow().host.close_exited()
            })
        }),
        refresh: Box::new(move |session| {
            if let Some(window) = refresh.upgrade() {
                let state = window.state();
                state.borrow_mut().host.refresh_widget_session(session);
            }
        }),
        publish: Box::new(move |model| {
            if let Some(window) = publish.upgrade()
                && let Some(slot) = window.borrow_mut().aux.window_mut(id)
            {
                *slot = model;
            }
        }),
        submit: Box::new(move |submission| {
            let Some(window) = submit.upgrade() else {
                return false;
            };
            let result = {
                let core = window.borrow();
                core.state
                    .borrow_mut()
                    .host
                    .apply_window_submission(&core.aux, id, submission)
            };
            match result {
                Ok(effects) => {
                    apply_window_effects(&window, effects);
                    true
                }
                // Host publishes the error through the same opaque mailbox.
                Err(error) => {
                    log::warn!("GTK widget apply failed: {error}");
                    false
                }
            }
        }),
        close: Box::new(move || {
            if let Some(window) = close.upgrade()
                && close_aux(&window, id)
            {
                window.update_actions();
            }
        }),
    }
}

fn apply_window_effects(window: &Window, effects: WindowEffects) {
    if effects.rebuild_renderer {
        window.state().borrow_mut().renderer_reload_pending = true;
    }
    let native = window.window();
    if let Some(fullscreen) = effects.fullscreen {
        native.set_fullscreened(fullscreen);
    }
    if let Some((width, height)) = effects.resize {
        native.set_default_size(width as i32, height as i32);
        native.queue_resize();
    }
    if effects.redraw {
        native.queue_draw();
    }
    if effects.refresh_chrome {
        window.update_actions();
    }
}

/// Shared identity cache only; no borrow may survive a native call.
#[derive(Clone, Default)]
pub(crate) struct ActionRegistry(Rc<RefCell<HashMap<CommandId, gio::SimpleAction>>>);

impl ActionRegistry {
    fn get(&self, id: CommandId) -> Option<gio::SimpleAction> {
        self.0.borrow().get(&id).cloned()
    }
}

pub(crate) type Window = Rc<RefCell<WindowCore>>;

struct WindowEventSink(Weak<RefCell<WindowCore>>);

impl EventSink for WindowEventSink {
    fn send(&self, event: EventCommand) -> bool {
        let Some(window) = self.0.upgrade() else {
            return false;
        };
        let (state, settings_open) = {
            let core = window.borrow();
            (core.state.clone(), core.aux.contains(AuxKind::Settings))
        };
        let outcome = {
            let mut state = state.borrow_mut();
            let handler: &mut dyn EventHandler = &mut state.host;
            handler.handle_event(event, settings_open)
        };
        // Native constructors, setters and effects may synchronously reenter.
        match outcome {
            Ok(outcome) => {
                if let Some(effect) = outcome.effect {
                    apply_command_effect(&window, effect);
                }
                if matches!(event, EventCommand::Menu(_))
                    || outcome.session.command.is_some()
                    || outcome.session.settings_changed
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

fn apply_command_effect(window: &Window, effect: CommandEffect) {
    match effect {
        CommandEffect::OpenRom => window.open(),
        CommandEffect::OpenAux(AuxKind::Settings) => open_auxiliary(window, AuxKind::Settings),
        CommandEffect::OpenAux(AuxKind::Debug) => window.open_debugger(),
        CommandEffect::OpenAux(AuxKind::History) => window.open_history(),
        // PPU windows are owned by the native debugger, not main chrome.
        CommandEffect::OpenAux(AuxKind::Ppu) => (),
        CommandEffect::About => open_about(window),
        CommandEffect::Quit => window.window().close(),
        CommandEffect::ToggleFullscreen => toggle_window_fullscreen(window),
        CommandEffect::LiveTick => (),
    }
}

fn open_about(window: &Window) {
    let existing = window.borrow().about_dialog.clone();
    if let Some(dialog) = existing {
        dialog.present();
        return;
    }
    {
        let mut core = window.borrow_mut();
        if core.about_opening {
            return;
        }
        core.about_opening = true;
    }
    let dialog: gtk::AboutDialog =
        gtk::Builder::from_string(include_str!("../resources/about.xml"))
            .object("about")
            .unwrap();
    dialog.set_transient_for(Some(&window.window()));
    let weak = Rc::downgrade(window);
    dialog.connect_close_request(move |dialog| {
        if let Some(window) = weak.upgrade() {
            let old = {
                let mut core = window.borrow_mut();
                if core.about_dialog.as_ref() == Some(dialog) {
                    core.about_dialog.take()
                } else {
                    None
                }
            };
            drop(old);
        }
        glib::Propagation::Proceed
    });
    {
        let mut core = window.borrow_mut();
        core.about_dialog = Some(dialog.clone());
        core.about_opening = false;
    }
    dialog.present();
}

pub(crate) trait WindowExtend {
    fn bind(
        application: gtk::Application,
        window: gtk::ApplicationWindow,
        picture: gtk::Picture,
        state: Rc<RefCell<State>>,
        menubar: MenubarModels,
    ) -> Window;
    fn window(&self) -> gtk::ApplicationWindow;
    fn application(&self) -> gtk::Application;
    fn state(&self) -> Rc<RefCell<State>>;
    fn open_debugger(&self);
    fn open_history(&self);
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

impl WindowExtend for Window {
    fn state(&self) -> Rc<RefCell<State>> {
        self.borrow().state.clone()
    }

    fn bind(
        application: gtk::Application,
        window: gtk::ApplicationWindow,
        picture: gtk::Picture,
        state: Rc<RefCell<State>>,
        menubar: MenubarModels,
    ) -> Window {
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

        {
            let weak = Rc::downgrade(&result);
            let _ = glib::timeout_add_local(TITLE_UPDATE_INTERVAL, move || {
                let Some(result) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                let due = result
                    .state()
                    .borrow_mut()
                    .title_refresh_due(std::time::Instant::now());
                if due {
                    result.event_sink().send(EventCommand::LiveTick);
                }
                glib::ControlFlow::Continue
            });
        }

        result.rebuild_menubar();
        window.present();
        result.sync_fullscreen_from_settings();
        let _ = window.grab_focus();
        result
    }

    fn open_history(&self) {
        open_auxiliary(self, AuxKind::History);
    }

    fn open_debugger(&self) {
        let Some(id) = request_aux(self, AuxKind::Debug) else {
            return;
        };
        // The reservation owns the blueprint before any native callback runs.
        let model = self.borrow().aux.window(id).cloned();
        let debugger = model.and_then(|model| {
            crate::debugger::DebuggerWindowCore::from_window(
                &model,
                &self.application(),
                &self.window(),
                self.state(),
            )
        });
        if let Some(debugger) = debugger {
            complete_aux(self, id, NativeAux::Debug(debugger));
        } else {
            self.borrow_mut().aux.cancel_open(id);
        }
        self.update_actions();
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
        let filters = self.state().borrow().host.rom_dialog_filters();
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
            show_rom_load_error(&self.window(), self.state(), &detail);
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
                .session_mut()
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
        let settings_open = self.borrow().aux.contains(AuxKind::Settings);
        let model = {
            let mut state = state.borrow_mut();
            let ctx = state.host.frame_ctx();
            let menu_changed = state
                .host
                .menubar_update(&ctx, settings_open, GTK_MENU_CAPABILITIES)
                .is_some();
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
        // No hardcoded F-keys: the debugger open rides the shortcut schema
        // (default F4, rebindable in Settings), so a user binding always wins.
        if let Some(controller_input) = crate::keyboard::key_from_gdk(key) {
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

pub(crate) const GTK_MENU_CAPABILITIES: MenuCapabilities = MenuCapabilities {
    about: true,
    dialog_ellipsis: false,
    application_menu: false,
};

/// A stack over persistent native objects, keyed by host semantic identity.
pub(crate) struct GtkMenuVisitor {
    menubar: MenubarModels,
    window: Option<gtk::ApplicationWindow>,
    application: Option<gtk::Application>,
    actions: ActionRegistry,
    sink: Option<Rc<dyn EventSink>>,
    stack: Vec<(gio::Menu, bool)>,
}

impl GtkMenuVisitor {
    #[cfg(test)]
    pub(crate) fn new(menubar: MenubarModels) -> Self {
        Self {
            menubar,
            window: None,
            application: None,
            actions: ActionRegistry::default(),
            sink: None,
            stack: Vec::new(),
        }
    }
}

/// Decode strictly before binding: a bad or missing slot is never slot zero.
fn send_action(
    sink: &dyn EventSink,
    descriptor: &CommandDescriptor,
    value: Option<&glib::Variant>,
) {
    let argument = match (descriptor.parameter, value) {
        (CommandParameter::None, None) => None,
        (CommandParameter::SlotId, Some(value)) => {
            let Some(id) = value.get::<u64>() else { return };
            Some(id)
        }
        _ => return,
    };
    if let Some(command) = descriptor.bind(argument) {
        sink.send(EventCommand::Menu(command));
    }
}

impl CommandVisitor for GtkMenuVisitor {
    fn command(&mut self, descriptor: &CommandDescriptor) {
        let action = if let Some(action) = self.actions.get(descriptor.id) {
            action
        } else {
            let parameter_type = match descriptor.parameter {
                CommandParameter::None => None,
                CommandParameter::SlotId => Some(u64::static_variant_type()),
            };
            let action = gio::SimpleAction::new(descriptor.id.name(), parameter_type.as_deref());
            if let Some(sink) = self.sink.clone() {
                let descriptor = *descriptor;
                action.connect_activate(move |_, parameter| {
                    send_action(sink.as_ref(), &descriptor, parameter);
                });
            }
            self.actions
                .0
                .borrow_mut()
                .insert(descriptor.id, action.clone());
            if let Some(window) = &self.window {
                window.add_action(&action);
            }
            action
        };
        // Once per declaration, never per slot entry. No registry borrow here.
        action.set_enabled(descriptor.enabled);
    }
}

impl MenuVisitor for GtkMenuVisitor {
    fn begin_menubar(&mut self, _menubar: &Menubar, rebuild: bool) {
        if rebuild {
            self.menubar.menu_model.remove_all();
        }
        self.stack.push((self.menubar.menu_model.clone(), rebuild));
    }

    fn begin_menu(&mut self, menu: &Submenu, rebuild: bool) {
        let native = self.menubar.submenu(menu.id);
        let (parent, parent_rebuild) = self.stack.last().expect("host menu parent");
        if *parent_rebuild {
            parent.append_submenu(Some(&menu.label), &native);
        }
        if rebuild {
            native.remove_all();
        }
        self.stack.push((native, rebuild));
    }

    fn item(&mut self, item: &MenuItem) {
        let action = item.command.map(|command| command.binding());
        let (parent, rebuild) = self.stack.last().expect("host item parent");
        if *rebuild {
            let native = gio::MenuItem::new(Some(&item.label), None);
            if let Some((id, target)) = action {
                native.set_action_and_target_value(
                    Some(&format!("win.{}", id.name())),
                    target.map(|id| id.to_variant()).as_ref(),
                );
            }
            parent.append_item(&native);
        }
    }

    fn end_menu(&mut self, _id: MenuId) {
        self.stack.pop();
    }
    fn end_menubar(&mut self) {
        self.stack.pop();
        debug_assert!(self.stack.is_empty());
        if let (Some(application), Some(window)) = (&self.application, &self.window)
            && !self.menubar.attached.replace(true)
        {
            application.set_menubar(Some(&self.menubar.menu_model));
            window.set_show_menubar(true);
        }
    }
}

fn set_window_fullscreen(window: &gtk::ApplicationWindow, fullscreen: bool) {
    window.set_fullscreened(fullscreen);
}

fn persist_window_fullscreen_default(window: &Window, fullscreen: bool) -> Result<bool, String> {
    let state = window.state();
    let mut state = state.borrow_mut();
    let plan = state
        .host
        .session_mut()
        .set_fullscreen_default(fullscreen)
        .map_err(|error| error.to_string())?;
    if plan.renderer_rebuild_required || plan.window_settings_changed {
        state.renderer_reload_pending = true;
    }
    Ok(plan.fullscreen_default_changed)
}

fn sync_persisted_window_fullscreen(window: &Window, fullscreen: bool) {
    if !window.state().borrow_mut().observe_fullscreen(fullscreen) {
        return;
    }
    match persist_window_fullscreen_default(window, fullscreen) {
        Ok(true) => window.update_actions(),
        Ok(false) => (),
        Err(error) => log::warn!("failed to persist fullscreen setting: {error}"),
    }
}

fn apply_persisted_window_fullscreen(window: &Window, fullscreen: bool) {
    match persist_window_fullscreen_default(window, fullscreen) {
        Ok(_) => window.sync_fullscreen_from_settings(),
        Err(error) => {
            log::warn!("failed to persist fullscreen setting: {error}");
            set_window_fullscreen(&window.window(), fullscreen);
        }
    }
}

fn toggle_window_fullscreen(window: &Window) {
    let fullscreen = !window.window().is_fullscreen();
    apply_persisted_window_fullscreen(window, fullscreen);
}

/// Use the single modal queue, including its pause ownership and acknowledgement path.
fn show_rom_load_error(_parent: &gtk::ApplicationWindow, state: Rc<RefCell<State>>, detail: &str) {
    state.borrow_mut().host.report_host_error(detail.to_owned());
}

#[cfg(test)]
pub(crate) mod tests {
    pub(crate) fn program(window: &super::Window, kind: super::AuxKind) -> super::WidgetDialog {
        super::open_auxiliary(window, kind);
        let Some(super::NativeAux::Controls(program)) = window.borrow().aux.get(kind).cloned()
        else {
            panic!("generic widget program");
        };
        program
    }

    pub(crate) fn route(window: &super::Window, kind: super::AuxKind) -> Option<super::AuxId> {
        window.borrow().aux.route_id(kind)
    }

    pub(crate) fn inert_callbacks() -> super::Callbacks {
        super::Callbacks {
            current: Box::new(|| true),
            refresh: Box::new(|_| {}),
            publish: Box::new(|_| {}),
            submit: Box::new(|_| true),
            close: Box::new(|| {}),
        }
    }
    use gio::prelude::ActionExt as _;
    use gtk::prelude::{
        ActionMapExt as _, ApplicationExt as _, Cast as _, CastNone as _, DialogExt as _,
        GtkApplicationExt as _, GtkWindowExt as _, ObjectExt as _,
    };

    use super::{
        CommandId, GTK_MENU_CAPABILITIES, GtkMenuVisitor, KeyEventState, MenuId, MenubarModels,
        WindowExtend, key_event_pressed,
    };
    use nerust_frontend_host::{application::MenuSig, menu::MenuModel};

    use crate::surface_state::surface_gone;

    #[test]
    fn submenu_registry_is_lazy_and_shared_across_clones() {
        let bars = MenubarModels::new();
        let shared = bars.clone();
        assert!(bars.models.borrow().is_empty());
        let file = bars.submenu(MenuId::File);
        assert_eq!(bars.models.borrow().len(), 1);
        assert_eq!(shared.submenu(MenuId::File), file);
        let states = shared.submenu(MenuId::States);
        assert_ne!(states, file);
        assert_eq!(bars.submenu(MenuId::States), states);
        assert_eq!(bars.models.borrow().len(), 2);
    }

    /// Externally destroyed dialogs keep their slot (no signal fires):
    /// reopening must rebuild instead of panicking on the slot borrow.
    /// Widget-only: needs a display, run under Xvfb (skips headless).
    #[test]
    fn open_history_rebuilds_dead_dialog_without_borrow_panic() {
        let Some(win) = bound_window() else {
            eprintln!("SKIP: window widget test needs a display (run under Xvfb)");
            return;
        };
        // Bypass the slot wiring: a directly built dialog carries no
        // destroy hook, like an externally destroyed one whose
        // close-request never ran.
        win.open_history();
        let built = win
            .borrow()
            .aux
            .get(super::AuxKind::History)
            .cloned()
            .unwrap();
        // External destroy disposes the widget without signals; the
        // slot keeps the dead dialog.
        let dead = built.native_window();
        dead.destroy();
        assert!(surface_gone(&dead));
        // Must rebuild, not panic on the slot borrow.
        win.open_history();
        let slot = win.borrow().aux.get(super::AuxKind::History).cloned();
        let Some(super::NativeAux::Controls(program)) = slot else {
            panic!("rebuilt dialog");
        };
        let dialog = program.dialog;
        assert!(!surface_gone(&dialog));
        dialog.destroy();
    }

    pub(crate) fn bound_window() -> Option<super::Window> {
        let (app, window, state) = harness()?;
        Some(super::Window::bind(
            app,
            window,
            gtk::Picture::new(),
            state,
            super::MenubarModels::new(),
        ))
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
        use super::{AuxKind, HostWindow, NativeAux, OpenRequest};
        use gio::prelude::MenuModelExt;
        use gtk::prelude::WidgetExt;
        use std::{cell::Cell, rc::Rc};

        let win = bound_window().expect("run in an isolated Xvfb process");
        let mut model = win.state().borrow().host.main_window().clone();
        assert!(matches!(
            model.contents.contents,
            nerust_frontend_host::window::WidgetContents::Main(_)
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
                let id = win.borrow().aux.route_id(AuxKind::Debug).unwrap();
                let model = win.borrow().aux.window(id).cloned().unwrap();
                assert!(matches!(
                    model.contents.contents,
                    nerust_frontend_host::window::WidgetContents::Debug(_)
                ));
                assert_eq!(
                    win.borrow_mut().aux.request_open(AuxKind::Debug),
                    OpenRequest::Pending(id)
                );
                win.open_debugger();
                added.set(added.get() + 1);
                let weak = Rc::downgrade(&win);
                let mapped = mapped.clone();
                native.connect_map(move |native| {
                    let win = weak.upgrade().unwrap();
                    assert!(win.try_borrow_mut().is_ok());
                    assert!(win.state().try_borrow_mut().is_ok());
                    assert_eq!(native.title().as_deref(), Some(model.title.as_str()));
                    assert!(win.borrow().aux.get(AuxKind::Debug).is_none());
                    mapped.set(mapped.get() + 1);
                });
            }
        });
        // Resolve the real GMenu action rather than bypassing native dispatch.
        let emulation = bars.submenu(MenuId::Emulation);
        let debug_action = (0..emulation.n_items()).find_map(|i| {
            emulation
                .item_attribute_value(i, "action", None)
                .and_then(|value| {
                    value
                        .str()
                        .filter(|name| *name == "win.debugger")
                        .map(str::to_owned)
                })
        });
        let name = debug_action.expect("host Debug entry in main_window.menu");
        let action = win
            .window()
            .lookup_action(name.strip_prefix("win.").unwrap())
            .unwrap();
        action.activate(None);
        win.application().disconnect(signal);
        assert_eq!(added.get(), 1);
        assert_eq!(mapped.get(), 1);
        let id = win.borrow().aux.route_id(AuxKind::Debug).unwrap();
        let Some(NativeAux::Debug(debugger)) = win.borrow().aux.get(AuxKind::Debug).cloned() else {
            panic!("real debugger group constructed");
        };
        let native = debugger.borrow().native_window();
        assert!(native.child().is_some());
        assert_eq!(
            native.title().as_deref(),
            Some(HostWindow::auxiliary(AuxKind::Debug).title.as_str())
        );
        win.open_debugger();
        assert_eq!(win.borrow().aux.route_id(AuxKind::Debug), Some(id));
        native.destroy();
        win.window().destroy();
    }

    #[test]
    #[ignore = "needs display; CI-only under xvfb"]
    fn common_auxiliary_contents_choose_native_groups_and_owned_titles() {
        use super::{AuxKind, HostWindow};
        use gtk::prelude::WidgetExt;
        use nerust_frontend_host::window::WindowId;
        let win = bound_window().expect("run in an isolated Xvfb process");
        let app = win.application();
        let main = win.window();
        let state = win.state();
        let mut debug = HostWindow::auxiliary(AuxKind::Debug);
        debug.id = WindowId::Auxiliary(AuxKind::History);
        debug.title = "owned debug title".into();
        let native_debug =
            crate::debugger::DebuggerWindowCore::from_window(&debug, &app, &main, state.clone())
                .expect("Debug contents, regardless of WindowId");
        let native = native_debug.borrow().native_window();
        assert_eq!(native.title().as_deref(), Some(debug.title.as_str()));
        assert!(native.child().is_some());
        assert!(super::render_dialog(&debug, &main, None, inert_callbacks()).is_none());

        let mut ppu = HostWindow::auxiliary(AuxKind::Ppu);
        ppu.id = WindowId::Main;
        ppu.title = "owned PPU title".into();
        let viewer = crate::ppu_window::PpuWindowCore::from_window(
            &ppu,
            &app,
            &main,
            state.clone(),
            Vec::new(),
            0,
            None,
            nerust_frontend_host::debugger::PpuChrome::new(
                nerust_settings_core::model::language::AppLanguage::English,
            ),
        )
        .expect("PPU grouped body");
        assert_eq!(
            viewer.borrow().window.title().as_deref(),
            Some(ppu.title.as_str())
        );
        assert!(viewer.borrow().window.child().is_some());
        assert!(
            crate::debugger::DebuggerWindowCore::from_window(&ppu, &app, &main, state.clone())
                .is_none()
        );

        let id = super::request_aux(&win, AuxKind::History).unwrap();
        let session = {
            let mut core = win.borrow_mut();
            let mut state = state.borrow_mut();
            state
                .host
                .prepare_widget_window(&mut core.aux, id)
                .unwrap()
                .unwrap()
        };
        let mut history = session.window();
        history.id = WindowId::Auxiliary(AuxKind::Debug);
        let Some(super::RenderedDialog::Controls(built)) = super::render_dialog(
            &history,
            &main,
            Some(session),
            super::widget_callbacks(&win, id),
        ) else {
            panic!("History contents, not ID, select grouped controls")
        };
        assert!(built.dialog.content_area().first_child().is_some());
        let pending = HostWindow::auxiliary(AuxKind::Settings);
        assert!(super::render_dialog(&pending, &main, None, inert_callbacks()).is_none());
        assert!(
            crate::debugger::DebuggerWindowCore::from_window(&pending, &app, &main, state.clone())
                .is_none()
        );
        assert!(
            crate::ppu_window::PpuWindowCore::from_window(
                &pending,
                &app,
                &main,
                state,
                Vec::new(),
                0,
                None,
                nerust_frontend_host::debugger::PpuChrome::new(
                    nerust_settings_core::model::language::AppLanguage::English
                ),
            )
            .is_none()
        );
        built.dialog.destroy();
        viewer.borrow().window.destroy();
        native.destroy();
        main.destroy();
    }

    #[test]
    #[ignore = "needs display; CI-only under xvfb"]
    fn common_dialog_map_reentry_sees_committed_identity_without_borrows() {
        use super::{AuxKind, OpenRequest};
        use gtk::prelude::{ListModelExt, WidgetExt};
        use std::{cell::Cell, rc::Rc};
        let win = bound_window().expect("isolated Xvfb process");
        for kind in [AuxKind::History, AuxKind::Settings] {
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
                            win.borrow_mut().aux.request_open(kind),
                            OpenRequest::Pending(id)
                        );
                        super::open_auxiliary(&win, kind);
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
                                super::open_auxiliary(&win, kind);
                            }
                        });
                    }
                }
            });
            super::open_auxiliary(&win, kind);
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
        use super::AuxKind;
        use gtk::prelude::{ListModelExt, WidgetExt};
        use std::{cell::RefCell, rc::Rc};
        use wakui::pause::PauseOwner;
        let win = bound_window().expect("isolated Xvfb process");
        for kind in [AuxKind::History, AuxKind::Settings] {
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
                        assert!(super::close_aux(&win, old));
                        super::open_auxiliary(&win, kind);
                        assert!(!super::close_aux(&win, old));
                    }
                }
            });
            super::open_auxiliary(&win, kind);
            toplevels.disconnect(signal);
            let stale = stale.borrow().clone().unwrap();
            assert!(surface_gone(&stale));
            let current = win.borrow().aux.get(kind).unwrap().native_window();
            assert_ne!(current, stale.clone().upcast::<gtk::Window>());
            assert!(current.is_visible());
            if kind == AuxKind::Settings {
                assert!(win.state().borrow().host.owns_pause(PauseOwner::Settings));
                assert!(
                    !win.borrow()
                        .actions
                        .get(CommandId::Settings)
                        .unwrap()
                        .is_enabled()
                );
            }
            stale.destroy();
            assert!(win.borrow().aux.contains(kind));
            current.destroy();
            assert!(!win.borrow().aux.contains(kind));
            assert!(!win.state().borrow().host.owns_pause(PauseOwner::Settings));
        }
        win.window().destroy();
    }

    #[test]
    #[ignore = "needs display; CI-only under xvfb"]
    fn common_debug_stale_native_completion_preserves_replacement_without_borrows() {
        use super::AuxKind;
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
                let old = win.borrow().aux.route_id(AuxKind::Debug).unwrap();
                assert!(win.borrow_mut().aux.cancel_open(old));
                win.open_debugger();
                assert!(!super::close_aux(&win, old));
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
        win.open_debugger();
        win.application().disconnect(signal);
        win.application().disconnect(removed);
        let stale = stale.borrow().clone().unwrap();
        assert!(surface_gone(&stale));
        let current = win
            .borrow()
            .aux
            .get(AuxKind::Debug)
            .unwrap()
            .native_window();
        assert_ne!(current, stale);
        assert!(
            win.borrow()
                .aux
                .window(win.borrow().aux.route_id(AuxKind::Debug).unwrap())
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
        let action = win.borrow().actions.get(CommandId::Open).unwrap();
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
        let super::OpenRequest::Create(id) =
            win.borrow_mut().aux.request_open(super::AuxKind::Settings)
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
                .get(CommandId::Settings)
                .unwrap()
                .is_enabled()
        );
        assert!(super::close_aux(&win, id));
        win.update_actions();
        assert_eq!(notifications.get(), 2);
        assert!(borrow_free.get());
        assert!(action.is_enabled());
        assert!(
            win.borrow()
                .actions
                .get(CommandId::Settings)
                .unwrap()
                .is_enabled()
        );
    }

    #[test]
    fn unchanged_menu_models_emit_no_items_changed_and_relabels_allow_reentry() {
        use gio::prelude::MenuModelExt as _;
        use nerust_settings_core::model::language::AppLanguage;
        use std::{cell::Cell, rc::Rc};
        let Some(win) = bound_window() else {
            eprintln!("SKIP: window widget test needs a display");
            return;
        };
        // Auto and English may render identical strings. Establish an explicit
        // baseline so the later language change must change the visible model.
        let state = win.state();
        let mut settings = state.borrow().host.settings_snapshot().clone();
        settings.shared.general.language = AppLanguage::English;
        state.borrow_mut().host.apply_settings(settings).unwrap();
        win.update_actions();
        let menubar = win.borrow().menubar.clone();
        assert!(menubar.attached.get());
        assert_eq!(
            win.application().menubar().as_ref(),
            Some(menubar.menu_model.upcast_ref())
        );
        let models = [
            menubar.menu_model.clone(),
            menubar.submenu(MenuId::File),
            menubar.submenu(MenuId::States),
            menubar.submenu(MenuId::Emulation),
            menubar.submenu(MenuId::Help),
            menubar.submenu(MenuId::SelectSlot),
            menubar.submenu(MenuId::SaveSlot),
            menubar.submenu(MenuId::LoadSlot),
            menubar.submenu(MenuId::DeleteSlot),
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
        let super::OpenRequest::Create(id) =
            win.borrow_mut().aux.request_open(super::AuxKind::Settings)
        else {
            panic!("expected settings reservation");
        };
        win.update_actions();
        assert!(super::close_aux(&win, id));
        win.update_actions();
        assert_eq!(
            changes.get(),
            0,
            "action-only changes must not rebuild models"
        );

        let state = win.state();
        let mut settings = state.borrow().host.settings_snapshot().clone();
        settings.shared.general.language = AppLanguage::Japanese;
        state.borrow_mut().host.apply_settings(settings).unwrap();
        win.update_actions();
        assert!(changes.get() > 0);
        assert!(borrow_free.get());
        assert_eq!(
            win.application().menubar().as_ref(),
            Some(menubar.menu_model.upcast_ref())
        );
        for (model, id) in models[1..].iter().zip([
            MenuId::File,
            MenuId::States,
            MenuId::Emulation,
            MenuId::Help,
            MenuId::SelectSlot,
            MenuId::SaveSlot,
            MenuId::LoadSlot,
            MenuId::DeleteSlot,
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
        let action = win.borrow().actions.get(CommandId::Open).unwrap();
        let quit = win.borrow().actions.get(CommandId::Quit).unwrap();
        let super::OpenRequest::Create(id) =
            win.borrow_mut().aux.request_open(super::AuxKind::Settings)
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
            assert!(super::close_aux(&win, id));
            win.update_actions();
        });
        win.update_actions();
        assert!(called.get());
        assert!(action.is_enabled());
        assert!(
            win.borrow()
                .actions
                .get(CommandId::Settings)
                .unwrap()
                .is_enabled()
        );
        assert!(
            win.borrow()
                .actions
                .get(CommandId::Debugger)
                .unwrap()
                .is_enabled()
        );
        assert!(
            win.borrow()
                .actions
                .get(CommandId::NotificationHistory)
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
                .get(CommandId::Close)
                .unwrap()
                .is_enabled()
        );
    }

    #[test]
    fn items_changed_callback_mutates_settings_and_newest_labels_win() {
        use gio::prelude::MenuModelExt;
        use nerust_settings_core::model::{input::ShortcutAction, language::AppLanguage};
        use std::{cell::Cell, rc::Rc};
        let Some(win) = bound_window() else {
            eprintln!("SKIP: window widget test needs a display");
            return;
        };
        let state = win.state();
        let mut initial = state.borrow().host.settings_snapshot().clone();
        initial.shared.general.language = AppLanguage::English;
        state
            .borrow_mut()
            .host
            .apply_settings(initial.clone())
            .unwrap();
        win.update_actions();
        let called = Rc::new(Cell::new(false));
        let once = called.clone();
        let weak = Rc::downgrade(&win);
        let menubar = win.borrow().menubar.clone();
        let model = menubar.submenu(MenuId::States);
        model.connect_items_changed(move |_, _, _, _| {
            if once.replace(true) {
                return;
            }
            let win = weak.upgrade().unwrap();
            assert!(win.try_borrow_mut().is_ok());
            let state = win.state();
            let mut newest = state.borrow().host.settings_snapshot().clone();
            newest.shared.general.language = AppLanguage::English;
            let binding = newest
                .shared
                .input
                .shortcuts
                .keyboard
                .iter_mut()
                .find(|binding| binding.action == ShortcutAction::OpenDebugger)
                .unwrap();
            binding.key = Some(wakui::Key::F2);
            state.borrow_mut().host.apply_settings(newest).unwrap();
            win.update_actions();
        });
        initial.shared.general.language = AppLanguage::Japanese;
        state.borrow_mut().host.apply_settings(initial).unwrap();
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
        assert!(rendered.iter().any(|label| label == "File"), "{rendered:?}");
        assert!(
            rendered.iter().any(|label| label == "Open Debugger (F2)"),
            "{rendered:?}"
        );
        assert!(
            !rendered
                .iter()
                .any(|label| label == "ファイル" || label.contains("(F4)")),
            "{rendered:?}"
        );
        assert_eq!(
            state
                .borrow()
                .host
                .settings_snapshot()
                .shared
                .general
                .language,
            AppLanguage::English
        );
        win.update_actions();
    }

    #[test]
    fn auxiliary_open_reservations_block_reentrant_native_creation() {
        use super::{AuxKind, OpenRequest};
        let Some(win) = bound_window() else {
            eprintln!("SKIP: window widget test needs a display");
            return;
        };
        for kind in [AuxKind::Debug, AuxKind::History, AuxKind::Settings] {
            let OpenRequest::Create(id) = win.borrow_mut().aux.request_open(kind) else {
                panic!("expected reservation");
            };
            match kind {
                AuxKind::Debug => win.open_debugger(),
                AuxKind::History => win.open_history(),
                AuxKind::Settings => {
                    let action = win.borrow().actions.get(CommandId::Settings).unwrap();
                    super::open_auxiliary(&win, AuxKind::Settings);
                    assert!(action.is_enabled());
                }
                AuxKind::Ppu => unreachable!(),
            }
            assert_eq!(win.borrow().aux.route_id(kind), Some(id));
            assert!(win.borrow().aux.get(kind).is_none());
            assert!(win.borrow_mut().aux.cancel_open(id));
        }
    }

    #[test]
    fn history_reuses_identity_and_old_destroy_cannot_close_replacement() {
        use super::{AuxKind, NativeAux};
        let Some(win) = bound_window() else {
            eprintln!("SKIP: window widget test needs a display");
            return;
        };
        win.open_history();
        let old_id = win.borrow().aux.route_id(AuxKind::History).unwrap();
        let Some(NativeAux::Controls(program)) = win.borrow().aux.get(AuxKind::History).cloned()
        else {
            panic!("history dialog");
        };
        let old = program.dialog;
        win.open_history();
        assert_eq!(win.borrow().aux.route_id(AuxKind::History), Some(old_id));
        assert!(super::close_aux(&win, old_id));
        win.open_history();
        let new_id = win.borrow().aux.route_id(AuxKind::History).unwrap();
        assert_ne!(old_id, new_id);
        old.destroy();
        assert!(!super::close_aux(&win, old_id));
        assert_eq!(win.borrow().aux.route_id(AuxKind::History), Some(new_id));
        let current = win
            .borrow()
            .aux
            .get(AuxKind::History)
            .unwrap()
            .native_window();
        current.destroy();
        assert!(!win.borrow().aux.contains(AuxKind::History));
    }

    #[test]
    fn settings_identity_guards_pause_owner_and_native_finish() {
        use super::{AuxKind, NativeAux};
        use wakui::pause::PauseOwner;
        let Some(win) = bound_window() else {
            eprintln!("SKIP: window widget test needs a display");
            return;
        };
        let action = win.borrow().actions.get(CommandId::Settings).unwrap();
        action.activate(None);
        let old_id = win.borrow().aux.route_id(AuxKind::Settings).unwrap();
        let Some(NativeAux::Controls(program)) = win.borrow().aux.get(AuxKind::Settings).cloned()
        else {
            panic!("settings dialog");
        };
        let old = program.dialog;
        super::open_auxiliary(&win, AuxKind::Settings);
        assert_eq!(win.borrow().aux.route_id(AuxKind::Settings), Some(old_id));
        win.update_actions();
        assert!(!action.is_enabled());
        old.response(gtk::ResponseType::Cancel);
        assert!(action.is_enabled());
        assert!(!win.borrow().aux.contains(AuxKind::Settings));
        assert!(!win.state().borrow().host.owns_pause(PauseOwner::Settings));
        super::open_auxiliary(&win, AuxKind::Settings);
        let new_id = win.borrow().aux.route_id(AuxKind::Settings).unwrap();
        assert_ne!(old_id, new_id);
        assert!(!super::close_aux(&win, old_id));
        old.destroy();
        assert!(win.state().borrow().host.owns_pause(PauseOwner::Settings));
        assert!(!action.is_enabled());
        win.state()
            .borrow_mut()
            .host
            .set_pause_owner(PauseOwner::Modal, true);
        let current = win
            .borrow()
            .aux
            .get(AuxKind::Settings)
            .unwrap()
            .native_window();
        current.destroy();
        assert!(action.is_enabled());
        assert!(!win.borrow().aux.contains(AuxKind::Settings));
        assert!(!win.state().borrow().host.owns_pause(PauseOwner::Settings));
        assert!(win.state().borrow().host.owns_pause(PauseOwner::Modal));
    }

    /// Headless-capable state: session plus settings only, no renderer.
    fn harness() -> Option<(
        gtk::Application,
        gtk::ApplicationWindow,
        std::rc::Rc<std::cell::RefCell<crate::State>>,
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
        Some((app, window, model_state()))
    }

    pub(crate) fn model_state() -> std::rc::Rc<std::cell::RefCell<crate::State>> {
        use std::sync::Arc;
        let registry = Arc::new(nerust_gui_shell::registry::SystemRegistry::new(Vec::new()));
        let audio = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
        let ctx = nerust_gui_shell::context::FrontendContext {
            gpu_factory: std::rc::Rc::new(NullFactory {
                fail_create: true,
                ..Default::default()
            }),
            registry,
            rom_loader: Box::new(MockRomLoader),
            audio_registry: audio,
        };
        std::rc::Rc::new(std::cell::RefCell::new(
            crate::State::new_for_tests(ctx).expect("test state builds"),
        ))
    }

    use nerust_frontend_host::viewport::testkit::NullFactory;

    #[derive(Debug)]
    struct MockRomLoader;

    impl nerust_gui_shell::load::RomLoader for MockRomLoader {
        fn load_rom(
            &mut self,
            _path: &std::path::Path,
            _target: &mut dyn nerust_gui_shell::load::RomLoadTarget,
        ) -> Result<(), nerust_gui_shell::load::RomLoaderError> {
            Err(nerust_gui_shell::load::RomLoaderError::from_session(
                nerust_gui_shell::session::SessionError::NoCore,
            ))
        }
    }

    #[test]
    fn key_press_state_maps_to_pressed_flag() {
        assert!(key_event_pressed(KeyEventState::Press));
        assert!(!key_event_pressed(KeyEventState::Release));
    }

    #[test]
    fn command_declarations_generate_and_reuse_every_action() {
        use super::{CommandParameter, StaticVariantType};
        use gio::prelude::MenuModelExt;
        let state = model_state();
        let model = {
            let mut state = state.borrow_mut();
            let ctx = state.host.frame_ctx();
            state
                .host
                .menubar_update(&ctx, false, GTK_MENU_CAPABILITIES)
                .unwrap()
                .menubar
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
                let action = visitor.actions.get(descriptor.id).unwrap();
                assert_eq!(action.name().as_str(), descriptor.id.name());
                assert_eq!(action.is_enabled(), descriptor.enabled);
                assert_eq!(
                    action.parameter_type(),
                    match descriptor.parameter {
                        CommandParameter::None => None,
                        CommandParameter::SlotId => Some(u64::static_variant_type().into_owned()),
                    }
                );
                (descriptor.id, action)
            })
            .collect();
        model.visit(Some(&model), &mut visitor);
        for (id, action) in actions {
            assert_eq!(visitor.actions.get(id).unwrap(), action);
        }
        let file = visitor.menubar.submenu(MenuId::File);
        assert_eq!(
            file.item_attribute_value(file.n_items() - 1, "action", None)
                .unwrap()
                .str(),
            Some("win.quit")
        );
        assert_eq!(
            visitor
                .menubar
                .submenu(MenuId::Help)
                .item_attribute_value(0, "action", None)
                .unwrap()
                .str(),
            Some("win.about")
        );
    }

    #[test]
    fn generated_actions_send_typed_commands_to_dyn_sink_and_reject_bad_arguments() {
        use super::{CommandVisitor, EventCommand, EventSink, ToVariant};
        use nerust_frontend_host::commands::{MenuCommand, SessionCommand};
        use std::{cell::RefCell, rc::Rc};
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
            let ctx = state.host.frame_ctx();
            state
                .host
                .menubar_update(&ctx, false, GTK_MENU_CAPABILITIES)
                .unwrap()
                .menubar
        };
        let recording = Rc::new(RecordingSink::default());
        let mut visitor = GtkMenuVisitor::new(MenubarModels::new());
        visitor.sink = Some(recording.clone());
        for descriptor in &model.commands.definitions {
            let mut descriptor = *descriptor;
            descriptor.enabled = true;
            visitor.command(&descriptor);
        }
        let reset = visitor.actions.get(CommandId::Reset).unwrap();
        let slot = visitor.actions.get(CommandId::LoadSlot).unwrap();
        reset.activate(None);
        slot.activate(Some(&42_u64.to_variant()));
        assert!(matches!(
            recording.0.borrow().as_slice(),
            [
                EventCommand::Menu(MenuCommand::Session(SessionCommand::Reset)),
                EventCommand::Menu(MenuCommand::Session(SessionCommand::LoadSlot(42))),
            ]
        ));
        let descriptor = model.commands.get(CommandId::LoadSlot).unwrap();
        let sink: Rc<dyn EventSink> = recording.clone();
        // Exercise the callback decoder directly: GAction itself also rejects
        // these before emitting activate, which would otherwise mask a bug here.
        super::send_action(sink.as_ref(), descriptor, None);
        super::send_action(sink.as_ref(), descriptor, Some(&"42".to_variant()));
        super::send_action(sink.as_ref(), descriptor, Some(&42_i32.to_variant()));
        super::send_action(
            sink.as_ref(),
            model.commands.get(CommandId::Reset).unwrap(),
            Some(&42_u64.to_variant()),
        );
        assert_eq!(recording.0.borrow().len(), 2);
        slot.activate(Some(&0_u64.to_variant()));
        assert!(
            matches!(
                recording.0.borrow().last(),
                Some(EventCommand::Menu(MenuCommand::Session(
                    SessionCommand::LoadSlot(0)
                )))
            ),
            "explicit zero is valid, never a missing-argument fallback"
        );
    }

    #[test]
    fn native_roles_go_through_host_settings_guard_but_quit_remains_available() {
        use super::{AuxKind, EventCommand, OpenRequest};
        use nerust_frontend_host::commands::{MenuCommand, NativeRole};
        use wakui::pause::PauseOwner;
        let Some(win) = bound_window() else {
            eprintln!("SKIP: window widget test needs a display");
            return;
        };
        assert!(win.application().lookup_action("about").is_none());
        assert!(win.application().lookup_action("quit").is_none());
        let about = win.borrow().actions.get(CommandId::About).unwrap();
        let quit = win.borrow().actions.get(CommandId::Quit).unwrap();
        assert_eq!(
            win.window().lookup_action("about").unwrap(),
            about.clone().upcast::<gio::Action>()
        );
        assert_eq!(
            win.window().lookup_action("quit").unwrap(),
            quit.clone().upcast::<gio::Action>()
        );
        let sink = win.event_sink();
        let about_event = EventCommand::Menu(MenuCommand::Native(NativeRole::About));
        let OpenRequest::Create(id) = win.borrow_mut().aux.request_open(AuxKind::Settings) else {
            panic!("settings reservation");
        };
        // Bypass native enablement to emulate an already queued activation.
        assert!(sink.send(about_event));
        assert!(win.borrow().about_dialog.is_none());
        assert!(!about.is_enabled());
        assert!(quit.is_enabled());
        assert!(super::close_aux(&win, id));
        win.state()
            .borrow_mut()
            .host
            .set_pause_owner(PauseOwner::Settings, true);
        assert!(sink.send(about_event));
        assert!(
            win.borrow().about_dialog.is_none(),
            "host pause guard also applies without aux"
        );
        win.state()
            .borrow_mut()
            .host
            .set_pause_owner(PauseOwner::Settings, false);
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
        let OpenRequest::Create(_) = win.borrow_mut().aux.request_open(AuxKind::Settings) else {
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
                            win.event_sink().send(super::EventCommand::LiveTick);
                        }
                    });
                }
            }
        });
        let about = win.borrow().actions.get(CommandId::About).unwrap();
        about.activate(None);
        assert_eq!(mapped.get(), 1);
        let dialog = win.borrow().about_dialog.clone().unwrap();
        dialog.close();
        toplevels.disconnect(signal);
    }

    #[test]
    fn event_sink_and_action_callbacks_do_not_keep_window_alive() {
        use super::{EventCommand, SessionEvent};
        use std::rc::Rc;
        let Some(win) = bound_window() else {
            eprintln!("SKIP: window widget test needs a display");
            return;
        };
        let weak = Rc::downgrade(&win);
        let sink = win.event_sink();
        let action = win.borrow().actions.get(CommandId::About).unwrap();
        let native = win.window();
        drop(win);
        assert!(weak.upgrade().is_none());
        assert!(!sink.send(EventCommand::Session(SessionEvent::ClearInput)));
        action.activate(None);
        native.destroy();
    }

    #[test]
    fn keys_and_shortcuts_use_the_same_dyn_event_boundary() {
        use super::{EventCommand, EventSink, SessionEvent};
        use nerust_gui_shell::session::KeyboardShortcut;
        use std::{cell::RefCell, rc::Rc};
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
        let shortcut = SessionEvent::Shortcut(KeyboardShortcut::OpenDebugger);
        win.event_sink().send(EventCommand::Session(shortcut));
        assert!(matches!(
            sink.0.borrow().as_slice(),
            [
                EventCommand::Session(SessionEvent::Keyboard { pressed: true, .. }),
                EventCommand::Session(SessionEvent::Keyboard { pressed: false, .. }),
                EventCommand::Session(SessionEvent::Shortcut(KeyboardShortcut::OpenDebugger)),
            ]
        ));
        win.window().destroy();
    }

    #[test]
    fn slot_menu_explains_empty_state() {
        use gio::prelude::MenuModelExt;
        use nerust_settings_core::model::language::AppLanguage;
        let sig = MenuSig {
            loaded: true,
            paused: false,
            settings_open: false,
            language: AppLanguage::English,
            active_slot: None,
            slots: Vec::new().into(),
            shortcuts: Vec::new().into(),
        };
        let bars = MenubarModels::new();
        let model = MenuModel::build(&sig, GTK_MENU_CAPABILITIES);
        let mut visitor = GtkMenuVisitor::new(bars.clone());
        model.visit(None, &mut visitor);
        for descriptor in &model.commands.definitions {
            if descriptor.parameter == super::CommandParameter::SlotId {
                assert!(!visitor.actions.get(descriptor.id).unwrap().is_enabled());
            }
        }
        // One disabled item names the empty state, never a blank list.
        for id in [
            MenuId::SelectSlot,
            MenuId::SaveSlot,
            MenuId::LoadSlot,
            MenuId::DeleteSlot,
        ] {
            let menu = bars.submenu(id);
            assert_eq!(menu.n_items(), 1);
            assert!(menu.item_attribute_value(0, "action", None).is_none());
            assert!(menu.item_attribute_value(0, "target", None).is_none());
            assert_eq!(
                menu.item_attribute_value(0, "label", None).unwrap().str(),
                Some("No save slots yet")
            );
        }
    }

    #[test]
    fn slot_menu_lists_slots_with_action_targets() {
        use nerust_persistence::model::StateSlotSummary;
        use nerust_settings_core::model::language::AppLanguage;
        let slots = vec![
            StateSlotSummary {
                schema_version: 1,
                slot_id: 1,
                path: nerust_persistence::sidecar::StorageLocation::Native("slot1".into()),
                saved_at: std::time::SystemTime::UNIX_EPOCH,
                has_thumbnail: false,
                emulator_version: "test".to_string(),
            },
            StateSlotSummary {
                schema_version: 1,
                slot_id: 2,
                path: nerust_persistence::sidecar::StorageLocation::Native("slot2".into()),
                saved_at: std::time::SystemTime::UNIX_EPOCH,
                has_thumbnail: false,
                emulator_version: "test".to_string(),
            },
        ];
        let sig = MenuSig {
            loaded: true,
            paused: false,
            settings_open: false,
            language: AppLanguage::English,
            active_slot: Some(2),
            slots: slots.into(),
            shortcuts: Vec::new().into(),
        };
        let bars = MenubarModels::new();
        let mut model = MenuModel::build(&sig, GTK_MENU_CAPABILITIES);
        fn disable_items(menu: &mut super::Submenu) {
            use nerust_frontend_host::menu::MenuEntry;
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
        assert!(
            visitor
                .actions
                .get(CommandId::LoadSlot)
                .unwrap()
                .is_enabled(),
            "slot entries cannot overwrite shared declaration enablement"
        );
        let menu = bars.submenu(MenuId::LoadSlot);
        use gio::prelude::MenuModelExt;
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
                Some("win.state-load-slot")
            );
        }
    }
}
