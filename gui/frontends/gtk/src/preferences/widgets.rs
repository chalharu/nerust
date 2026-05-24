use super::values::{mmc3_variant_id, nes_filter_id, storage_policy_id};
use gtk::prelude::*;
use nerust_contract_options::Mmc3IrqVariant;
use nerust_contract_settings::{
    desktop::StoragePolicy,
    input::{PersistedAttachmentId, PersistedControlId},
    nes::NesVideoFilter,
};
use nerust_gui_shell::settings::bindings::{
    descriptors::{keyboard_binding_descriptors, shortcut_descriptors},
    keys::{editable_keys, keyboard_key_id, keyboard_key_label},
};

#[derive(Clone)]
pub(super) struct ControllerRow {
    pub(super) attachment: PersistedAttachmentId,
    pub(super) control: PersistedControlId,
    pub(super) combo: gtk::ComboBoxText,
    pub(super) saved: gtk::Label,
}

#[derive(Clone)]
pub(super) struct ShortcutRow {
    pub(super) action: nerust_contract_settings::shortcut::ShortcutAction,
    pub(super) combo: gtk::ComboBoxText,
    pub(super) saved: gtk::Label,
}

pub(super) struct GeneralWidgets {
    pub(super) last_open_directory: gtk::Entry,
    pub(super) last_open_directory_saved: gtk::Label,
    pub(super) default_open_dir: gtk::Entry,
    pub(super) default_open_dir_saved: gtk::Label,
    pub(super) screenshot_dir: gtk::Entry,
    pub(super) screenshot_dir_saved: gtk::Label,
    pub(super) export_dir: gtk::Entry,
    pub(super) export_dir_saved: gtk::Label,
    pub(super) remember_window_bounds: gtk::CheckButton,
    pub(super) remember_window_bounds_saved: gtk::Label,
    pub(super) pause_on_focus_loss: gtk::CheckButton,
    pub(super) pause_on_focus_loss_saved: gtk::Label,
    pub(super) clear_input_on_focus_loss: gtk::CheckButton,
    pub(super) clear_input_on_focus_loss_saved: gtk::Label,
    pub(super) window_width: gtk::SpinButton,
    pub(super) window_width_saved: gtk::Label,
    pub(super) window_height: gtk::SpinButton,
    pub(super) window_height_saved: gtk::Label,
}

pub(super) struct StorageWidgets {
    pub(super) storage_policy: gtk::ComboBoxText,
    pub(super) storage_policy_saved: gtk::Label,
    pub(super) state_root: gtk::Entry,
    pub(super) state_root_saved: gtk::Label,
    pub(super) mapper_save_root: gtk::Entry,
    pub(super) mapper_save_root_saved: gtk::Label,
}

pub(super) struct VideoWidgets {
    pub(super) fullscreen: gtk::CheckButton,
    pub(super) fullscreen_saved: gtk::Label,
}

pub(super) struct AudioWidgets {
    pub(super) sample_rate: gtk::SpinButton,
    pub(super) sample_rate_saved: gtk::Label,
    pub(super) buffer_size: gtk::SpinButton,
    pub(super) buffer_size_saved: gtk::Label,
    pub(super) latency_ms: gtk::SpinButton,
    pub(super) latency_ms_saved: gtk::Label,
    pub(super) master_volume: gtk::SpinButton,
    pub(super) master_volume_saved: gtk::Label,
    pub(super) muted: gtk::CheckButton,
    pub(super) muted_saved: gtk::Label,
}

pub(super) struct SystemWidgets {
    pub(super) mmc3_irq_variant: gtk::ComboBoxText,
    pub(super) mmc3_irq_variant_saved: gtk::Label,
    pub(super) nes_filter: gtk::ComboBoxText,
    pub(super) nes_filter_saved: gtk::Label,
}

pub(super) fn build_general_page() -> (gtk::ScrolledWindow, GeneralWidgets) {
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

pub(super) fn build_storage_page() -> (gtk::ScrolledWindow, StorageWidgets) {
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

pub(super) fn build_controllers_page() -> (gtk::ScrolledWindow, Vec<ControllerRow>) {
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

pub(super) fn build_shortcuts_page() -> (gtk::ScrolledWindow, Vec<ShortcutRow>) {
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

pub(super) fn build_video_page() -> (gtk::ScrolledWindow, VideoWidgets) {
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

pub(super) fn build_audio_page() -> (gtk::ScrolledWindow, AudioWidgets) {
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

pub(super) fn build_system_page() -> (gtk::ScrolledWindow, SystemWidgets) {
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
    for key in editable_keys() {
        combo.append(Some(keyboard_key_id(*key)), keyboard_key_label(*key));
    }
    combo
}
