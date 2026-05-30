use crate::State;
use gtk::glib;
use gtk::prelude::*;
use nerust_contract_settings::input::KeyboardKey;
use nerust_contract_settings::language::AppLanguage;
use nerust_contract_settings::local::ScalingMode;
use nerust_contract_settings::shared::StoragePolicy;
use nerust_gui_runtime::settings::{SettingsSnapshot, validate_shared_settings};
use nerust_gui_shell::descriptor::{
    SystemSettingsChoiceId, SystemSettingsFieldId, SystemSettingsFieldKind,
    SystemSettingsPageModel, apply_system_settings_choice_for_system, input_topology_for_system,
    settings_system_ids, system_settings_page_for_system,
};
use nerust_gui_shell::settings::bindings::conflicting_keys;
use nerust_gui_shell::settings::bindings::descriptors::{
    keyboard_binding_sections, shortcut_descriptors,
};
use nerust_gui_shell::settings::bindings::keys::keyboard_key_label;
use nerust_gui_shell::settings::editor::{
    CaptureTarget, apply_capture_target, current_binding_label,
};
use nerust_gui_shell::settings::i18n::{UiText, text};
use nerust_input_schema::{InputTopologyDescriptor, SystemId};
use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone)]
struct InputRow {
    target: CaptureTarget,
    value_label: gtk::Label,
    change_button: gtk::Button,
    clear_button: gtk::Button,
}

#[derive(Clone)]
struct SystemRow {
    system_id: SystemId,
    field_id: String,
    combo: gtk::ComboBoxText,
}

