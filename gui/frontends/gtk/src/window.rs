use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::Path,
    rc::{Rc, Weak},
};

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
    widgets::MenuPresenter,
};
use nerust_gui_shell::session::{SessionError, access::SessionControl};
use nerust_render_traits::renderer::PipelineFactory;
use nerust_settings_core::model::local::ScalingMode;

use super::{
    State, TITLE_UPDATE_INTERVAL,
    surface::{Surface, SurfaceExtend},
    surface_state::surface_gone,
};
use crate::preferences::present_preferences_dialog;

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
}

/// Native resources only. Opening, presence, identity and routing belong to host.
#[derive(Clone)]
enum NativeAux {
    Debug(crate::debugger::DebuggerWindow),
    History(gtk::Dialog, Rc<dyn Fn()>),
    Settings(gtk::Dialog),
}

impl NativeAux {
    fn is_gone(&self) -> bool {
        match self {
            Self::Debug(debugger) => debugger.borrow().is_gone(),
            Self::History(dialog, _) | Self::Settings(dialog) => surface_gone(dialog),
        }
    }

    fn present(&self) {
        match self {
            Self::Debug(debugger) => {
                crate::debugger::DebuggerWindowCore::refresh(debugger);
                debugger.borrow().present();
            }
            Self::History(dialog, refresh) => {
                dialog.present();
                refresh();
            }
            Self::Settings(dialog) => dialog.present(),
        }
    }

