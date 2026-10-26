use std::{cell::RefCell, path::Path, rc::Rc};

use gtk::{
    gio, glib,
    glib::variant::{StaticVariantType, ToVariant},
    prelude::*,
};
use nerust_frontend_host::{
    aux::{AuxId, AuxWindows, OpenRequest},
    focus::AuxKind,
    session::{SessionEffect, SessionEvent, SessionOutcome},
};
use nerust_gui_runtime::slots::slot_label;
use nerust_gui_shell::session::{
    KeyboardShortcut, SessionError, access::SessionControl, commands::SessionCommand,
};
use nerust_persistence::model::StateSlotSummary;
use nerust_render_traits::renderer::PipelineFactory;
use nerust_settings_core::model::local::ScalingMode;

use super::{
    State, TITLE_UPDATE_INTERVAL, build_menu_model,
    surface::{Surface, SurfaceExtend},
    surface_state::surface_gone,
};
use crate::preferences::present_preferences_dialog;

pub(crate) struct StateMenus {
    pub(crate) select_active_slot_menu: gio::Menu,
    pub(crate) save_slot_menu: gio::Menu,
    pub(crate) load_slot_menu: gio::Menu,
    pub(crate) delete_slot_menu: gio::Menu,
}

/// Persistent menubar skeleton: the top-level model plus its section menus.
/// Built once, bound once via `set_menubar`; language switches refill these
/// same objects in place because replacing the top-level model at runtime
/// does not propagate to the visible menubar.
pub(crate) struct MenubarModels {
    pub(crate) menu_model: gio::Menu,
    pub(crate) file_menu: gio::Menu,
    pub(crate) state_menu: gio::Menu,
    pub(crate) emulation_menu: gio::Menu,
    pub(crate) help_menu: gio::Menu,
}

impl MenubarModels {
    pub(crate) fn new() -> Self {
        Self {
            menu_model: gio::Menu::new(),
            file_menu: gio::Menu::new(),
            state_menu: gio::Menu::new(),
            emulation_menu: gio::Menu::new(),
            help_menu: gio::Menu::new(),
        }
    }
}

