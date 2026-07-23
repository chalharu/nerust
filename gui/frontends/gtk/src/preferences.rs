use std::{cell::RefCell, collections::HashSet, rc::Rc, sync::Arc};

use gio::glib::object::{Cast as _, IsA};
use gtk::{
    glib,
    prelude::{
        BoxExt as _, ButtonExt as _, CheckButtonExt as _, ComboBoxExt as _, ComboBoxExtManual,
        DialogExt as _, EditableExt as _, GridExt as _, GtkWindowExt as _, WidgetExt as _,
    },
};
use nerust_core_traits::{
    factory::{
        CoreFactory,
        descriptor::{SystemSettingsFieldKind, SystemSettingsFieldModel, SystemSettingsPageModel},
    },
    identity::SystemId,
};
use nerust_gui_runtime::settings::{SettingsSnapshot, apply::validate_shared_settings};
use nerust_gui_settings::{language::AppLanguage, local::ScalingMode, shared::StoragePolicy};
use nerust_gui_shell::{
    session::{
        SessionError,
        access::{FrontendSession, SettingsResult},
        input::build_topology,
    },
    settings::{
        bindings::{
            conflicting_keys,
            descriptors::{keyboard_binding_sections, shortcut_descriptors},
        },
        editor::{CaptureTarget, apply_capture_target, current_binding_label},
        factory::{apply_settings_choice, resolve_label, settings_view},
        i18n::{UiText, text},
    },
};
use nerust_input_traits::{AttachmentId, ControllerProfile, InputTopologyDescriptor, SlotInfo};

use crate::State;

#[derive(Clone)]
struct InputRow {
    target: CaptureTarget,
    value_label: gtk::Label,
    change_button: gtk::Button,
    clear_button: gtk::Button,
}

struct SlotCombo {
    slot_id: AttachmentId,
    combo: gtk::ComboBoxText,
}

struct InputTab {
    _factory: Arc<dyn CoreFactory>,
    slot_combos: Rc<RefCell<Vec<SlotCombo>>>,
    _key_binding_box: Rc<gtk::Box>,
    input_rows: Rc<RefCell<Vec<InputRow>>>,
}

struct SystemTab {
    factory: Arc<dyn CoreFactory>,
    field_widgets: Vec<(String, gtk::ComboBoxText)>,
}

