use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};

use gio::glib::{
    self,
    object::{Cast as _, IsA},
};
use gtk::prelude::{
    BoxExt as _, ButtonExt as _, CheckButtonExt as _, ComboBoxExt as _, DialogExt as _,
    EditableExt as _, GtkWindowExt as _, WidgetExt as _,
};
use nerust_core_traits::factory::CoreFactory;
use nerust_gui_shell::session::access::FrontendSession as _;
use nerust_gui_viewmodel::settings::{
    SettingsViewModel, StoragePathError, StoragePathValidator, Subscription,
    dto::{
        AudioView, BindingRowView, BindingValueView, GeneralView, InputTabView, SystemTabView,
        VideoView,
    },
};
use nerust_settings_core::{
    editor::CaptureTarget,
    i18n::{UiText, text as ui_text},
};

use nerust_gui_settings::{language::AppLanguage, local::ScalingMode};

use crate::{mapping::*, State};

// ---------------------------------------------------------------------------
// Widget group structs
// ---------------------------------------------------------------------------

struct GeneralWidgets {
    language_combo: gtk::ComboBoxText,
    storage_policy_combo: gtk::ComboBoxText,
    storage_dir_entry: gtk::Entry,
    storage_dir_row: gtk::Box,
    storage_error_label: gtk::Label,
    language_label: gtk::Label,
    storage_policy_label: gtk::Label,
    storage_dir_label: gtk::Label,
}

struct VideoWidgets {
    fullscreen_check: gtk::CheckButton,
    scaling_combo: gtk::ComboBoxText,
    scaling_label: gtk::Label,
    vsync_check: gtk::CheckButton,
}

struct AudioWidgets {
    mute_check: gtk::CheckButton,
    volume_spin: gtk::SpinButton,
    volume_label: gtk::Label,
    sample_rate_combo: gtk::ComboBoxText,
    sample_rate_label: gtk::Label,
    latency_spin: gtk::SpinButton,
    latency_label: gtk::Label,
}

struct InputTabWidgets {
    _notebook: gtk::Notebook,
    pages: Vec<gtk::Box>,
    section_notebooks: RefCell<Vec<Option<gtk::Notebook>>>,
}

struct SystemTabWidgets {
    _notebook: gtk::Notebook,
    pages: Vec<gtk::Box>,
}

struct PreferencesWidgets {
    general: GeneralWidgets,
    video: VideoWidgets,
    audio: AudioWidgets,
    input: InputTabWidgets,
    system: SystemTabWidgets,
    ok_button: gtk::Widget,
    cancel_button: gtk::Widget,
    error_label: gtk::Label,
    dialog: gtk::Dialog,
    stack: gtk::Stack,
    page_ids: Vec<&'static str>,
}

// ---------------------------------------------------------------------------
// PreferencesBinding
// ---------------------------------------------------------------------------

struct PreferencesBinding {
    self_weak: std::rc::Weak<PreferencesBinding>,
    vm: SettingsViewModel,
    _subscriptions: RefCell<Vec<Subscription>>,
    general: GeneralWidgets,
    video: VideoWidgets,
    audio: AudioWidgets,
    input: InputTabWidgets,
    system: SystemTabWidgets,
    ok_button: gtk::Widget,
    cancel_button: gtk::Widget,
    error_label: gtk::Label,
    dialog: gtk::Dialog,
    stack: gtk::Stack,
    page_ids: Vec<&'static str>,
    refreshing: Cell<bool>,
}

