mod crash_handler;
mod glarea;
mod window;

use self::window::{StateMenus, Window, WindowExtend};
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use nerust_console::{Console, ConsoleMetrics, PreviewFrame};
use nerust_core::controller::standard_controller::Buttons;
use nerust_persistence::{
    SidecarPaths, StateSlotSummary, ThumbnailSource, allocate_next_slot_id, delete_state_slot,
    latest_saved_slot_id, load_mapper_save, load_state_slot, resolve_sidecars,
    scan_state_slots_for_target, state_slot_path, write_mapper_save, write_recovery_mapper_save,
    write_state_slot,
};
use nerust_screen_filter::FilterType;
use nerust_screen_opengl::GlView;
use nerust_screen_traits::{LogicalSize, PhysicalSize};
use nerust_sound_openal::{OpenAl, prepare_macos_runtime};
use nerust_timer::CLOCK_RATE;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

const TITLE_UPDATE_INTERVAL: Duration = Duration::from_millis(500);

fn window_title(paused: bool, console_metrics: ConsoleMetrics) -> String {
    let state = if paused { "Nes -- Paused" } else { "Nes" };
    if console_metrics.loaded {
        format!(
            "{state} | FPS {:.1} | Speed x{:.2}",
            console_metrics.emulation_fps, console_metrics.speed_multiplier
        )
    } else {
        format!("{state} | No ROM")
    }
}

#[derive(Debug)]
pub(crate) struct State {
    view: Option<GlView>,
    paused: bool,
    loaded: bool,
    console: Console,
    physical_size: PhysicalSize,
    rom_path: Option<PathBuf>,
    sidecars: Option<SidecarPaths>,
    mapper_save_flush_allowed: bool,
    mapper_save_recovery_written: bool,
    slots: Vec<StateSlotSummary>,
    active_slot_id: Option<u64>,
}

impl State {
    pub(crate) fn new(filter_type: FilterType, source_logical_size: LogicalSize) -> Self {
        let speaker = OpenAl::new(48000, CLOCK_RATE as i32, 128, 20);
        let console = Console::new_gpu(speaker, filter_type, source_logical_size);
        let physical_size = console.video().presentation().physical_size();
        Self {
            view: None,
            console,
            paused: false,
            loaded: false,
            physical_size,
            rom_path: None,
            sidecars: None,
            mapper_save_flush_allowed: true,
            mapper_save_recovery_written: false,
            slots: Vec::new(),
            active_slot_id: None,
        }
    }

    pub(crate) fn pause(&mut self) {
        self.console.pause();
        self.paused = true;
    }

    #[allow(dead_code, reason = "reserved for GTK menu state bindings")]
    pub(crate) fn paused(&self) -> bool {
        self.paused
    }

    pub(crate) fn can_pause(&self) -> bool {
        !self.paused && self.loaded
    }

    pub(crate) fn resume(&mut self) {
        self.console.resume();
        self.paused = false;
    }

    pub(crate) fn can_resume(&self) -> bool {
        self.paused && self.loaded
    }

    pub(crate) fn load_from_path(&mut self, rom_path: Option<PathBuf>, data: Vec<u8>) {
        if let Err(error) = self.flush_mapper_save() {
            log::warn!("mapper save flush before load failed: {error}");
            return;
        }
        if self.console.load(data).is_ok() {
            self.loaded = true;
            self.rom_path = rom_path;
            self.sidecars = self.rom_path.as_deref().map(resolve_sidecars);
            self.mapper_save_flush_allowed = true;
            self.mapper_save_recovery_written = false;
            self.active_slot_id = None;
            self.refresh_slots();
            self.active_slot_id = latest_saved_slot_id(&self.slots);
            if let Err(error) = self.load_mapper_save_if_available() {
                self.mapper_save_flush_allowed = false;
                log::warn!("mapper save auto-load failed: {error}");
            }
            self.resume();
        }
    }

    pub(crate) fn loaded(&self) -> bool {
        self.loaded
    }

    pub(crate) fn title(&self) -> String {
        window_title(self.paused, self.console.metrics())
    }

    pub(crate) fn unload(&mut self) -> bool {
        if let Err(error) = self.flush_mapper_save() {
            log::warn!("mapper save flush before unload failed: {error}");
            return false;
        }
        let _ = self.console.unload();
        self.loaded = false;
        self.rom_path = None;
        self.sidecars = None;
        self.mapper_save_flush_allowed = true;
        self.mapper_save_recovery_written = false;
        self.active_slot_id = None;
        self.slots.clear();
        true
    }

