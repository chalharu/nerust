mod gdk_raw;
mod preferences;
mod renderer;
mod surface;
mod window;

use std::{
    cell::{Ref, RefCell, RefMut},
    path::Path,
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use gtk::{
    gio, glib,
    prelude::{
        ActionMapExt as _, ApplicationExt as _, ApplicationExtManual as _,
        ApplicationWindowExt as _, DialogExt as _, FileExt as _, GtkApplicationExt as _,
        GtkWindowExt as _, WidgetExt as _,
    },
};
use nerust_core_traits::factory::{CoreFactory, descriptor::SystemSettingsPageModel};
use nerust_gui_runtime::settings::{
    HostBackendCapabilities, HostWindowCapabilities, SettingsSnapshot,
};
use nerust_gui_settings::language::AppLanguage;
use nerust_gui_shell::{
    context::FrontendContext,
    session::{
        KeyboardShortcut, SessionError, SessionHandle,
        access::{FrontendSession, SettingsResult},
        commands::SessionCommand,
    },
    settings::{
        factory::settings_view,
        i18n::{UiText, text},
    },
};
use nerust_keyboard::Key;
use nerust_persistence::model::StateSlotSummary;
use nerust_render_traits::{FrameBuffer, VideoRenderProfile, renderer::GpuFactory};
use nerust_run_options::RunOptions;

use self::window::{StateMenus, Window, WindowExtend};

const TITLE_UPDATE_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Clone)]
pub(crate) struct StateRef(Rc<RefCell<Option<State>>>);

impl StateRef {
    fn borrow(&self) -> Ref<'_, State> {
        Ref::map(self.0.borrow(), |opt| opt.as_ref().unwrap())
    }

    fn borrow_mut(&self) -> RefMut<'_, State> {
        RefMut::map(self.0.borrow_mut(), |opt| opt.as_mut().unwrap())
    }

    fn try_borrow_mut(&self) -> Option<RefMut<'_, State>> {
        let inner = self.0.try_borrow_mut().ok()?;
        if inner.is_some() {
            Some(RefMut::map(inner, |opt| opt.as_mut().unwrap()))
        } else {
            None
        }
    }
}

impl From<State> for StateRef {
    fn from(state: State) -> Self {
        Self(Rc::new(RefCell::new(Some(state))))
    }
}

pub(crate) struct State {
    session: SessionHandle,
    ctx: FrontendContext,
    renderer_reload_pending: bool,
}

impl State {
    pub(crate) fn new(ctx: FrontendContext) -> Result<Self, SessionError> {
        let capabilities = HostBackendCapabilities {
            window: HostWindowCapabilities {
                remembers_window_size: true,
                supports_fullscreen_default: true,
                supports_scaling: true,
            },
            presentation: None,
        };
        let session = SessionHandle::new(
            capabilities,
            ctx.registry.clone(),
            ctx.audio_registry.clone(),
        )?;

        Ok(Self {
            session,
            ctx,
            renderer_reload_pending: false,
        })
    }

    pub(crate) fn swap_frame_buffer(&mut self) {
        self.session.swap_frame_buffer();
    }

    pub(crate) fn frame_buffer(&self) -> Option<&FrameBuffer> {
        self.session.frame_buffer()
    }

    pub(crate) fn active_factory(&self) -> Arc<dyn CoreFactory> {
        self.session
            .active_factory()
            .expect("no active system")
            .clone()
    }

    pub(crate) fn settings_pages(&self) -> Vec<(&'static str, SystemSettingsPageModel)> {
        let snapshot = self.session.settings_snapshot();
        self.ctx
            .registry
            .all()
            .iter()
            .map(|factory| {
                let system_id = factory.system_id();
                let view = settings_view(snapshot, &system_id);
                (factory.display_name(), factory.settings_page(&view))
            })
            .collect()
    }

    pub(crate) fn render_profile(&self) -> Option<&VideoRenderProfile> {
        self.session.render_profile()
    }

    pub(crate) fn can_pause(&self) -> bool {
        self.session.can_pause()
    }

    pub(crate) fn can_resume(&self) -> bool {
        self.session.can_resume()
    }

    pub(crate) fn load_path(&mut self, path: &Path) {
        if let Err(e) = self.ctx.rom_loader.load_rom(path, &mut self.session) {
            log::warn!("ROM load failed: {e}");
        }
    }

    pub(crate) fn loaded(&self) -> bool {
        self.session.loaded()
    }

    pub(crate) fn paused(&self) -> bool {
        self.session.paused()
    }

    pub(crate) fn title(&self) -> String {
        self.session.window_title()
    }

    pub(crate) fn unload(&mut self) -> bool {
        self.session.unload().is_ok()
    }

    pub(crate) fn flush_before_exit(&mut self) {
        self.session.flush_before_exit();
    }

    pub(crate) fn take_renderer_reload_pending(&mut self) -> bool {
        std::mem::take(&mut self.renderer_reload_pending)
    }
}

