use std::{cell::RefCell, rc::Rc, sync::Arc};

use gio::glib::object::{Cast as _, IsA};
use gtk::{
    glib,
    prelude::{
        BoxExt as _, ButtonExt as _, CheckButtonExt as _, ComboBoxExt as _, DialogExt as _,
        EditableExt as _, GridExt as _, GtkWindowExt as _, WidgetExt as _,
    },
};
use nerust_core_traits::SystemId;
use nerust_gui_runtime::settings::{SettingsSnapshot, apply::validate_shared_settings};
use nerust_gui_settings::{
    input::KeyboardKey, language::AppLanguage, local::ScalingMode, shared::StoragePolicy,
};
use nerust_gui_shell::session::access::{FrontendSession, SettingsResult};
use nerust_gui_shell::{
    descriptor::SystemSettingsFieldKind,
    factory::CoreFactory,
    session::SessionError,
    settings::{
        bindings::{
            conflicting_keys,
            descriptors::{keyboard_binding_sections, shortcut_descriptors},
            keys::keyboard_key_label,
        },
        editor::{CaptureTarget, apply_capture_target, current_binding_label},
        i18n::{UiText, text},
    },
};
use nerust_input_traits::{ControllerProfile, InputTopologyDescriptor};
use std::collections::HashSet;

use crate::State;

#[derive(Clone)]
struct InputRow {
    target: CaptureTarget,
    value_label: gtk::Label,
    change_button: gtk::Button,
    clear_button: gtk::Button,
}