    pub(crate) fn flush_before_exit(&mut self) {
        if let Err(error) = self.flush_mapper_save() {
            log::warn!("mapper save flush before close failed: {error}");
        }
    }

    pub(crate) fn set_pad1(&mut self, data: Buttons) {
        self.console.set_pad1(data)
    }

    pub(crate) fn slots(&self) -> &[StateSlotSummary] {
        &self.slots
    }

    pub(crate) fn active_slot_id(&self) -> Option<u64> {
        self.active_slot_id
    }

    fn sync_paused_from_console(&mut self) {
        self.paused = self.console.metrics().paused;
    }

    pub(crate) fn save_active_slot_or_new(&mut self) {
        let Some(sidecars) = self.sidecars.as_ref() else {
            return;
        };
        let slot_id =
            self.active_slot_id
                .or_else(|| match allocate_next_slot_id(&sidecars.states_dir) {
                    Ok(slot_id) => Some(slot_id),
                    Err(error) => {
                        log::warn!("allocating state slot failed: {error}");
                        None
                    }
                });
        if let Some(slot_id) = slot_id {
            self.save_slot(slot_id, true);
        }
    }

    pub(crate) fn create_slot(&mut self) {
        let Some(sidecars) = self.sidecars.as_ref() else {
            return;
        };
        match allocate_next_slot_id(&sidecars.states_dir) {
            Ok(slot_id) => self.save_slot(slot_id, true),
            Err(error) => log::warn!("allocating state slot failed: {error}"),
        }
    }

    pub(crate) fn save_slot(&mut self, slot_id: u64, make_active: bool) {
        let Some(sidecars) = self.sidecars.as_ref() else {
            return;
        };
        match self.console.export_state() {
            Ok(export) => {
                let preview = export.preview.as_ref().map(preview_to_thumbnail_source);
                match write_state_slot(
                    &sidecars.states_dir,
                    slot_id,
                    &export.machine_state,
                    export.rom_identity,
                    export.options,
                    preview.as_ref(),
                ) {
                    Ok(_) => {
                        if make_active {
                            self.active_slot_id = Some(slot_id);
                        }
                        self.refresh_slots();
                    }
                    Err(error) => log::warn!("saving state slot failed: {error}"),
                }
            }
            Err(error) => log::warn!("state export failed: {error}"),
        }
    }

    pub(crate) fn load_slot(&mut self, slot_id: u64) {
        let Some(sidecars) = self.sidecars.as_ref() else {
            return;
        };
        match load_state_slot(&state_slot_path(&sidecars.states_dir, slot_id)) {
            Ok(slot) => {
                if let Err(error) = self.console.import_state(slot.machine_state) {
                    log::warn!("state import failed: {error}");
                } else {
                    self.active_slot_id = Some(slot_id);
                    self.sync_paused_from_console();
                    self.refresh_slots();
                }
            }
            Err(error) => log::warn!("loading state slot failed: {error}"),
        }
    }

    pub(crate) fn delete_slot(&mut self, slot_id: u64) {
        let Some(sidecars) = self.sidecars.as_ref() else {
            return;
        };
        if delete_state_slot(&state_slot_path(&sidecars.states_dir, slot_id)).is_ok() {
            if self.active_slot_id == Some(slot_id) {
                self.active_slot_id = None;
            }
            self.refresh_slots();
        }
    }

    pub(crate) fn select_active_slot(&mut self, slot_id: u64) {
        self.active_slot_id = Some(slot_id);
    }

