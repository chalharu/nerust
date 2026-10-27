use std::{cell::RefCell, collections::HashMap, path::Path, rc::Rc};

use gtk::{
    gio, glib,
    glib::variant::{StaticVariantType, ToVariant},
    prelude::*,
};
use nerust_frontend_host::{
    aux::{AuxId, AuxWindows, OpenRequest},
    focus::AuxKind,
    menu::{
        MenuCapabilities, MenuCommand, MenuId, MenuItem, MenuModel, MenuVisitor, NativeRole,
        Submenu,
    },
    session::{SessionEffect, SessionEvent, SessionOutcome},
    widgets::MenuPresenter,
};
use nerust_gui_shell::session::{
    KeyboardShortcut, SessionError, access::SessionControl, commands::SessionCommand,
};
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
}

impl MenubarModels {
    pub(crate) fn new() -> Self {
        Self {
            menu_model: gio::Menu::new(),
            models: Rc::default(),
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
    aux: AuxWindows<u64, NativeAux>,
    #[cfg(test)]
    actions: WindowActions,
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

fn open_settings(window: &Window, _action: &gio::SimpleAction) {
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

/// Window action cluster: every menu/toolbar command in one place.
/// `WindowCore` owns one; wiring and gating go through these narrow ports.
#[derive(Clone)]
pub(crate) struct WindowActions {
    pub open: gio::SimpleAction,
    pub settings: gio::SimpleAction,
    pub close: gio::SimpleAction,
    pub pause: gio::SimpleAction,
    pub resume: gio::SimpleAction,
    pub reset: gio::SimpleAction,
    pub debugger: gio::SimpleAction,
    pub history: gio::SimpleAction,
    pub state_create: gio::SimpleAction,
    pub state_save_active: gio::SimpleAction,
    pub state_load_active: gio::SimpleAction,
    pub state_select_slot: gio::SimpleAction,
    pub state_save_slot: gio::SimpleAction,
    pub state_load_slot: gio::SimpleAction,
    pub state_delete_slot: gio::SimpleAction,
}

impl WindowActions {
    /// Fresh actions; the shared host snapshot gates them before presentation.
    pub fn new() -> Self {
        Self {
            open: gio::SimpleAction::new("open", None),
            settings: gio::SimpleAction::new("settings", None),
            close: gio::SimpleAction::new("close", None),
            pause: gio::SimpleAction::new("pause", None),
            resume: gio::SimpleAction::new("resume", None),
            reset: gio::SimpleAction::new("reset", None),
            debugger: gio::SimpleAction::new("debugger", None),
            history: gio::SimpleAction::new("history", None),
            state_create: gio::SimpleAction::new("state-create", None),
            state_save_active: gio::SimpleAction::new("state-save-active", None),
            state_load_active: gio::SimpleAction::new("state-load-active", None),
            state_select_slot: gio::SimpleAction::new(
                "state-select-slot",
                Some(&u64::static_variant_type()),
            ),
            state_save_slot: gio::SimpleAction::new(
                "state-save-slot",
                Some(&u64::static_variant_type()),
            ),
            state_load_slot: gio::SimpleAction::new(
                "state-load-slot",
                Some(&u64::static_variant_type()),
            ),
            state_delete_slot: gio::SimpleAction::new(
                "state-delete-slot",
                Some(&u64::static_variant_type()),
            ),
        }
    }

    /// Seat every action on the window.
    pub fn add_to(&self, window: &gtk::ApplicationWindow) {
        window.add_action(&self.open);
        window.add_action(&self.settings);
        window.add_action(&self.close);
        window.add_action(&self.pause);
        window.add_action(&self.resume);
        window.add_action(&self.reset);
        window.add_action(&self.debugger);
        window.add_action(&self.history);
        window.add_action(&self.state_create);
        window.add_action(&self.state_save_active);
        window.add_action(&self.state_load_active);
        window.add_action(&self.state_select_slot);
        window.add_action(&self.state_save_slot);
        window.add_action(&self.state_load_slot);
        window.add_action(&self.state_delete_slot);
    }
}

pub(crate) type Window = Rc<RefCell<WindowCore>>;

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
    fn close(&self);
    fn update_actions(&self);
    fn rebuild_menubar(&self);
    fn refresh_title(&self);
    fn sync_fullscreen_from_settings(&self);
    fn key_event(&self, key: gdk::Key, enevt: KeyEventState) -> bool;
    fn apply_keyboard_shortcut(&self, shortcut: KeyboardShortcut);
    fn apply_session_outcome(&self, outcome: SessionOutcome);
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
        let actions = WindowActions::new();
        let close_action = actions.close.clone();
        let pause_action = actions.pause.clone();
        let resume_action = actions.resume.clone();
        let reset_action = actions.reset.clone();
        let debugger_action = actions.debugger.clone();
        let history_action = actions.history.clone();
        let state_create_action = actions.state_create.clone();
        let state_save_active_action = actions.state_save_active.clone();
        let state_load_active_action = actions.state_load_active.clone();
        let state_select_slot_action = actions.state_select_slot.clone();
        let state_save_slot_action = actions.state_save_slot.clone();
        let state_load_slot_action = actions.state_load_slot.clone();
        let state_delete_slot_action = actions.state_delete_slot.clone();
        let settings_action = actions.settings.clone();
        let open_action = actions.open.clone();
        let _ = Surface::bind(&window, &picture, state.clone(), factory);
        actions.add_to(&window);
        let result = Rc::new(RefCell::new(WindowCore {
            application,
            window: window.clone(),
            state,
            open_dialog: None,
            aux: AuxWindows::new(),
            #[cfg(test)]
            actions,
            menubar,
            menu_presenter: Rc::default(),
        }));
        {
            let result = result.clone();
            let _ = window.connect_realize(move |_window| result.realize());
        }
        {
            let result = result.clone();
            let _ = window
                .connect_close_request(move |_| glib::Propagation::from(result.close_request()));
        }
        {
            let result = result.clone();
            let _ = window.connect_is_active_notify(move |window| {
                if window.is_active() {
                    result
                        .state()
                        .borrow_mut()
                        .focus_event(nerust_frontend_host::focus::FocusEvent::Main(true));
                } else {
                    result.state().borrow_mut().clear_input();
                    result
                        .state()
                        .borrow_mut()
                        .focus_event(nerust_frontend_host::focus::FocusEvent::Main(false));
                }
            });
        }
        {
            let result = result.clone();
            let _ = window.connect_notify_local(Some("fullscreened"), move |window, _| {
                sync_persisted_window_fullscreen(&result, window.is_fullscreen());
            });
        }

        let key_controller = gtk::EventControllerKey::new();
        {
            let result = result.clone();
            let _ = key_controller.connect_key_pressed(move |_, key, _, _| {
                glib::Propagation::from(result.key_event(key, KeyEventState::Press))
            });
        }
        {
            let result = result.clone();
            let _ = key_controller.connect_key_released(move |_, key, _, _| {
                result.key_event(key, KeyEventState::Release);
            });
        }
        window.add_controller(key_controller);

        {
            let result = result.clone();
            let _ = open_action.connect_activate(move |_, _| {
                result.open();
            });
        }

        {
            let result = result.clone();
            let _ = close_action.connect_activate(move |_, _| {
                result.close();
            });
        }

        // Menu and keyboard use the same command vocabulary and pause arbiter.
        for (action, command) in [
            (pause_action, SessionCommand::Pause),
            (resume_action, SessionCommand::Resume),
            (reset_action, SessionCommand::Reset),
            (state_create_action, SessionCommand::CreateSlot),
            (
                state_save_active_action,
                SessionCommand::SaveActiveSlotOrNew,
            ),
            (state_load_active_action, SessionCommand::LoadActiveSlot),
        ] {
            let result = result.clone();
            let _ = action.connect_activate(move |_, _| {
                let _ = result
                    .state()
                    .borrow_mut()
                    .dispatch_session(SessionEvent::Command(command));
                result.update_actions();
            });
        }

        {
            let result = result.clone();
            let _ = debugger_action.connect_activate(move |_, _| {
                result.apply_keyboard_shortcut(KeyboardShortcut::OpenDebugger);
            });
        }

        {
            let result = result.clone();
            let _ = history_action.connect_activate(move |_, _| {
                result.open_history();
            });
        }

        for (action, command) in [
            (
                state_select_slot_action,
                SessionCommand::SelectActiveSlot as fn(u64) -> SessionCommand,
            ),
            (state_save_slot_action, SessionCommand::SaveSlot),
            (state_load_slot_action, SessionCommand::LoadSlot),
            (state_delete_slot_action, SessionCommand::DeleteSlot),
        ] {
            let result = result.clone();
            let _ = action.connect_activate(move |_, parameter| {
                if let Some(slot_id) = parameter.and_then(|value| value.get::<u64>()) {
                    let _ = result
                        .state()
                        .borrow_mut()
                        .dispatch_session(SessionEvent::Command(command(slot_id)));
                    result.update_actions();
                }
            });
        }

        {
            let result = result.clone();
            let _ = settings_action.connect_activate(move |action, _| {
                open_settings(&result, action);
            });
        }

        {
            let result = result.clone();
            let _ = glib::timeout_add_local(TITLE_UPDATE_INTERVAL, move || {
                let due = result
                    .state()
                    .borrow_mut()
                    .title_refresh_due(std::time::Instant::now());
                if due {
                    result.refresh_title();
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
        let mut first_filter = None;
        for (name, extensions) in nerust_gui_shell::registry::rom_picker_filters(
            self.state().borrow().host.session().factories(),
        ) {
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
        let result = self.clone();
        let _ = file_chooser_native.connect_response(move |file_chooser_native, response| {
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

    fn close(&self) {
        self.state().borrow_mut().unload();
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
            state.host.menu_update(&ctx, settings_open)
        };
        if let Some(menu) = menu {
            let (mut visitor, presenter, mut next) = {
                let core = self.borrow();
                let visitor = GtkMenuVisitor {
                    menubar: core.menubar.clone(),
                    window: Some(core.window.clone()),
                    application: Some(core.application.clone()),
                    stack: Vec::new(),
                };
                let model = MenuModel::build(&menu, GTK_MENU_CAPABILITIES);
                let next = core.menu_presenter.submit(model);
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
            let outcome = self
                .state()
                .borrow_mut()
                .dispatch_session(SessionEvent::Keyboard {
                    key: controller_input,
                    pressed: key_event_pressed(event),
                });
            match outcome {
                Ok(outcome) if outcome.command.is_some() || outcome.effect.is_some() => {
                    self.apply_session_outcome(outcome);
                }
                Err(_) => self.update_actions(),
                _ => (),
            }
        }
        false
    }

    fn apply_keyboard_shortcut(&self, shortcut: KeyboardShortcut) {
        let outcome = self
            .state()
            .borrow_mut()
            .dispatch_session(SessionEvent::Shortcut(shortcut));
        match outcome {
            Ok(outcome) => self.apply_session_outcome(outcome),
            Err(_) => self.update_actions(),
        }
    }

    fn apply_session_outcome(&self, outcome: SessionOutcome) {
        match outcome.effect {
            Some(SessionEffect::ToggleFullscreen) => {
                toggle_window_fullscreen(self);
            }
            Some(SessionEffect::NativeOpenDebugger) => {
                self.open_debugger();
            }
            None => (),
        }
        self.update_actions();
    }
}

pub(crate) const GTK_MENU_CAPABILITIES: MenuCapabilities = MenuCapabilities {
    about: true,
    dialog_ellipsis: false,
};

/// A stack over persistent native objects, keyed by host semantic identity.
pub(crate) struct GtkMenuVisitor {
    menubar: MenubarModels,
    window: Option<gtk::ApplicationWindow>,
    application: Option<gtk::Application>,
    stack: Vec<(gio::Menu, bool)>,
}

impl GtkMenuVisitor {
    #[cfg(test)]
    pub(crate) fn new(menubar: MenubarModels) -> Self {
        Self {
            menubar,
            window: None,
            application: None,
            stack: Vec::new(),
        }
    }

    fn enabled(&self, name: &str, enabled: bool) {
        let action = if let Some(name) = name.strip_prefix("app.") {
            self.application
                .as_ref()
                .and_then(|app| app.lookup_action(name))
        } else {
            self.window
                .as_ref()
                .and_then(|window| window.lookup_action(name.strip_prefix("win.")?))
        };
        if let Some(action) = action.and_then(|action| action.downcast::<gio::SimpleAction>().ok())
        {
            action.set_enabled(enabled);
        }
    }
}

/// Native action names and target encoding are adapter policy, not host policy.
fn native_action(command: MenuCommand) -> Option<(&'static str, Option<u64>)> {
    use MenuCommand as C;
    use SessionCommand as S;
    Some(match command {
        C::Open => ("win.open", None),
        C::Close => ("win.close", None),
        C::Settings => ("win.settings", None),
        C::Debugger => ("win.debugger", None),
        C::NotificationHistory => ("win.history", None),
        C::Native(NativeRole::Quit) => ("app.quit", None),
        C::Native(NativeRole::About) => ("app.about", None),
        C::Session(command) => match command {
            S::Pause => ("win.pause", None),
            S::Resume => ("win.resume", None),
            S::Reset => ("win.reset", None),
            S::CreateSlot => ("win.state-create", None),
            S::SaveActiveSlotOrNew => ("win.state-save-active", None),
            S::LoadActiveSlot => ("win.state-load-active", None),
            S::SelectActiveSlot(id) => ("win.state-select-slot", Some(id)),
            S::SaveSlot(id) => ("win.state-save-slot", Some(id)),
            S::LoadSlot(id) => ("win.state-load-slot", Some(id)),
            S::DeleteSlot(id) => ("win.state-delete-slot", Some(id)),
            S::TogglePause | S::SelectNextSlot | S::SelectPreviousSlot => return None,
        },
    })
}

impl MenuVisitor for GtkMenuVisitor {
    fn begin_model(&mut self, rebuild: bool) {
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
        // Parameterized GTK actions also need gating when their submenu is empty.
        let action = match menu.id {
            MenuId::SelectSlot => Some("win.state-select-slot"),
            MenuId::SaveSlot => Some("win.state-save-slot"),
            MenuId::LoadSlot => Some("win.state-load-slot"),
            MenuId::DeleteSlot => Some("win.state-delete-slot"),
            _ => None,
        };
        if let Some(action) = action {
            self.enabled(action, menu.enabled);
        }
        if rebuild {
            native.remove_all();
        }
        self.stack.push((native, rebuild));
    }

    fn item(&mut self, item: &MenuItem) {
        let action = item.command.and_then(native_action);
        if let Some((name, _)) = action {
            self.enabled(name, item.enabled);
        }
        let (parent, rebuild) = self.stack.last().expect("host item parent");
        if *rebuild {
            let native = gio::MenuItem::new(Some(&item.label), None);
            if let Some((name, target)) = action {
                native.set_action_and_target_value(
                    Some(name),
                    target.map(|id| id.to_variant()).as_ref(),
                );
            }
            parent.append_item(&native);
        }
    }

    fn end_menu(&mut self, _id: MenuId) {
        self.stack.pop();
    }
    fn end_model(&mut self) {
        self.stack.pop();
        debug_assert!(self.stack.is_empty());
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
    use gtk::prelude::{ActionMapExt as _, Cast as _, DialogExt as _, GtkWindowExt as _};

    use super::{
        GTK_MENU_CAPABILITIES, GtkMenuVisitor, KeyEventState, MenuId, MenuModel, MenubarModels,
        WindowActions, WindowExtend, key_event_pressed,
    };
    use nerust_frontend_host::application::MenuSig;

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
        let action = win.borrow().actions.open.clone();
        let weak = Rc::downgrade(&win);
        let count = notifications.clone();
        let available = borrow_free.clone();
        action.connect_enabled_notify(move |_| {
            let win = weak.upgrade().unwrap();
            let state = win.state();
            let free = win.try_borrow_mut().is_ok()
                && state.try_borrow_mut().is_ok()
                && win.borrow().menubar.models.try_borrow_mut().is_ok();
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
        assert!(!win.borrow().actions.settings.is_enabled());
        assert!(super::close_aux(&win, id));
        win.update_actions();
        assert_eq!(notifications.get(), 2);
        assert!(borrow_free.get());
        assert!(action.is_enabled());
        assert!(win.borrow().actions.settings.is_enabled());
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
                    && win.borrow().menubar.models.try_borrow_mut().is_ok();
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
        let action = win.borrow().actions.open.clone();
        let quit = gio::SimpleAction::new("quit", None);
        win.application().add_action(&quit);
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
        assert!(win.borrow().actions.settings.is_enabled());
        assert!(win.borrow().actions.debugger.is_enabled());
        assert!(win.borrow().actions.history.is_enabled());
        assert!(
            quit.is_enabled(),
            "old outer traversal must not leave Quit disabled"
        );
        assert!(!win.borrow().actions.close.is_enabled());
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
                    let action = gio::SimpleAction::new("settings-test", None);
                    super::open_settings(&win, &action);
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
        let action = win.borrow().actions.settings.clone();
        action.activate(None);
        let old_id = win.borrow().aux.route_id(AuxKind::Settings).unwrap();
        let Some(NativeAux::Settings(old)) = win.borrow().aux.get(AuxKind::Settings).cloned()
        else {
            panic!("settings dialog");
        };
        super::open_settings(&win, &action);
        assert_eq!(win.borrow().aux.route_id(AuxKind::Settings), Some(old_id));
        win.update_actions();
        assert!(!action.is_enabled());
        old.response(gtk::ResponseType::Cancel);
        assert!(action.is_enabled());
        assert!(!win.borrow().aux.contains(AuxKind::Settings));
        assert!(!win.state().borrow().host.owns_pause(PauseOwner::Settings));
        super::open_settings(&win, &action);
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
            .build();
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
    fn action_cluster_names_every_command() {
        // One home for all command names: renaming a command updates
        // construction and wiring together, never just one side.
        let actions = WindowActions::new();
        for (action, name) in [
            (&actions.open, "open"),
            (&actions.settings, "settings"),
            (&actions.close, "close"),
            (&actions.pause, "pause"),
            (&actions.resume, "resume"),
            (&actions.reset, "reset"),
            (&actions.debugger, "debugger"),
            (&actions.history, "history"),
            (&actions.state_create, "state-create"),
            (&actions.state_save_active, "state-save-active"),
            (&actions.state_load_active, "state-load-active"),
            (&actions.state_select_slot, "state-select-slot"),
            (&actions.state_save_slot, "state-save-slot"),
            (&actions.state_load_slot, "state-load-slot"),
            (&actions.state_delete_slot, "state-delete-slot"),
        ] {
            assert_eq!(action.name().as_str(), name);
        }
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
        MenuModel::build(&sig, GTK_MENU_CAPABILITIES)
            .visit(None, &mut GtkMenuVisitor::new(bars.clone()));
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
        MenuModel::build(&sig, GTK_MENU_CAPABILITIES)
            .visit(None, &mut GtkMenuVisitor::new(bars.clone()));
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
