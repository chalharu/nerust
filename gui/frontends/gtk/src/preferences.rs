use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};

use gio::glib::object::{Cast as _, IsA};
use gtk::prelude::{
    BoxExt as _, CheckButtonExt as _, ComboBoxExt as _, DialogExt as _, EditableExt as _,
    GtkWindowExt as _, WidgetExt as _,
};
use nerust_core_traits::factory::CoreFactory;
use nerust_gui_settings::{language::AppLanguage, local::ScalingMode, shared::StoragePolicy};
use nerust_gui_viewmodel::settings::{
    SettingsViewModel, StoragePathError, StoragePathValidator, Subscription,
};

use crate::State;

// ---------------------------------------------------------------------------
// Widget group structs
// ---------------------------------------------------------------------------

struct GeneralWidgets {
    language_combo: gtk::ComboBoxText,
    storage_policy_combo: gtk::ComboBoxText,
    storage_dir_entry: gtk::Entry,
    storage_dir_row: gtk::Box,
    storage_error_label: gtk::Label,
}

struct VideoWidgets {
    fullscreen_check: gtk::CheckButton,
    scaling_combo: gtk::ComboBoxText,
    vsync_check: gtk::CheckButton,
}

struct AudioWidgets {
    mute_check: gtk::CheckButton,
    volume_spin: gtk::SpinButton,
    sample_rate_combo: gtk::ComboBoxText,
    latency_spin: gtk::SpinButton,
}

struct InputTabWidgets {
    _notebook: gtk::Notebook,
    _pages: Vec<gtk::Box>,
}

struct SystemTabWidgets {
    _notebook: gtk::Notebook,
    _pages: Vec<gtk::Box>,
}

// ---------------------------------------------------------------------------
// PreferencesBinding
// ---------------------------------------------------------------------------

pub(crate) struct PreferencesBinding {
    pub vm: SettingsViewModel,
    pub subscriptions: RefCell<Vec<Subscription>>,
    pub general: GeneralWidgets,
    pub video: VideoWidgets,
    pub audio: AudioWidgets,
    pub input: InputTabWidgets,
    pub system: SystemTabWidgets,
    pub ok_button: gtk::Widget,
    pub error_label: gtk::Label,
    pub refreshing: Cell<bool>,
}