    fn native_window(&self) -> gtk::Window {
        match self {
            Self::Debug(debugger) => debugger.borrow().native_window().upcast(),
            Self::History(dialog, _) | Self::Settings(dialog) => dialog.clone().upcast(),
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
    let request = window.borrow_mut().aux.request_open(kind);
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
    let history = matches!(&handle, NativeAux::History(_, _));
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
    if history {
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
    let (kind, handle) = {
        let mut window = window.borrow_mut();
        let kind = window.aux.kind_of_id(id);
        (kind, window.aux.remove_id(id))
    };
    if kind == Some(AuxKind::Settings) {
        let _ = window
            .state()
            .borrow_mut()
            .dispatch_session(SessionEvent::SettingsClosed);
    }
    drop(handle);
    kind.is_some()
}

fn open_settings(window: &Window) {
    let Some(id) = request_aux(window, AuxKind::Settings) else {
        return;
    };
    if !window
        .state()
        .borrow_mut()
        .dispatch_session(SessionEvent::SettingsOpened)
        .is_ok_and(|out| out.settings_changed)
    {
        window.borrow_mut().aux.cancel_open(id);
        return;
    }
    window.update_actions();
    let weak = Rc::downgrade(window);
    let dialog = present_preferences_dialog(&window.window(), window.state(), move || {
        if let Some(window) = weak.upgrade()
            && close_aux(&window, id)
        {
            window.update_actions();
        }
    });
    if let Some(dialog) = dialog {
        complete_aux(window, id, NativeAux::Settings(dialog));
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
        CommandEffect::OpenAux(AuxKind::Settings) => open_settings(window),
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
        factory: Rc<dyn PipelineFactory>,
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
    fn refresh_title(&self);
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
        factory: Rc<dyn PipelineFactory>,
        menubar: MenubarModels,
    ) -> Window {
        let _ = Surface::bind(&window, &picture, state.clone(), factory);
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
            })
        });
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
        let Some(id) = request_aux(self, AuxKind::History) else {
            return;
        };
        let built = crate::osd::GtkOsd::build_history(&self.window(), &self.state());
        if complete_aux(
            self,
            id,
            NativeAux::History(built.dialog.clone(), built.refresh),
        ) {
            built.dialog.present();
        }
    }

    fn open_debugger(&self) {
        let Some(id) = request_aux(self, AuxKind::Debug) else {
            return;
        };
        let debugger = crate::debugger::DebuggerWindowCore::open(
            &self.application(),
            &self.window(),
            self.state(),
        );
        complete_aux(self, id, NativeAux::Debug(debugger));
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
        let filters = nerust_gui_shell::registry::rom_picker_filters(
            self.state().borrow().host.session().factories(),
        );
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
        let menu = {
            let mut state = state.borrow_mut();
            let ctx = state.host.frame_ctx();
            state
                .host
                .menubar_update(&ctx, settings_open, GTK_MENU_CAPABILITIES)
        };
        if let Some(menu) = menu {
            let (mut visitor, presenter, mut next) = {
                let core = self.borrow();
                let visitor = GtkMenuVisitor {
                    menubar: core.menubar.clone(),
                    window: Some(core.window.clone()),
                    application: Some(core.application.clone()),
                    actions: core.actions.clone(),
                    sink: Some(core.sink.clone()),
                    stack: Vec::new(),
                };
                let next = core.menu_presenter.submit(menu.menubar);
                (visitor, core.menu_presenter.clone(), next)
            };
            // Both RefCells are released before synchronous native signals.
            while let Some(plan) = next {
                plan.visit(&mut visitor);
                next = presenter.complete(plan);
            }
        }
        self.refresh_title();
    }

    fn rebuild_menubar(&self) {
        self.update_actions();
    }

    fn refresh_title(&self) {
        let title = self.state().borrow_mut().title_update();
        if let Some(title) = title {
            self.window().set_title(Some(title.as_str()));
        }
    }

    fn sync_fullscreen_from_settings(&self) {
        let fullscreen = {
            let state = self.state();
            let mut borrowed = state.borrow_mut();
            let native = self.window().is_fullscreen();
            let Some(fullscreen) = borrowed.fullscreen_request(native) else {
                return;
            };
            let snapshot = borrowed.settings_snapshot();
            let scaling = snapshot.local.video.window.scaling;
            let size = snapshot.app_state.window_size("main");
            (fullscreen, scaling, size)
        };

        // Restore remembered window size when not in fullscreen or scaling mode.
        if !fullscreen.0
            && fullscreen.1 == ScalingMode::FitToWindow
            && let Some(size) = fullscreen.2
        {
            self.window()
                .set_default_size(size.width as i32, size.height as i32);
        }

        set_window_fullscreen(&self.window(), fullscreen.0);
    }

    fn key_event(&self, key: gdk::Key, event: KeyEventState) -> bool {
        // No hardcoded F-keys: the debugger open rides the shortcut schema
        // (default F4, rebindable in Settings), so a user binding always wins.
        if let Ok(controller_input) = key.try_into() {
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

fn persist_window_fullscreen_default(
    window: &Window,
    fullscreen: bool,
) -> Result<bool, SessionError> {
    let state = window.state();
    Ok(state
        .borrow_mut()
        .set_fullscreen_default(fullscreen)?
        .fullscreen_default_changed)
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
    state.borrow_mut().host.session_mut().report_external_error(
        nerust_gui_shell::session::notification::FailureOrigin::Host,
        detail.to_owned(),
    );
}

#[cfg(test)]
mod tests {
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
        let window = win.window();
        let state = win.state();
        // Bypass the slot wiring: a directly built dialog carries no
        // destroy hook, like an externally destroyed one whose
        // close-request never ran.
        let built = crate::osd::GtkOsd::build_history(&window, &state);
        let super::OpenRequest::Create(id) =
            win.borrow_mut().aux.request_open(super::AuxKind::History)
        else {
            panic!("expected reservation");
        };
        assert!(
            win.borrow_mut()
                .aux
                .complete_open(
                    id,
                    None,
                    super::NativeAux::History(built.dialog.clone(), built.refresh.clone()),
                )
                .is_ok()
        );
        // External destroy disposes the widget without signals; the
        // slot keeps the dead dialog.
        built.dialog.destroy();
        assert!(surface_gone(&built.dialog));
        // Must rebuild, not panic on the slot borrow.
        win.open_history();
        let slot = win.borrow().aux.get(super::AuxKind::History).cloned();
        let Some(super::NativeAux::History(dialog, _)) = slot else {
            panic!("rebuilt dialog");
        };
        assert!(!surface_gone(&dialog));
        dialog.destroy();
    }

    fn bound_window() -> Option<super::Window> {
        let (app, window, state) = harness()?;
        Some(super::Window::bind(
            app,
            window,
            gtk::Picture::new(),
            state,
            std::rc::Rc::new(MockPipelineFactory),
            super::MenubarModels::new(),
        ))
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
        let mut settings = state.borrow().settings_snapshot().clone();
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
        let mut settings = state.borrow().settings_snapshot().clone();
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
        let mut initial = state.borrow().settings_snapshot().clone();
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
            let mut newest = state.borrow().settings_snapshot().clone();
            newest.shared.general.language = AppLanguage::English;
            let binding = newest
                .shared
                .input
                .shortcuts
                .keyboard
                .iter_mut()
                .find(|binding| binding.action == ShortcutAction::OpenDebugger)
                .unwrap();
            binding.key = Some(nerust_keyboard::Key::F2);
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
            state.borrow().settings_snapshot().shared.general.language,
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
                    super::open_settings(&win);
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
        let Some(NativeAux::History(old, _)) = win.borrow().aux.get(AuxKind::History).cloned()
        else {
            panic!("history dialog");
        };
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
        use nerust_frontend_kernel::pause::PauseOwner;
        let Some(win) = bound_window() else {
            eprintln!("SKIP: window widget test needs a display");
            return;
        };
        let action = win.borrow().actions.get(CommandId::Settings).unwrap();
        action.activate(None);
        let old_id = win.borrow().aux.route_id(AuxKind::Settings).unwrap();
        let Some(NativeAux::Settings(old)) = win.borrow().aux.get(AuxKind::Settings).cloned()
        else {
            panic!("settings dialog");
        };
        super::open_settings(&win);
        assert_eq!(win.borrow().aux.route_id(AuxKind::Settings), Some(old_id));
        win.update_actions();
        assert!(!action.is_enabled());
        old.response(gtk::ResponseType::Cancel);
        assert!(action.is_enabled());
        assert!(!win.borrow().aux.contains(AuxKind::Settings));
        assert!(!win.state().borrow().host.owns_pause(PauseOwner::Settings));
        super::open_settings(&win);
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
        let app = gtk::Application::builder()
            .application_id("org.nerust.window-test")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        // Menubar binding requires startup, just as in the real activate path.
        app.register(None::<&gio::Cancellable>)
            .expect("test application registers");
        let window = gtk::ApplicationWindow::builder().application(&app).build();
        Some((app, window, model_state()))
    }

    fn model_state() -> std::rc::Rc<std::cell::RefCell<crate::State>> {
        use std::sync::Arc;
        let registry = Arc::new(nerust_gui_shell::registry::SystemRegistry::new(Vec::new()));
        let audio = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
        let ctx = nerust_gui_shell::context::FrontendContext {
            gpu_factory: std::rc::Rc::new(MockPipelineFactory),
            registry,
            rom_loader: Box::new(MockRomLoader),
            audio_registry: audio,
        };
        std::rc::Rc::new(std::cell::RefCell::new(
            crate::State::new_for_tests(ctx).expect("test state builds"),
        ))
    }

    #[derive(Debug)]
    struct MockPipelineFactory;

    impl nerust_render_traits::renderer::PipelineFactory for MockPipelineFactory {
        fn create_pipeline(
            &self,
            _config: &nerust_render_traits::renderer::RendererConfig,
            _display: raw_window_handle::RawDisplayHandle,
        ) -> Result<
            Box<dyn nerust_render_traits::renderer::RenderPipeline>,
            nerust_render_traits::renderer::RendererError,
        > {
            Err(nerust_render_traits::renderer::RendererError::new(
                "test",
                Box::new(std::io::Error::other("no renderer in tests")),
            ))
        }
    }

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
        use nerust_frontend_kernel::pause::PauseOwner;
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
