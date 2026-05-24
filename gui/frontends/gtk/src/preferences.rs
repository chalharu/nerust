use gtk::prelude::*;
use nerust_contract_options::Mmc3IrqVariant;
use nerust_contract_settings::{
    desktop::{DesktopSettings, StoragePolicy, SystemSettings},
    input::{
        BindingProfile, ControlBinding, HostInputSource, KeyboardKey, PersistedAttachmentId,
        PersistedControlId,
    },
    nes::{NesCoreSettings, NesSettings, NesVideoFilter, NesVideoSettings},
    shortcut::{ShortcutAction, ShortcutBinding},
};
use nerust_gui_runtime::settings::DesktopSettingsManager;
use nerust_gui_shell::settings::{
    EDITABLE_KEYS, current_or_default, keyboard_binding_descriptors, keyboard_key_from_id,
    keyboard_key_id, keyboard_key_label, shortcut_descriptors,
};
use nerust_input_schema::SystemId;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

const PAGE_GENERAL: u32 = 0;
const PAGE_STORAGE: u32 = 1;
const PAGE_CONTROLLERS: u32 = 2;
const PAGE_SHORTCUTS: u32 = 3;
const PAGE_VIDEO: u32 = 4;
const PAGE_AUDIO: u32 = 5;
const PAGE_SYSTEM: u32 = 6;

#[derive(Clone)]
struct ControllerRow {
    attachment: PersistedAttachmentId,
    control: PersistedControlId,
    combo: gtk::ComboBoxText,
    saved: gtk::Label,
}

#[derive(Clone)]
struct ShortcutRow {
    action: ShortcutAction,
    combo: gtk::ComboBoxText,
    saved: gtk::Label,
}

struct PreferencesDialogState {
    manager: DesktopSettingsManager,
    saved_settings: Rc<RefCell<DesktopSettings>>,
    dialog: gtk::Dialog,
    notebook: gtk::Notebook,
    status_label: gtk::Label,
    last_open_directory: gtk::Entry,
    last_open_directory_saved: gtk::Label,
    default_open_dir: gtk::Entry,
    default_open_dir_saved: gtk::Label,
    screenshot_dir: gtk::Entry,
    screenshot_dir_saved: gtk::Label,
    export_dir: gtk::Entry,
    export_dir_saved: gtk::Label,
    remember_window_bounds: gtk::CheckButton,
    remember_window_bounds_saved: gtk::Label,
    pause_on_focus_loss: gtk::CheckButton,
    pause_on_focus_loss_saved: gtk::Label,
    clear_input_on_focus_loss: gtk::CheckButton,
    clear_input_on_focus_loss_saved: gtk::Label,
    window_width: gtk::SpinButton,
    window_width_saved: gtk::Label,
    window_height: gtk::SpinButton,
    window_height_saved: gtk::Label,
    storage_policy: gtk::ComboBoxText,
    storage_policy_saved: gtk::Label,
    state_root: gtk::Entry,
    state_root_saved: gtk::Label,
    mapper_save_root: gtk::Entry,
    mapper_save_root_saved: gtk::Label,
    controller_rows: Vec<ControllerRow>,
    shortcut_rows: Vec<ShortcutRow>,
    fullscreen: gtk::CheckButton,
    fullscreen_saved: gtk::Label,
    sample_rate: gtk::SpinButton,
    sample_rate_saved: gtk::Label,
    buffer_size: gtk::SpinButton,
    buffer_size_saved: gtk::Label,
    latency_ms: gtk::SpinButton,
    latency_ms_saved: gtk::Label,
    master_volume: gtk::SpinButton,
    master_volume_saved: gtk::Label,
    muted: gtk::CheckButton,
    muted_saved: gtk::Label,
    mmc3_irq_variant: gtk::ComboBoxText,
    mmc3_irq_variant_saved: gtk::Label,
    nes_filter: gtk::ComboBoxText,
    nes_filter_saved: gtk::Label,
}

pub(crate) fn present_preferences_dialog(
    parent: &gtk::ApplicationWindow,
    manager: DesktopSettingsManager,
) {
    let state = PreferencesDialogState::new(parent, manager);
    state.dialog.present();
}