pub(crate) fn present_preferences_dialog(
    parent: &gtk::ApplicationWindow,
    state: Rc<RefCell<State>>,
    on_close: impl FnOnce() + 'static,
) {
    let language = state.borrow().settings_snapshot().shared.general.language;
    let dialog = gtk::Dialog::builder()
        .transient_for(parent)
        .modal(true)
        .title(text(language, UiText::Preferences))
        .default_width(900)
        .default_height(560)
        .build();
    dialog.add_button(text(language, UiText::Cancel), gtk::ResponseType::Cancel);
    dialog.add_button(text(language, UiText::Ok), gtk::ResponseType::Ok);

    let finish = Rc::new(RefCell::new(Some(Box::new(on_close) as Box<dyn FnOnce()>)));
    let draft = Rc::new(RefCell::new(state.borrow().settings_snapshot().clone()));
    let capture_target = Rc::new(RefCell::new(None::<CaptureTarget>));
    let ok_button: gtk::Widget = dialog
        .widget_for_response(gtk::ResponseType::Ok)
        .expect("OK button");
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
    stack.add_titled(
        &general_page_scroller,
        Some("general"),
        text(language, UiText::General),
    );
    stack.add_titled(
        &input_page_scroller,
        Some("input"),
        text(language, UiText::Input),
    );
    stack.add_titled(
        &video_page_scroller,
        Some("video"),
        text(language, UiText::Video),
    );
    stack.add_titled(
        &audio_page_scroller,
        Some("audio"),
        text(language, UiText::Audio),
    );
    stack.add_titled(
        &system_page_scroller,
        Some("system"),
        text(language, UiText::System),
    );

    let error_label = gtk::Label::new(None);
    error_label.set_xalign(0.0);
    content.append(&error_label);

    let language_combo = combo_box(&[
        ("system_default", text(language, UiText::SystemDefault)),
        ("japanese", text(language, UiText::Japanese)),
        ("english", text(language, UiText::English)),
    ]);
    general_page.append(&labeled_row(
        text(language, UiText::Language),
        &language_combo,
    ));

    let storage_policy_combo = combo_box(&[
        ("sidecar", text(language, UiText::Sidecar)),
        ("app_shared_data", text(language, UiText::AppSharedData)),
        ("custom_directory", text(language, UiText::CustomDirectory)),
    ]);
    general_page.append(&labeled_row(
        text(language, UiText::SaveStoragePolicy),
        &storage_policy_combo,
    ));
    let storage_dir_entry = gtk::Entry::new();
    let storage_dir_row = labeled_row(
        text(language, UiText::SaveStorageDirectory),
        &storage_dir_entry,
    );
    let storage_error_label = gtk::Label::new(None);
    storage_error_label.set_xalign(0.0);
    general_page.append(&storage_dir_row);
    general_page.append(&storage_error_label);

    let input_conflict_label = gtk::Label::new(None);
    input_conflict_label.set_xalign(0.0);
    input_page.append(&input_conflict_label);
    let input_topologies = input_topologies();
    let input_rows = build_input_rows(language, &input_page, &input_topologies);

    let fullscreen_check = gtk::CheckButton::with_label(text(language, UiText::FullscreenDefault));
    video_page.append(&fullscreen_check);
    let scaling_combo = combo_box(&[
        ("fit", text(language, UiText::FitToWindow)),
        ("1", "1x"),
        ("2", "2x"),
        ("3", "3x"),
        ("4", "4x"),
        ("5", "5x"),
    ]);
    video_page.append(&labeled_row(
        text(language, UiText::Scaling),
        &scaling_combo,
    ));
    let vsync_check = gtk::CheckButton::with_label(text(language, UiText::Vsync));
    // GTK's GLArea path does not expose a portable VSync toggle, so keep the
    // backend-local value preserved but hidden in this frontend.

    let mute_check = gtk::CheckButton::with_label(text(language, UiText::Mute));
    audio_page.append(&mute_check);
    let volume_spin = gtk::SpinButton::with_range(0.0, 100.0, 1.0);
    audio_page.append(&labeled_row(
        text(language, UiText::MasterVolume),
        &volume_spin,
    ));
    let sample_rate_combo =
        combo_box(&[("22050", "22050"), ("44100", "44100"), ("48000", "48000")]);
    audio_page.append(&labeled_row(
        text(language, UiText::SampleRate),
        &sample_rate_combo,
    ));
    let latency_spin = gtk::SpinButton::with_range(10.0, 200.0, 1.0);
    audio_page.append(&labeled_row(
        text(language, UiText::AudioLatency),
        &latency_spin,
    ));

    let system_rows = build_system_rows(language, &system_page, &draft.borrow());

    apply_snapshot_to_widgets(
        &draft.borrow(),
        &language_combo,
        &storage_policy_combo,
        &storage_dir_entry,
        &storage_dir_row,
        &fullscreen_check,
        &scaling_combo,
        &vsync_check,
        &mute_check,
        &volume_spin,
        &sample_rate_combo,
        &latency_spin,
        &system_rows,
        &input_rows,
        &capture_target,
        &input_topologies,
        text(language, UiText::Unbound),
        text(language, UiText::CapturePrompt),
    );
    refresh_validation(
        &draft.borrow(),
        language,
        &ok_button,
        &storage_dir_row,
        &storage_error_label,
        &input_conflict_label,
        &input_topologies,
    );

    {
        let draft = draft.clone();
        let widgets = widget_bundle(
            &ok_button,
            &storage_dir_row,
            &storage_error_label,
            &input_conflict_label,
            &language_combo,
            &storage_policy_combo,
            &storage_dir_entry,
            &fullscreen_check,
            &scaling_combo,
            &vsync_check,
            &mute_check,
            &volume_spin,
            &sample_rate_combo,
            &latency_spin,
            &system_rows,
            &input_rows,
            &capture_target,
            &input_topologies,
            language,
        );
        let _ = language_combo.connect_changed(move |combo| {
            draft.borrow_mut().shared.general.language = match combo.active_id().as_deref() {
                Some("japanese") => AppLanguage::Japanese,
                Some("english") => AppLanguage::English,
                _ => AppLanguage::SystemDefault,
            };
            refresh_all_from_draft(&draft.borrow(), &widgets);
        });
    }
    connect_general_updates(
        language,
        &draft,
        &storage_policy_combo,
        &storage_dir_entry,
        widget_bundle(
            &ok_button,
            &storage_dir_row,
            &storage_error_label,
            &input_conflict_label,
            &language_combo,
            &storage_policy_combo,
            &storage_dir_entry,
            &fullscreen_check,
            &scaling_combo,
            &vsync_check,
            &mute_check,
            &volume_spin,
            &sample_rate_combo,
            &latency_spin,
            &system_rows,
            &input_rows,
            &capture_target,
            &input_topologies,
            language,
        ),
    );
    connect_local_updates(
        &draft,
        &fullscreen_check,
        &scaling_combo,
        &vsync_check,
        &mute_check,
        &volume_spin,
        &sample_rate_combo,
        &latency_spin,
        &system_rows,
        widget_bundle(
            &ok_button,
            &storage_dir_row,
            &storage_error_label,
            &input_conflict_label,
            &language_combo,
            &storage_policy_combo,
            &storage_dir_entry,
            &fullscreen_check,
            &scaling_combo,
            &vsync_check,
            &mute_check,
            &volume_spin,
            &sample_rate_combo,
            &latency_spin,
            &system_rows,
            &input_rows,
            &capture_target,
            &input_topologies,
            language,
        ),
    );

    let key_controller = gtk::EventControllerKey::new();
    {
        let draft = draft.clone();
        let capture_target = capture_target.clone();
        let widgets = widget_bundle(
            &ok_button,
            &storage_dir_row,
            &storage_error_label,
            &input_conflict_label,
            &language_combo,
            &storage_policy_combo,
            &storage_dir_entry,
            &fullscreen_check,
            &scaling_combo,
            &vsync_check,
            &mute_check,
            &volume_spin,
            &sample_rate_combo,
            &latency_spin,
            &system_rows,
            &input_rows,
            &capture_target,
            &input_topologies,
            language,
        );
        let _ = key_controller.connect_key_pressed(move |_, key, _, _| {
            let Some(target) = capture_target.borrow().clone() else {
                return glib::Propagation::Proceed;
            };
            let Some(mapped_key) = gdk_key_to_keyboard_key(key) else {
                return glib::Propagation::Stop;
            };
            apply_capture_target(&mut draft.borrow_mut(), &target, Some(mapped_key));
            *capture_target.borrow_mut() = None;
            refresh_all_from_draft(&draft.borrow(), &widgets);
            glib::Propagation::Stop
        });
    }
    dialog.add_controller(key_controller);

    for row in input_rows.iter().cloned() {
        let capture_target = capture_target.clone();
        let draft = draft.clone();
        let widgets = widget_bundle(
            &ok_button,
            &storage_dir_row,
            &storage_error_label,
            &input_conflict_label,
            &language_combo,
            &storage_policy_combo,
            &storage_dir_entry,
            &fullscreen_check,
            &scaling_combo,
            &vsync_check,
            &mute_check,
            &volume_spin,
            &sample_rate_combo,
            &latency_spin,
            &system_rows,
            &input_rows,
            &capture_target,
            &input_topologies,
            language,
        );
        let target = row.target.clone();
        let _ = row.change_button.connect_clicked(move |_| {
            *capture_target.borrow_mut() = Some(target.clone());
            refresh_all_from_draft(&draft.borrow(), &widgets);
        });
    }
    for row in input_rows.iter().cloned() {
        let capture_target = capture_target.clone();
        let draft = draft.clone();
        let widgets = widget_bundle(
            &ok_button,
            &storage_dir_row,
            &storage_error_label,
            &input_conflict_label,
            &language_combo,
            &storage_policy_combo,
            &storage_dir_entry,
            &fullscreen_check,
            &scaling_combo,
            &vsync_check,
            &mute_check,
            &volume_spin,
            &sample_rate_combo,
            &latency_spin,
            &system_rows,
            &input_rows,
            &capture_target,
            &input_topologies,
            language,
        );
        let target = row.target.clone();
        let _ = row.clear_button.connect_clicked(move |_| {
            apply_capture_target(&mut draft.borrow_mut(), &target, None);
            *capture_target.borrow_mut() = None;
            refresh_all_from_draft(&draft.borrow(), &widgets);
        });
    }

    let finish_for_response = finish.clone();
    {
        let draft = draft.clone();
        let state = state.clone();
        let parent = parent.clone();
        let error_label = error_label.clone();
        let capture_target = capture_target.clone();
        let widgets = widget_bundle(
            &ok_button,
            &storage_dir_row,
            &storage_error_label,
            &input_conflict_label,
            &language_combo,
            &storage_policy_combo,
            &storage_dir_entry,
            &fullscreen_check,
            &scaling_combo,
            &vsync_check,
            &mute_check,
            &volume_spin,
            &sample_rate_combo,
            &latency_spin,
            &system_rows,
            &input_rows,
            &capture_target,
            &input_topologies,
            language,
        );
        let _ = dialog.connect_response(move |dialog, response| match response {
            gtk::ResponseType::Ok => {
                let snapshot = draft.borrow().clone();
                if !validation_errors(&snapshot, &input_topologies).is_empty() {
                    refresh_all_from_draft(&snapshot, &widgets);
                    return;
                }
                match apply_settings_without_reentrant_borrow(state.as_ref(), snapshot.clone()) {
                    Ok(plan) => {
                        if plan.fullscreen_default_changed {
                            parent.set_fullscreened(snapshot.local.video.window.fullscreen_default);
                        }
                        dialog.close();
                        run_finish_callback(&finish_for_response);
                    }
                    Err(error) => {
                        error_label.set_text(&error);
                    }
                }
            }
            _ => {
                dialog.close();
                run_finish_callback(&finish_for_response);
            }
        });
    }

    dialog.present();
}