impl PreferencesBinding {
    fn new(vm: SettingsViewModel, widgets: PreferencesWidgets) -> Rc<Self> {
        let PreferencesWidgets {
            general,
            video,
            audio,
            input,
            system,
            ok_button,
            cancel_button,
            error_label,
            dialog,
            stack,
            page_ids,
        } = widgets;
        let gv = vm.general.view.get();
        let vv = vm.video.view.get();
        let av = vm.audio.view.get();

        let binding: Rc<Self> = Rc::new_cyclic(|weak: &std::rc::Weak<Self>| {
            let mut subs = Vec::new();
            subs.push(vm.general.view.observe({
                let weak = weak.clone();
                move |v| {
                    let Some(b) = weak.upgrade() else { return };
                    b.with_refreshing(|| b.refresh_general(v));
                }
            }));
            subs.push(vm.video.view.observe({
                let weak = weak.clone();
                move |v| {
                    let Some(b) = weak.upgrade() else { return };
                    b.with_refreshing(|| b.refresh_video(v));
                }
            }));
            subs.push(vm.audio.view.observe({
                let weak = weak.clone();
                move |v| {
                    let Some(b) = weak.upgrade() else { return };
                    b.with_refreshing(|| b.refresh_audio(v));
                }
            }));
            for (index, input_vm) in vm.inputs().iter().enumerate() {
                subs.push(input_vm.view.observe({
                    let weak = weak.clone();
                    move |v| {
                        let Some(b) = weak.upgrade() else { return };
                        b.with_refreshing(|| b.rebuild_input_page(index, v));
                    }
                }));
            }
            for (index, system_vm) in vm.systems().iter().enumerate() {
                subs.push(system_vm.view.observe({
                    let weak = weak.clone();
                    move |v| {
                        let Some(b) = weak.upgrade() else { return };
                        b.with_refreshing(|| b.rebuild_system_page(index, v));
                    }
                }));
            }
            subs.push(vm.capture.view.observe({
                let weak = weak.clone();
                move |_| {
                    let Some(b) = weak.upgrade() else { return };
                    b.with_refreshing(|| b.rebuild_all_input_pages());
                }
            }));
            subs.push(vm.revision.observe({
                let weak = weak.clone();
                move |_| {
                    let Some(b) = weak.upgrade() else { return };
                    b.refresh_validation();
                }
            }));

            Self {
                self_weak: weak.clone(),
                vm,
                _subscriptions: RefCell::new(subs),
                general,
                video,
                audio,
                input,
                system,
                ok_button,
                cancel_button,
                error_label,
                dialog,
                stack,
                page_ids,
                refreshing: Cell::new(false),
            }
        });

        binding.with_refreshing(|| {
            binding.refresh_general(&gv);
            binding.refresh_video(&vv);
            binding.refresh_audio(&av);
            binding.rebuild_all_input_pages();
            binding.rebuild_all_system_pages();
        });
        binding.refresh_validation();

        binding
    }

    fn with_refreshing(&self, f: impl FnOnce()) {
        let previous = self.refreshing.replace(true);
        f();
        self.refreshing.set(previous);
    }

    fn refresh_general(&self, view: &GeneralView) {
        // Update localized labels
        let lang = view.language;
        self.dialog
            .set_title(Some(ui_text(lang, UiText::Preferences)));
        for page_id in self.page_ids.iter() {
            if let Some(child) = self.stack.child_by_name(page_id) {
                let label = match *page_id {
                    "general" => ui_text(lang, UiText::General),
                    "input" => ui_text(lang, UiText::Input),
                    "video" => ui_text(lang, UiText::Video),
                    "audio" => ui_text(lang, UiText::Audio),
                    "system" => ui_text(lang, UiText::System),
                    _ => continue,
                };
                self.stack.page(&child).set_title(label);
            }
        }

        // Update button labels
        if let Some(btn) = self.ok_button.downcast_ref::<gtk::Button>() {
            btn.set_label(ui_text(lang, UiText::Ok));
        }
        if let Some(btn) = self.cancel_button.downcast_ref::<gtk::Button>() {
            btn.set_label(ui_text(lang, UiText::Cancel));
        }

        // Update row labels stored in widget groups
        self.general
            .language_label
            .set_text(ui_text(lang, UiText::Language));
        self.general
            .storage_policy_label
            .set_text(ui_text(lang, UiText::SaveStoragePolicy));
        self.general
            .storage_dir_label
            .set_text(ui_text(lang, UiText::SaveStorageDirectory));
        self.video
            .fullscreen_check
            .set_label(Some(ui_text(lang, UiText::FullscreenDefault)));
        self.video
            .scaling_label
            .set_text(ui_text(lang, UiText::Scaling));
        self.video
            .vsync_check
            .set_label(Some(ui_text(lang, UiText::Vsync)));
        self.audio
            .mute_check
            .set_label(Some(ui_text(lang, UiText::Mute)));
        self.audio
            .volume_label
            .set_text(ui_text(lang, UiText::MasterVolume));
        self.audio
            .sample_rate_label
            .set_text(ui_text(lang, UiText::SampleRate));
        self.audio
            .latency_label
            .set_text(ui_text(lang, UiText::AudioLatency));

        self.general.language_combo.remove_all();
        for choice in &view.language_choices {
            self.general
                .language_combo
                .append(Some(map_language_id(choice.value)), &choice.label);
        }
        self.general
            .language_combo
            .set_active_id(Some(map_language_id(view.language)));
        self.general.storage_policy_combo.remove_all();
        for choice in &view.storage_policy_choices {
            self.general
                .storage_policy_combo
                .append(Some(map_storage_policy_id(choice.value)), &choice.label);
        }
        self.general
            .storage_policy_combo
            .set_active_id(Some(map_storage_policy_id(view.storage_policy)));
        self.general
            .storage_dir_row
            .set_visible(view.show_storage_directory);
        if self.general.storage_dir_entry.text() != view.storage_directory {
            self.general
                .storage_dir_entry
                .set_text(&view.storage_directory);
        }
    }