impl PreferencesDialogState {
    fn new(parent: &gtk::ApplicationWindow, manager: DesktopSettingsManager) -> Rc<Self> {
        let saved_settings = Rc::new(RefCell::new(current_or_default(&manager)));
        let dialog = gtk::Dialog::builder()
            .transient_for(parent)
            .modal(true)
            .title("Preferences")
            .default_width(960)
            .default_height(720)
            .build();
        let content = dialog.content_area();
        content.set_spacing(12);
        content.set_margin_start(12);
        content.set_margin_end(12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);

        let path_label = gtk::Label::new(Some(
            &manager
                .paths()
                .ok()
                .flatten()
                .map(|paths| format!("Settings file: {}", paths.settings_file.display()))
                .unwrap_or_else(|| "Settings file: <ephemeral>".to_string()),
        ));
        path_label.set_xalign(0.0);
        content.append(&path_label);

        let status_label = gtk::Label::new(None);
        status_label.set_xalign(0.0);
        content.append(&status_label);

        let notebook = gtk::Notebook::new();
        notebook.set_vexpand(true);
        content.append(&notebook);

        let (general_page, general_widgets) = build_general_page();
        notebook.append_page(&general_page, Some(&gtk::Label::new(Some("General"))));

        let (storage_page, storage_widgets) = build_storage_page();
        notebook.append_page(&storage_page, Some(&gtk::Label::new(Some("Storage"))));

        let (controllers_page, controller_rows) = build_controllers_page();
        notebook.append_page(
            &controllers_page,
            Some(&gtk::Label::new(Some("Controllers"))),
        );

        let (shortcuts_page, shortcut_rows) = build_shortcuts_page();
        notebook.append_page(&shortcuts_page, Some(&gtk::Label::new(Some("Shortcuts"))));

        let (video_page, video_widgets) = build_video_page();
        notebook.append_page(&video_page, Some(&gtk::Label::new(Some("Video"))));

        let (audio_page, audio_widgets) = build_audio_page();
        notebook.append_page(&audio_page, Some(&gtk::Label::new(Some("Audio"))));

        let (system_page, system_widgets) = build_system_page();
        notebook.append_page(&system_page, Some(&gtk::Label::new(Some("System (NES)"))));

        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        content.append(&buttons);
        let reload_button = gtk::Button::with_label("Load");
        let save_button = gtk::Button::with_label("Save");
        let defaults_button = gtk::Button::with_label("Defaults");
        let reset_page_button = gtk::Button::with_label("Reset Page");
        let close_button = gtk::Button::with_label("Close");
        for button in [
            &reload_button,
            &save_button,
            &defaults_button,
            &reset_page_button,
            &close_button,
        ] {
            buttons.append(button);
        }

        let state = Rc::new(Self {
            manager,
            saved_settings,
            dialog,
            notebook,
            status_label,
            last_open_directory: general_widgets.last_open_directory,
            last_open_directory_saved: general_widgets.last_open_directory_saved,
            default_open_dir: general_widgets.default_open_dir,
            default_open_dir_saved: general_widgets.default_open_dir_saved,
            screenshot_dir: general_widgets.screenshot_dir,
            screenshot_dir_saved: general_widgets.screenshot_dir_saved,
            export_dir: general_widgets.export_dir,
            export_dir_saved: general_widgets.export_dir_saved,
            remember_window_bounds: general_widgets.remember_window_bounds,
            remember_window_bounds_saved: general_widgets.remember_window_bounds_saved,
            pause_on_focus_loss: general_widgets.pause_on_focus_loss,
            pause_on_focus_loss_saved: general_widgets.pause_on_focus_loss_saved,
            clear_input_on_focus_loss: general_widgets.clear_input_on_focus_loss,
            clear_input_on_focus_loss_saved: general_widgets.clear_input_on_focus_loss_saved,
            window_width: general_widgets.window_width,
            window_width_saved: general_widgets.window_width_saved,
            window_height: general_widgets.window_height,
            window_height_saved: general_widgets.window_height_saved,
            storage_policy: storage_widgets.storage_policy,
            storage_policy_saved: storage_widgets.storage_policy_saved,
            state_root: storage_widgets.state_root,
            state_root_saved: storage_widgets.state_root_saved,
            mapper_save_root: storage_widgets.mapper_save_root,
            mapper_save_root_saved: storage_widgets.mapper_save_root_saved,
            controller_rows,
            shortcut_rows,
            fullscreen: video_widgets.fullscreen,
            fullscreen_saved: video_widgets.fullscreen_saved,
            sample_rate: audio_widgets.sample_rate,
            sample_rate_saved: audio_widgets.sample_rate_saved,
            buffer_size: audio_widgets.buffer_size,
            buffer_size_saved: audio_widgets.buffer_size_saved,
            latency_ms: audio_widgets.latency_ms,
            latency_ms_saved: audio_widgets.latency_ms_saved,
            master_volume: audio_widgets.master_volume,
            master_volume_saved: audio_widgets.master_volume_saved,
            muted: audio_widgets.muted,
            muted_saved: audio_widgets.muted_saved,
            mmc3_irq_variant: system_widgets.mmc3_irq_variant,
            mmc3_irq_variant_saved: system_widgets.mmc3_irq_variant_saved,
            nes_filter: system_widgets.nes_filter,
            nes_filter_saved: system_widgets.nes_filter_saved,
        });

        state.apply_settings(&state.saved_settings.borrow().clone());
        state.refresh_saved_labels();
        state.update_status("Ready");

        {
            let state = state.clone();
            reload_button.connect_clicked(move |_| match state.manager.reload() {
                Ok(settings) => {
                    *state.saved_settings.borrow_mut() = settings.clone();
                    state.apply_settings(&settings);
                    state.refresh_saved_labels();
                    state.update_status("Reloaded from disk");
                }
                Err(error) => state.update_status(&format!("Reload failed: {error}")),
            });
        }
        {
            let state = state.clone();
            save_button.connect_clicked(move |_| {
                let settings = state.collect_settings();
                match state.manager.save(settings.clone()) {
                    Ok(()) => {
                        *state.saved_settings.borrow_mut() = settings;
                        state.refresh_saved_labels();
                        state.update_status("Saved");
                    }
                    Err(error) => state.update_status(&format!("Save failed: {error}")),
                }
            });
        }
        {
            let state = state.clone();
            defaults_button.connect_clicked(move |_| {
                let defaults = current_or_default(&DesktopSettingsManager::ephemeral(
                    nerust_gui_shell::settings::default_desktop_settings(),
                ));
                state.apply_settings(&defaults);
                state.update_status("Showing defaults (not saved)");
            });
        }
        {
            let state = state.clone();
            reset_page_button.connect_clicked(move |_| {
                let defaults = nerust_gui_shell::settings::default_desktop_settings();
                state.apply_page_defaults(
                    state.notebook.current_page().unwrap_or(PAGE_GENERAL),
                    &defaults,
                );
                state.update_status("Reset current page to defaults (not saved)");
            });
        }
        {
            let dialog = state.dialog.clone();
            close_button.connect_clicked(move |_| dialog.close());
        }

        state
    }

