mod debugger;
mod gdk_raw;
mod osd;
mod ppu_window;
mod preferences;
mod renderer;
mod surface;
mod surface_state;
#[cfg(test)]
pub(crate) mod test_support;
mod window;

use std::{cell::RefCell, path::Path, rc::Rc, sync::Arc, time::Duration};

use gtk::{
    gdk::prelude::SurfaceExt as _,
    gio, glib,
    prelude::{
        ActionMapExt as _, ApplicationExt as _, ApplicationExtManual as _,
        ApplicationWindowExt as _, FileExt as _, GtkApplicationExt as _, GtkWindowExt as _,
        NativeExt as _, WidgetExt as _,
    },
};
use nerust_gui_runtime::settings::{
    HostBackendCapabilities, HostWindowCapabilities, SettingsSnapshot,
};
use nerust_gui_shell::{
    context::FrontendContext,
    session::{
        KeyboardShortcut, SessionError, SessionHandle,
        access::{PauseControl, ResetControl, SessionControl, SettingsResult, SlotIo, SlotNav},
        commands::SessionCommand,
    },
};
use nerust_keyboard::Key;
use nerust_persistence::model::StateSlotSummary;
use nerust_render_traits::{FrameBuffer, VideoRenderProfile, renderer::PipelineFactory};
use nerust_run_options::RunOptions;
use nerust_settings_core::i18n::{UiText, menu_label_with_shortcut, text};
use nerust_settings_core::model::input::{ShortcutAction, ShortcutBinding, shortcut_key_label};
use nerust_settings_core::model::language::AppLanguage;

use self::window::{MenubarModels, StateMenus, Window, WindowExtend};

const TITLE_UPDATE_INTERVAL: Duration = Duration::from_millis(500);

pub(crate) struct State {
    session: SessionHandle,
    ctx: FrontendContext,
    renderer_reload_pending: bool,
    pause_policy: nerust_frontend_kernel::pause::PausePolicy,
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
        Self::new_inner(ctx, capabilities, None)
    }

    /// Test-only construction: settings persist under a unique temp dir.
    #[cfg(test)]
    pub(crate) fn new_for_tests(ctx: FrontendContext) -> Result<Self, SessionError> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static TEST_SETTINGS_SEQ: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "nerust-gtk-test-{}-{}",
            std::process::id(),
            TEST_SETTINGS_SEQ.fetch_add(1, Ordering::SeqCst)
        ));
        let capabilities = HostBackendCapabilities {
            window: HostWindowCapabilities {
                remembers_window_size: false,
                supports_fullscreen_default: true,
                supports_scaling: true,
            },
            presentation: None,
        };
        Self::new_inner(
            ctx,
            capabilities,
            Some(nerust_gui_runtime::settings::SettingsPaths {
                settings_file: dir.join("settings.toml"),
                central_storage_root: dir.join("storage"),
            }),
        )
    }

    fn new_inner(
        ctx: FrontendContext,
        capabilities: HostBackendCapabilities,
        settings_paths: Option<nerust_gui_runtime::settings::SettingsPaths>,
    ) -> Result<Self, SessionError> {
        let session = match settings_paths {
            Some(paths) => SessionHandle::new_with_settings_paths(
                capabilities,
                ctx.registry.clone(),
                ctx.audio_registry.clone(),
                paths,
            ),
            None => SessionHandle::new(
                capabilities,
                ctx.registry.clone(),
                ctx.audio_registry.clone(),
            ),
        }
        .map_err(|e| {
            log::error!("failed to create core: {e}");
            e
        })?;

        Ok(Self {
            session,
            ctx,
            renderer_reload_pending: false,
            pause_policy: Default::default(),
        })
    }

    pub(crate) fn swap_frame_buffer(&mut self) {
        self.session.swap_frame_buffer();
    }

    pub(crate) fn frame_buffer(&self) -> Option<&FrameBuffer> {
        self.session.frame_buffer()
    }

    pub(crate) fn render_profile(&self) -> Option<&VideoRenderProfile> {
        self.session.render_profile()
    }

    /// Load a ROM, flagging a renderer rebuild on success.
    /// Returns the failure detail; presenting it is the caller's job.
    pub(crate) fn load_path(&mut self, path: &Path) -> Result<(), String> {
        if let Err(error) = self.ctx.rom_loader.load_rom(path, &mut self.session) {
            log::warn!("ROM load failed: {error}");
            return Err(error.to_string());
        }
        if self.session.tick_metrics().loaded {
            self.renderer_reload_pending = true;
        }
        self.reconcile_pause();
        Ok(())
    }

    pub(crate) fn paused(&self) -> bool {
        self.session.tick_metrics().paused
    }

    pub(crate) fn title_cached(&mut self) -> String {
        let tick = self.session.tick_metrics();
        self.session.window_title_cached(&tick).to_owned()
    }

    pub(crate) fn flush_before_exit(&mut self) {
        self.session.flush_before_exit();
    }

    pub(crate) fn history_lines_cached(
        &mut self,
        language: nerust_settings_core::model::language::AppLanguage,
    ) -> &[nerust_gui_shell::session::osd::HistoryLine] {
        self.session.notification_history_lines_cached(language)
    }

    /// History generation for the refresh gate: unchanged content
    /// skips the full row rebuild (same contract as the tao twin).
    pub(crate) fn notification_history_seq(&self) -> u64 {
        self.session.notification_history_seq()
    }

    pub(crate) fn clear_notification_history(&mut self) {
        self.session.clear_notification_history();
    }
}