    fn refresh_video(&self, view: &VideoView) {
        self.video
            .fullscreen_check
            .set_active(view.fullscreen_default);
        self.video.scaling_combo.remove_all();
        for choice in &view.scaling_choices {
            self.video
                .scaling_combo
                .append(Some(map_scaling_id(choice.value)), &choice.label);
        }
        self.video
            .scaling_combo
            .set_active_id(Some(map_scaling_id(view.scaling)));
        self.video.vsync_check.set_active(view.vsync);
    }

    fn refresh_audio(&self, view: &AudioView) {
        self.audio.mute_check.set_active(view.muted);
        self.audio
            .volume_spin
            .set_value(f64::from(view.volume_percent));
        self.audio
            .sample_rate_combo
            .set_active_id(Some(&view.sample_rate.to_string()));
        self.audio
            .latency_spin
            .set_value(f64::from(view.latency_ms));
    }

    fn refresh_validation(&self) {
        match self.vm.finish() {
            Ok(_) => {
                self.ok_button.set_sensitive(true);
                self.error_label.set_text("");
                self.general.storage_error_label.set_text("");
            }
            Err(validation) => {
                self.ok_button.set_sensitive(false);
                self.error_label.set_text(
                    validation
                        .issues
                        .first()
                        .map(|issue| issue.message.as_str())
                        .unwrap_or(""),
                );
                self.general.storage_error_label.set_text(
                    validation
                        .issues
                        .iter()
                        .find(|issue| {
                            matches!(
                                issue.scope,
                                nerust_gui_viewmodel::settings::ValidationScope::Persistence
                            )
                        })
                        .map(|issue| issue.message.as_str())
                        .unwrap_or(""),
                );
            }
        }
    }

    fn rebuild_all_system_pages(&self) {
        for (index, vm) in self.vm.systems().iter().enumerate() {
            self.rebuild_system_page(index, &vm.view.get());
        }
    }

    fn rebuild_system_page(&self, index: usize, view: &SystemTabView) {
        let Some(page) = self.system.pages.get(index) else {
            return;
        };
        clear_box(page);
        for field in &view.fields {
            let combo = gtk::ComboBoxText::new();
            for choice in &field.choices {
                combo.append(Some(choice.value.as_str()), &choice.label);
            }
            combo.set_active_id(Some(field.selected.as_str()));
            let field_id = field.id.clone();
            let choices = field.choices.clone();
            let weak = self.self_weak.clone();
            combo.connect_changed(move |combo| {
                let Some(binding) = weak.upgrade() else {
                    return;
                };
                if binding.refreshing.get() {
                    return;
                }
                let Some(active) = combo.active_id() else {
                    return;
                };
                let Some(choice) = choices
                    .iter()
                    .find(|choice| choice.value.as_str() == active)
                else {
                    return;
                };
                if let Some(vm) = binding.vm.systems().get(index)
                    && let Err(e) = vm.set_choice(&field_id, &choice.value)
                {
                    binding.error_label.set_text(&e.to_string());
                    let weak = binding.self_weak.clone();
                    glib::idle_add_local_once(move || {
                        if let Some(b) = weak.upgrade() {
                            b.with_refreshing(|| {
                                if let Some(vm) = b.vm.systems().get(index) {
                                    b.rebuild_system_page(index, &vm.view.get());
                                }
                            });
                        }
                    });
                }
            });
            page.append(&labeled_row(&field.label, &combo));
        }
    }