    fn collect_settings(&self) -> DesktopSettings {
        let mut settings = self.saved_settings.borrow().clone();
        settings.general.last_open_directory =
            parse_optional_path(&self.last_open_directory.text());
        settings.paths.default_open_dir = parse_optional_path(&self.default_open_dir.text());
        settings.paths.screenshot_dir = parse_optional_path(&self.screenshot_dir.text());
        settings.paths.export_dir = parse_optional_path(&self.export_dir.text());
        settings.host.remember_window_bounds = self.remember_window_bounds.is_active();
        settings.host.pause_on_focus_loss = self.pause_on_focus_loss.is_active();
        settings.host.clear_input_on_focus_loss = self.clear_input_on_focus_loss.is_active();
        settings.host.window_width = value_or_none(&self.window_width);
        settings.host.window_height = value_or_none(&self.window_height);
        settings.persistence.storage_policy = storage_policy_from_combo(&self.storage_policy);
        settings.persistence.state_root = parse_optional_path(&self.state_root.text());
        settings.persistence.mapper_save_root = parse_optional_path(&self.mapper_save_root.text());
        settings.input.keyboard_profiles.insert(
            SystemId::Nes,
            BindingProfile {
                bindings: self
                    .controller_rows
                    .iter()
                    .filter_map(|row| {
                        keyboard_key_from_combo(&row.combo).map(|key| ControlBinding {
                            attachment: row.attachment.clone(),
                            control: row.control.clone(),
                            source: HostInputSource::Keyboard(key),
                        })
                    })
                    .collect(),
            },
        );
        settings.shortcuts.keyboard = self
            .shortcut_rows
            .iter()
            .filter_map(|row| {
                keyboard_key_from_combo(&row.combo).map(|key| ShortcutBinding {
                    action: row.action,
                    key,
                })
            })
            .collect();
        settings.video.fullscreen = self.fullscreen.is_active();
        settings.audio.sample_rate = self.sample_rate.value() as u32;
        settings.audio.buffer_size = self.buffer_size.value() as u32;
        settings.audio.latency_ms = self.latency_ms.value() as u32;
        settings.audio.master_volume = self.master_volume.value() as f32;
        settings.audio.muted = self.muted.is_active();
        settings.systems.insert(
            SystemId::Nes,
            SystemSettings::Nes(NesSettings {
                core: NesCoreSettings {
                    mmc3_irq_variant: mmc3_variant_from_combo(&self.mmc3_irq_variant),
                },
                video: NesVideoSettings {
                    filter: nes_filter_from_combo(&self.nes_filter),
                },
            }),
        );
        settings
    }