    fn refresh_slots(&mut self) {
        self.slots = if let Some(sidecars) = self.sidecars.as_ref() {
            match self.console.persistence_target() {
                Ok((rom_identity, options)) => {
                    match scan_state_slots_for_target(&sidecars.states_dir, rom_identity, options) {
                        Ok(slots) => slots,
                        Err(error) => {
                            log::warn!("slot scan failed: {error}");
                            Vec::new()
                        }
                    }
                }
                Err(error) => {
                    log::warn!("state slot target unavailable: {error}");
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };
        if self
            .active_slot_id
            .is_some_and(|slot_id| !self.slots.iter().any(|slot| slot.slot_id == slot_id))
        {
            self.active_slot_id = None;
        }
    }

    fn load_mapper_save_if_available(&mut self) -> Result<(), String> {
        let Some(sidecars) = self.sidecars.as_ref() else {
            return Ok(());
        };
        if let Some(bytes) =
            load_mapper_save(&sidecars.mapper_save_path).map_err(|error| error.to_string())?
        {
            self.console
                .import_mapper_save(bytes)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn flush_mapper_save(&mut self) -> Result<(), String> {
        let Some(sidecars) = self.sidecars.as_ref() else {
            return Ok(());
        };
        if !self.mapper_save_flush_allowed {
            if self.mapper_save_recovery_written {
                return Ok(());
            }
            if let Some(bytes) = self
                .console
                .export_mapper_save()
                .map_err(|error| error.to_string())?
            {
                let path = write_recovery_mapper_save(&sidecars.mapper_save_path, &bytes)
                    .map_err(|error| error.to_string())?;
                self.mapper_save_recovery_written = true;
                log::warn!(
                    "mapper save auto-load failed earlier; wrote recovery save to {}",
                    path.display()
                );
            }
            return Ok(());
        }
        let bytes = self
            .console
            .export_mapper_save()
            .map_err(|error| error.to_string())?;
        match bytes {
            Some(bytes) => write_mapper_save(&sidecars.mapper_save_path, &bytes)
                .map_err(|error| error.to_string()),
            None => Ok(()),
        }
    }
}

fn preview_to_thumbnail_source(preview: &PreviewFrame) -> ThumbnailSource {
    ThumbnailSource {
        width: preview.width,
        height: preview.height,
        rgba: preview.rgba.clone(),
    }
}

impl Drop for State {
    fn drop(&mut self) {
        if let Err(error) = self.flush_mapper_save() {
            log::warn!("mapper save flush during shutdown failed: {error}");
        }
    }
}

fn build_window(app: &gtk::Application) -> Window {
    let builder = gtk::Builder::from_string(include_str!("../resources/ui.xml"));
    let window: gtk::ApplicationWindow = builder.object("window").unwrap();
    let menu_model = gtk::Builder::from_string(include_str!("../resources/menu.xml"))
        .object::<gio::Menu>("menu")
        .unwrap();
    let state_menu = gio::Menu::new();
    let select_active_slot_menu = gio::Menu::new();
    let save_slot_menu = gio::Menu::new();
    let load_slot_menu = gio::Menu::new();
    let delete_slot_menu = gio::Menu::new();
    state_menu.append(Some("Create Save Slot"), Some("win.state-create"));
    state_menu.append(Some("Save Active Slot"), Some("win.state-save-active"));
    state_menu.append(Some("Load Active Slot"), Some("win.state-load-active"));
    state_menu.append_submenu(Some("Select Active Slot"), &select_active_slot_menu);
    state_menu.append_submenu(Some("Save Slot"), &save_slot_menu);
    state_menu.append_submenu(Some("Load Slot"), &load_slot_menu);
    state_menu.append_submenu(Some("Delete Slot"), &delete_slot_menu);
    menu_model.append_submenu(Some("Save States"), &state_menu);

    let state: Rc<RefCell<State>> = Rc::new(RefCell::new(State::new(
        FilterType::NtscComposite,
        LogicalSize {
            width: 256,
            height: 240,
        },
    )));

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
        builder.object("glarea").unwrap(),
        state,
        StateMenus {
            select_active_slot_menu,
            save_slot_menu,
            load_slot_menu,
            delete_slot_menu,
        },
    )
}

fn ensure_window(app: &gtk::Application, current_window: &Rc<RefCell<Option<Window>>>) -> Window {
    if let Some(window) = current_window.borrow().as_ref().cloned() {
        return window;
    }

    let window = build_window(app);
    *current_window.borrow_mut() = Some(window.clone());
    window
}

fn main() {
    // log initialize
    simple_logger::init().unwrap();
    crash_handler::install();
    prepare_macos_runtime();

    let app = gtk::Application::new(
        Some("com.github.chalharu"),
        gio::ApplicationFlags::HANDLES_OPEN,
    );

    let current_window = Rc::new(RefCell::new(None));
    {
        let current_window = current_window.clone();
        let _ = app.connect_activate(move |app| {
            let window = ensure_window(app, &current_window);
            window.window().present();
        });
    }
    {
        let current_window = current_window.clone();
        let _ = app.connect_open(move |app, files, _| {
            let window = ensure_window(app, &current_window);
            if let Some(path) = files.iter().find_map(|file| file.path()) {
                window.load_path(&path);
            }
            window.window().present();
        });
    }

    let _ = app.run();
}

#[cfg(test)]
mod tests {
    use super::window_title;
    use nerust_console::ConsoleMetrics;

    #[test]
    fn window_title_surfaces_runtime_metrics() {
        let title = window_title(
            false,
            ConsoleMetrics {
                loaded: true,
                emulation_fps: 59.9,
                speed_multiplier: 1.01,
                ..ConsoleMetrics::default()
            },
        );

        assert!(title.contains("FPS 59.9"));
        assert!(title.contains("Speed x1.01"));
    }
}