impl FrontendSession for State {
    fn pause(&mut self) {
        let _ = self.session.run_command(SessionCommand::Pause);
    }
    fn resume(&mut self) {
        let _ = self.session.run_command(SessionCommand::Resume);
    }
    fn toggle_pause(&mut self) {
        let _ = self.session.run_command(SessionCommand::TogglePause);
    }
    fn save_active_slot(&mut self) {
        let _ = self
            .session
            .run_command(SessionCommand::SaveActiveSlotOrNew);
    }
    fn load_active_slot(&mut self) -> bool {
        self.session
            .run_command(SessionCommand::LoadActiveSlot)
            .unwrap_or_default()
            .executed
    }
    fn select_next_slot(&mut self) {
        let _ = self.session.run_command(SessionCommand::SelectNextSlot);
    }
    fn select_previous_slot(&mut self) {
        let _ = self.session.run_command(SessionCommand::SelectPreviousSlot);
    }
    fn load_slot(&mut self, slot_id: u64) -> bool {
        self.session
            .run_command(SessionCommand::LoadSlot(slot_id))
            .unwrap_or_default()
            .executed
    }
    fn save_slot(&mut self, slot_id: u64) {
        let _ = self.session.run_command(SessionCommand::SaveSlot(slot_id));
    }
    fn delete_slot(&mut self, slot_id: u64) {
        let _ = self
            .session
            .run_command(SessionCommand::DeleteSlot(slot_id));
    }
    fn select_slot(&mut self, slot_id: u64) {
        let _ = self
            .session
            .run_command(SessionCommand::SelectActiveSlot(slot_id));
    }
    fn create_slot(&mut self) {
        let _ = self.session.run_command(SessionCommand::CreateSlot);
    }
    fn reset(&mut self) {
        let _ = self.session.run_command(SessionCommand::Reset);
    }
    fn run_command(&mut self, command: SessionCommand) {
        let _ = self.session.run_command(command);
    }
    fn apply_settings(
        &mut self,
        settings: SettingsSnapshot,
    ) -> Result<SettingsResult, SessionError> {
        let plan = self.session.apply_settings(settings)?;
        if plan.session_rebuild_required || plan.window_settings_changed {
            self.renderer_reload_pending = true;
        }
        Ok(SettingsResult {
            renderer_needs_rebuild: self.renderer_reload_pending,
            fullscreen_default_changed: plan.fullscreen_default_changed,
            scaling_changed: plan.scaling_changed,
        })
    }
    fn set_fullscreen_default(&mut self, fullscreen: bool) -> Result<SettingsResult, SessionError> {
        let plan = self.session.set_fullscreen_default(fullscreen)?;
        if plan.session_rebuild_required || plan.window_settings_changed {
            self.renderer_reload_pending = true;
        }
        Ok(SettingsResult {
            renderer_needs_rebuild: self.renderer_reload_pending,
            fullscreen_default_changed: plan.fullscreen_default_changed,
            scaling_changed: false,
        })
    }
}

impl State {
    pub(crate) fn handle_keyboard_key(
        &mut self,
        key: Key,
        pressed: bool,
    ) -> Option<KeyboardShortcut> {
        self.session.handle_keyboard_key(key, pressed)
    }

    pub(crate) fn clear_input(&mut self) {
        self.session.clear_input();
    }

    pub(crate) fn slots(&self) -> &[StateSlotSummary] {
        self.session.slots()
    }

    pub(crate) fn active_slot_id(&self) -> Option<u64> {
        self.session.active_slot_id()
    }

    pub(crate) fn settings_snapshot(&self) -> &SettingsSnapshot {
        self.session.settings_snapshot()
    }
}

fn build_window(app: &gtk::Application, factory: &Rc<dyn GpuFactory>, state: StateRef) -> Window {
    let builder = gtk::Builder::from_string(include_str!("../resources/ui.xml"));
    let window: gtk::ApplicationWindow = builder.object("window").unwrap();
    let state_menu = gio::Menu::new();
    let select_active_slot_menu = gio::Menu::new();
    let save_slot_menu = gio::Menu::new();
    let load_slot_menu = gio::Menu::new();
    let delete_slot_menu = gio::Menu::new();

    let language = state.borrow().settings_snapshot().shared.general.language;
    let menu_model = build_menu_model(
        language,
        &state_menu,
        &select_active_slot_menu,
        &save_slot_menu,
        &load_slot_menu,
        &delete_slot_menu,
    );

    app.set_menubar(Some(&menu_model));
    app.add_window(&window);
    window.set_show_menubar(true);

    let quit_action = gio::SimpleAction::new("quit", None);
    {
        let window = window.clone();
        let _ = quit_action.connect_activate(move |_, _| {
            window.close();
        });
    }
    app.add_action(&quit_action);

    fn create_about_dialog() -> gtk::AboutDialog {
        gtk::Builder::from_string(include_str!("../resources/about.xml"))
            .object("about")
            .unwrap()
    }
    let about_action = gio::SimpleAction::new("about", None);
    {
        let window = window.clone();
        let window_about: Rc<RefCell<Option<gtk::AboutDialog>>> = Rc::new(RefCell::new(None));
        let _ = about_action.connect_activate(move |_, _| {
            if let Some(dialog) = window_about.borrow().as_ref() {
                dialog.present();
                return;
            }

            let dialog = create_about_dialog();
            dialog.set_transient_for(Some(&window));

            let window_about_on_close = window_about.clone();
            let _ = dialog.connect_close_request(move |_| {
                *window_about_on_close.borrow_mut() = None;
                glib::Propagation::Proceed
            });

            dialog.present();
            *window_about.borrow_mut() = Some(dialog);
        });
    }
    app.add_action(&about_action);

    Window::bind(
        app.clone(),
        window,
        state,
        Rc::clone(factory),
        StateMenus {
            select_active_slot_menu,
            save_slot_menu,
            load_slot_menu,
            delete_slot_menu,
        },
    )
}

