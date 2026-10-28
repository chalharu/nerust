mod debugger;
mod gdk_raw;
mod keyboard;
mod native;
mod ppu_window;
mod renderer;
mod surface;
mod surface_state;
#[cfg(test)]
pub(crate) mod test_support;
mod widgets;
mod window;

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use gtk::{
    gdk::prelude::SurfaceExt as _,
    gio, glib,
    prelude::{
        ApplicationExt as _, ApplicationExtManual as _, FileExt as _, GtkApplicationExt as _,
        GtkWindowExt as _, NativeExt as _, WidgetExt as _,
    },
};
use nerust_frontend_host::session::{SessionEvent, SessionOutcome};
use nerust_gui_runtime::settings::{
    HostBackendCapabilities, HostWindowCapabilities, SettingsSnapshot,
};
use nerust_gui_shell::{
    context::FrontendContext,
    session::{
        SessionError, SessionHandle,
        access::{SessionControl, SettingsResult},
    },
};
use nerust_render_traits::{FrameBuffer, VideoRenderProfile, renderer::PipelineFactory};

use self::window::{MenubarModels, Window, WindowExtend};

const TITLE_UPDATE_INTERVAL: Duration = Duration::from_millis(500);

pub(crate) struct State {
    pub(crate) host: nerust_frontend_host::application::ApplicationHost,
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
            host: nerust_frontend_host::application::ApplicationHost::new(session),
            ctx,
            renderer_reload_pending: false,
        })
    }

    pub(crate) fn swap_frame_buffer(&mut self) {
        self.host.swap_frame_buffer();
    }

    pub(crate) fn frame_buffer(&self) -> Option<&FrameBuffer> {
        self.host.frame_buffer()
    }

    pub(crate) fn render_profile(&self) -> Option<VideoRenderProfile> {
        self.host.render_profile()
    }

    /// Load a ROM, flagging a renderer rebuild on success.
    /// Returns the failure detail; presenting it is the caller's job.
    pub(crate) fn load_path(&mut self, path: &Path) -> Result<(), String> {
        let Self { host, ctx, .. } = self;
        if let Err(error) = host.load_path(&mut *ctx.rom_loader, path) {
            log::warn!("ROM load failed: {error}");
            return Err(error.to_string());
        }
        if host.tick().loaded {
            self.renderer_reload_pending = true;
        }
        Ok(())
    }

    pub(crate) fn title_update(&mut self) -> Option<String> {
        let tick = self.host.tick();
        self.host.title_update(&tick)
    }

    pub(crate) fn title_refresh_due(&mut self, now: std::time::Instant) -> bool {
        self.host.title_refresh_due(now)
    }

    /// Ignore intermediate native observations while awaiting the target.
    pub(crate) fn observe_fullscreen(&mut self, native: bool) -> bool {
        self.host.observe_fullscreen(native)
    }

    /// Ordered teardown entry: false when a close is already in flight.
    pub(crate) fn begin_close(&mut self) -> bool {
        self.host.begin_close()
    }

    pub(crate) fn finish_close(&mut self) {
        self.host.finish_close();
    }
}

nerust_frontend_host::impl_session_controls!(State);

impl State {
    pub(crate) fn dispatch_session(
        &mut self,
        event: SessionEvent,
    ) -> Result<SessionOutcome, SessionError> {
        self.host.dispatch(event)
    }

    pub(crate) fn tick_focus_sync(&mut self) {
        use nerust_frontend_host::{
            event::{EventCommand, EventHandler},
            focus::FocusEvent,
        };
        let handler: &mut dyn EventHandler = &mut self.host;
        let _ = handler.handle_event(EventCommand::Focus(FocusEvent::BurstEnd), false);
    }
}