    fn rebuild_all_input_pages(&self) {
        for (index, vm) in self.vm.inputs().iter().enumerate() {
            self.rebuild_input_page(index, &vm.view.get());
        }
    }

    fn rebuild_input_page(&self, index: usize, view: &InputTabView) {
        let Some(page) = self.input.pages.get(index) else {
            return;
        };
        let selected_section = self
            .input
            .section_notebooks
            .borrow()
            .get(index)
            .and_then(|notebook| notebook.as_ref())
            .and_then(gtk::Notebook::current_page);
        clear_box(page);
        for conflict in &view.conflicts {
            let label = gtk::Label::new(Some(&conflict.message));
            label.set_xalign(0.0);
            page.append(&label);
        }
        for slot in &view.slots {
            let combo = gtk::ComboBoxText::new();
            combo.append(Some("__none__"), ui_text(self.language(), UiText::None));
            for choice in &slot.choices {
                if let Some(id) = &choice.value {
                    combo.append(Some(id), &choice.label);
                }
            }
            combo.set_active_id(Some(
                slot.selected_profile_id.as_deref().unwrap_or("__none__"),
            ));
            combo.set_sensitive(!slot.occupied_by_other_slot);
            let slot_id = slot.slot_id;
            let weak = self.self_weak.clone();
            combo.connect_changed(move |combo| {
                let Some(binding) = weak.upgrade() else {
                    return;
                };
                if binding.refreshing.get() {
                    return;
                }
                let profile = combo
                    .active_id()
                    .and_then(|id| (id.as_str() != "__none__").then(|| id.to_string()));
                if let Some(vm) = binding.vm.inputs().get(index)
                    && let Err(e) = vm.set_controller_slot(slot_id, profile.as_deref())
                {
                    binding.error_label.set_text(&e.to_string());
                    let weak = binding.self_weak.clone();
                    glib::idle_add_local_once(move || {
                        if let Some(b) = weak.upgrade() {
                            b.with_refreshing(|| {
                                if let Some(vm) = b.vm.inputs().get(index) {
                                    b.rebuild_input_page(index, &vm.view.get());
                                }
                            });
                        }
                    });
                }
            });
            page.append(&labeled_row(&slot.label, &combo));
        }

        let sections_notebook = gtk::Notebook::new();
        sections_notebook.set_scrollable(true);
        sections_notebook.set_tab_pos(gtk::PositionType::Top);
        sections_notebook.set_hexpand(true);
        sections_notebook.set_vexpand(true);
        for section in &view.sections {
            let section_page = input_section_page();
            for row in &section.rows {
                self.append_binding_row(&section_page, row);
            }
            sections_notebook
                .append_page(&section_page, Some(&gtk::Label::new(Some(&section.label))));
        }

        // Shortcuts are included as the last section in the projection
        if let Some(selected) = selected_section
            && selected < sections_notebook.n_pages()
        {
            sections_notebook.set_current_page(Some(selected));
        }
        page.append(&sections_notebook);
        if let Some(notebook) = self.input.section_notebooks.borrow_mut().get_mut(index) {
            *notebook = Some(sections_notebook);
        }
    }

    fn append_binding_row(&self, page: &gtk::Box, row: &BindingRowView) {
        let value = match &row.value {
            BindingValueView::Unbound(value)
            | BindingValueView::Bound(value)
            | BindingValueView::Capturing(value) => value,
        };
        self.append_capture_row(page, &row.label, value, row.target.clone());
    }