    fn apply_settings(&self, settings: &DesktopSettings) {
        self.last_open_directory
            .set_text(&path_text(settings.general.last_open_directory.as_ref()));
        self.default_open_dir
            .set_text(&path_text(settings.paths.default_open_dir.as_ref()));
        self.screenshot_dir
            .set_text(&path_text(settings.paths.screenshot_dir.as_ref()));
        self.export_dir
            .set_text(&path_text(settings.paths.export_dir.as_ref()));
        self.remember_window_bounds
            .set_active(settings.host.remember_window_bounds);
        self.pause_on_focus_loss
            .set_active(settings.host.pause_on_focus_loss);
        self.clear_input_on_focus_loss
            .set_active(settings.host.clear_input_on_focus_loss);
        self.window_width
            .set_value(settings.host.window_width.unwrap_or(0) as f64);
        self.window_height
            .set_value(settings.host.window_height.unwrap_or(0) as f64);
        self.storage_policy
            .set_active_id(Some(storage_policy_id(settings.persistence.storage_policy)));
        self.state_root
            .set_text(&path_text(settings.persistence.state_root.as_ref()));
        self.mapper_save_root
            .set_text(&path_text(settings.persistence.mapper_save_root.as_ref()));
        let profile = settings.input.keyboard_profiles.get(&SystemId::Nes);
        for row in &self.controller_rows {
            let binding_key = profile
                .and_then(|profile| {
                    profile.bindings.iter().find_map(|binding| {
                        if binding.attachment == row.attachment && binding.control == row.control {
                            match binding.source {
                                HostInputSource::Keyboard(key) => Some(key),
                                _ => None,
                            }
                        } else {
                            None
                        }
                    })
                })
                .unwrap_or(KeyboardKey::KeyZ);
            row.combo.set_active_id(Some(keyboard_key_id(binding_key)));
        }
        for row in &self.shortcut_rows {
            let key = settings
                .shortcuts
                .keyboard
                .iter()
                .find(|binding| binding.action == row.action)
                .map(|binding| binding.key)
                .unwrap_or(KeyboardKey::F5);
            row.combo.set_active_id(Some(keyboard_key_id(key)));
        }
        self.fullscreen.set_active(settings.video.fullscreen);
        self.sample_rate
            .set_value(settings.audio.sample_rate as f64);
        self.buffer_size
            .set_value(settings.audio.buffer_size as f64);
        self.latency_ms.set_value(settings.audio.latency_ms as f64);
        self.master_volume
            .set_value(settings.audio.master_volume as f64);
        self.muted.set_active(settings.audio.muted);
        let nes = settings
            .systems
            .get(&SystemId::Nes)
            .map(|settings| match settings {
                SystemSettings::Nes(nes) => nes,
            })
            .cloned()
            .unwrap_or_default();
        self.mmc3_irq_variant
            .set_active_id(Some(mmc3_variant_id(nes.core.mmc3_irq_variant)));
        self.nes_filter
            .set_active_id(Some(nes_filter_id(nes.video.filter)));
    }

    fn apply_page_defaults(&self, page: u32, defaults: &DesktopSettings) {
        match page {
            PAGE_GENERAL => {
                self.last_open_directory.set_text("");
                self.default_open_dir
                    .set_text(&path_text(defaults.paths.default_open_dir.as_ref()));
                self.screenshot_dir
                    .set_text(&path_text(defaults.paths.screenshot_dir.as_ref()));
                self.export_dir
                    .set_text(&path_text(defaults.paths.export_dir.as_ref()));
                self.remember_window_bounds
                    .set_active(defaults.host.remember_window_bounds);
                self.pause_on_focus_loss
                    .set_active(defaults.host.pause_on_focus_loss);
                self.clear_input_on_focus_loss
                    .set_active(defaults.host.clear_input_on_focus_loss);
                self.window_width
                    .set_value(defaults.host.window_width.unwrap_or(0) as f64);
                self.window_height
                    .set_value(defaults.host.window_height.unwrap_or(0) as f64);
            }
            PAGE_STORAGE => {
                self.storage_policy
                    .set_active_id(Some(storage_policy_id(defaults.persistence.storage_policy)));
                self.state_root
                    .set_text(&path_text(defaults.persistence.state_root.as_ref()));
                self.mapper_save_root
                    .set_text(&path_text(defaults.persistence.mapper_save_root.as_ref()));
            }
            PAGE_CONTROLLERS => {
                self.apply_settings(defaults);
            }
            PAGE_SHORTCUTS => {
                self.apply_settings(defaults);
            }
            PAGE_VIDEO => {
                self.fullscreen.set_active(defaults.video.fullscreen);
            }
            PAGE_AUDIO => {
                self.sample_rate
                    .set_value(defaults.audio.sample_rate as f64);
                self.buffer_size
                    .set_value(defaults.audio.buffer_size as f64);
                self.latency_ms.set_value(defaults.audio.latency_ms as f64);
                self.master_volume
                    .set_value(defaults.audio.master_volume as f64);
                self.muted.set_active(defaults.audio.muted);
            }
            PAGE_SYSTEM => {
                let nes = defaults
                    .systems
                    .get(&SystemId::Nes)
                    .map(|settings| match settings {
                        SystemSettings::Nes(nes) => nes,
                    })
                    .cloned()
                    .unwrap_or_default();
                self.mmc3_irq_variant
                    .set_active_id(Some(mmc3_variant_id(nes.core.mmc3_irq_variant)));
                self.nes_filter
                    .set_active_id(Some(nes_filter_id(nes.video.filter)));
            }
            _ => {}
        }
    }