impl PreferencesBinding {
    fn new(
        vm: SettingsViewModel,
        general: GeneralWidgets,
        video: VideoWidgets,
        audio: AudioWidgets,
        input: InputTabWidgets,
        system: SystemTabWidgets,
        ok_button: gtk::Widget,
        error_label: gtk::Label,
    ) -> Rc<Self> {
        let gv = vm.general.view.get();
        let vv = vm.video.view.get();
        let av = vm.audio.view.get();
        let ok = vm.finish().is_ok();

        let binding: Rc<Self> = Rc::new_cyclic(|weak: &std::rc::Weak<Self>| {
            let mut subs = Vec::new();

            // --- general.view ---
            subs.push(vm.general.view.observe({
                let weak = weak.clone();
                move |v| {
                    let Some(b) = weak.upgrade() else { return };
                    if b.refreshing.get() { return; }
                    b.general.language_combo.set_active_id(Some(match v.language {
                        AppLanguage::Japanese => "japanese",
                        AppLanguage::English => "english",
                        AppLanguage::SystemDefault => "system_default",
                    }));
                    b.general.storage_policy_combo.set_active_id(Some(match v.storage_policy {
                        StoragePolicy::AppSharedData => "app_shared_data",
                        StoragePolicy::CustomDirectory => "custom_directory",
                        StoragePolicy::Sidecar => "sidecar",
                    }));
                    let show_dir = matches!(v.storage_policy, StoragePolicy::CustomDirectory);
                    b.general.storage_dir_row.set_visible(show_dir);
                    if !v.storage_directory.is_empty() {
                        b.general.storage_dir_entry.set_text(&v.storage_directory);
                    }
                }
            }));

            // --- video.view ---
            subs.push(vm.video.view.observe({
                let weak = weak.clone();
                move |v| {
                    let Some(b) = weak.upgrade() else { return };
                    if b.refreshing.get() { return; }
                    b.video.fullscreen_check.set_active(v.fullscreen_default);
                    b.video.scaling_combo.set_active_id(Some(match v.scaling {
                        ScalingMode::FitToWindow => "fit",
                        ScalingMode::X1 => "1",
                        ScalingMode::X2 => "2",
                        ScalingMode::X3 => "3",
                        ScalingMode::X4 => "4",
                        ScalingMode::X5 => "5",
                    }));
                    b.video.vsync_check.set_active(v.vsync);
                }
            }));

            // --- audio.view ---
            subs.push(vm.audio.view.observe({
                let weak = weak.clone();
                move |v| {
                    let Some(b) = weak.upgrade() else { return };
                    if b.refreshing.get() { return; }
                    b.audio.mute_check.set_active(v.muted);
                    b.audio.volume_spin.set_value(f64::from(v.volume_percent));
                    let active = format!("{}", v.sample_rate);
                    b.audio.sample_rate_combo.set_active_id(Some(&active));
                    b.audio.latency_spin.set_value(f64::from(v.latency_ms));
                }
            }));

            // --- revision → validation ---
            subs.push(vm.revision.observe({
                let weak = weak.clone();
                move |_| {
                    let Some(b) = weak.upgrade() else { return };
                    match b.vm.finish() {
                        Ok(_) => b.ok_button.set_sensitive(true),
                        Err(validation) => {
                            b.ok_button.set_sensitive(false);
                            if let Some(first) = validation.issues.first() {
                                b.error_label.set_text(&first.message);
                            }
                        }
                    }
                }
            }));

            Self {
                vm,
                subscriptions: RefCell::new(subs),
                general,
                video,
                audio,
                input,
                system,
                ok_button,
                error_label,
                refreshing: Cell::new(false),
            }
        });

        // --- Initial widget population ---
        binding.general.language_combo.set_active_id(Some(match gv.language {
            AppLanguage::Japanese => "japanese",
            AppLanguage::English => "english",
            AppLanguage::SystemDefault => "system_default",
        }));
        binding.general.storage_policy_combo.set_active_id(Some(match gv.storage_policy {
            StoragePolicy::AppSharedData => "app_shared_data",
            StoragePolicy::CustomDirectory => "custom_directory",
            StoragePolicy::Sidecar => "sidecar",
        }));
        let show_dir = matches!(gv.storage_policy, StoragePolicy::CustomDirectory);
        binding.general.storage_dir_row.set_visible(show_dir);
        if !gv.storage_directory.is_empty() {
            binding.general.storage_dir_entry.set_text(&gv.storage_directory);
        }

        // Video initial values
        binding.video.fullscreen_check.set_active(vv.fullscreen_default);
        binding.video.scaling_combo.set_active_id(Some(match vv.scaling {
            ScalingMode::FitToWindow => "fit",
            ScalingMode::X1 => "1",
            ScalingMode::X2 => "2",
            ScalingMode::X3 => "3",
            ScalingMode::X4 => "4",
            ScalingMode::X5 => "5",
        }));
        binding.video.vsync_check.set_active(vv.vsync);

        // Audio initial values
        binding.audio.mute_check.set_active(av.muted);
        binding.audio.volume_spin.set_value(f64::from(av.volume_percent));
        let active = format!("{}", av.sample_rate);
        binding.audio.sample_rate_combo.set_active_id(Some(&active));
        binding.audio.latency_spin.set_value(f64::from(av.latency_ms));

        binding.ok_button.set_sensitive(ok);

        binding
    }
}