#[derive(Clone)]
struct WidgetBundle {
    ok_button: gtk::Widget,
    storage_dir_row: gtk::Box,
    storage_error_label: gtk::Label,
    input_conflict_label: gtk::Label,
    language_combo: gtk::ComboBoxText,
    storage_policy_combo: gtk::ComboBoxText,
    storage_dir_entry: gtk::Entry,
    fullscreen_check: gtk::CheckButton,
    scaling_combo: gtk::ComboBoxText,
    vsync_check: gtk::CheckButton,
    mute_check: gtk::CheckButton,
    volume_spin: gtk::SpinButton,
    sample_rate_combo: gtk::ComboBoxText,
    latency_spin: gtk::SpinButton,
    system_rows: Vec<SystemRow>,
    input_rows: Vec<InputRow>,
    capture_target: Rc<RefCell<Option<CaptureTarget>>>,
    input_topologies: Vec<InputTopologyDescriptor>,
    language: AppLanguage,
}

type FinishCallback = Rc<RefCell<Option<Box<dyn FnOnce()>>>>;

trait SettingsApplier {
    fn apply_settings(
        &mut self,
        settings: SettingsSnapshot,
    ) -> Result<nerust_gui_runtime::settings::SettingsApplyPlan, String>;
}

impl SettingsApplier for State {
    fn apply_settings(
        &mut self,
        settings: SettingsSnapshot,
    ) -> Result<nerust_gui_runtime::settings::SettingsApplyPlan, String> {
        State::apply_settings(self, settings)
    }
}

fn apply_settings_without_reentrant_borrow<T: SettingsApplier>(
    state: &RefCell<T>,
    snapshot: SettingsSnapshot,
) -> Result<nerust_gui_runtime::settings::SettingsApplyPlan, String> {
    state.borrow_mut().apply_settings(snapshot)
}