    fn refresh_saved_labels(&self) {
        let saved = self.saved_settings.borrow();
        self.last_open_directory_saved
            .set_text(&path_text(saved.general.last_open_directory.as_ref()));
        self.default_open_dir_saved
            .set_text(&path_text(saved.paths.default_open_dir.as_ref()));
        self.screenshot_dir_saved
            .set_text(&path_text(saved.paths.screenshot_dir.as_ref()));
        self.export_dir_saved
            .set_text(&path_text(saved.paths.export_dir.as_ref()));
        self.remember_window_bounds_saved
            .set_text(bool_text(saved.host.remember_window_bounds));
        self.pause_on_focus_loss_saved
            .set_text(bool_text(saved.host.pause_on_focus_loss));
        self.clear_input_on_focus_loss_saved
            .set_text(bool_text(saved.host.clear_input_on_focus_loss));
        self.window_width_saved
            .set_text(&option_u32_text(saved.host.window_width));
        self.window_height_saved
            .set_text(&option_u32_text(saved.host.window_height));
        self.storage_policy_saved
            .set_text(storage_policy_label(saved.persistence.storage_policy));
        self.state_root_saved
            .set_text(&path_text(saved.persistence.state_root.as_ref()));
        self.mapper_save_root_saved
            .set_text(&path_text(saved.persistence.mapper_save_root.as_ref()));
        for row in &self.controller_rows {
            let label = saved
                .input
                .keyboard_profiles
                .get(&SystemId::Nes)
                .and_then(|profile| {
                    profile.bindings.iter().find_map(|binding| {
                        if binding.attachment == row.attachment && binding.control == row.control {
                            match binding.source {
                                HostInputSource::Keyboard(key) => Some(keyboard_key_label(key)),
                                _ => None,
                            }
                        } else {
                            None
                        }
                    })
                })
                .unwrap_or("Unbound");
            row.saved.set_text(label);
        }
        for row in &self.shortcut_rows {
            let label = saved
                .shortcuts
                .keyboard
                .iter()
                .find(|binding| binding.action == row.action)
                .map(|binding| keyboard_key_label(binding.key))
                .unwrap_or("Unbound");
            row.saved.set_text(label);
        }
        self.fullscreen_saved
            .set_text(bool_text(saved.video.fullscreen));
        self.sample_rate_saved
            .set_text(&saved.audio.sample_rate.to_string());
        self.buffer_size_saved
            .set_text(&saved.audio.buffer_size.to_string());
        self.latency_ms_saved
            .set_text(&saved.audio.latency_ms.to_string());
        self.master_volume_saved
            .set_text(&format!("{:.2}", saved.audio.master_volume));
        self.muted_saved.set_text(bool_text(saved.audio.muted));
        let nes = saved
            .systems
            .get(&SystemId::Nes)
            .map(|settings| match settings {
                SystemSettings::Nes(nes) => nes,
            })
            .cloned()
            .unwrap_or_default();
        self.mmc3_irq_variant_saved
            .set_text(mmc3_variant_label(nes.core.mmc3_irq_variant));
        self.nes_filter_saved
            .set_text(nes_filter_label(nes.video.filter));
    }

    fn update_status(&self, prefix: &str) {
        let dirty = self.collect_settings() != *self.saved_settings.borrow();
        let suffix = if dirty { "dirty" } else { "saved" };
        self.status_label.set_text(&format!("{prefix} — {suffix}"));
    }
}

struct GeneralWidgets {
    last_open_directory: gtk::Entry,
    last_open_directory_saved: gtk::Label,
    default_open_dir: gtk::Entry,
    default_open_dir_saved: gtk::Label,
    screenshot_dir: gtk::Entry,
    screenshot_dir_saved: gtk::Label,
    export_dir: gtk::Entry,
    export_dir_saved: gtk::Label,
    remember_window_bounds: gtk::CheckButton,
    remember_window_bounds_saved: gtk::Label,
    pause_on_focus_loss: gtk::CheckButton,
    pause_on_focus_loss_saved: gtk::Label,
    clear_input_on_focus_loss: gtk::CheckButton,
    clear_input_on_focus_loss_saved: gtk::Label,
    window_width: gtk::SpinButton,
    window_width_saved: gtk::Label,
    window_height: gtk::SpinButton,
    window_height_saved: gtk::Label,
}

struct StorageWidgets {
    storage_policy: gtk::ComboBoxText,
    storage_policy_saved: gtk::Label,
    state_root: gtk::Entry,
    state_root_saved: gtk::Label,
    mapper_save_root: gtk::Entry,
    mapper_save_root_saved: gtk::Label,
}

struct VideoWidgets {
    fullscreen: gtk::CheckButton,
    fullscreen_saved: gtk::Label,
}