impl PauseControl for State {
    fn pause(&mut self) {
        self.set_pause_owner(nerust_frontend_kernel::pause::PauseOwner::User, true);
    }
    fn resume(&mut self) {
        self.set_pause_owner(nerust_frontend_kernel::pause::PauseOwner::User, false);
    }
    fn toggle_pause(&mut self) {
        self.set_pause_owner(
            nerust_frontend_kernel::pause::PauseOwner::User,
            !self.paused(),
        );
    }
}

impl State {
    fn reconcile_pause(&mut self) {
        let tick = self.session.tick_metrics();
        self.session.set_execution_held(self.pause_policy.paused());
        if self.pause_policy.paused() && tick.loaded && !tick.paused {
            let _ = self.session.run_command(SessionCommand::Pause);
        }
    }

    pub(crate) fn set_pause_owner(
        &mut self,
        owner: nerust_frontend_kernel::pause::PauseOwner,
        active: bool,
    ) {
        let tick = self.session.tick_metrics();
        let paused = self.pause_policy.set(owner, active, tick.paused);
        self.session.set_execution_held(paused);
        if tick.loaded && tick.paused != paused {
            let _ = self.session.run_command(if paused {
                SessionCommand::Pause
            } else {
                SessionCommand::Resume
            });
        }
    }
}