#[allow(clippy::too_many_arguments)]
fn widget_bundle(
    ok_button: &gtk::Widget,
    storage_dir_row: &gtk::Box,
    storage_error_label: &gtk::Label,
    input_conflict_label: &gtk::Label,
    language_combo: &gtk::ComboBoxText,
    storage_policy_combo: &gtk::ComboBoxText,
    storage_dir_entry: &gtk::Entry,
    fullscreen_check: &gtk::CheckButton,
    scaling_combo: &gtk::ComboBoxText,
    vsync_check: &gtk::CheckButton,
    mute_check: &gtk::CheckButton,
    volume_spin: &gtk::SpinButton,
    sample_rate_combo: &gtk::ComboBoxText,
    latency_spin: &gtk::SpinButton,
    system_rows: &[SystemRow],
    input_rows: &[InputRow],
    capture_target: &Rc<RefCell<Option<CaptureTarget>>>,
    input_topologies: &[InputTopologyDescriptor],
    language: AppLanguage,
) -> WidgetBundle {
    WidgetBundle {
        ok_button: ok_button.clone(),
        storage_dir_row: storage_dir_row.clone(),
        storage_error_label: storage_error_label.clone(),
        input_conflict_label: input_conflict_label.clone(),
        language_combo: language_combo.clone(),
        storage_policy_combo: storage_policy_combo.clone(),
        storage_dir_entry: storage_dir_entry.clone(),
        fullscreen_check: fullscreen_check.clone(),
        scaling_combo: scaling_combo.clone(),
        vsync_check: vsync_check.clone(),
        mute_check: mute_check.clone(),
        volume_spin: volume_spin.clone(),
        sample_rate_combo: sample_rate_combo.clone(),
        latency_spin: latency_spin.clone(),
        system_rows: system_rows.to_vec(),
        input_rows: input_rows.to_vec(),
        capture_target: capture_target.clone(),
        input_topologies: input_topologies.to_vec(),
        language,
    }
}

fn refresh_all_from_draft(snapshot: &SettingsSnapshot, widgets: &WidgetBundle) {
    apply_snapshot_to_widgets(
        snapshot,
        &widgets.language_combo,
        &widgets.storage_policy_combo,
        &widgets.storage_dir_entry,
        &widgets.storage_dir_row,
        &widgets.fullscreen_check,
        &widgets.scaling_combo,
        &widgets.vsync_check,
        &widgets.mute_check,
        &widgets.volume_spin,
        &widgets.sample_rate_combo,
        &widgets.latency_spin,
        &widgets.system_rows,
        &widgets.input_rows,
        &widgets.capture_target,
        &widgets.input_topologies,
        text(widgets.language, UiText::Unbound),
        text(widgets.language, UiText::CapturePrompt),
    );
    refresh_validation(
        snapshot,
        widgets.language,
        &widgets.ok_button,
        &widgets.storage_dir_row,
        &widgets.storage_error_label,
        &widgets.input_conflict_label,
        &widgets.input_topologies,
    );
}