struct AudioWidgets {
    sample_rate: gtk::SpinButton,
    sample_rate_saved: gtk::Label,
    buffer_size: gtk::SpinButton,
    buffer_size_saved: gtk::Label,
    latency_ms: gtk::SpinButton,
    latency_ms_saved: gtk::Label,
    master_volume: gtk::SpinButton,
    master_volume_saved: gtk::Label,
    muted: gtk::CheckButton,
    muted_saved: gtk::Label,
}

struct SystemWidgets {
    mmc3_irq_variant: gtk::ComboBoxText,
    mmc3_irq_variant_saved: gtk::Label,
    nes_filter: gtk::ComboBoxText,
    nes_filter_saved: gtk::Label,
}

fn build_general_page() -> (gtk::ScrolledWindow, GeneralWidgets) {
    let grid = settings_grid();
    let last_open_directory = gtk::Entry::new();
    let last_open_directory_saved = add_row(
        &grid,
        0,
        "Last open directory",
        &last_open_directory,
        "Immediate",
    );
    let default_open_dir = gtk::Entry::new();
    let default_open_dir_saved = add_row(
        &grid,
        1,
        "Default open directory",
        &default_open_dir,
        "Immediate",
    );
    let screenshot_dir = gtk::Entry::new();
    let screenshot_dir_saved = add_row(
        &grid,
        2,
        "Screenshot directory",
        &screenshot_dir,
        "Next export",
    );
    let export_dir = gtk::Entry::new();
    let export_dir_saved = add_row(&grid, 3, "Export directory", &export_dir, "Next export");
    let remember_window_bounds = gtk::CheckButton::new();
    let remember_window_bounds_saved = add_row(
        &grid,
        4,
        "Remember window bounds",
        &remember_window_bounds,
        "Next window recreation",
    );
    let pause_on_focus_loss = gtk::CheckButton::new();
    let pause_on_focus_loss_saved = add_row(
        &grid,
        5,
        "Pause on focus loss",
        &pause_on_focus_loss,
        "Immediate",
    );
    let clear_input_on_focus_loss = gtk::CheckButton::new();
    let clear_input_on_focus_loss_saved = add_row(
        &grid,
        6,
        "Clear input on focus loss",
        &clear_input_on_focus_loss,
        "Immediate",
    );
    let window_width = gtk::SpinButton::with_range(0.0, 8192.0, 1.0);
    let window_width_saved = add_row(
        &grid,
        7,
        "Window width",
        &window_width,
        "Next window recreation",
    );
    let window_height = gtk::SpinButton::with_range(0.0, 8192.0, 1.0);
    let window_height_saved = add_row(
        &grid,
        8,
        "Window height",
        &window_height,
        "Next window recreation",
    );
    (
        wrap_page(&grid),
        GeneralWidgets {
            last_open_directory,
            last_open_directory_saved,
            default_open_dir,
            default_open_dir_saved,
            screenshot_dir,
            screenshot_dir_saved,
            export_dir,
            export_dir_saved,
            remember_window_bounds,
            remember_window_bounds_saved,
            pause_on_focus_loss,
            pause_on_focus_loss_saved,
            clear_input_on_focus_loss,
            clear_input_on_focus_loss_saved,
            window_width,
            window_width_saved,
            window_height,
            window_height_saved,
        },
    )
}

fn build_storage_page() -> (gtk::ScrolledWindow, StorageWidgets) {
    let grid = settings_grid();
    let storage_policy = gtk::ComboBoxText::new();
    for (id, label) in [
        (storage_policy_id(StoragePolicy::RomSidecar), "ROM sidecar"),
        (storage_policy_id(StoragePolicy::AppData), "App data"),
        (
            storage_policy_id(StoragePolicy::CustomRoots),
            "Custom roots",
        ),
    ] {
        storage_policy.append(Some(id), label);
    }
    let storage_policy_saved =
        add_row(&grid, 0, "Storage policy", &storage_policy, "Next ROM load");
    let state_root = gtk::Entry::new();
    let state_root_saved = add_row(&grid, 1, "State root", &state_root, "Next ROM load");
    let mapper_save_root = gtk::Entry::new();
    let mapper_save_root_saved = add_row(
        &grid,
        2,
        "Mapper save root",
        &mapper_save_root,
        "Next ROM load",
    );
    (
        wrap_page(&grid),
        StorageWidgets {
            storage_policy,
            storage_policy_saved,
            state_root,
            state_root_saved,
            mapper_save_root,
            mapper_save_root_saved,
        },
    )
}