/// Build a dynamic InputTopologyDescriptor from controller assignments.
fn dynamic_topology(
    assignments: &[(AttachmentId, Option<Rc<dyn ControllerProfile>>)],
    slots: &[SlotInfo],
) -> InputTopologyDescriptor {
    build_topology(assignments, slots)
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
    let factory: Option<Arc<dyn CoreFactory>> = state.borrow().active_factory();
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

    // Input: per-system controller assignments and key bindings.
    let input_notebook = gtk::Notebook::new();
    input_notebook.set_scrollable(true);
    input_notebook.set_tab_pos(gtk::PositionType::Top);
    input_page.append(&input_notebook);
    let mut input_tabs: Vec<InputTab> = Vec::new();
    let input_conflict_label = gtk::Label::new(None);
    input_conflict_label.set_xalign(0.0);

    for factory in state.borrow().ctx.registry.all() {
        let tab_label = gtk::Label::new(Some(factory.display_name()));
        let tab_page = gtk::Box::new(gtk::Orientation::Vertical, 6);
        tab_page.set_margin_start(6);
        tab_page.set_margin_end(6);
        tab_page.set_margin_top(6);
        tab_page.set_margin_bottom(6);

        // Controller assignment ComboBoxes per slot
        let input_factory = factory.input_system_factory();
        let slots = input_factory.slots().to_vec();
        let controllers: Vec<Rc<dyn ControllerProfile>> = input_factory.controllers();
        let slot_combos: Rc<RefCell<Vec<SlotCombo>>> = Rc::new(RefCell::new(Vec::new()));
        let factory2 = factory.clone();

        // Read current assignments to pre-select combo boxes.
        let sid = factory.system_id().to_string();
        let default_assignments = factory.input_system_factory().default_assignments();
        let current_assignments: Vec<(AttachmentId, Option<Rc<dyn ControllerProfile>>)> = draft
            .borrow()
            .app_state
            .controller_assignments
            .get(&sid)
            .map(|pairs| {
                pairs
                    .iter()
                    .filter_map(|(slot_id, ctrl_opt)| {
                        let att = match input_factory.resolve_slot(slot_id) {
                            Some(a) => a,
                            None => {
                                log::warn!(
                                    "unknown persisted slot ID in GTK preferences: {slot_id}"
                                );
                                return None;
                            }
                        };
                        let profile = ctrl_opt
                            .as_ref()
                            .and_then(|id| input_factory.resolve_controller(id));
                        Some((att, profile))
                    })
                    .collect()
            })
            .unwrap_or_else(|| {
                default_assignments
                    .slots
                    .iter()
                    .map(|(slot_id, profile)| (*slot_id, profile.clone()))
                    .collect()
            });

        // Key binding section (rebuilt dynamically on controller change)
        let key_binding_box = Rc::new(gtk::Box::new(gtk::Orientation::Vertical, 0));
        key_binding_box.set_vexpand(true);
        let input_rows: Rc<RefCell<Vec<InputRow>>> = Rc::new(RefCell::new(Vec::new()));

        // Rebuild key binding UI from current assignments.
        fn rebuild_input_ui(
            key_binding_box: &gtk::Box,
            input_rows: &Rc<RefCell<Vec<InputRow>>>,
            factory: &dyn CoreFactory,
            assignments: &[(AttachmentId, Option<Rc<dyn ControllerProfile>>)],
            language: AppLanguage,
            slots: &[SlotInfo],
        ) {
            while let Some(child) = key_binding_box.first_child() {
                key_binding_box.remove(&child);
            }
            let topology = dynamic_topology(assignments, slots);
            let rows = build_input_rows(language, key_binding_box, &topology, factory.system_id());
            *input_rows.borrow_mut() = rows;
        }

        if !controllers.is_empty() {
            for slot in &slots {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                let lbl = gtk::Label::new(Some(slot.label));
                row.append(&lbl);
                let combo = gtk::ComboBoxText::new();
                combo.append_text(text(language, UiText::None)); // "None" option
                for c in controllers.iter().filter(|c| {
                    c.port_sets()
                        .iter()
                        .any(|ps| ps.ports.first() == Some(&slot.id))
                }) {
                    combo.append_text(c.label());
                }
                {
                    let sc = slot_combos.clone();
                    let f = factory2.clone();
                    let kb_box = key_binding_box.clone();
                    let kb_rows = input_rows.clone();
                    let lang = language;
                    let d = draft.clone();
                    let ok = ok_button.clone();
                    combo.connect_changed(move |_| {
                        let combos = sc.borrow();
                        let input = f.input_system_factory();
                        // Build occupied from current selections
                        let mut occupied = HashSet::new();
                        for sc_item in combos.iter() {
                            let Some(label) = sc_item.combo.active_text() else {
                                continue;
                            };
                            for p in input.controllers().iter() {
                                if p.label() != label.as_str() {
                                    continue;
                                }
                                for ps in p.port_sets() {
                                    if ps.ports.contains(&sc_item.slot_id) {
                                        for &port in ps.ports {
                                            occupied.insert(port);
                                        }
                                    }
                                }
                            }
                        }
                        for sc_item in combos.iter() {
                            let occ = occupied.contains(&sc_item.slot_id)
                                && sc_item
                                    .combo
                                    .active_text()
                                    .is_none_or(|t| t.is_empty() || t == text(lang, UiText::None));
                            sc_item.combo.set_sensitive(!occ);
                        }
                        // Rebuild key binding UI
                        drop(combos);
                        let mut current_assignments: Vec<(
                            AttachmentId,
                            Option<Rc<dyn ControllerProfile>>,
                        )> = sc
                            .borrow()
                            .iter()
                            .map(|sc_item| {
                                let profile = sc_item.combo.active_text().and_then(|label| {
                                    input
                                        .controllers()
                                        .iter()
                                        .find(|p| p.label() == label)
                                        .cloned()
                                });
                                (sc_item.slot_id, profile)
                            })
                            .collect();
                        // Clear multi-port conflicts
                        let snapshot = current_assignments.clone();
                        for (slot_id, ctrl_opt) in &snapshot {
                            let profile = match ctrl_opt {
                                Some(p) => p.as_ref(),
                                None => continue,
                            };
                            for ps in profile.port_sets() {
                                if ps.ports.len() <= 1 {
                                    continue;
                                }
                                if !ps.ports.contains(slot_id) {
                                    continue;
                                }
                                for &port in ps.ports {
                                    if port != *slot_id
                                        && let Some(other) =
                                            current_assignments.iter_mut().find(|(s, _)| *s == port)
                                    {
                                        other.1 = None;
                                        // Also clear the combo box for this slot
                                        if let Some(sc_item) =
                                            sc.borrow().iter().find(|s| s.slot_id == port)
                                        {
                                            sc_item.combo.set_active(Some(0));
                                        }
                                    }
                                }
                            }
                        }
                        rebuild_input_ui(
                            &kb_box,
                            &kb_rows,
                            f.as_ref(),
                            &current_assignments,
                            lang,
                            f.input_system_factory().slots(),
                        );
                        // Update key binding labels from current settings
                        let snapshot = d.borrow();
                        for row in kb_rows.borrow().iter() {
                            row.value_label.set_text(
                                current_binding_label(&snapshot, &row.target).unwrap_or(""),
                            );
                        }
                        // Disable OK button if no controller is assigned
                        ok.set_sensitive(current_assignments.iter().any(|(_, c)| c.is_some()));
                    });
                }
                slot_combos.borrow_mut().push(SlotCombo {
                    slot_id: slot.id,
                    combo: combo.clone(),
                });
                // Pre-select based on current assignment
                if let Some((_, Some(profile))) =
                    current_assignments.iter().find(|(s, _)| *s == slot.id)
                {
                    let idx = controllers
                        .iter()
                        .filter(|c| {
                            c.port_sets()
                                .iter()
                                .any(|ps| ps.ports.first() == Some(&slot.id))
                        })
                        .position(|c| c.profile_id() == profile.profile_id())
                        .map(|pos| pos as u32 + 1); // +1 for "None" at index 0
                    if let Some(active) = idx {
                        combo.set_active(Some(active));
                    }
                } else {
                    combo.set_active(Some(0)); // None
                }
                row.append(&combo);
                tab_page.append(&row);
            }
        }

        tab_page.append(&input_conflict_label);
        tab_page.append(&*key_binding_box);
        rebuild_input_ui(
            &key_binding_box,
            &input_rows,
            factory.as_ref(),
            &current_assignments,
            language,
            input_factory.slots(),
        );

        input_notebook.append_page(&tab_page, Some(&tab_label));
        input_tabs.push(InputTab {
            _factory: factory.clone(),
            slot_combos: slot_combos.clone(),
            _key_binding_box: key_binding_box.clone(),
            input_rows: input_rows.clone(),
        });
    }
    let input_tabs = Rc::new(RefCell::new(input_tabs));

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
    let sample_rate_combo = {
        let registry = &state.borrow().ctx.audio_registry;
        let rates = registry.supported_rates();
        let rates: &[u32] = if rates.is_empty() {
            &[44_100, 48_000]
        } else {
            rates
        };
        let combo = gtk::ComboBoxText::new();
        for &rate in rates {
            let id = format!("{rate}");
            combo.append(Some(&id), &id);
        }
        combo
    };
    audio_page.append(&labeled_row(
        text(language, UiText::SampleRate),
        &sample_rate_combo,
    ));
    let latency_spin = gtk::SpinButton::with_range(10.0, 200.0, 1.0);
    audio_page.append(&labeled_row(
        text(language, UiText::AudioLatency),
        &latency_spin,
    ));

    let _snapshot = state.borrow().settings_snapshot().clone();
    let system_notebook = gtk::Notebook::new();
    system_notebook.set_scrollable(true);
    system_notebook.set_tab_pos(gtk::PositionType::Top);
    system_page.append(&system_notebook);

    let mut system_tabs: Vec<SystemTab> = Vec::new();
    for (factory, (name, model)) in state
        .borrow()
        .ctx
        .registry
        .all()
        .iter()
        .zip(state.borrow().settings_pages().iter())
    {
        let tab_label = gtk::Label::new(Some(name));
        let tab_page = gtk::Box::new(gtk::Orientation::Vertical, 6);
        tab_page.set_margin_start(6);
        tab_page.set_margin_end(6);
        tab_page.set_margin_top(6);
        tab_page.set_margin_bottom(6);

        let mut field_widgets: Vec<(String, gtk::ComboBoxText)> = Vec::new();
        for field in model.fields.iter() {
            let label = resolve_label(field.label_id, language);
            let combo = combo_from_optional_system_field(Some(field), language);
            tab_page.append(&labeled_row(&label, &combo));
            field_widgets.push((field.id.as_str().to_string(), combo));
        }

        system_notebook.append_page(&tab_page, Some(&tab_label));
        system_tabs.push(SystemTab {
            factory: (*factory).clone(),
            field_widgets,
        });
    }
    let system_tabs = Rc::new(RefCell::new(system_tabs));

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
        &system_tabs,
        &input_tabs,
        &capture_target,
        text(language, UiText::Unbound),
        text(language, UiText::CapturePrompt),
        factory.as_deref(),
    );
    refresh_validation(
        &draft.borrow(),
        language,
        &ok_button,
        &storage_dir_row,
        &storage_error_label,
        &input_conflict_label,
        factory.as_deref(),
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
            &system_tabs,
            &input_tabs,
            &capture_target,
            language,
            factory.clone(),
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
            &system_tabs,
            &input_tabs,
            &capture_target,
            language,
            factory.clone(),
        ),
    );
    connect_local_updates(
        &draft,
        factory.as_ref(),
        &fullscreen_check,
        &scaling_combo,
        &vsync_check,
        &mute_check,
        &volume_spin,
        &sample_rate_combo,
        &latency_spin,
        &system_tabs,
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
            &system_tabs,
            &input_tabs,
            &capture_target,
            language,
            factory.clone(),
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
            &system_tabs,
            &input_tabs,
            &capture_target,
            language,
            factory.clone(),
        );
        let _ = key_controller.connect_key_pressed(move |_, key, _, _| {
            let Some(target) = capture_target.borrow().clone() else {
                return glib::Propagation::Proceed;
            };
            let Some(mapped_key) = key.try_into().ok() else {
                return glib::Propagation::Stop;
            };
            apply_capture_target(&mut draft.borrow_mut(), &target, Some(mapped_key));
            *capture_target.borrow_mut() = None;
            refresh_all_from_draft(&draft.borrow(), &widgets);
            glib::Propagation::Stop
        });
    }
    dialog.add_controller(key_controller);

    for tab in input_tabs.borrow().iter() {
        for row in tab.input_rows.borrow().iter().cloned() {
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
                &system_tabs,
                &input_tabs,
                &capture_target,
                language,
                factory.clone(),
            );
            let target = row.target.clone();
            let _ = row.change_button.connect_clicked(move |_| {
                *capture_target.borrow_mut() = Some(target.clone());
                refresh_all_from_draft(&draft.borrow(), &widgets);
            });
        }
    }
    for tab in input_tabs.borrow().iter() {
        for row in tab.input_rows.borrow().iter().cloned() {
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
                &system_tabs,
                &input_tabs,
                &capture_target,
                language,
                factory.clone(),
            );
            let target = row.target.clone();
            let _ = row.clear_button.connect_clicked(move |_| {
                apply_capture_target(&mut draft.borrow_mut(), &target, None);
                *capture_target.borrow_mut() = None;
                refresh_all_from_draft(&draft.borrow(), &widgets);
            });
        }
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
            &system_tabs,
            &input_tabs,
            &capture_target,
            language,
            factory.clone(),
        );
        let input_tabs_ok = input_tabs.clone();
        let _ = dialog.connect_response(move |dialog, response| {
            if should_apply_response(response) {
                for tab in input_tabs_ok.borrow().iter() {
                    let factory = &tab._factory;
                    let input_factory = factory.input_system_factory();
                    let assignments = {
                        let combos = tab.slot_combos.borrow();
                        let mut slots: Vec<(AttachmentId, Option<Rc<dyn ControllerProfile>>)> =
                            combos
                                .iter()
                                .map(|sc| {
                                    let profile = sc.combo.active_text().and_then(|t| {
                                        let ctrls = input_factory.controllers();
                                        ctrls.iter().find(|c| c.label() == t).cloned()
                                    });
                                    (sc.slot_id, profile)
                                })
                                .collect();
                        let assigned = slots.clone();
                        for (slot_id, ctrl_opt) in &assigned {
                            let profile = match ctrl_opt {
                                Some(p) => p.as_ref(),
                                None => continue,
                            };
                            for ps in profile.port_sets() {
                                if ps.ports.len() <= 1 {
                                    continue;
                                }
                                if !ps.ports.contains(slot_id) {
                                    continue;
                                }
                                for &port in ps.ports {
                                    if port != *slot_id
                                        && let Some(other) =
                                            slots.iter_mut().find(|(s, _)| *s == port)
                                    {
                                        other.1 = None;
                                    }
                                }
                            }
                        }
                        nerust_input_traits::InputAssignments { slots }
                    };
                    let current_pairs = state.borrow().session.current_assignments_pairs();
                    let new_pairs = assignments.to_string_pairs();
                    if current_pairs != new_pairs
                        && let Err(e) = state
                            .borrow_mut()
                            .session
                            .reassign_controllers(&assignments)
                    {
                        log::warn!("controller reassign failed: {e}");
                    }
                    let sid = factory.system_id().to_string();
                    draft
                        .borrow_mut()
                        .app_state
                        .controller_assignments
                        .insert(sid, assignments.to_string_pairs());
                }

                let snapshot = draft.borrow().clone();
                if let Some(ref f) = factory
                    && !validation_errors(&snapshot, &**f).is_empty()
                {
                    refresh_all_from_draft(&snapshot, &widgets);
                    return;
                }
                match apply_settings_without_reentrant_borrow(state.as_ref(), snapshot.clone()) {
                    Ok(plan) => {
                        if plan.fullscreen_default_changed {
                            parent.set_fullscreened(snapshot.local.video.window.fullscreen_default);
                        }
                        if plan.scaling_changed
                            && let Some(profile) = state.borrow().render_profile()
                        {
                            apply_scaling_to_window(
                                &parent,
                                snapshot.local.video.window.scaling,
                                profile,
                            );
                        }
                        dialog.close();
                        run_finish_callback(&finish_for_response);
                    }
                    Err(error) => {
                        error_label.set_text(&error.to_string());
                    }
                }
            } else {
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
    system_tabs: Rc<RefCell<Vec<SystemTab>>>,
    input_tabs: Rc<RefCell<Vec<InputTab>>>,
    capture_target: Rc<RefCell<Option<CaptureTarget>>>,
    language: AppLanguage,
    factory: Option<Arc<dyn CoreFactory>>,
}

type FinishCallback = Rc<RefCell<Option<Box<dyn FnOnce()>>>>;

trait SettingsApplier {
    fn apply_settings(
        &mut self,
        settings: SettingsSnapshot,
    ) -> Result<SettingsResult, SessionError>;
}

impl SettingsApplier for State {
    fn apply_settings(
        &mut self,
        settings: SettingsSnapshot,
    ) -> Result<SettingsResult, SessionError> {
        FrontendSession::apply_settings(self, settings)
    }
}

fn apply_settings_without_reentrant_borrow<T: SettingsApplier>(
    state: &RefCell<T>,
    snapshot: SettingsSnapshot,
) -> Result<SettingsResult, SessionError> {
    state.borrow_mut().apply_settings(snapshot)
}

fn should_apply_response(response: gtk::ResponseType) -> bool {
    matches!(response, gtk::ResponseType::Ok)
}

#[expect(clippy::too_many_arguments)]
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
    system_tabs: &Rc<RefCell<Vec<SystemTab>>>,
    input_tabs: &Rc<RefCell<Vec<InputTab>>>,
    capture_target: &Rc<RefCell<Option<CaptureTarget>>>,
    language: AppLanguage,
    factory: Option<Arc<dyn CoreFactory>>,
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
        system_tabs: system_tabs.clone(),
        input_tabs: input_tabs.clone(),
        capture_target: capture_target.clone(),
        language,
        factory,
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
        &widgets.system_tabs,
        &widgets.input_tabs,
        &widgets.capture_target,
        text(widgets.language, UiText::Unbound),
        text(widgets.language, UiText::CapturePrompt),
        widgets.factory.as_deref(),
    );
    refresh_validation(
        snapshot,
        widgets.language,
        &widgets.ok_button,
        &widgets.storage_dir_row,
        &widgets.storage_error_label,
        &widgets.input_conflict_label,
        widgets.factory.as_deref(),
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

#[expect(clippy::too_many_arguments)]
fn connect_local_updates(
    draft: &Rc<RefCell<SettingsSnapshot>>,
    _factory: Option<&Arc<dyn CoreFactory>>,
    fullscreen_check: &gtk::CheckButton,
    scaling_combo: &gtk::ComboBoxText,
    vsync_check: &gtk::CheckButton,
    mute_check: &gtk::CheckButton,
    volume_spin: &gtk::SpinButton,
    sample_rate_combo: &gtk::ComboBoxText,
    latency_spin: &gtk::SpinButton,
    system_tabs: &Rc<RefCell<Vec<SystemTab>>>,
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
    for tab in system_tabs.borrow().iter() {
        for (field_id, combo) in tab.field_widgets.iter() {
            let draft = draft.clone();
            let widgets = widgets.clone();
            let factory = tab.factory.clone();
            let field_id = field_id.clone();
            let _ = combo.connect_changed(move |combo| {
                {
                    let mut snapshot = draft.borrow_mut();
                    let _ = apply_settings_choice(
                        &*factory,
                        &mut snapshot,
                        &nerust_core_traits::factory::descriptor::SystemSettingsFieldId(
                            field_id.clone().into(),
                        ),
                        &nerust_core_traits::factory::descriptor::SystemSettingsChoiceId(
                            combo
                                .active_id()
                                .map(|value| value.to_string())
                                .unwrap_or_default()
                                .into(),
                        ),
                    );
                }
                refresh_all_from_draft(&draft.borrow(), &widgets);
            });
        }
    }
}

fn refresh_validation(
    snapshot: &SettingsSnapshot,
    language: AppLanguage,
    ok_button: &gtk::Widget,
    storage_dir_row: &gtk::Box,
    storage_error_label: &gtk::Label,
    input_conflict_label: &gtk::Label,
    factory: Option<&dyn CoreFactory>,
) {
    let Some(factory) = factory else {
        let storage_error = validate_shared_settings(&snapshot.shared)
            .err()
            .map(|error| error.to_string());
        let has_errors = storage_error.is_some();
        ok_button.set_sensitive(!has_errors);
        storage_dir_row.set_visible(matches!(
            snapshot.shared.persistence.storage_policy,
            StoragePolicy::CustomDirectory
        ));
        if let Some(error) = storage_error {
            storage_error_label.set_text(&error);
            storage_error_label.set_visible(true);
        } else {
            storage_error_label.set_visible(false);
        }
        input_conflict_label.set_visible(false);
        return;
    };
    let system = factory.system_id();
    let sid = system.to_string();
    let input_factory = factory.input_system_factory();
    let assignments: Vec<(AttachmentId, Option<Rc<dyn ControllerProfile>>)> = snapshot
        .app_state
        .controller_assignments
        .get(&sid)
        .map(|pairs| {
            pairs
                .iter()
                .filter_map(|(slot_id, ctrl_opt)| {
                    let att = match input_factory.resolve_slot(slot_id) {
                        Some(a) => a,
                        None => {
                            log::warn!("unknown persisted slot ID in validation: {slot_id}");
                            return None;
                        }
                    };
                    let profile = ctrl_opt
                        .as_ref()
                        .and_then(|id| input_factory.resolve_controller(id));
                    Some((att, profile))
                })
                .collect()
        })
        .unwrap_or_else(|| input_factory.default_assignments().slots);
    let storage_error = validate_shared_settings(&snapshot.shared)
        .err()
        .map(|error| error.to_string());
    let conflicts = conflicting_keys(
        &snapshot.shared,
        &dynamic_topology(&assignments, input_factory.slots()),
        system,
    );
    let has_errors = storage_error.is_some()
        || !conflicts.is_empty()
        || !assignments.iter().any(|(_, c)| c.is_some());
    storage_dir_row.set_visible(matches!(
        snapshot.shared.persistence.storage_policy,
        StoragePolicy::CustomDirectory
    ));
    storage_error_label.set_text(storage_error.as_deref().unwrap_or(""));
    if let Some((key, labels)) = conflicts.iter().next() {
        input_conflict_label.set_text(&format!("{}: {}", key.label(), labels.join(", ")));
    } else {
        input_conflict_label.set_text("");
    }
    if !conflicts.is_empty() && input_conflict_label.text().is_empty() {
        input_conflict_label.set_text(text(language, UiText::ConflictDetected));
    }
    ok_button.set_sensitive(!has_errors);
}

fn validation_errors(snapshot: &SettingsSnapshot, factory: &dyn CoreFactory) -> Vec<String> {
    let mut errors = Vec::new();
    if let Err(error) = validate_shared_settings(&snapshot.shared) {
        errors.push(error.to_string());
    }
    let sid = factory.system_id().to_string();
    let input_factory = factory.input_system_factory();
    let assignments: Vec<(AttachmentId, Option<Rc<dyn ControllerProfile>>)> = snapshot
        .app_state
        .controller_assignments
        .get(&sid)
        .map(|pairs| {
            pairs
                .iter()
                .filter_map(|(slot_id, ctrl_opt)| {
                    let att = match input_factory.resolve_slot(slot_id) {
                        Some(a) => a,
                        None => {
                            log::warn!("unknown persisted slot ID in validation errors: {slot_id}");
                            return None;
                        }
                    };
                    let profile = ctrl_opt
                        .as_ref()
                        .and_then(|id| input_factory.resolve_controller(id));
                    Some((att, profile))
                })
                .collect()
        })
        .unwrap_or_else(|| input_factory.default_assignments().slots);
    if !assignments.iter().any(|(_, c)| c.is_some()) {
        errors.push("At least one controller must be assigned".to_string());
    }
    for (key, labels) in conflicting_keys(
        &snapshot.shared,
        &dynamic_topology(&assignments, input_factory.slots()),
        factory.system_id(),
    ) {
        errors.push(format!("{}: {}", key.label(), labels.join(", ")));
    }
    errors
}

#[expect(clippy::too_many_arguments)]
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
    system_tabs: &Rc<RefCell<Vec<SystemTab>>>,
    input_tabs: &Rc<RefCell<Vec<InputTab>>>,
    capture_target: &Rc<RefCell<Option<CaptureTarget>>>,
    unbound_label: &str,
    capture_label: &str,
    _factory: Option<&dyn CoreFactory>,
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
    let active = format!("{}", snapshot.local.audio.sample_rate);
    sample_rate_combo.set_active_id(Some(&active));
    latency_spin.set_value(f64::from(snapshot.local.audio.latency_ms));
    for tab in system_tabs.borrow().iter() {
        let sid = tab.factory.system_id();
        let view = settings_view(snapshot, &sid);
        let page = tab.factory.settings_page(&view);
        for (field_id, combo) in &tab.field_widgets {
            apply_system_field_by_id_to_combo(&page, field_id, combo);
        }
    }

    for tab in input_tabs.borrow().iter() {
        for row in tab.input_rows.borrow().iter() {
            row.value_label
                .set_text(if capture_target.borrow().as_ref() == Some(&row.target) {
                    capture_label
                } else {
                    current_binding_label(snapshot, &row.target).unwrap_or(unbound_label)
                });
        }
    }
}

fn build_input_rows(
    language: AppLanguage,
    input_page: &gtk::Box,
    topology: &InputTopologyDescriptor,
    system: SystemId,
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
    for section in keyboard_binding_sections(topology, system) {
        let section_page = gtk::Box::new(gtk::Orientation::Vertical, 12);
        section_page.set_hexpand(true);
        let grid = gtk::Grid::new();
        grid.set_column_spacing(12);
        grid.set_row_spacing(6);
        grid.set_hexpand(true);
        section_page.append(&grid);
        input_stack.add_titled(
            &section_page,
            Some(section.attachment.as_str()),
            section.attachment_label,
        );

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
    page: &'a SystemSettingsPageModel,
    field_id: &str,
) -> Option<&'a SystemSettingsFieldModel> {
    page.fields
        .iter()
        .find(|field| field.id.as_str() == field_id)
}

fn combo_from_optional_system_field(
    field: Option<&SystemSettingsFieldModel>,
    language: nerust_gui_settings::language::AppLanguage,
) -> gtk::ComboBoxText {
    let combo = gtk::ComboBoxText::new();
    if let Some(field) = field {
        let SystemSettingsFieldKind::Choice { options, .. } = &field.kind;
        for option in options.iter() {
            let label = resolve_label(option.label_id, language);
            combo.append(Some(option.id.as_str()), &label);
        }
    }
    combo
}

fn apply_system_field_by_id_to_combo(
    page: &SystemSettingsPageModel,
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

fn apply_scaling_to_window(
    window: &gtk::ApplicationWindow,
    scaling: nerust_gui_settings::local::ScalingMode,
    render_profile: &nerust_render_traits::VideoRenderProfile,
) {
    let base_width = render_profile.physical_size.width as i32;
    let base_height = render_profile.physical_size.height as i32;
    let (w, h) = match scaling {
        nerust_gui_settings::local::ScalingMode::FitToWindow => (0, 0),
        nerust_gui_settings::local::ScalingMode::X1 => (base_width, base_height),
        nerust_gui_settings::local::ScalingMode::X2 => (base_width * 2, base_height * 2),
        nerust_gui_settings::local::ScalingMode::X3 => (base_width * 3, base_height * 3),
        nerust_gui_settings::local::ScalingMode::X4 => (base_width * 4, base_height * 4),
        nerust_gui_settings::local::ScalingMode::X5 => (base_width * 5, base_height * 5),
    };
    if w > 0 && h > 0 {
        window.set_default_size(w, h);
    }
    window.queue_resize();
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

fn run_finish_callback(finish: &FinishCallback) {
    if let Some(callback) = finish.borrow_mut().take() {
        callback();
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, sync::Arc};

    use nerust_core_traits::factory::descriptor::{
        SystemSettingsChoiceId, SystemSettingsFieldId, SystemSettingsFieldKind,
        SystemSettingsFieldModel, SystemSettingsPageModel,
    };
    use nerust_gui_runtime::settings::SettingsSnapshot;
    use nerust_gui_shell::{
        session::{SessionError, access::SettingsResult},
        settings::defaults::seed::{
            default_app_state, default_local_settings, default_shared_settings,
        },
    };

    use super::{
        SettingsApplier, apply_settings_without_reentrant_borrow, should_apply_response,
        system_field_by_id,
    };

    #[derive(Default)]
    struct FakeState {
        apply_calls: usize,
        finish_calls: usize,
    }

    impl SettingsApplier for FakeState {
        fn apply_settings(
            &mut self,
            _settings: SettingsSnapshot,
        ) -> Result<SettingsResult, SessionError> {
            self.apply_calls += 1;
            Ok(SettingsResult::default())
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

    #[test]
    fn close_button_uses_cancel_path() {
        assert!(should_apply_response(gtk::ResponseType::Ok));
        assert!(!should_apply_response(gtk::ResponseType::DeleteEvent));
        assert!(!should_apply_response(gtk::ResponseType::Cancel));
    }

    #[test]
    fn system_field_by_id_finds_existing_field() {
        let model = SystemSettingsPageModel {
            fields: Arc::new([SystemSettingsFieldModel {
                id: SystemSettingsFieldId("video.filter".into()),
                label_id: "nes.video.filter",
                kind: SystemSettingsFieldKind::Choice {
                    selected: SystemSettingsChoiceId("ntsc_composite".into()),
                    options: Arc::new([]),
                },
            }]),
        };
        assert!(system_field_by_id(&model, "video.filter").is_some());
        assert!(system_field_by_id(&model, "nonexistent").is_none());
    }
}