impl SessionControl for State {
    fn unload(&mut self) {
        if let Err(error) = self.host.session_mut().unload() {
            log::warn!("ROM unload failed: {error}");
        }
    }
    fn apply_settings(
        &mut self,
        settings: SettingsSnapshot,
    ) -> Result<SettingsResult, SessionError> {
        let plan = self.host.apply_settings(settings)?;
        if plan.renderer_needs_rebuild {
            self.renderer_reload_pending = true;
        }
        Ok(SettingsResult {
            renderer_needs_rebuild: self.renderer_reload_pending,
            fullscreen_default_changed: plan.fullscreen_default_changed,
            scaling_changed: plan.scaling_changed,
        })
    }
    fn set_fullscreen_default(&mut self, fullscreen: bool) -> Result<SettingsResult, SessionError> {
        let plan = self.host.session_mut().set_fullscreen_default(fullscreen)?;
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

fn build_window(
    app: &gtk::Application,
    factory: &Rc<dyn PipelineFactory>,
    state: Rc<RefCell<State>>,
) -> Window {
    let builder = gtk::Builder::from_string(include_str!("../resources/ui.xml"));
    let window: gtk::ApplicationWindow = builder.object("window").unwrap();
    let menubar = MenubarModels::new();

    app.add_window(&window);
    // Game viewport: a memory picture composited by GTK; backends never present here.
    let picture = gtk::Picture::new();
    picture.set_hexpand(true);
    picture.set_vexpand(true);
    picture.set_can_shrink(true);

    Window::bind(
        app.clone(),
        window,
        picture,
        state,
        Rc::clone(factory),
        menubar,
    )
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
/// Crash containment lives in the composition root; see `nerust::run`.
pub fn run(ctx: FrontendContext, _rom_path: Option<PathBuf>) -> i32 {
    match run_inner(ctx) {
        Ok(()) => 0,
        Err(error) => {
            log::error!("{error}");
            1
        }
    }
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
    use nerust_frontend_host::{
        application::MenuSig,
        menu::{MenuId, MenuModel},
        widgets::MenuPresenter,
    };
    use nerust_settings_core::model::{
        input::{ShortcutAction, ShortcutBinding, default_shortcut_bindings},
        language::AppLanguage,
    };
    use window::{GTK_MENU_CAPABILITIES, GtkMenuVisitor};

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
        let sig = menu_signature(language, shortcuts);
        MenuModel::build(&sig, GTK_MENU_CAPABILITIES)
            .visit(None, &mut GtkMenuVisitor::new(bars.clone()));
        let mut labels = Vec::new();
        collect_labels(bars.menu_model.upcast_ref::<gio::MenuModel>(), &mut labels);
        labels
    }

    fn menu_signature(language: AppLanguage, shortcuts: &[ShortcutBinding]) -> MenuSig {
        MenuSig {
            loaded: false,
            paused: false,
            settings_open: false,
            language,
            active_slot: None,
            slots: Vec::new().into(),
            shortcuts: shortcuts.into(),
        }
    }

    #[test]
    fn menubar_refill_relabels_same_objects_in_place() {
        use gio::glib::object::Cast;
        use gio::prelude::MenuModelExt;
        use nerust_persistence::model::StateSlotSummary;
        use std::cell::Cell;

        let bars = MenubarModels::new();
        let mut sig = menu_signature(AppLanguage::English, &default_shortcut_bindings());
        sig.loaded = true;
        sig.active_slot = Some(2);
        sig.slots = [1, 2]
            .map(|slot_id| StateSlotSummary {
                schema_version: 1,
                slot_id,
                path: nerust_persistence::sidecar::StorageLocation::Native(
                    format!("slot{slot_id}").into(),
                ),
                saved_at: std::time::SystemTime::UNIX_EPOCH,
                has_thumbnail: false,
                emulator_version: "test".into(),
            })
            .to_vec()
            .into();
        let presenter = MenuPresenter::default();
        let mut visitor = GtkMenuVisitor::new(bars.clone());
        let plan = presenter
            .submit(MenuModel::build(&sig, GTK_MENU_CAPABILITIES))
            .unwrap();
        plan.visit(&mut visitor);
        assert!(presenter.complete(plan).is_none());
        let top = bars.menu_model.upcast_ref::<gio::MenuModel>().n_items();
        assert_eq!(top, 3);
        let mut before = Vec::new();
        collect_labels(bars.menu_model.upcast_ref(), &mut before);
        assert!(before.iter().any(|label| label == "File"));

        let ids = [
            MenuId::File,
            MenuId::Emulation,
            MenuId::States,
            MenuId::Help,
            MenuId::SelectSlot,
            MenuId::SaveSlot,
            MenuId::LoadSlot,
            MenuId::DeleteSlot,
        ];
        let models = ids.map(|id| bars.submenu(id));
        let slot_actions = [
            (MenuId::SelectSlot, "win.state-select-slot"),
            (MenuId::SaveSlot, "win.state-save-slot"),
            (MenuId::LoadSlot, "win.state-load-slot"),
            (MenuId::DeleteSlot, "win.state-delete-slot"),
        ];
        let assert_slots = || {
            let states = bars.submenu(MenuId::States);
            for (submenu_index, (id, action)) in slot_actions.into_iter().enumerate() {
                let menu = bars.submenu(id);
                assert_eq!(
                    states
                        .item_link(submenu_index as i32 + 3, gio::MENU_LINK_SUBMENU)
                        .as_ref(),
                    Some(menu.upcast_ref::<gio::MenuModel>())
                );
                assert_eq!(menu.n_items(), 2);
                for (index, slot) in sig.slots.iter().enumerate() {
                    let index = index as i32;
                    assert_eq!(
                        menu.item_attribute_value(index, "target", None)
                            .unwrap()
                            .get::<u64>(),
                        Some(slot.slot_id)
                    );
                    assert_eq!(
                        menu.item_attribute_value(index, "action", None)
                            .unwrap()
                            .str(),
                        Some(action)
                    );
                    assert_eq!(
                        menu.item_attribute_value(index, "label", None)
                            .unwrap()
                            .str(),
                        Some(nerust_gui_runtime::slots::slot_label(slot, sig.active_slot).as_str())
                    );
                }
            }
        };
        assert_slots();
        let slot_changes = Rc::new(Cell::new(0));
        for (id, _) in slot_actions {
            let changes = slot_changes.clone();
            bars.submenu(id).connect_items_changed(move |_, _, _, _| {
                changes.set(changes.get() + 1);
            });
        }

        // Same objects, new language: the runtime path calls no set_menubar.
        sig.language = AppLanguage::Japanese;
        let plan = presenter
            .submit(MenuModel::build(&sig, GTK_MENU_CAPABILITIES))
            .unwrap();
        plan.visit(&mut visitor);
        assert!(presenter.complete(plan).is_none());
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
        for (id, model) in ids.into_iter().zip(models) {
            assert_eq!(bars.submenu(id), model);
        }
        assert_slots();
        assert_eq!(slot_changes.get(), 0, "unchanged slot entries stay intact");
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
                key: Some(wakui::Key::F2),
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
    fn menu_model_covers_all_sections_in_japanese() {
        let labels = menu_labels(AppLanguage::Japanese, &default_shortcut_bindings());
        assert!(!labels.is_empty());
        // Three top-level sections, none empty after i18n lookup.
        assert!(labels.iter().all(|label| !label.is_empty()));
        assert!(labels.len() > 10);
    }
}