impl SlotIo for State {
    fn save_active_slot(&mut self) -> bool {
        self.session
            .run_command(SessionCommand::SaveActiveSlotOrNew)
            .unwrap_or_default()
            .executed
    }
    fn load_active_slot(&mut self) -> bool {
        self.session
            .run_command(SessionCommand::LoadActiveSlot)
            .unwrap_or_default()
            .executed
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
}

impl SlotNav for State {
    fn select_next_slot(&mut self) {
        let _ = self.session.run_command(SessionCommand::SelectNextSlot);
    }
    fn select_previous_slot(&mut self) {
        let _ = self.session.run_command(SessionCommand::SelectPreviousSlot);
    }
    fn select_slot(&mut self, slot_id: u64) {
        let _ = self
            .session
            .run_command(SessionCommand::SelectActiveSlot(slot_id));
    }
    fn create_slot(&mut self) {
        let _ = self.session.run_command(SessionCommand::CreateSlot);
    }
}

impl ResetControl for State {
    fn reset(&mut self) {
        let _ = self.session.run_command(SessionCommand::Reset);
    }
}

impl SessionControl for State {
    fn unload(&mut self) {
        if let Err(error) = self.session.unload() {
            log::warn!("ROM unload failed: {error}");
        }
    }
    fn apply_settings(
        &mut self,
        settings: SettingsSnapshot,
    ) -> Result<SettingsResult, SessionError> {
        let plan = self.session.apply_settings(settings);
        self.reconcile_pause();
        let plan = plan?;
        if plan.renderer_rebuild_required || plan.window_settings_changed {
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
        if plan.renderer_rebuild_required || plan.window_settings_changed {
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

    /// Share the authoritative session allocation without structural comparison.
    pub(crate) fn settings_arc(&self) -> Arc<SettingsSnapshot> {
        self.session.settings_arc()
    }
}

fn build_window(
    app: &gtk::Application,
    factory: &Rc<dyn PipelineFactory>,
    state: Rc<RefCell<State>>,
) -> Window {
    let builder = gtk::Builder::from_string(include_str!("../resources/ui.xml"));
    let window: gtk::ApplicationWindow = builder.object("window").unwrap();
    let menubar = MenubarModels::new();
    let select_active_slot_menu = gio::Menu::new();
    let save_slot_menu = gio::Menu::new();
    let load_slot_menu = gio::Menu::new();
    let delete_slot_menu = gio::Menu::new();

    let language = state.borrow().settings_snapshot().shared.general.language;
    let shortcuts = state
        .borrow()
        .settings_snapshot()
        .shared
        .input
        .shortcuts
        .keyboard
        .clone();
    build_menu_model(
        language,
        &shortcuts,
        &menubar,
        &select_active_slot_menu,
        &save_slot_menu,
        &load_slot_menu,
        &delete_slot_menu,
    );

    app.set_menubar(Some(&menubar.menu_model));
    app.add_window(&window);
    window.set_show_menubar(true);
    // Game viewport: a memory picture composited by GTK; backends never present here.
    let picture = gtk::Picture::new();
    picture.set_hexpand(true);
    picture.set_vexpand(true);
    picture.set_can_shrink(true);
    window.set_child(Some(&picture));

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
        picture,
        state,
        Rc::clone(factory),
        StateMenus {
            select_active_slot_menu,
            save_slot_menu,
            load_slot_menu,
            delete_slot_menu,
        },
        menubar,
    )
}

/// Menu label with the action's *current* shortcut hint, or the bare label.
/// Binding-driven, so Settings rebinding never leaves a stale hardcoded key.
fn shortcut_hint(base: &str, shortcuts: &[ShortcutBinding], action: ShortcutAction) -> String {
    match shortcut_key_label(shortcuts, action) {
        Some(key) => menu_label_with_shortcut(base, key),
        None => base.to_string(),
    }
}

pub(crate) fn build_menu_model(
    language: AppLanguage,
    shortcuts: &[ShortcutBinding],
    bars: &MenubarModels,
    select_active_slot_menu: &gio::Menu,
    save_slot_menu: &gio::Menu,
    load_slot_menu: &gio::Menu,
    delete_slot_menu: &gio::Menu,
) {
    // In-place refill of the persistent skeleton: the same objects stay
    // bound to the application, so runtime relabeling propagates.
    let state_menu = &bars.state_menu;
    state_menu.remove_all();
    state_menu.append(
        Some(text(language, UiText::CreateSaveSlot)),
        Some("win.state-create"),
    );
    state_menu.append(
        Some(&shortcut_hint(
            text(language, UiText::SaveActiveSlot),
            shortcuts,
            ShortcutAction::SaveActiveSlot,
        )),
        Some("win.state-save-active"),
    );
    state_menu.append(
        Some(&shortcut_hint(
            text(language, UiText::LoadActiveSlot),
            shortcuts,
            ShortcutAction::LoadActiveSlot,
        )),
        Some("win.state-load-active"),
    );
    state_menu.append_submenu(
        Some(text(language, UiText::SelectActiveSlot)),
        select_active_slot_menu,
    );
    state_menu.append_submenu(Some(text(language, UiText::SaveSlot)), save_slot_menu);
    state_menu.append_submenu(Some(text(language, UiText::LoadSlot)), load_slot_menu);
    state_menu.append_submenu(Some(text(language, UiText::DeleteSlot)), delete_slot_menu);

    let file_menu = &bars.file_menu;
    file_menu.remove_all();
    file_menu.append(Some(text(language, UiText::Open)), Some("win.open"));
    file_menu.append(Some(text(language, UiText::Close)), Some("win.close"));
    file_menu.append(Some(text(language, UiText::Settings)), Some("win.settings"));
    file_menu.append(Some(text(language, UiText::Quit)), Some("app.quit"));

    // Order mirrors the tao frontend: Pause, Resume, Reset, Save States,
    // Debugger. The debugger label carries its live shortcut like tao.
    let emulation_menu = &bars.emulation_menu;
    emulation_menu.remove_all();
    emulation_menu.append(Some(text(language, UiText::Pause)), Some("win.pause"));
    emulation_menu.append(Some(text(language, UiText::Resume)), Some("win.resume"));
    emulation_menu.append(Some(text(language, UiText::Reset)), Some("win.reset"));
    emulation_menu.append_submenu(Some(text(language, UiText::SaveStates)), state_menu);
    emulation_menu.append(
        Some(&shortcut_hint(
            text(language, UiText::OpenDebugger),
            shortcuts,
            ShortcutAction::OpenDebugger,
        )),
        Some("win.debugger"),
    );
    emulation_menu.append(
        Some(text(language, UiText::NotificationHistory)),
        Some("win.history"),
    );

    let help_menu = &bars.help_menu;
    help_menu.remove_all();
    help_menu.append(Some(text(language, UiText::About)), Some("app.about"));

    let menu = &bars.menu_model;
    menu.remove_all();
    menu.append_submenu(Some(text(language, UiText::File)), file_menu);
    menu.append_submenu(Some(text(language, UiText::Emulation)), emulation_menu);
    menu.append_submenu(Some(text(language, UiText::Help)), help_menu);
}

fn ensure_window(
    app: &gtk::Application,
    factory: &Rc<dyn PipelineFactory>,
    state: &Rc<RefCell<State>>,
    current_window: &Rc<RefCell<Option<Window>>>,
) -> Window {
    if let Some(window) = current_window.borrow().as_ref().cloned() {
        return window;
    }

    let window = build_window(app, factory, Rc::clone(state));
    *current_window.borrow_mut() = Some(window.clone());
    window
}

/// Exit status for the binary wrapper: the library never terminates the process.
pub fn run(ctx: FrontendContext, _options: RunOptions) -> i32 {
    // No dialog here: the event loop died with the panic, and stderr
    // (already logged by the guard) is the UX. Match exhaustively.
    match nerust_crash_guard::run_guarded_with(
        || run_inner(ctx),
        nerust_crash_guard::default_sink,
        nerust_crash_guard::capture_backtrace,
    ) {
        // Rust panic convention: a crash never exits 0.
        nerust_crash_guard::GuardOutcome::Completed(Ok(())) => 0,
        nerust_crash_guard::GuardOutcome::Completed(Err(error)) => {
            log::error!("{error}");
            1
        }
        nerust_crash_guard::GuardOutcome::Crashed(report) => {
            log::error!("{}", crash_note(report.message()));
            101
        }
    }
}

/// Crash log line: the guard already logged the full report; this
/// names the missing-dialog situation for stderr readers.
fn crash_note(message: &str) -> String {
    format!("gtk frontend crashed with no dialog available: {message}")
}

fn run_inner(ctx: FrontendContext) -> Result<(), SessionError> {
    let app = gtk::Application::new(
        Some("com.github.chalharu"),
        gio::ApplicationFlags::HANDLES_OPEN,
    );

    let current_window = Rc::new(RefCell::new(None));
    let state = Rc::new(RefCell::new(State::new(ctx)?));
    {
        let state = Rc::clone(&state);
        let gpu_factory = state.borrow().ctx.gpu_factory.clone();
        let current_window = current_window.clone();
        let _ = app.connect_activate(move |app| {
            let window = ensure_window(app, &gpu_factory, &state, &current_window);
            window.window().present();
        });
    }
    {
        let state = Rc::clone(&state);
        let gpu_factory = state.borrow().ctx.gpu_factory.clone();
        let current_window = current_window.clone();
        let _ = app.connect_open(move |app, files, _| {
            let window = ensure_window(app, &gpu_factory, &state, &current_window);
            // Present first: a failure dialog raised by load_path must
            // stay above the main window.
            window.window().present();
            if let Some(path) = files.iter().find_map(|file| file.path()) {
                // One path for every open: failure dialog and action
                // sync live in `Window::load_path`.
                window.load_path(&path);
            }
        });
    }

    // External destruction bypasses close-request: poll the GDK flag so the app can quit.
    {
        let app = app.clone();
        let current_window = current_window.clone();
        glib::timeout_add_seconds_local(1, move || {
            let destroyed = current_window
                .borrow()
                .as_ref()
                .and_then(|window| window.window().surface())
                .is_some_and(|surface| surface.is_destroyed());
            if destroyed {
                log::warn!("gtk: native surface destroyed externally; quitting");
                app.quit();
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    let _ = app.run();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_settings_core::model::input::default_shortcut_bindings;

    #[test]
    fn crash_note_names_missing_dialog() {
        let text = crash_note("boom");
        assert!(text.contains("boom"), "{text}");
        assert!(text.contains("no dialog"), "{text}");
    }

    fn collect_labels(model: &gio::MenuModel, out: &mut Vec<String>) {
        use gio::prelude::MenuModelExt;
        for index in 0..model.n_items() {
            if let Some(label) = model
                .item_attribute_value(index, "label", None)
                .and_then(|variant| variant.str().map(str::to_string))
            {
                out.push(label);
            }
            if let Some(submenu) = model.item_link(index, gio::MENU_LINK_SUBMENU) {
                collect_labels(&submenu, out);
            }
        }
    }

    fn menu_labels(language: AppLanguage, shortcuts: &[ShortcutBinding]) -> Vec<String> {
        use gio::glib::object::Cast;
        let bars = MenubarModels::new();
        let select = gio::Menu::new();
        let save = gio::Menu::new();
        let load = gio::Menu::new();
        let delete = gio::Menu::new();
        build_menu_model(language, shortcuts, &bars, &select, &save, &load, &delete);
        let mut labels = Vec::new();
        collect_labels(bars.menu_model.upcast_ref::<gio::MenuModel>(), &mut labels);
        labels
    }

    #[test]
    fn menubar_refill_relabels_same_objects_in_place() {
        use gio::glib::object::Cast;
        use gio::prelude::MenuModelExt;
        let bars = MenubarModels::new();
        let select = gio::Menu::new();
        let save = gio::Menu::new();
        let load = gio::Menu::new();
        let delete = gio::Menu::new();
        // A live slot entry: refill must not drop submenu contents.
        select.append(Some("seed"), None);
        let shortcuts = default_shortcut_bindings();
        build_menu_model(
            AppLanguage::English,
            &shortcuts,
            &bars,
            &select,
            &save,
            &load,
            &delete,
        );
        let top = bars.menu_model.upcast_ref::<gio::MenuModel>().n_items();
        assert_eq!(top, 3);
        let before = menu_labels(AppLanguage::English, &shortcuts);
        assert!(before.iter().any(|label| label == "File"));

        // Same objects, new language: the runtime path calls no set_menubar.
        build_menu_model(
            AppLanguage::Japanese,
            &shortcuts,
            &bars,
            &select,
            &save,
            &load,
            &delete,
        );
        assert_eq!(
            bars.menu_model.upcast_ref::<gio::MenuModel>().n_items(),
            top
        );
        let after = {
            let mut labels = Vec::new();
            collect_labels(bars.menu_model.upcast_ref::<gio::MenuModel>(), &mut labels);
            labels
        };
        assert!(after.iter().any(|label| label == "ファイル"), "{after:?}");
        assert!(!after.iter().any(|label| label == "File"), "{after:?}");
        assert!(after.iter().any(|label| label == "seed"), "{after:?}");
    }

    #[test]
    fn menu_labels_follow_live_shortcut_bindings() {
        let labels = menu_labels(AppLanguage::English, &default_shortcut_bindings());
        assert!(labels.iter().any(|label| label == "Save Active Slot (F5)"));
        assert!(labels.iter().any(|label| label == "Open Debugger (F4)"));
        // Rebound actions advertise the new key, never the default.
        let rebound = vec![
            ShortcutBinding {
                action: ShortcutAction::SaveActiveSlot,
                key: Some(nerust_keyboard::Key::F2),
            },
            ShortcutBinding {
                action: ShortcutAction::OpenDebugger,
                key: None,
            },
        ];
        let labels = menu_labels(AppLanguage::English, &rebound);
        assert!(labels.iter().any(|label| label == "Save Active Slot (F2)"));
        assert!(labels.iter().any(|label| label == "Open Debugger"));
        assert!(!labels.iter().any(|label| label.contains("(F4)")));
    }

    #[test]
    fn shortcut_hint_falls_back_to_bare_label() {
        // Unbound action advertises no key, never a stale default.
        let unbound = vec![ShortcutBinding {
            action: ShortcutAction::SaveActiveSlot,
            key: None,
        }];
        assert_eq!(
            shortcut_hint("Save Active Slot", &unbound, ShortcutAction::SaveActiveSlot),
            "Save Active Slot"
        );
        assert_eq!(
            shortcut_hint(
                "Save Active Slot",
                &default_shortcut_bindings(),
                ShortcutAction::SaveActiveSlot
            ),
            "Save Active Slot (F5)"
        );
    }

    #[test]
    fn menu_model_covers_all_sections_in_japanese() {
        let labels = menu_labels(AppLanguage::Japanese, &default_shortcut_bindings());
        assert!(!labels.is_empty());
        // Three top-level sections, none empty after i18n lookup.
        assert!(labels.iter().all(|label| !label.is_empty()));
        assert!(labels.len() > 10);
    }
}