// ---------------------------------------------------------------------------
// GtkStoragePathValidator
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct GtkStoragePathValidator;
impl StoragePathValidator for GtkStoragePathValidator {
    fn validate(&self, path: &std::path::Path) -> Result<(), StoragePathError> {
        let result = nerust_gui_runtime::settings::apply::validate_directory_path_typed(path);
        match result {
            Ok(()) => Ok(()),
            Err(nerust_gui_runtime::settings::apply::DirectoryValidationError::NotDirectory) => {
                Err(StoragePathError::NotDirectory)
            }
            Err(nerust_gui_runtime::settings::apply::DirectoryValidationError::Inaccessible(e)) => {
                Err(StoragePathError::Inaccessible(e.to_string()))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Main dialog entry point
// ---------------------------------------------------------------------------

pub(crate) fn present_preferences_dialog(
    parent: &gtk::ApplicationWindow,
    state: Rc<RefCell<State>>,
    on_close: impl FnOnce() + 'static,
) {
    let (snapshot, registry, audio_registry) = {
        let s = state.borrow();
        (s.settings_snapshot().clone(), s.ctx.registry.clone(), s.ctx.audio_registry.clone())
    };

    let factories: Vec<Arc<dyn CoreFactory>> =
        registry.all().iter().map(Arc::clone).collect();

    let supported_sample_rates: Arc<[u32]> = {
        let rates = audio_registry.supported_rates();
        if rates.is_empty() {
            Arc::new([44_100, 48_000])
        } else {
            rates.iter().copied().collect()
        }
    };

    let vm = SettingsViewModel::new(
        snapshot,
        factories,
        supported_sample_rates,
        Rc::new(GtkStoragePathValidator) as Rc<dyn StoragePathValidator>,
    );

    let dialog = gtk::Dialog::builder()
        .transient_for(parent)
        .modal(true)
        .title("Preferences")
        .default_width(900)
        .default_height(560)
        .build();
    dialog.add_button("Cancel", gtk::ResponseType::Cancel);
    dialog.add_button("OK", gtk::ResponseType::Ok);

    let Some(ok_button) = dialog.widget_for_response(gtk::ResponseType::Ok) else {
        log::error!("preferences dialog missing OK button, aborting");
        return;
    };
    if let Some(action_box) = ok_button
        .parent()
        .and_then(|parent| parent.downcast::<gtk::Box>().ok())
    {
        action_box.set_spacing(12);
        action_box.set_margin_top(12);
        action_box.set_margin_bottom(12);
        action_box.set_margin_start(12);
        action_box.set_margin_end(12);
    }

    let content = dialog.content_area();
    content.set_spacing(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_vexpand(true);

    let root = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    root.set_hexpand(true);
    root.set_vexpand(true);
    content.append(&root);

    let stack = gtk::Stack::new();
    stack.set_hexpand(true);
    stack.set_vexpand(true);
    let sidebar = gtk::StackSidebar::new();
    sidebar.set_stack(&stack);
    sidebar.set_vexpand(true);
    root.append(&sidebar);
    root.append(&stack);

    let (general_page_scroller, general_page) = stack_page();
    let (input_page_scroller, input_page) = stack_page();
    let (video_page_scroller, video_page) = stack_page();
    let (audio_page_scroller, audio_page) = stack_page();
    let (system_page_scroller, system_page) = stack_page();
    stack.add_titled(&general_page_scroller, Some("general"), "General");
    stack.add_titled(&input_page_scroller, Some("input"), "Input");
    stack.add_titled(&video_page_scroller, Some("video"), "Video");
    stack.add_titled(&audio_page_scroller, Some("audio"), "Audio");
    stack.add_titled(&system_page_scroller, Some("system"), "System");

    let error_label = gtk::Label::new(None);
    error_label.set_xalign(0.0);
    content.append(&error_label);

    // ---- General page ----
    let language_combo = combo_box(&[
        ("system_default", "System Default"),
        ("japanese", "Japanese"),
        ("english", "English"),
    ]);
    general_page.append(&labeled_row("Language", &language_combo));

    let storage_policy_combo = combo_box(&[
        ("sidecar", "Sidecar"),
        ("app_shared_data", "App Shared Data"),
        ("custom_directory", "Custom Directory"),
    ]);
    general_page.append(&labeled_row("Save Storage Policy", &storage_policy_combo));
    let storage_dir_entry = gtk::Entry::new();
    let storage_dir_row = labeled_row("Save Storage Directory", &storage_dir_entry);
    let storage_error_label = gtk::Label::new(None);
    storage_error_label.set_xalign(0.0);
    general_page.append(&storage_dir_row);
    general_page.append(&storage_error_label);

    // ---- Video page ----
    let fullscreen_check = gtk::CheckButton::with_label("Fullscreen Default");
    video_page.append(&fullscreen_check);
    let scaling_combo = combo_box(&[
        ("fit", "Fit to Window"),
        ("1", "1x"),
        ("2", "2x"),
        ("3", "3x"),
        ("4", "4x"),
        ("5", "5x"),
    ]);
    video_page.append(&labeled_row("Scaling", &scaling_combo));
    let vsync_check = gtk::CheckButton::with_label("Vsync");
    video_page.append(&vsync_check);

    // ---- Audio page ----
    let mute_check = gtk::CheckButton::with_label("Mute");
    audio_page.append(&mute_check);
    let volume_spin = gtk::SpinButton::with_range(0.0, 100.0, 1.0);
    audio_page.append(&labeled_row("Master Volume", &volume_spin));
    let sample_rate_combo = {
        let rates: &[u32] = if audio_registry.supported_rates().is_empty() {
            &[44_100, 48_000]
        } else {
            audio_registry.supported_rates()
        };
        let combo = gtk::ComboBoxText::new();
        for &rate in rates {
            let id = format!("{rate}");
            combo.append(Some(&id), &id);
        }
        combo
    };
    audio_page.append(&labeled_row("Sample Rate", &sample_rate_combo));
    let latency_spin = gtk::SpinButton::with_range(10.0, 200.0, 1.0);
    audio_page.append(&labeled_row("Audio Latency", &latency_spin));

    // ---- Input page (tabs) ----
    let input_notebook = gtk::Notebook::new();
    input_notebook.set_scrollable(true);
    input_notebook.set_tab_pos(gtk::PositionType::Top);
    input_page.append(&input_notebook);
    let mut input_pages = Vec::new();
    for factory in registry.all() {
        let tab_label = gtk::Label::new(Some(factory.display_name()));
        let tab_page = gtk::Box::new(gtk::Orientation::Vertical, 6);
        tab_page.set_margin_start(6);
        tab_page.set_margin_end(6);
        tab_page.set_margin_top(6);
        tab_page.set_margin_bottom(6);
        input_notebook.append_page(&tab_page, Some(&tab_label));
        input_pages.push(tab_page);
    }

    // ---- System page (tabs) ----
    let system_notebook = gtk::Notebook::new();
    system_notebook.set_scrollable(true);
    system_notebook.set_tab_pos(gtk::PositionType::Top);
    system_page.append(&system_notebook);
    let mut system_pages = Vec::new();
    for factory in registry.all() {
        let tab_label = gtk::Label::new(Some(factory.display_name()));
        let tab_page = gtk::Box::new(gtk::Orientation::Vertical, 6);
        tab_page.set_margin_start(6);
        tab_page.set_margin_end(6);
        tab_page.set_margin_top(6);
        tab_page.set_margin_bottom(6);
        system_notebook.append_page(&tab_page, Some(&tab_label));
        system_pages.push(tab_page);
    }

    // Build binding
    let general_w = GeneralWidgets {
        language_combo: language_combo.clone(),
        storage_policy_combo: storage_policy_combo.clone(),
        storage_dir_entry: storage_dir_entry.clone(),
        storage_dir_row: storage_dir_row.clone(),
        storage_error_label: storage_error_label.clone(),
    };
    let video_w = VideoWidgets {
        fullscreen_check: fullscreen_check.clone(),
        scaling_combo: scaling_combo.clone(),
        vsync_check: vsync_check.clone(),
    };
    let audio_w = AudioWidgets {
        mute_check: mute_check.clone(),
        volume_spin: volume_spin.clone(),
        sample_rate_combo: sample_rate_combo.clone(),
        latency_spin: latency_spin.clone(),
    };
    let input_w = InputTabWidgets {
        _notebook: input_notebook,
        _pages: input_pages,
    };
    let system_w = SystemTabWidgets {
        _notebook: system_notebook,
        _pages: system_pages,
    };

    let _binding = PreferencesBinding::new(
        vm, general_w, video_w, audio_w, input_w, system_w, ok_button, error_label,
    );

    // ---- Signal handlers ----
    fn weak_handler(b: &Rc<PreferencesBinding>) -> std::rc::Weak<PreferencesBinding> {
        Rc::downgrade(b)
    }

    language_combo.connect_changed({
        let w = weak_handler(&_binding);
        move |combo| {
            let Some(b) = w.upgrade() else { return };
            if b.refreshing.get() { return; }
            let lang = match combo.active_id().as_deref() {
                Some("japanese") => AppLanguage::Japanese,
                Some("english") => AppLanguage::English,
                _ => AppLanguage::SystemDefault,
            };
            let _ = b.vm.general.set_language(lang);
        }
    });
    storage_policy_combo.connect_changed({
        let w = weak_handler(&_binding);
        move |combo| {
            let Some(b) = w.upgrade() else { return };
            if b.refreshing.get() { return; }
            let policy = match combo.active_id().as_deref() {
                Some("app_shared_data") => StoragePolicy::AppSharedData,
                Some("custom_directory") => StoragePolicy::CustomDirectory,
                _ => StoragePolicy::Sidecar,
            };
            let _ = b.vm.general.set_storage_policy(policy);
        }
    });
    storage_dir_entry.connect_changed({
        let w = weak_handler(&_binding);
        move |entry| {
            let Some(b) = w.upgrade() else { return };
            if b.refreshing.get() { return; }
            let text = entry.text();
            let path = (!text.is_empty()).then(|| std::path::PathBuf::from(text.as_str()));
            let _ = b.vm.general.set_storage_directory(path);
        }
    });
    fullscreen_check.connect_toggled({
        let w = weak_handler(&_binding);
        move |button| {
            let Some(b) = w.upgrade() else { return };
            if b.refreshing.get() { return; }
            let _ = b.vm.video.set_fullscreen_default(button.is_active());
        }
    });
    scaling_combo.connect_changed({
        let w = weak_handler(&_binding);
        move |combo| {
            let Some(b) = w.upgrade() else { return };
            if b.refreshing.get() { return; }
            let scaling = match combo.active_id().as_deref() {
                Some("1") => ScalingMode::X1,
                Some("2") => ScalingMode::X2,
                Some("3") => ScalingMode::X3,
                Some("4") => ScalingMode::X4,
                Some("5") => ScalingMode::X5,
                _ => ScalingMode::FitToWindow,
            };
            let _ = b.vm.video.set_scaling(scaling);
        }
    });
    vsync_check.connect_toggled({
        let w = weak_handler(&_binding);
        move |button| {
            let Some(b) = w.upgrade() else { return };
            if b.refreshing.get() { return; }
            let _ = b.vm.video.set_vsync(button.is_active());
        }
    });
    mute_check.connect_toggled({
        let w = weak_handler(&_binding);
        move |button| {
            let Some(b) = w.upgrade() else { return };
            if b.refreshing.get() { return; }
            let _ = b.vm.audio.set_mute(button.is_active());
        }
    });
    volume_spin.connect_value_changed({
        let w = weak_handler(&_binding);
        move |spin| {
            let Some(b) = w.upgrade() else { return };
            if b.refreshing.get() { return; }
            let _ = b.vm.audio.set_volume(spin.value() as u8);
        }
    });
    sample_rate_combo.connect_changed({
        let w = weak_handler(&_binding);
        move |combo| {
            let Some(b) = w.upgrade() else { return };
            if b.refreshing.get() { return; }
            let rate = combo
                .active_id()
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(48_000);
            let _ = b.vm.audio.set_sample_rate(rate);
        }
    });
    latency_spin.connect_value_changed({
        let w = weak_handler(&_binding);
        move |spin| {
            let Some(b) = w.upgrade() else { return };
            if b.refreshing.get() { return; }
            let _ = b.vm.audio.set_latency(spin.value() as u16);
        }
    });

    // Submit
    let parent_clone = parent.clone();
    let state_clone = state.clone();
    let finish_cb = Rc::new(RefCell::new(Some(Box::new(on_close) as Box<dyn FnOnce()>)));
    let _binding_owned = Rc::clone(&_binding);
    dialog.connect_response(move |dialog, response| {
        if response != gtk::ResponseType::Ok {
            dialog.close();
            if let Some(cb) = finish_cb.borrow_mut().take() {
                cb();
            }
            return;
        }
        match _binding_owned.vm.finish() {
            Ok(snapshot) => match state_clone.borrow_mut().session.apply_settings(snapshot) {
                Ok(result) => {
                    if result.fullscreen_default_changed {
                        parent_clone.set_fullscreened(
                            _binding_owned.vm.snapshot().local.video.window.fullscreen_default,
                        );
                    }
                    dialog.close();
                    if let Some(cb) = finish_cb.borrow_mut().take() {
                        cb();
                    }
                }
                Err(e) => {
                    _binding_owned.error_label.set_text(&e.to_string());
                }
            },
            Err(_) => {
                // validation errors already shown via revision callback
            }
        }
    });

    dialog.present();
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

fn labeled_row(label: &str, widget: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let label = gtk::Label::new(Some(label));
    label.set_xalign(0.0);
    label.set_width_chars(24);
    row.append(&label);
    row.append(widget);
    row
}

fn stack_page() -> (gtk::ScrolledWindow, gtk::Box) {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 12);
    page.set_hexpand(true);

    let scroller = gtk::ScrolledWindow::new();
    scroller.set_hexpand(true);
    scroller.set_vexpand(true);
    scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroller.set_propagate_natural_height(false);
    scroller.set_child(Some(&page));

    (scroller, page)
}

fn combo_box(entries: &[(&str, &str)]) -> gtk::ComboBoxText {
    let combo = gtk::ComboBoxText::new();
    for (id, label) in entries {
        combo.append(Some(id), label);
    }
    combo
}