pub(crate) fn build_menu_model(
    language: AppLanguage,
    state_menu: &gio::Menu,
    select_active_slot_menu: &gio::Menu,
    save_slot_menu: &gio::Menu,
    load_slot_menu: &gio::Menu,
    delete_slot_menu: &gio::Menu,
) -> gio::Menu {
    state_menu.remove_all();
    state_menu.append(
        Some(text(language, UiText::CreateSaveSlot)),
        Some("win.state-create"),
    );
    state_menu.append(
        Some(text(language, UiText::SaveActiveSlot)),
        Some("win.state-save-active"),
    );
    state_menu.append(
        Some(text(language, UiText::LoadActiveSlot)),
        Some("win.state-load-active"),
    );
    state_menu.append_submenu(
        Some(text(language, UiText::SelectActiveSlot)),
        select_active_slot_menu,
    );
    state_menu.append_submenu(Some(text(language, UiText::SaveSlot)), save_slot_menu);
    state_menu.append_submenu(Some(text(language, UiText::LoadSlot)), load_slot_menu);
    state_menu.append_submenu(Some(text(language, UiText::DeleteSlot)), delete_slot_menu);

    let file_menu = gio::Menu::new();
    file_menu.append(Some(text(language, UiText::Open)), Some("win.open"));
    file_menu.append(Some(text(language, UiText::Close)), Some("win.close"));
    file_menu.append(Some(text(language, UiText::Settings)), Some("win.settings"));
    file_menu.append(Some(text(language, UiText::Quit)), Some("app.quit"));

    let emulation_menu = gio::Menu::new();
    emulation_menu.append(Some(text(language, UiText::Pause)), Some("win.pause"));
    emulation_menu.append(Some(text(language, UiText::Resume)), Some("win.resume"));
    emulation_menu.append_submenu(Some(text(language, UiText::SaveStates)), state_menu);

    let help_menu = gio::Menu::new();
    help_menu.append(Some(text(language, UiText::About)), Some("app.about"));

    let menu = gio::Menu::new();
    menu.append_submenu(Some(text(language, UiText::File)), &file_menu);
    menu.append_submenu(Some(text(language, UiText::Emulation)), &emulation_menu);
    menu.append_submenu(Some(text(language, UiText::About)), &help_menu);
    menu
}

fn ensure_window(
    app: &gtk::Application,
    factory: &Rc<dyn GpuFactory>,
    state: &StateRef,
    current_window: &Rc<RefCell<Option<Window>>>,
) -> Window {
    if let Some(window) = current_window.borrow().as_ref().cloned() {
        return window;
    }

    let window = build_window(app, factory, state.clone());
    *current_window.borrow_mut() = Some(window.clone());
    window
}

pub fn run(ctx: FrontendContext, _options: RunOptions) {
    let app = gtk::Application::new(
        Some("com.github.chalharu"),
        gio::ApplicationFlags::HANDLES_OPEN,
    );

    let current_window = Rc::new(RefCell::new(None));
    let state: StateRef = match State::new(ctx) {
        Ok(s) => s.into(),
        Err(e) => {
            let err = e.to_string();
            let _state = StateRef(Rc::new(RefCell::new(None)));
            let _ = app.connect_activate(move |_app| {
                let dialog = gtk::MessageDialog::new(
                    None::<&gtk::ApplicationWindow>,
                    gtk::DialogFlags::MODAL,
                    gtk::MessageType::Error,
                    gtk::ButtonsType::Ok,
                    format!("Failed to initialize: {err}"),
                );
                dialog.connect_response(|dialog, _| {
                    dialog.close();
                    std::process::exit(1);
                });
                dialog.show();
            });
            let _ = app.run();
            return;
        }
    };
    {
        let state = state.clone();
        let gpu_factory = state.borrow().ctx.gpu_factory.clone();
        let current_window = current_window.clone();
        let _ = app.connect_activate(move |app| {
            let window = ensure_window(app, &gpu_factory, &state, &current_window);
            window.window().present();
        });
    }
    {
        let state = state.clone();
        let gpu_factory = state.borrow().ctx.gpu_factory.clone();
        let current_window = current_window.clone();
        let _ = app.connect_open(move |app, files, _| {
            let window = ensure_window(app, &gpu_factory, &state, &current_window);
            if let Some(path) = files.iter().find_map(|file| file.path()) {
                state.borrow_mut().load_path(&path);
                window.update_actions();
            }
            window.window().present();
        });
    }

    let _ = app.run();
}