fn build_controllers_page() -> (gtk::ScrolledWindow, Vec<ControllerRow>) {
    let grid = settings_grid();
    let mut rows = Vec::new();
    for (index, descriptor) in keyboard_binding_descriptors().iter().enumerate() {
        let combo = keyboard_key_combo();
        let saved = add_row(
            &grid,
            index as i32,
            &format!(
                "{} {}",
                descriptor.attachment_label, descriptor.control_label
            ),
            &combo,
            "Immediate",
        );
        rows.push(ControllerRow {
            attachment: PersistedAttachmentId::new(descriptor.attachment.as_str()),
            control: PersistedControlId::digital(descriptor.control.as_str()),
            combo,
            saved,
        });
    }
    (wrap_page(&grid), rows)
}

fn build_shortcuts_page() -> (gtk::ScrolledWindow, Vec<ShortcutRow>) {
    let grid = settings_grid();
    let mut rows = Vec::new();
    for (index, descriptor) in shortcut_descriptors().iter().enumerate() {
        let combo = keyboard_key_combo();
        let saved = add_row(&grid, index as i32, descriptor.label, &combo, "Immediate");
        rows.push(ShortcutRow {
            action: descriptor.action,
            combo,
            saved,
        });
    }
    (wrap_page(&grid), rows)
}

fn build_video_page() -> (gtk::ScrolledWindow, VideoWidgets) {
    let grid = settings_grid();
    let fullscreen = gtk::CheckButton::new();
    let fullscreen_saved = add_row(
        &grid,
        0,
        "Start fullscreen",
        &fullscreen,
        "Next window recreation",
    );
    (
        wrap_page(&grid),
        VideoWidgets {
            fullscreen,
            fullscreen_saved,
        },
    )
}

fn build_audio_page() -> (gtk::ScrolledWindow, AudioWidgets) {
    let grid = settings_grid();
    let sample_rate = gtk::SpinButton::with_range(8_000.0, 192_000.0, 1_000.0);
    let sample_rate_saved = add_row(
        &grid,
        0,
        "Sample rate",
        &sample_rate,
        "Next audio/session recreation",
    );
    let buffer_size = gtk::SpinButton::with_range(32.0, 4096.0, 32.0);
    let buffer_size_saved = add_row(
        &grid,
        1,
        "Buffer size",
        &buffer_size,
        "Next audio/session recreation",
    );
    let latency_ms = gtk::SpinButton::with_range(2.0, 500.0, 1.0);
    let latency_ms_saved = add_row(
        &grid,
        2,
        "Latency (ms)",
        &latency_ms,
        "Next audio/session recreation",
    );
    let master_volume = gtk::SpinButton::with_range(0.0, 1.0, 0.05);
    let master_volume_saved = add_row(
        &grid,
        3,
        "Master volume",
        &master_volume,
        "Next audio/session recreation",
    );
    let muted = gtk::CheckButton::new();
    let muted_saved = add_row(&grid, 4, "Muted", &muted, "Next audio/session recreation");
    (
        wrap_page(&grid),
        AudioWidgets {
            sample_rate,
            sample_rate_saved,
            buffer_size,
            buffer_size_saved,
            latency_ms,
            latency_ms_saved,
            master_volume,
            master_volume_saved,
            muted,
            muted_saved,
        },
    )
}

fn build_system_page() -> (gtk::ScrolledWindow, SystemWidgets) {
    let grid = settings_grid();
    let mmc3_irq_variant = gtk::ComboBoxText::new();
    for (id, label) in [
        (mmc3_variant_id(None), "Default"),
        (mmc3_variant_id(Some(Mmc3IrqVariant::Sharp)), "Sharp"),
        (mmc3_variant_id(Some(Mmc3IrqVariant::Nec)), "NEC"),
    ] {
        mmc3_irq_variant.append(Some(id), label);
    }
    let mmc3_irq_variant_saved = add_row(
        &grid,
        0,
        "MMC3 IRQ variant",
        &mmc3_irq_variant,
        "Next ROM load",
    );
    let nes_filter = gtk::ComboBoxText::new();
    for (id, label) in [
        (nes_filter_id(NesVideoFilter::None), "None"),
        (nes_filter_id(NesVideoFilter::NtscRgb), "NTSC RGB"),
        (
            nes_filter_id(NesVideoFilter::NtscComposite),
            "NTSC Composite",
        ),
        (nes_filter_id(NesVideoFilter::NtscSVideo), "NTSC S-Video"),
    ] {
        nes_filter.append(Some(id), label);
    }
    let nes_filter_saved = add_row(
        &grid,
        1,
        "NES filter",
        &nes_filter,
        "Next session recreation",
    );
    (
        wrap_page(&grid),
        SystemWidgets {
            mmc3_irq_variant,
            mmc3_irq_variant_saved,
            nes_filter,
            nes_filter_saved,
        },
    )
}

fn settings_grid() -> gtk::Grid {
    let grid = gtk::Grid::new();
    grid.set_row_spacing(6);
    grid.set_column_spacing(12);
    grid.set_margin_start(12);
    grid.set_margin_end(12);
    grid.set_margin_top(12);
    grid.set_margin_bottom(12);
    grid
}