    fn append_capture_row(&self, page: &gtk::Box, label: &str, value: &str, target: CaptureTarget) {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let name = gtk::Label::new(Some(label));
        name.set_xalign(0.0);
        name.set_width_chars(20);
        let current = gtk::Label::new(Some(value));
        current.set_xalign(0.0);
        current.set_hexpand(true);
        let change = gtk::Button::with_label(ui_text(self.language(), UiText::Change));
        let clear = gtk::Button::with_label(ui_text(self.language(), UiText::Clear));
        let weak_c = self.self_weak.clone();
        let change_target_c = target.clone();
        change.connect_clicked(move |_| {
            if let Some(binding) = weak_c.upgrade()
                && let Err(e) = binding.vm.capture.start_capture(change_target_c.clone())
            {
                binding.error_label.set_text(&e.to_string());
            }
        });
        let weak_cl = self.self_weak.clone();
        clear.connect_clicked(move |_| {
            if let Some(binding) = weak_cl.upgrade()
                && let Err(e) = binding.vm.capture.clear_binding(&target)
            {
                binding.error_label.set_text(&e.to_string());
            }
        });
        row.append(&name);
        row.append(&current);
        row.append(&change);
        row.append(&clear);
        page.append(&row);
    }

    fn language(&self) -> AppLanguage {
        self.vm.general.view.get().language
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
        (
            s.settings_snapshot().clone(),
            s.ctx.registry.clone(),
            s.ctx.audio_registry.clone(),
        )
    };

    let factories: Vec<Arc<dyn CoreFactory>> = registry.all().iter().map(Arc::clone).collect();

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
    let cancel_button = dialog.add_button("Cancel", gtk::ResponseType::Cancel);
    let ok_button = dialog.add_button("OK", gtk::ResponseType::Ok);
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
    let language_label = gtk::Label::new(Some("Language"));
    let language_combo = combo_box(&[
        ("system_default", "System Default"),
        ("japanese", "Japanese"),
        ("english", "English"),
    ]);
    let lang_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    lang_row.append(&language_label);
    lang_row.append(&language_combo);
    general_page.append(&lang_row);

    let storage_policy_label = gtk::Label::new(Some("Save Storage Policy"));
    let storage_policy_combo = combo_box(&[
        ("sidecar", "Sidecar"),
        ("app_shared_data", "App Shared Data"),
        ("custom_directory", "Custom Directory"),
    ]);
    let sp_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    sp_row.append(&storage_policy_label);
    sp_row.append(&storage_policy_combo);
    general_page.append(&sp_row);

    let storage_dir_label = gtk::Label::new(Some("Save Storage Directory"));
    let storage_dir_entry = gtk::Entry::new();
    let storage_dir_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    storage_dir_row.append(&storage_dir_label);
    storage_dir_row.append(&storage_dir_entry);
    let storage_error_label = gtk::Label::new(None);
    storage_error_label.set_xalign(0.0);
    general_page.append(&storage_dir_row);
    general_page.append(&storage_error_label);

    // ---- Video page ----
    let fullscreen_check = gtk::CheckButton::with_label("Fullscreen Default");
    video_page.append(&fullscreen_check);
    let scaling_label = gtk::Label::new(Some("Scaling"));
    let scaling_combo = combo_box(&[
        ("fit", "Fit to Window"),
        ("1", "1x"),
        ("2", "2x"),
        ("3", "3x"),
        ("4", "4x"),
        ("5", "5x"),
    ]);
    let scaling_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    scaling_row.append(&scaling_label);
    scaling_row.append(&scaling_combo);
    video_page.append(&scaling_row);
    let vsync_check = gtk::CheckButton::with_label("Vsync");
    video_page.append(&vsync_check);