/// Build a dynamic InputTopologyDescriptor from controller default assignments.
fn dynamic_topology(
    factory: &dyn nerust_gui_shell::factory::CoreFactory,
) -> InputTopologyDescriptor {
    use nerust_input_traits::*;
    let input = factory.input_system_factory();
    let defaults = input.default_assignments();
    let mut ports = Vec::new();
    let mut seen_devices = HashSet::new();
    let mut devices = Vec::new();

    fn att(slot: &str) -> &'static str {
        match slot {
            "player1" => "nes.attachment.player1",
            "player2" => "nes.attachment.player2",
            _ => "unknown",
        }
    }

    for (slot_id, ctrl_opt) in &defaults.slots {
        let ctrl_id: &'static str = match ctrl_opt {
            Some(id) if id == "nes.standard_pad" => "nes.standard_pad",
            Some(id) if id == "nes.famicom" => "nes.famicom",
            _ => continue,
        };
        let Some(profile) = input.controllers().iter().find(|p| p.id() == ctrl_id) else {
            continue;
        };
        for ps in profile.port_sets() {
            if let Some(pos) = ps.ports.iter().position(|&p| p == slot_id) {
                if seen_devices.insert(ctrl_id) {
                    let controls = profile.port_groups()[pos];
                    devices.push(DeviceDescriptor {
                        kind: DeviceKindId::new(ctrl_id),
                        label: profile.label(),
                        controls: controls
                            .iter()
                            .map(|ci| {
                                ControlDescriptor::Digital(DigitalControlDescriptor {
                                    id: DigitalControlId::new(ci.id),
                                    label: ci.label,
                                    description: ci.label,
                                })
                            })
                            .collect(),
                    });
                }
                for &port in ps.ports {
                    let full = att(port);
                    if !ports.iter().any(|p: &PortDescriptor| p.id.as_str() == full) {
                        ports.push(PortDescriptor {
                            id: PortId::new(full),
                            label: port,
                            attachments: vec![AttachmentSlotDescriptor {
                                id: AttachmentId::new(full),
                                label: port,
                                device: DeviceKindId::new(ctrl_id),
                                supported_devices: vec![DeviceKindId::new(ctrl_id)],
                            }],
                        });
                    }
                }
            }
        }
    }
    if ports.is_empty() {
        factory.system_descriptor().input_topology
    } else {
        InputTopologyDescriptor { ports, devices }
    }
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
    let factory: Arc<dyn CoreFactory> = state.borrow().ctx.core_factory.clone();
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

    // Controller assignment ComboBoxes per slot
    let input_factory = factory.input_system_factory();
    let slots = input_factory.slots().to_vec();
    let controllers: Vec<&'static dyn ControllerProfile> = input_factory.controllers().to_vec();
    struct SlotCombo {
        slot_id: String,
        combo: gtk::ComboBoxText,
    }
    let slot_combos: Rc<RefCell<Vec<SlotCombo>>> = Rc::new(RefCell::new(Vec::new()));
    let factory2 = factory.clone();
    if !controllers.is_empty() {
        for slot in &slots {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let lbl = gtk::Label::new(Some(slot.label));
            row.append(&lbl);
            let combo = gtk::ComboBoxText::new();
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
                combo.connect_changed(move |_| {
                    let combos = sc.borrow();
                    let input = f.input_system_factory();
                    // Build occupied from current selections
                    let mut occupied = std::collections::HashSet::new();
                    for sc_item in combos.iter() {
                        let Some(label) = sc_item.combo.active_text() else {
                            continue;
                        };
                        for p in input.controllers().iter() {
                            if p.label() != label.as_str() {
                                continue;
                            }
                            for ps in p.port_sets() {
                                if ps.ports.contains(&sc_item.slot_id.as_str()) {
                                    for &port in ps.ports {
                                        occupied.insert(port);
                                    }
                                }
                            }
                        }
                    }
                    for sc_item in combos.iter() {
                        let occ = occupied.contains(sc_item.slot_id.as_str())
                            && sc_item.combo.active_text().is_none();
                        sc_item.combo.set_sensitive(!occ);
                    }
                });
            }
            slot_combos.borrow_mut().push(SlotCombo {
                slot_id: slot.id.to_string(),
                combo: combo.clone(),
            });
            row.append(&combo);
            input_page.append(&row);
        }
    }

    let input_conflict_label = gtk::Label::new(None);
    input_conflict_label.set_xalign(0.0);
    input_page.append(&input_conflict_label);
    let topology = dynamic_topology(factory.as_ref());
    let input_rows = build_input_rows(language, &input_page, &topology, factory.system_id());

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
    let system_page_model = state.borrow().settings_page();
    let filter_field = system_field_by_id(&system_page_model, "video.filter");
    let filter_combo = combo_from_optional_system_field(filter_field, language);
    let filter_label = filter_field
        .map(|field| nerust_gui_shell::settings::resolve_label(field.label_id, language))
        .unwrap_or_else(|| nerust_gui_shell::settings::resolve_label("nes.video.filter", language));
    system_page.append(&labeled_row(&filter_label, &filter_combo));
    let mmc3_field = system_field_by_id(&system_page_model, "core.mmc3_irq_variant");
    let mmc3_combo = combo_from_optional_system_field(mmc3_field, language);
    let mmc3_label = mmc3_field
        .map(|field| nerust_gui_shell::settings::resolve_label(field.label_id, language))
        .unwrap_or_else(|| {
            nerust_gui_shell::settings::resolve_label("nes.core.mmc3_irq_variant", language)
        });
    system_page.append(&labeled_row(&mmc3_label, &mmc3_combo));

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
        &filter_combo,
        &mmc3_combo,
        &input_rows,
        &capture_target,
        text(language, UiText::Unbound),
        text(language, UiText::CapturePrompt),
        factory.as_ref(),
    );
    refresh_validation(
        &draft.borrow(),
        language,
        &ok_button,
        &storage_dir_row,
        &storage_error_label,
        &input_conflict_label,
        factory.as_ref(),
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
            &filter_combo,
            &mmc3_combo,
            &input_rows,
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
            &filter_combo,
            &mmc3_combo,
            &input_rows,
            &capture_target,
            language,
            factory.clone(),
        ),
    );
    connect_local_updates(
        &draft,
        &factory,
        &fullscreen_check,
        &scaling_combo,
        &vsync_check,
        &mute_check,
        &volume_spin,
        &sample_rate_combo,
        &latency_spin,
        &filter_combo,
        &mmc3_combo,
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
            &filter_combo,
            &mmc3_combo,
            &input_rows,
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
            &filter_combo,
            &mmc3_combo,
            &input_rows,
            &capture_target,
            language,
            factory.clone(),
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
            &filter_combo,
            &mmc3_combo,
            &input_rows,
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
            &filter_combo,
            &mmc3_combo,
            &input_rows,
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
            &filter_combo,
            &mmc3_combo,
            &input_rows,
            &capture_target,
            language,
            factory.clone(),
        );
        let factory = factory.clone();
        let slot_combos = slot_combos.clone();
        let _ = dialog.connect_response(move |dialog, response| {
            if should_apply_response(response) {
                // Apply controller assignment changes
                let input_factory = factory.input_system_factory();
                let assignments = {
                    let combos = slot_combos.borrow();
                    let mut slots: Vec<(String, Option<String>)> = combos
                        .iter()
                        .map(|sc| {
                            let controller_id = sc.combo.active_text().and_then(|t| {
                                let ctrls = input_factory.controllers();
                                ctrls
                                    .iter()
                                    .find(|c| c.label() == t)
                                    .map(|c| c.id().to_string())
                            });
                            (sc.slot_id.clone(), controller_id)
                        })
                        .collect();
                    // Clear conflicting assignments from multi-port controllers
                    for i in 0..slots.len() {
                        let Some(ref ctrl_id) = slots[i].1.clone() else {
                            continue;
                        };
                        for p in input_factory.controllers().iter() {
                            if p.id() != ctrl_id {
                                continue;
                            }
                            for ps in p.port_sets() {
                                if ps.ports.len() <= 1 {
                                    continue;
                                }
                                if !ps.ports.contains(&slots[i].0.as_str()) {
                                    continue;
                                }
                                for &port in ps.ports {
                                    if port != slots[i].0 {
                                        if let Some(other) =
                                            slots.iter_mut().find(|(s, _)| *s == port)
                                        {
                                            other.1 = None;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    nerust_input_traits::InputAssignments { slots }
                };
                if let Err(e) = state
                    .borrow_mut()
                    .session
                    .reassign_controllers(&assignments)
                {
                    log::warn!("controller reassign failed: {e}");
                }

                let snapshot = draft.borrow().clone();
                if !validation_errors(&snapshot, factory.as_ref()).is_empty() {
                    refresh_all_from_draft(&snapshot, &widgets);
                    return;
                }
                match apply_settings_without_reentrant_borrow(state.as_ref(), snapshot.clone()) {
                    Ok(plan) => {
                        if plan.fullscreen_default_changed {
                            parent.set_fullscreened(snapshot.local.video.window.fullscreen_default);
                        }
                        if plan.scaling_changed {
                            apply_scaling_to_window(
                                &parent,
                                snapshot.local.video.window.scaling,
                                state.borrow().render_profile(),
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
    filter_combo: gtk::ComboBoxText,
    mmc3_combo: gtk::ComboBoxText,
    input_rows: Vec<InputRow>,
    capture_target: Rc<RefCell<Option<CaptureTarget>>>,
    language: AppLanguage,
    factory: Arc<dyn CoreFactory>,
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
    matches!(
        response,
        gtk::ResponseType::Ok | gtk::ResponseType::DeleteEvent
    )
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
    filter_combo: &gtk::ComboBoxText,
    mmc3_combo: &gtk::ComboBoxText,
    input_rows: &[InputRow],
    capture_target: &Rc<RefCell<Option<CaptureTarget>>>,
    language: AppLanguage,
    factory: Arc<dyn CoreFactory>,
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
        filter_combo: filter_combo.clone(),
        mmc3_combo: mmc3_combo.clone(),
        input_rows: input_rows.to_vec(),
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
        &widgets.filter_combo,
        &widgets.mmc3_combo,
        &widgets.input_rows,
        &widgets.capture_target,
        text(widgets.language, UiText::Unbound),
        text(widgets.language, UiText::CapturePrompt),
        widgets.factory.as_ref(),
    );
    refresh_validation(
        snapshot,
        widgets.language,
        &widgets.ok_button,
        &widgets.storage_dir_row,
        &widgets.storage_error_label,
        &widgets.input_conflict_label,
        widgets.factory.as_ref(),
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
    factory: &Arc<dyn CoreFactory>,
    fullscreen_check: &gtk::CheckButton,
    scaling_combo: &gtk::ComboBoxText,
    vsync_check: &gtk::CheckButton,
    mute_check: &gtk::CheckButton,
    volume_spin: &gtk::SpinButton,
    sample_rate_combo: &gtk::ComboBoxText,
    latency_spin: &gtk::SpinButton,
    filter_combo: &gtk::ComboBoxText,
    mmc3_combo: &gtk::ComboBoxText,
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
    {
        let draft = draft.clone();
        let widgets = widgets.clone();
        let factory = factory.clone();
        let _ = filter_combo.connect_changed(move |combo| {
            {
                let mut snapshot = draft.borrow_mut();
                let _ = nerust_gui_shell::settings::apply_settings_choice(
                    &*factory,
                    &mut snapshot,
                    &nerust_core_traits::factory::descriptor::SystemSettingsFieldId(
                        "video.filter".into(),
                    ),
                    &nerust_core_traits::factory::descriptor::SystemSettingsChoiceId(
                        combo
                            .active_id()
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "ntsc_composite".to_string())
                            .into(),
                    ),
                );
            }
            refresh_all_from_draft(&draft.borrow(), &widgets);
        });
    }
    {
        let draft = draft.clone();
        let widgets = widgets.clone();
        let factory = factory.clone();
        let _ = mmc3_combo.connect_changed(move |combo| {
            {
                let mut snapshot = draft.borrow_mut();
                let _ = nerust_gui_shell::settings::apply_settings_choice(
                    &*factory,
                    &mut snapshot,
                    &nerust_core_traits::factory::descriptor::SystemSettingsFieldId(
                        "core.mmc3_irq_variant".into(),
                    ),
                    &nerust_core_traits::factory::descriptor::SystemSettingsChoiceId(
                        combo
                            .active_id()
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "auto".to_string())
                            .into(),
                    ),
                );
            }
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
    factory: &dyn CoreFactory,
) {
    let system = factory.system_id();
    let storage_error = validate_shared_settings(&snapshot.shared)
        .err()
        .map(|error| error.to_string());
    let conflicts = conflicting_keys(
        &snapshot.shared,
        &dynamic_topology(factory.as_ref()),
        system,
    );
    let has_errors = storage_error.is_some() || !conflicts.is_empty();
    storage_dir_row.set_visible(matches!(
        snapshot.shared.persistence.storage_policy,
        StoragePolicy::CustomDirectory
    ));
    storage_error_label.set_text(storage_error.as_deref().unwrap_or(""));
    if let Some((key, labels)) = conflicts.iter().next() {
        input_conflict_label.set_text(&format!(
            "{}: {}",
            keyboard_key_label(*key),
            labels.join(", ")
        ));
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
    for (key, labels) in conflicting_keys(
        &snapshot.shared,
        &dynamic_topology(factory),
        factory.system_id(),
    ) {
        errors.push(format!(
            "{}: {}",
            keyboard_key_label(key),
            labels.join(", ")
        ));
    }
    errors
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
    filter_combo: &gtk::ComboBoxText,
    mmc3_combo: &gtk::ComboBoxText,
    input_rows: &[InputRow],
    capture_target: &Rc<RefCell<Option<CaptureTarget>>>,
    unbound_label: &str,
    capture_label: &str,
    factory: &dyn CoreFactory,
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
    let system_id = factory.system_id();
    let view = nerust_gui_shell::settings::settings_view(snapshot, &system_id);
    let system_page = factory.settings_page(&view);
    apply_system_field_by_id_to_combo(&system_page, "video.filter", filter_combo);
    apply_system_field_by_id_to_combo(&system_page, "core.mmc3_irq_variant", mmc3_combo);

    for row in input_rows {
        row.value_label
            .set_text(if capture_target.borrow().as_ref() == Some(&row.target) {
                capture_label
            } else {
                current_binding_label(snapshot, &row.target).unwrap_or(unbound_label)
            });
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
    page: &'a nerust_gui_shell::descriptor::SystemSettingsPageModel,
    field_id: &str,
) -> Option<&'a nerust_gui_shell::descriptor::SystemSettingsFieldModel> {
    page.fields
        .iter()
        .find(|field| field.id.as_str() == field_id)
}

fn combo_from_optional_system_field(
    field: Option<&nerust_gui_shell::descriptor::SystemSettingsFieldModel>,
    language: nerust_gui_settings::language::AppLanguage,
) -> gtk::ComboBoxText {
    let combo = gtk::ComboBoxText::new();
    if let Some(field) = field {
        let SystemSettingsFieldKind::Choice { options, .. } = &field.kind;
        for option in options.iter() {
            let label = nerust_gui_shell::settings::resolve_label(option.label_id, language);
            combo.append(Some(option.id.as_str()), &label);
        }
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

fn apply_scaling_to_window(
    window: &gtk::ApplicationWindow,
    scaling: nerust_gui_settings::local::ScalingMode,
    render_profile: &nerust_render_base::VideoRenderProfile,
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
    use std::cell::RefCell;

    use nerust_gui_runtime::settings::SettingsSnapshot;
    use nerust_gui_shell::{
        session::SessionError,
        session::access::SettingsResult,
        settings::defaults::seed::{
            default_app_state, default_local_settings, default_shared_settings,
        },
    };

    use super::{SettingsApplier, apply_settings_without_reentrant_borrow, should_apply_response};

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
    fn close_button_uses_apply_path() {
        assert!(should_apply_response(gtk::ResponseType::Ok));
        assert!(should_apply_response(gtk::ResponseType::DeleteEvent));
        assert!(!should_apply_response(gtk::ResponseType::Cancel));
    }
}