pub(crate) struct WindowCore {
    application: gtk::Application,
    window: gtk::ApplicationWindow,
    state: Rc<RefCell<State>>,
    open_dialog: Option<gtk::FileChooserNative>,
    aux: AuxWindows<u64, NativeAux>,
    actions: WindowActions,
    menubar: MenubarModels,
    select_active_slot_menu: gio::Menu,
    save_slot_menu: gio::Menu,
    load_slot_menu: gio::Menu,
    delete_slot_menu: gio::Menu,
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

fn open_settings(window: &Window, action: &gio::SimpleAction) {
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
    action.set_enabled(false);
    let action = action.clone();
    let weak = Rc::downgrade(window);
    let dialog = present_preferences_dialog(&window.window(), window.state(), move || {
        if let Some(window) = weak.upgrade()
            && close_aux(&window, id)
        {
            action.set_enabled(true);
            window.rebuild_menubar();
            window.update_actions();
        }
    });
    if let Some(dialog) = dialog {
        complete_aux(window, id, NativeAux::Settings(dialog));
    }
}

/// Window action cluster: every menu/toolbar command in one place.
/// `WindowCore` owns one; wiring and gating go through these narrow ports.
pub(crate) struct WindowActions {
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
    /// Fresh disabled-by-default actions; `update` gates them per tick.
    pub fn new() -> Self {
        Self {
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

    /// Seat every action on the window: one call instead of twelve.
    pub fn add_to(&self, window: &gtk::ApplicationWindow) {
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
        state_menus: StateMenus,
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
        state_menus: StateMenus,
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
        let settings_action = gio::SimpleAction::new("settings", None);
        let _ = Surface::bind(&window, &picture, state.clone(), factory);
        actions.add_to(&window);
        let result = Rc::new(RefCell::new(WindowCore {
            application,
            window: window.clone(),
            state,
            open_dialog: None,
            aux: AuxWindows::new(),
            actions,
            menubar,
            select_active_slot_menu: state_menus.select_active_slot_menu,
            save_slot_menu: state_menus.save_slot_menu,
            load_slot_menu: state_menus.load_slot_menu,
            delete_slot_menu: state_menus.delete_slot_menu,
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
                if !window.is_active() {
                    result.state().borrow_mut().clear_input();
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

        let open_action = gio::SimpleAction::new("open", None);

        {
            let result = result.clone();
            let _ = open_action.connect_activate(move |_, _| {
                result.open();
            });
        }
        window.add_action(&open_action);

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
        window.add_action(&settings_action);

        {
            let result = result.clone();
            let _ = glib::timeout_add_local(TITLE_UPDATE_INTERVAL, move || {
                result.refresh_title();
                glib::ControlFlow::Continue
            });
        }

        result.rebuild_menubar();

        result.update_actions();
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
            self.state().borrow().session.factories(),
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
        self.state().borrow_mut().flush_before_exit();
        let w = self.window().width();
        let h = self.window().height();
        if w > 0 && h > 0 {
            let state = self.state();
            let _ = state
                .borrow_mut()
                .session
                .update_window_size(w as u32, h as u32);
        }
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
        let state = state.borrow();
        let tick = state.session.tick_metrics();
        self.borrow().actions.close.set_enabled(tick.loaded);
        self.borrow().actions.pause.set_enabled(tick.can_pause());
        self.borrow().actions.resume.set_enabled(tick.can_resume());
        self.borrow().actions.reset.set_enabled(tick.loaded);
        // Coreless open is a designed state: gating on `loaded` would disagree
        // with the shortcut, so the action stays enabled.
        self.borrow().actions.debugger.set_enabled(true);
        self.borrow().actions.history.set_enabled(true);
        self.borrow().actions.state_create.set_enabled(tick.loaded);
        self.borrow()
            .actions
            .state_save_active
            .set_enabled(tick.loaded);
        self.borrow()
            .actions
            .state_load_active
            .set_enabled(state.active_slot_id().is_some());
        self.borrow()
            .actions
            .state_select_slot
            .set_enabled(tick.loaded && !state.slots().is_empty());
        self.borrow()
            .actions
            .state_save_slot
            .set_enabled(tick.loaded && !state.slots().is_empty());
        self.borrow()
            .actions
            .state_load_slot
            .set_enabled(tick.loaded && !state.slots().is_empty());
        self.borrow()
            .actions
            .state_delete_slot
            .set_enabled(tick.loaded && !state.slots().is_empty());

        let language = state.settings_snapshot().shared.general.language;
        rebuild_slot_menu(
            &self.borrow().select_active_slot_menu,
            state.slots(),
            state.active_slot_id(),
            "win.state-select-slot",
            language,
        );
        rebuild_slot_menu(
            &self.borrow().save_slot_menu,
            state.slots(),
            state.active_slot_id(),
            "win.state-save-slot",
            language,
        );
        rebuild_slot_menu(
            &self.borrow().load_slot_menu,
            state.slots(),
            state.active_slot_id(),
            "win.state-load-slot",
            language,
        );
        rebuild_slot_menu(
            &self.borrow().delete_slot_menu,
            state.slots(),
            state.active_slot_id(),
            "win.state-delete-slot",
            language,
        );
        drop(state);
        self.refresh_title();
    }

    fn rebuild_menubar(&self) {
        let state = self.state();
        let guarded = state.borrow();
        let snapshot = guarded.settings_snapshot();
        // In-place refill only, no set_menubar: replacing the top-level
        // model at runtime leaves the visible menubar stale, while the
        // binding set once at build propagates label updates on these same
        // objects. Slot submenu contents are deliberately untouched here.
        let core = self.borrow();
        build_menu_model(
            snapshot.shared.general.language,
            &snapshot.shared.input.shortcuts.keyboard,
            &core.menubar,
            &core.select_active_slot_menu,
            &core.save_slot_menu,
            &core.load_slot_menu,
            &core.delete_slot_menu,
        );
    }

    fn refresh_title(&self) {
        let title = self.state().borrow_mut().title_cached();
        self.window().set_title(Some(title.as_str()));
    }

    fn sync_fullscreen_from_settings(&self) {
        let state = self.state();
        let borrowed = state.borrow();
        let snapshot = borrowed.settings_snapshot();
        let fullscreen = snapshot.local.video.window.fullscreen_default;

        // Restore remembered window size when not in fullscreen or scaling mode.
        if !fullscreen
            && snapshot.local.video.window.scaling == ScalingMode::FitToWindow
            && let Some(size) = snapshot.app_state.window_size("main")
        {
            self.window()
                .set_default_size(size.width as i32, size.height as i32);
        }

        set_window_fullscreen(&self.window(), fullscreen);
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

fn rebuild_slot_menu(
    menu: &gio::Menu,
    slots: &[StateSlotSummary],
    active_slot: Option<u64>,
    action: &str,
    language: nerust_settings_core::model::language::AppLanguage,
) {
    use nerust_settings_core::i18n::{UiText, text};
    menu.remove_all();
    if slots.is_empty() {
        // An action-less item renders disabled: the empty submenu
        // explains itself instead of showing a blank list.
        let item = gio::MenuItem::new(Some(text(language, UiText::NoSaveSlots)), None);
        menu.append_item(&item);
        return;
    }
    for slot in slots {
        let item = gio::MenuItem::new(Some(&slot_label(slot, active_slot)), None);
        item.set_action_and_target_value(Some(action), Some(&slot.slot_id.to_variant()));
        menu.append_item(&item);
    }
}

/// Use the single modal queue, including its pause ownership and acknowledgement path.
fn show_rom_load_error(_parent: &gtk::ApplicationWindow, state: Rc<RefCell<State>>, detail: &str) {
    state.borrow_mut().session.report_external_error(
        nerust_gui_shell::session::notification::FailureOrigin::Host,
        detail.to_owned(),
    );
}

#[cfg(test)]
mod tests {
    use gio::prelude::ActionExt as _;
    use gtk::prelude::{DialogExt as _, GtkWindowExt as _};

    use super::{KeyEventState, WindowActions, WindowExtend, key_event_pressed, rebuild_slot_menu};
    use crate::surface_state::surface_gone;

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
            super::StateMenus {
                select_active_slot_menu: gio::Menu::new(),
                save_slot_menu: gio::Menu::new(),
                load_slot_menu: gio::Menu::new(),
                delete_slot_menu: gio::Menu::new(),
            },
            super::MenubarModels::new(),
        ))
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
        let action = gio::SimpleAction::new("settings-test", None);
        super::open_settings(&win, &action);
        let old_id = win.borrow().aux.route_id(AuxKind::Settings).unwrap();
        let Some(NativeAux::Settings(old)) = win.borrow().aux.get(AuxKind::Settings).cloned()
        else {
            panic!("settings dialog");
        };
        super::open_settings(&win, &action);
        assert_eq!(win.borrow().aux.route_id(AuxKind::Settings), Some(old_id));
        assert!(!action.is_enabled());
        old.response(gtk::ResponseType::Cancel);
        assert!(action.is_enabled());
        assert!(!win.borrow().aux.contains(AuxKind::Settings));
        assert!(!win.state().borrow().pause_policy.owns(PauseOwner::Settings));
        super::open_settings(&win, &action);
        let new_id = win.borrow().aux.route_id(AuxKind::Settings).unwrap();
        assert_ne!(old_id, new_id);
        assert!(!super::close_aux(&win, old_id));
        old.destroy();
        assert!(win.state().borrow().pause_policy.owns(PauseOwner::Settings));
        assert!(!action.is_enabled());
        win.state()
            .borrow_mut()
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
        assert!(!win.state().borrow().pause_policy.owns(PauseOwner::Settings));
        assert!(win.state().borrow().pause_policy.owns(PauseOwner::Modal));
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

    fn slot_menu_count(menu: &gio::Menu) -> i32 {
        use gio::prelude::MenuModelExt;
        menu.n_items()
    }

    #[test]
    fn slot_menu_explains_empty_state() {
        use nerust_settings_core::model::language::AppLanguage;
        let menu = gio::Menu::new();
        rebuild_slot_menu(
            &menu,
            &[],
            None,
            "win.state-save-slot",
            AppLanguage::English,
        );
        // One disabled item names the empty state, never a blank list.
        assert_eq!(slot_menu_count(&menu), 1);
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
        let menu = gio::Menu::new();
        rebuild_slot_menu(
            &menu,
            &slots,
            Some(2),
            "win.state-load-slot",
            AppLanguage::English,
        );
        assert_eq!(slot_menu_count(&menu), 2);
    }
}