    // ---- Audio page ----
    let mute_check = gtk::CheckButton::with_label("Mute");
    audio_page.append(&mute_check);
    let volume_label = gtk::Label::new(Some("Master Volume"));
    let volume_spin = gtk::SpinButton::with_range(0.0, 100.0, 1.0);
    let vol_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    vol_row.append(&volume_label);
    vol_row.append(&volume_spin);
    audio_page.append(&vol_row);
    let sample_rate_label = gtk::Label::new(Some("Sample Rate"));
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
    let sr_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    sr_row.append(&sample_rate_label);
    sr_row.append(&sample_rate_combo);
    audio_page.append(&sr_row);
    let latency_label = gtk::Label::new(Some("Audio Latency"));
    let latency_spin = gtk::SpinButton::with_range(10.0, 200.0, 1.0);
    let lat_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    lat_row.append(&latency_label);
    lat_row.append(&latency_spin);
    audio_page.append(&lat_row);

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
        language_label,
        storage_policy_label,
        storage_dir_label,
    };
    let video_w = VideoWidgets {
        fullscreen_check: fullscreen_check.clone(),
        scaling_combo: scaling_combo.clone(),
        scaling_label,
        vsync_check: vsync_check.clone(),
    };
    let audio_w = AudioWidgets {
        mute_check: mute_check.clone(),
        volume_spin: volume_spin.clone(),
        volume_label,
        sample_rate_combo: sample_rate_combo.clone(),
        sample_rate_label,
        latency_spin: latency_spin.clone(),
        latency_label,
    };
    let input_w = InputTabWidgets {
        _notebook: input_notebook,
        section_notebooks: RefCell::new(vec![None; input_pages.len()]),
        pages: input_pages,
    };
    let system_w = SystemTabWidgets {
        _notebook: system_notebook,
        pages: system_pages,
    };

    let page_ids: Vec<&'static str> = vec!["general", "input", "video", "audio", "system"];
    let _binding = PreferencesBinding::new(
        vm,
        PreferencesWidgets {
            general: general_w,
            video: video_w,
            audio: audio_w,
            input: input_w,
            system: system_w,
            ok_button,
            cancel_button,
            error_label,
            dialog: dialog.clone(),
            stack: stack.clone(),
            page_ids,
        },
    );

    // ---- Signal handlers ----
    language_combo.connect_changed({
        let w = weak_handler(&_binding);
        move |combo| {
            let Some(b) = w.upgrade() else { return };
            if b.refreshing.get() { return; }
            cmd(&b, b.vm.general.set_language(parse_language_id(combo.active_id().as_deref())));
        }
    });
    storage_policy_combo.connect_changed({
        let w = weak_handler(&_binding);
        move |combo| {
            let Some(b) = w.upgrade() else { return };
            if b.refreshing.get() { return; }
            cmd(&b, b.vm.general.set_storage_policy(parse_storage_policy_id(combo.active_id().as_deref())));
        }
    });
    storage_dir_entry.connect_changed({
        let w = weak_handler(&_binding);
        move |entry| {
            let Some(b) = w.upgrade() else { return };
            if b.refreshing.get() {
                return;
            }
            let text = entry.text();
            let path = (!text.is_empty()).then(|| std::path::PathBuf::from(text.as_str()));
            cmd(&b, b.vm.general.set_storage_directory(path));
        }
    });
    fullscreen_check.connect_toggled({
        let w = weak_handler(&_binding);
        move |button| {
            let Some(b) = w.upgrade() else { return };
            if b.refreshing.get() {
                return;
            }
            cmd(&b, b.vm.video.set_fullscreen_default(button.is_active()));
        }
    });
    scaling_combo.connect_changed({
        let w = weak_handler(&_binding);
        move |combo| {
            let Some(b) = w.upgrade() else { return };
            if b.refreshing.get() { return; }
            cmd(&b, b.vm.video.set_scaling(parse_scaling_id(combo.active_id().as_deref())));
        }
    });
    vsync_check.connect_toggled({
        let w = weak_handler(&_binding);
        move |button| {
            let Some(b) = w.upgrade() else { return };
            if b.refreshing.get() {
                return;
            }
            cmd(&b, b.vm.video.set_vsync(button.is_active()));
        }
    });
    mute_check.connect_toggled({
        let w = weak_handler(&_binding);
        move |button| {
            let Some(b) = w.upgrade() else { return };
            if b.refreshing.get() {
                return;
            }
            cmd(&b, b.vm.audio.set_mute(button.is_active()));
        }
    });
    volume_spin.connect_value_changed({
        let w = weak_handler(&_binding);
        move |spin| {
            let Some(b) = w.upgrade() else { return };
            if b.refreshing.get() {
                return;
            }
            cmd(&b, b.vm.audio.set_volume(spin.value() as u8));
        }
    });
    sample_rate_combo.connect_changed({
        let w = weak_handler(&_binding);
        move |combo| {
            let Some(b) = w.upgrade() else { return };
            if b.refreshing.get() {
                return;
            }
            let rate = combo
                .active_id()
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(48_000);
            cmd(&b, b.vm.audio.set_sample_rate(rate));
        }
    });
    latency_spin.connect_value_changed({
        let w = weak_handler(&_binding);
        move |spin| {
            let Some(b) = w.upgrade() else { return };
            if b.refreshing.get() {
                return;
            }
            cmd(&b, b.vm.audio.set_latency(spin.value() as u16));
        }
    });

    let key_controller = gtk::EventControllerKey::new();
    key_controller.connect_key_pressed({
        let weak = Rc::downgrade(&_binding);
        move |_, key, _, _| {
            let Some(binding) = weak.upgrade() else {
                return gtk::glib::Propagation::Proceed;
            };
            if binding.vm.capture.view.get().target.is_none() {
                return gtk::glib::Propagation::Proceed;
            }
            let Ok(mapped_key) = nerust_keyboard::Key::try_from(key) else {
                return gtk::glib::Propagation::Stop;
            };
            binding.vm.capture.apply_captured_key(mapped_key);
            gtk::glib::Propagation::Stop
        }
    });
    dialog.add_controller(key_controller);

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
            Ok(snapshot) => {
                let apply_result = with_mut(&state_clone, |state| state.apply_settings(snapshot));
                match apply_result {
                    Ok(result) => {
                        if result.fullscreen_default_changed {
                            parent_clone.set_fullscreened(
                                _binding_owned
                                    .vm
                                    .snapshot()
                                    .local
                                    .video
                                    .window
                                    .fullscreen_default,
                            );
                        }
                        if result.scaling_changed
                            && let Some(profile) = state_clone.borrow().render_profile()
                        {
                            apply_scaling_to_window(
                                &parent_clone,
                                _binding_owned.vm.snapshot().local.video.window.scaling,
                                profile,
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
                }
            }
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

fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn with_mut<T, R>(value: &RefCell<T>, f: impl FnOnce(&mut T) -> R) -> R {
    let mut value = value.borrow_mut();
    f(&mut value)
}

fn input_section_page() -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 8);
    page.set_margin_start(6);
    page.set_margin_end(6);
    page.set_margin_top(6);
    page.set_margin_bottom(6);
    page
}

fn apply_scaling_to_window(
    window: &gtk::ApplicationWindow,
    scaling: ScalingMode,
    render_profile: &nerust_render_traits::VideoRenderProfile,
) {
    let base_width = render_profile.physical_size.width as i32;
    let base_height = render_profile.physical_size.height as i32;
    let scale = match scaling {
        ScalingMode::FitToWindow => None,
        ScalingMode::X1 => Some(1),
        ScalingMode::X2 => Some(2),
        ScalingMode::X3 => Some(3),
        ScalingMode::X4 => Some(4),
        ScalingMode::X5 => Some(5),
    };
    if let Some(scale) = scale {
        window.set_default_size(base_width * scale, base_height * scale);
    }
    window.queue_resize();
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

fn weak_handler(b: &Rc<PreferencesBinding>) -> std::rc::Weak<PreferencesBinding> {
    Rc::downgrade(b)
}

fn cmd<T>(
    binding: &Rc<PreferencesBinding>,
    result: Result<T, nerust_gui_viewmodel::settings::ViewModelError>,
) {
    if let Err(e) = result {
        binding.error_label.set_text(&e.to_string());
    }
}

fn combo_box(entries: &[(&str, &str)]) -> gtk::ComboBoxText {
    let combo = gtk::ComboBoxText::new();
    for (id, label) in entries {
        combo.append(Some(id), label);
    }
    combo
}

/// GTK widget tests — only run on Linux with a display server.
/// macOS requires main-thread GTK init which Rust test harness cannot
/// guarantee. These tests compile on all platforms but only execute
/// when `gtk::init()` succeeds.
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[cfg(not(target_os = "macos"))]
    fn gtk_available() -> bool {
        gtk::init().is_ok()
    }

    #[cfg(target_os = "macos")]
    fn gtk_available() -> bool {
        false
    }

    #[test]
    fn with_mut_releases_borrow_before_follow_up_work() {
        let value = RefCell::new(0);
        with_mut(&value, |current| *current = 1);
        *value.borrow_mut() = 2;
        assert_eq!(*value.borrow(), 2);
    }

    macro_rules! gtk_test {
        ($name:ident, $body:expr) => {
            #[test]
            fn $name() {
                if gtk_available() {
                    $body()
                } else {
                    eprintln!("skipped (no GTK display)");
                }
            }
        };
    }

    gtk_test!(refreshing_guard_prevents_reentrant_signal, || {
        let binding = create_test_binding();
        binding.with_refreshing(|| {
            let prev = binding.refreshing.get();
            assert!(prev, "refreshing should be true during guard");
        });
        assert!(
            !binding.refreshing.get(),
            "refreshing should be false after guard"
        );
    });

    gtk_test!(cmd_helper_shows_error_on_error_label, || {
        let binding = create_test_binding();
        let err = nerust_gui_viewmodel::settings::ViewModelError::UnknownSystem("test".into());
        cmd(&binding, Err::<(), _>(err));
        assert!(
            !binding.error_label.text().is_empty(),
            "error_label should show the error"
        );
    });

    gtk_test!(general_widgets_initial_values_match_viewmodel, || {
        let binding = create_test_binding();
        let gv = binding.vm.general.view.get();
        assert_eq!(
            binding.general.language_combo.active_id().as_deref(),
            Some(map_language_id(gv.language))
        );
    });

    gtk_test!(ok_button_sensitivity_matches_validation, || {
        let binding = create_test_binding();
        let ok = binding.vm.finish().is_ok();
        assert_eq!(binding.ok_button.is_sensitive(), ok);
    });

    fn create_test_binding() -> Rc<PreferencesBinding> {
        #[derive(Debug)]
        struct TestNoopValidator;
        impl StoragePathValidator for TestNoopValidator {
            fn validate(&self, _: &std::path::Path) -> Result<(), StoragePathError> {
                Ok(())
            }
        }
        use std::sync::Arc;

        let snapshot = nerust_gui_settings::snapshot::SettingsSnapshot {
            shared: nerust_gui_settings::shared::DesktopSharedSettings::default(),
            local: nerust_gui_settings::local::HostBackendLocalSettings::default(),
            app_state: nerust_gui_settings::app_state::DesktopAppState::default(),
        };
        let audio_registry = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
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
            vec![],
            supported_sample_rates,
            Rc::new(TestNoopValidator) as Rc<dyn StoragePathValidator>,
        );

        let dialog = gtk::Dialog::builder().title("test").build();
        let stack = gtk::Stack::new();
        let ok_button = gtk::Button::with_label("OK");
        let error_label = gtk::Label::new(None);

        PreferencesBinding::new(
            vm,
            PreferencesWidgets {
                general: GeneralWidgets {
                    language_combo: gtk::ComboBoxText::new(),
                    storage_policy_combo: gtk::ComboBoxText::new(),
                    storage_dir_entry: gtk::Entry::new(),
                    storage_dir_row: gtk::Box::new(gtk::Orientation::Horizontal, 0),
                    storage_error_label: gtk::Label::new(None),
                    language_label: gtk::Label::new(None),
                    storage_policy_label: gtk::Label::new(None),
                    storage_dir_label: gtk::Label::new(None),
                },
                video: VideoWidgets {
                    fullscreen_check: gtk::CheckButton::new(),
                    scaling_combo: gtk::ComboBoxText::new(),
                    scaling_label: gtk::Label::new(None),
                    vsync_check: gtk::CheckButton::new(),
                },
                audio: AudioWidgets {
                    mute_check: gtk::CheckButton::new(),
                    volume_spin: gtk::SpinButton::with_range(0.0, 100.0, 1.0),
                    volume_label: gtk::Label::new(None),
                    sample_rate_combo: gtk::ComboBoxText::new(),
                    sample_rate_label: gtk::Label::new(None),
                    latency_spin: gtk::SpinButton::with_range(10.0, 200.0, 1.0),
                    latency_label: gtk::Label::new(None),
                },
                input: InputTabWidgets {
                    _notebook: gtk::Notebook::new(),
                    pages: vec![],
                    section_notebooks: RefCell::new(vec![]),
                },
                system: SystemTabWidgets {
                    _notebook: gtk::Notebook::new(),
                    pages: vec![],
                },
                ok_button: ok_button.clone().upcast::<gtk::Widget>(),
                cancel_button: gtk::Button::new().upcast::<gtk::Widget>(),
                error_label: error_label.clone(),
                dialog: dialog.clone(),
                stack: stack.clone(),
                page_ids: vec![],
            },
        )
    }
}