fn wrap_page(grid: &gtk::Grid) -> gtk::ScrolledWindow {
    let scrolled = gtk::ScrolledWindow::new();
    scrolled.set_child(Some(grid));
    scrolled
}

fn add_row<T: IsA<gtk::Widget>>(
    grid: &gtk::Grid,
    row: i32,
    label: &str,
    widget: &T,
    timing: &str,
) -> gtk::Label {
    let title = gtk::Label::new(Some(label));
    title.set_xalign(0.0);
    let saved = gtk::Label::new(Some(""));
    saved.set_xalign(0.0);
    let timing_label = gtk::Label::new(Some(timing));
    timing_label.set_xalign(0.0);
    grid.attach(&title, 0, row, 1, 1);
    grid.attach(widget, 1, row, 1, 1);
    grid.attach(&saved, 2, row, 1, 1);
    grid.attach(&timing_label, 3, row, 1, 1);
    saved
}

fn keyboard_key_combo() -> gtk::ComboBoxText {
    let combo = gtk::ComboBoxText::new();
    for key in EDITABLE_KEYS {
        combo.append(Some(keyboard_key_id(*key)), keyboard_key_label(*key));
    }
    combo
}

fn keyboard_key_from_combo(combo: &gtk::ComboBoxText) -> Option<KeyboardKey> {
    combo.active_id().as_deref().and_then(keyboard_key_from_id)
}

fn parse_optional_path(text: &str) -> Option<PathBuf> {
    optional_string(text).map(PathBuf::from)
}

fn optional_string(text: &str) -> Option<String> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn path_text(path: Option<&PathBuf>) -> String {
    path.map(|path| path.display().to_string())
        .unwrap_or_else(|| "Default".into())
}

fn bool_text(value: bool) -> &'static str {
    if value { "On" } else { "Off" }
}

fn option_u32_text(value: Option<u32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "Default".into())
}

fn value_or_none(spin: &gtk::SpinButton) -> Option<u32> {
    let value = spin.value() as u32;
    (value > 0).then_some(value)
}

fn storage_policy_id(policy: StoragePolicy) -> &'static str {
    match policy {
        StoragePolicy::RomSidecar => "rom_sidecar",
        StoragePolicy::AppData => "app_data",
        StoragePolicy::CustomRoots => "custom_roots",
    }
}

fn storage_policy_label(policy: StoragePolicy) -> &'static str {
    match policy {
        StoragePolicy::RomSidecar => "ROM sidecar",
        StoragePolicy::AppData => "App data",
        StoragePolicy::CustomRoots => "Custom roots",
    }
}

fn storage_policy_from_combo(combo: &gtk::ComboBoxText) -> StoragePolicy {
    match combo.active_id().as_deref() {
        Some("app_data") => StoragePolicy::AppData,
        Some("custom_roots") => StoragePolicy::CustomRoots,
        _ => StoragePolicy::RomSidecar,
    }
}

fn mmc3_variant_id(value: Option<Mmc3IrqVariant>) -> &'static str {
    match value {
        None => "default",
        Some(Mmc3IrqVariant::Sharp) => "sharp",
        Some(Mmc3IrqVariant::Nec) => "nec",
    }
}

fn mmc3_variant_label(value: Option<Mmc3IrqVariant>) -> &'static str {
    match value {
        None => "Default",
        Some(Mmc3IrqVariant::Sharp) => "Sharp",
        Some(Mmc3IrqVariant::Nec) => "NEC",
    }
}

fn mmc3_variant_from_combo(combo: &gtk::ComboBoxText) -> Option<Mmc3IrqVariant> {
    match combo.active_id().as_deref() {
        Some("sharp") => Some(Mmc3IrqVariant::Sharp),
        Some("nec") => Some(Mmc3IrqVariant::Nec),
        _ => None,
    }
}

fn nes_filter_id(value: NesVideoFilter) -> &'static str {
    match value {
        NesVideoFilter::None => "none",
        NesVideoFilter::NtscRgb => "ntsc_rgb",
        NesVideoFilter::NtscComposite => "ntsc_composite",
        NesVideoFilter::NtscSVideo => "ntsc_svideo",
    }
}

fn nes_filter_label(value: NesVideoFilter) -> &'static str {
    match value {
        NesVideoFilter::None => "None",
        NesVideoFilter::NtscRgb => "NTSC RGB",
        NesVideoFilter::NtscComposite => "NTSC Composite",
        NesVideoFilter::NtscSVideo => "NTSC S-Video",
    }
}

fn nes_filter_from_combo(combo: &gtk::ComboBoxText) -> NesVideoFilter {
    match combo.active_id().as_deref() {
        Some("none") => NesVideoFilter::None,
        Some("ntsc_rgb") => NesVideoFilter::NtscRgb,
        Some("ntsc_svideo") => NesVideoFilter::NtscSVideo,
        _ => NesVideoFilter::NtscComposite,
    }
}