fn connect_general_updates(
    _language: AppLanguage,
    draft: &Rc<RefCell<SettingsSnapshot>>,
    storage_policy_combo: &gtk::ComboBoxText,
    storage_dir_entry: &gtk::Entry,
    widgets: WidgetBundle,
) {
    {
        let draft = draft.clone();
        let widgets = widgets.clone();
        let _ = storage_policy_combo.connect_changed(move |combo| {
            draft.borrow_mut().shared.persistence.storage_policy =
                match combo.active_id().as_deref() {
                    Some("app_shared_data") => StoragePolicy::AppSharedData,
                    Some("custom_directory") => StoragePolicy::CustomDirectory,
                    _ => StoragePolicy::Sidecar,
                };
            refresh_all_from_draft(&draft.borrow(), &widgets);
        });
    }
    {
        let draft = draft.clone();
        let widgets = widgets.clone();
        let _ = storage_dir_entry.connect_changed(move |entry| {
            let text = entry.text();
            draft.borrow_mut().shared.persistence.storage_directory =
                (!text.is_empty()).then(|| text.as_str().into());
            refresh_all_from_draft(&draft.borrow(), &widgets);
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn connect_local_updates(
    draft: &Rc<RefCell<SettingsSnapshot>>,
    fullscreen_check: &gtk::CheckButton,
    scaling_combo: &gtk::ComboBoxText,
    vsync_check: &gtk::CheckButton,
    mute_check: &gtk::CheckButton,
    volume_spin: &gtk::SpinButton,
    sample_rate_combo: &gtk::ComboBoxText,
    latency_spin: &gtk::SpinButton,
    system_rows: &[SystemRow],
    widgets: WidgetBundle,
) {
    {
        let draft = draft.clone();
        let widgets = widgets.clone();
        let _ = fullscreen_check.connect_toggled(move |button| {
            draft.borrow_mut().local.video.window.fullscreen_default = button.is_active();
            refresh_all_from_draft(&draft.borrow(), &widgets);
        });
    }
    {
        let draft = draft.clone();
        let widgets = widgets.clone();
        let _ = scaling_combo.connect_changed(move |combo| {
            draft.borrow_mut().local.video.window.scaling = match combo.active_id().as_deref() {
                Some("1") => ScalingMode::X1,
                Some("2") => ScalingMode::X2,
                Some("3") => ScalingMode::X3,
                Some("4") => ScalingMode::X4,
                Some("5") => ScalingMode::X5,
                _ => ScalingMode::FitToWindow,
            };
            refresh_all_from_draft(&draft.borrow(), &widgets);
        });
    }
    {
        let draft = draft.clone();
        let widgets = widgets.clone();
        let _ = vsync_check.connect_toggled(move |button| {
            draft.borrow_mut().local.video.presentation.vsync = button.is_active();
            refresh_all_from_draft(&draft.borrow(), &widgets);
        });
    }
    {
        let draft = draft.clone();
        let widgets = widgets.clone();
        let _ = mute_check.connect_toggled(move |button| {
            draft.borrow_mut().local.audio.muted = button.is_active();
            refresh_all_from_draft(&draft.borrow(), &widgets);
        });
    }
    {
        let draft = draft.clone();
        let widgets = widgets.clone();
        let _ = volume_spin.connect_value_changed(move |spin| {
            draft.borrow_mut().local.audio.master_volume_percent = spin.value() as u8;
            refresh_all_from_draft(&draft.borrow(), &widgets);
        });
    }
    {
        let draft = draft.clone();
        let widgets = widgets.clone();
        let _ = sample_rate_combo.connect_changed(move |combo| {
            draft.borrow_mut().local.audio.sample_rate = combo
                .active_id()
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(48_000);
            refresh_all_from_draft(&draft.borrow(), &widgets);
        });
    }
    {
        let draft = draft.clone();
        let widgets = widgets.clone();
        let _ = latency_spin.connect_value_changed(move |spin| {
            draft.borrow_mut().local.audio.latency_ms = spin.value() as u16;
            refresh_all_from_draft(&draft.borrow(), &widgets);
        });
    }
    for row in system_rows.iter().cloned() {
        let draft = draft.clone();
        let widgets = widgets.clone();
        let row_for_update = row.clone();
        let _ = row.combo.clone().connect_changed(move |combo| {
            let Some(choice_id) = combo.active_id().map(|value| value.to_string()) else {
                return;
            };
            let mut snapshot = draft.borrow_mut();
            if !apply_system_settings_choice_for_system_row(
                &row_for_update,
                &mut snapshot,
                choice_id,
            ) {
                return;
            }
            drop(snapshot);
            refresh_all_from_draft(&draft.borrow(), &widgets);
        });
    }
}

fn refresh_validation(
    snapshot: &SettingsSnapshot,
    language: AppLanguage,
    ok_button: &gtk::Widget,
    storage_dir_row: &gtk::Box,
    storage_error_label: &gtk::Label,
    input_conflict_label: &gtk::Label,
    input_topologies: &[InputTopologyDescriptor],
) {
    let storage_error = validate_shared_settings(&snapshot.shared)
        .err()
        .map(|error| error.to_string());
    let first_conflict = input_topologies.iter().find_map(|topology| {
        conflicting_keys(&snapshot.shared, topology)
            .into_iter()
            .next()
            .map(|(key, labels)| (topology.system, key, labels))
    });
    let has_conflicts = first_conflict.is_some();
    let has_errors = storage_error.is_some() || has_conflicts;
    storage_dir_row.set_visible(matches!(
        snapshot.shared.persistence.storage_policy,
        StoragePolicy::CustomDirectory
    ));
    storage_error_label.set_text(storage_error.as_deref().unwrap_or(""));
    if let Some((system_id, key, labels)) = first_conflict {
        input_conflict_label.set_text(&format!(
            "{} {}: {}",
            system_label(system_id),
            keyboard_key_label(key),
            labels.join(", ")
        ));
    } else {
        input_conflict_label.set_text("");
    }
    if has_conflicts && input_conflict_label.text().is_empty() {
        input_conflict_label.set_text(text(language, UiText::ConflictDetected));
    }
    ok_button.set_sensitive(!has_errors);
}

fn validation_errors(
    snapshot: &SettingsSnapshot,
    input_topologies: &[InputTopologyDescriptor],
) -> Vec<String> {
    let mut errors = Vec::new();
    if let Err(error) = validate_shared_settings(&snapshot.shared) {
        errors.push(error.to_string());
    }
    for topology in input_topologies {
        for (key, labels) in conflicting_keys(&snapshot.shared, topology) {
            errors.push(format!(
                "{} {}: {}",
                system_label(topology.system),
                keyboard_key_label(key),
                labels.join(", ")
            ));
        }
    }
    errors
}

fn system_settings_page_model(
    system_id: SystemId,
    snapshot: &SettingsSnapshot,
) -> SystemSettingsPageModel {
    system_settings_page_for_system(system_id, snapshot)
        .expect("supported system definition should be available")
}

fn apply_system_settings_choice_for_system_row(
    row: &SystemRow,
    settings: &mut SettingsSnapshot,
    choice_id: String,
) -> bool {
    let page = system_settings_page_model(row.system_id, settings);
    if !page
        .fields
        .iter()
        .any(|field| field.id.as_str() == row.field_id)
    {
        return false;
    }

    let field = SystemSettingsFieldId(Cow::Owned(row.field_id.clone()));
    let choice = SystemSettingsChoiceId(Cow::Owned(choice_id));
    if let Err(error) =
        apply_system_settings_choice_for_system(row.system_id, settings, &field, &choice)
    {
        log::warn!("failed to apply system settings choice: {error}");
        return false;
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn apply_snapshot_to_widgets(
    snapshot: &SettingsSnapshot,
    language_combo: &gtk::ComboBoxText,
    storage_policy_combo: &gtk::ComboBoxText,
    storage_dir_entry: &gtk::Entry,
    storage_dir_row: &gtk::Box,
    fullscreen_check: &gtk::CheckButton,
    scaling_combo: &gtk::ComboBoxText,
    vsync_check: &gtk::CheckButton,
    mute_check: &gtk::CheckButton,
    volume_spin: &gtk::SpinButton,
    sample_rate_combo: &gtk::ComboBoxText,
    latency_spin: &gtk::SpinButton,
    system_rows: &[SystemRow],
    input_rows: &[InputRow],
    capture_target: &Rc<RefCell<Option<CaptureTarget>>>,
    _input_topologies: &[InputTopologyDescriptor],
    unbound_label: &str,
    capture_label: &str,
) {
    language_combo.set_active_id(Some(match snapshot.shared.general.language {
        AppLanguage::Japanese => "japanese",
        AppLanguage::English => "english",
        AppLanguage::SystemDefault => "system_default",
    }));
    storage_policy_combo.set_active_id(Some(match snapshot.shared.persistence.storage_policy {
        StoragePolicy::AppSharedData => "app_shared_data",
        StoragePolicy::CustomDirectory => "custom_directory",
        StoragePolicy::Sidecar => "sidecar",
    }));
    let storage_dir_text = snapshot
        .shared
        .persistence
        .storage_directory
        .as_ref()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_default();
    storage_dir_entry.set_text(&storage_dir_text);
    storage_dir_row.set_visible(matches!(
        snapshot.shared.persistence.storage_policy,
        StoragePolicy::CustomDirectory
    ));
    fullscreen_check.set_active(snapshot.local.video.window.fullscreen_default);
    scaling_combo.set_active_id(Some(match snapshot.local.video.window.scaling {
        ScalingMode::FitToWindow => "fit",
        ScalingMode::X1 => "1",
        ScalingMode::X2 => "2",
        ScalingMode::X3 => "3",
        ScalingMode::X4 => "4",
        ScalingMode::X5 => "5",
    }));
    vsync_check.set_active(snapshot.local.video.presentation.vsync);
    mute_check.set_active(snapshot.local.audio.muted);
    volume_spin.set_value(f64::from(snapshot.local.audio.master_volume_percent));
    sample_rate_combo.set_active_id(Some(match snapshot.local.audio.sample_rate {
        22_050 => "22050",
        44_100 => "44100",
        _ => "48000",
    }));
    latency_spin.set_value(f64::from(snapshot.local.audio.latency_ms));
    for row in system_rows {
        let system_page = system_settings_page_model(row.system_id, snapshot);
        apply_system_field_by_id_to_combo(&system_page, &row.field_id, &row.combo);
    }

    for row in input_rows {
        row.value_label
            .set_text(if capture_target.borrow().as_ref() == Some(&row.target) {
                capture_label
            } else {
                current_binding_label(snapshot, &row.target).unwrap_or(unbound_label)
            });
    }
}

fn input_topologies() -> Vec<InputTopologyDescriptor> {
    settings_system_ids()
        .iter()
        .map(|system_id| {
            input_topology_for_system(*system_id)
                .expect("supported system definition should be available")
        })
        .collect()
}

fn build_system_rows(
    _language: AppLanguage,
    system_page: &gtk::Box,
    snapshot: &SettingsSnapshot,
) -> Vec<SystemRow> {
    let system_stack = gtk::Stack::new();
    system_stack.set_hexpand(true);
    system_stack.set_vexpand(true);
    let system_switcher = gtk::StackSwitcher::new();
    system_switcher.set_stack(Some(&system_stack));
    system_switcher.set_halign(gtk::Align::Start);
    system_page.append(&system_switcher);
    system_page.append(&system_stack);

    let mut rows = Vec::new();
    for system_id in settings_system_ids() {
        let page = gtk::Box::new(gtk::Orientation::Vertical, 12);
        page.set_hexpand(true);
        system_stack.add_titled(
            &page,
            Some(system_id_key(*system_id)),
            system_label(*system_id),
        );

        let model = system_settings_page_model(*system_id, snapshot);
        if model.fields.is_empty() {
            let label = gtk::Label::new(Some("No system-specific settings"));
            label.set_xalign(0.0);
            page.append(&label);
            continue;
        }

        for field in model.fields.iter() {
            let combo = combo_from_system_field(field);
            page.append(&labeled_row(&field.label, &combo));
            rows.push(SystemRow {
                system_id: *system_id,
                field_id: field.id.as_str().to_string(),
                combo,
            });
        }
    }
    rows
}

fn system_id_key(system_id: SystemId) -> &'static str {
    match system_id {
        SystemId::Nes => "nes",
        SystemId::Snes => "snes",
        SystemId::Ps1 => "ps1",
        SystemId::MegaDrive => "mega_drive",
    }
}

fn system_label(system_id: SystemId) -> &'static str {
    match system_id {
        SystemId::Nes => "NES",
        SystemId::Snes => "SNES",
        SystemId::Ps1 => "PS1",
        SystemId::MegaDrive => "Mega Drive",
    }
}

fn build_input_rows(
    language: AppLanguage,
    input_page: &gtk::Box,
    topologies: &[InputTopologyDescriptor],
) -> Vec<InputRow> {
    let input_stack = gtk::Stack::new();
    input_stack.set_hexpand(true);
    input_stack.set_vexpand(true);
    let input_switcher = gtk::StackSwitcher::new();
    input_switcher.set_stack(Some(&input_stack));
    input_switcher.set_halign(gtk::Align::Start);
    input_page.append(&input_switcher);
    input_page.append(&input_stack);

    let mut rows = Vec::new();
    for topology in topologies {
        let system_page = gtk::Box::new(gtk::Orientation::Vertical, 16);
        system_page.set_hexpand(true);
        input_stack.add_titled(
            &system_page,
            Some(system_id_key(topology.system)),
            system_label(topology.system),
        );

        for section in keyboard_binding_sections(topology) {
            let section_label = gtk::Label::new(Some(section.attachment_label));
            section_label.set_xalign(0.0);
            system_page.append(&section_label);

            let grid = gtk::Grid::new();
            grid.set_column_spacing(12);
            grid.set_row_spacing(6);
            grid.set_hexpand(true);
            system_page.append(&grid);

            for (index, descriptor) in section.bindings.iter().enumerate() {
                rows.push(add_input_row(
                    &grid,
                    index as i32,
                    descriptor.control_label,
                    CaptureTarget::Binding {
                        system: descriptor.system,
                        attachment: descriptor.attachment.as_str().to_string(),
                        control: descriptor.control.as_str().to_string(),
                    },
                    language,
                ));
            }
        }
    }

    let section_page = gtk::Box::new(gtk::Orientation::Vertical, 12);
    section_page.set_hexpand(true);
    let grid = gtk::Grid::new();
    grid.set_column_spacing(12);
    grid.set_row_spacing(6);
    grid.set_hexpand(true);
    section_page.append(&grid);
    input_stack.add_titled(
        &section_page,
        Some("shortcuts"),
        text(language, UiText::Shortcuts),
    );
    for (index, descriptor) in shortcut_descriptors().iter().enumerate() {
        rows.push(add_input_row(
            &grid,
            index as i32,
            descriptor.label,
            CaptureTarget::Shortcut(descriptor.action),
            language,
        ));
    }
    rows
}

fn add_input_row(
    grid: &gtk::Grid,
    row: i32,
    label: &str,
    target: CaptureTarget,
    language: AppLanguage,
) -> InputRow {
    let action_label = gtk::Label::new(Some(label));
    action_label.set_xalign(0.0);
    let value_label = gtk::Label::new(Some(""));
    value_label.set_xalign(0.0);
    let change_button = gtk::Button::with_label(text(language, UiText::Change));
    let clear_button = gtk::Button::with_label(text(language, UiText::Clear));
    grid.attach(&action_label, 0, row, 1, 1);
    grid.attach(&value_label, 1, row, 1, 1);
    grid.attach(&change_button, 2, row, 1, 1);
    grid.attach(&clear_button, 3, row, 1, 1);
    InputRow {
        target,
        value_label,
        change_button,
        clear_button,
    }
}

fn combo_box(entries: &[(&str, &str)]) -> gtk::ComboBoxText {
    let combo = gtk::ComboBoxText::new();
    for (id, label) in entries {
        combo.append(Some(id), label);
    }
    combo
}

fn system_field_by_id<'a>(
    page: &'a nerust_gui_shell::descriptor::SystemSettingsPageModel,
    field_id: &str,
) -> Option<&'a nerust_gui_shell::descriptor::SystemSettingsFieldModel> {
    page.fields
        .iter()
        .find(|field| field.id.as_str() == field_id)
}

fn combo_from_system_field(
    field: &nerust_gui_shell::descriptor::SystemSettingsFieldModel,
) -> gtk::ComboBoxText {
    let combo = gtk::ComboBoxText::new();
    let SystemSettingsFieldKind::Choice { options, .. } = &field.kind;
    for option in options.iter() {
        combo.append(Some(option.id.as_str()), &option.label);
    }
    combo
}

fn apply_system_field_by_id_to_combo(
    page: &nerust_gui_shell::descriptor::SystemSettingsPageModel,
    field_id: &str,
    combo: &gtk::ComboBoxText,
) {
    let Some(field) = system_field_by_id(page, field_id) else {
        combo.set_active_id(None);
        return;
    };
    let SystemSettingsFieldKind::Choice { selected, .. } = &field.kind;
    combo.set_active_id(Some(selected.as_str()));
}

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

fn gdk_key_to_keyboard_key(key: gdk::Key) -> Option<KeyboardKey> {
    Some(match key {
        gdk::Key::_0 => KeyboardKey::Digit0,
        gdk::Key::_1 => KeyboardKey::Digit1,
        gdk::Key::_2 => KeyboardKey::Digit2,
        gdk::Key::_3 => KeyboardKey::Digit3,
        gdk::Key::_4 => KeyboardKey::Digit4,
        gdk::Key::_5 => KeyboardKey::Digit5,
        gdk::Key::_6 => KeyboardKey::Digit6,
        gdk::Key::_7 => KeyboardKey::Digit7,
        gdk::Key::_8 => KeyboardKey::Digit8,
        gdk::Key::_9 => KeyboardKey::Digit9,
        gdk::Key::a | gdk::Key::A => KeyboardKey::KeyA,
        gdk::Key::b | gdk::Key::B => KeyboardKey::KeyB,
        gdk::Key::c | gdk::Key::C => KeyboardKey::KeyC,
        gdk::Key::d | gdk::Key::D => KeyboardKey::KeyD,
        gdk::Key::e | gdk::Key::E => KeyboardKey::KeyE,
        gdk::Key::f | gdk::Key::F => KeyboardKey::KeyF,
        gdk::Key::g | gdk::Key::G => KeyboardKey::KeyG,
        gdk::Key::h | gdk::Key::H => KeyboardKey::KeyH,
        gdk::Key::i | gdk::Key::I => KeyboardKey::KeyI,
        gdk::Key::j | gdk::Key::J => KeyboardKey::KeyJ,
        gdk::Key::k | gdk::Key::K => KeyboardKey::KeyK,
        gdk::Key::l | gdk::Key::L => KeyboardKey::KeyL,
        gdk::Key::m | gdk::Key::M => KeyboardKey::KeyM,
        gdk::Key::n | gdk::Key::N => KeyboardKey::KeyN,
        gdk::Key::o | gdk::Key::O => KeyboardKey::KeyO,
        gdk::Key::p | gdk::Key::P => KeyboardKey::KeyP,
        gdk::Key::q | gdk::Key::Q => KeyboardKey::KeyQ,
        gdk::Key::r | gdk::Key::R => KeyboardKey::KeyR,
        gdk::Key::s | gdk::Key::S => KeyboardKey::KeyS,
        gdk::Key::t | gdk::Key::T => KeyboardKey::KeyT,
        gdk::Key::u | gdk::Key::U => KeyboardKey::KeyU,
        gdk::Key::v | gdk::Key::V => KeyboardKey::KeyV,
        gdk::Key::w | gdk::Key::W => KeyboardKey::KeyW,
        gdk::Key::x | gdk::Key::X => KeyboardKey::KeyX,
        gdk::Key::y | gdk::Key::Y => KeyboardKey::KeyY,
        gdk::Key::z | gdk::Key::Z => KeyboardKey::KeyZ,
        gdk::Key::Up => KeyboardKey::ArrowUp,
        gdk::Key::Down => KeyboardKey::ArrowDown,
        gdk::Key::Left => KeyboardKey::ArrowLeft,
        gdk::Key::Right => KeyboardKey::ArrowRight,
        gdk::Key::Return | gdk::Key::ISO_Enter | gdk::Key::KP_Enter => KeyboardKey::Enter,
        gdk::Key::Escape => KeyboardKey::Escape,
        gdk::Key::space => KeyboardKey::Space,
        gdk::Key::Tab | gdk::Key::ISO_Left_Tab | gdk::Key::KP_Tab => KeyboardKey::Tab,
        gdk::Key::F1 => KeyboardKey::F1,
        gdk::Key::F2 => KeyboardKey::F2,
        gdk::Key::F3 => KeyboardKey::F3,
        gdk::Key::F4 => KeyboardKey::F4,
        gdk::Key::F5 => KeyboardKey::F5,
        gdk::Key::F6 => KeyboardKey::F6,
        gdk::Key::F7 => KeyboardKey::F7,
        gdk::Key::F8 => KeyboardKey::F8,
        gdk::Key::F9 => KeyboardKey::F9,
        gdk::Key::F10 => KeyboardKey::F10,
        gdk::Key::F11 => KeyboardKey::F11,
        gdk::Key::F12 => KeyboardKey::F12,
        _ => return None,
    })
}

fn run_finish_callback(finish: &FinishCallback) {
    if let Some(callback) = finish.borrow_mut().take() {
        callback();
    }
}

#[cfg(test)]
mod tests {
    use super::{SettingsApplier, apply_settings_without_reentrant_borrow};
    use nerust_gui_runtime::settings::{SettingsApplyPlan, SettingsSnapshot};
    use nerust_gui_shell::settings::defaults::seed::{
        default_app_state, default_local_settings, default_shared_settings,
    };
    use std::cell::RefCell;

    #[derive(Default)]
    struct FakeState {
        apply_calls: usize,
        finish_calls: usize,
    }

    impl SettingsApplier for FakeState {
        fn apply_settings(
            &mut self,
            _settings: SettingsSnapshot,
        ) -> Result<SettingsApplyPlan, String> {
            self.apply_calls += 1;
            Ok(SettingsApplyPlan::default())
        }
    }

    fn snapshot() -> SettingsSnapshot {
        SettingsSnapshot {
            shared: default_shared_settings(),
            local: default_local_settings(),
            app_state: default_app_state(),
        }
    }

    #[test]
    fn apply_helper_releases_mutable_borrow_before_follow_up_work() {
        let state = RefCell::new(FakeState::default());

        match apply_settings_without_reentrant_borrow(&state, snapshot()) {
            Ok(_) => {
                state.borrow_mut().finish_calls += 1;
            }
            Err(error) => panic!("unexpected error: {error}"),
        }

        let state = state.borrow();
        assert_eq!(state.apply_calls, 1);
        assert_eq!(state.finish_calls, 1);
    }
}
