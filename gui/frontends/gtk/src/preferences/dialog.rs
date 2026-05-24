use super::values::{
    bool_text, keyboard_key_from_combo, mmc3_variant_from_combo, mmc3_variant_id,
    mmc3_variant_label, nes_filter_from_combo, nes_filter_id, nes_filter_label, option_u32_text,
    parse_optional_path, path_text, storage_policy_from_combo, storage_policy_id,
    storage_policy_label, value_or_none,
};
use super::widgets::{
    ControllerRow, ShortcutRow, build_audio_page, build_controllers_page, build_general_page,
    build_shortcuts_page, build_storage_page, build_system_page, build_video_page,
};
use gtk::prelude::*;
use nerust_contract_settings::{
    desktop::{DesktopSettings, SystemSettings},
    input::{BindingProfile, ControlBinding, HostInputSource, KeyboardKey},
    nes::{NesCoreSettings, NesSettings, NesVideoSettings},
    shortcut::ShortcutBinding,
};
use nerust_gui_runtime::settings::DesktopSettingsManager;
use nerust_gui_shell::settings::{
    bindings::keys::{keyboard_key_id, keyboard_key_label},
    defaults::{manager::current_or_default, seed::default_desktop_settings},
};
use nerust_input_schema::SystemId;
use std::cell::RefCell;
use std::rc::Rc;

const PAGE_GENERAL: u32 = 0;
const PAGE_STORAGE: u32 = 1;
const PAGE_CONTROLLERS: u32 = 2;
const PAGE_SHORTCUTS: u32 = 3;
const PAGE_VIDEO: u32 = 4;
const PAGE_AUDIO: u32 = 5;
const PAGE_SYSTEM: u32 = 6;

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

pub(super) fn present_preferences_dialog(
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
                let defaults = default_desktop_settings();
                state.apply_settings(&defaults);
                state.update_status("Showing defaults (not saved)");
            });
        }
        {
            let state = state.clone();
            reset_page_button.connect_clicked(move |_| {
                let defaults = default_desktop_settings();
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
