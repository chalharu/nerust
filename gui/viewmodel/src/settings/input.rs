use std::{rc::Rc, sync::Arc};

use nerust_core_traits::{factory::CoreFactory, identity::SystemId};
use nerust_gui_shell::{
    session::input::{build_topology, clear_multi_port_conflicts},
    settings::{
        bindings::{conflicting_keys, descriptors::keyboard_binding_sections},
        editor::{CaptureTarget, current_binding_label},
        i18n::{UiText, text as ui_text},
    },
};
use nerust_input_traits::{AttachmentId, ControllerProfile, InputTopologyDescriptor};

use super::{
    EditorState,
    dto::{
        BindingRowView, BindingSectionView, BindingValueView, ChoiceView, ControllerSlotView,
        InputConflictView, InputTabView,
    },
    editor::{SettingsEditor, ViewModelError},
    property::ReadOnlyObservableProperty,
};

/// Resolve persisted (slot_id, controller_id) pairs into resolved
/// (AttachmentId, Option<Rc<dyn ControllerProfile>>) assignments.
fn resolve_assignments(
    pairs: &[(String, Option<String>)],
    input_factory: &dyn nerust_input_traits::InputSystemFactory,
) -> Vec<(AttachmentId, Option<Rc<dyn ControllerProfile>>)> {
    let mut assignments: Vec<(AttachmentId, Option<Rc<dyn ControllerProfile>>)> = pairs
        .iter()
        .filter_map(|(slot_id, _)| {
            let att = input_factory.resolve_slot(slot_id)?;
            Some((att, None::<Rc<dyn ControllerProfile>>))
        })
        .collect();
    for (slot_id, ctrl_opt) in pairs {
        if let Some(id) = ctrl_opt
            && let Some(profile) = input_factory.resolve_controller(id)
            && let Some(entry) = assignments
                .iter_mut()
                .find(|(s, _)| s.to_string() == *slot_id)
        {
            entry.1 = Some(profile);
        }
    }
    assignments
}

fn persisted_pairs(
    draft: &super::EditorState,
    factory: &dyn CoreFactory,
) -> Vec<(String, Option<String>)> {
    let sid = factory.system_id();
    let input_factory = factory.input_system_factory();
    draft
        .draft
        .app_state
        .controller_assignments
        .get(&sid)
        .cloned()
        .unwrap_or_else(|| {
            let defaults = input_factory.default_assignments();
            defaults
                .slots
                .iter()
                .map(|(slot_id, ctrl)| {
                    (
                        slot_id.to_string(),
                        ctrl.as_ref().map(|p| p.profile_id().to_string()),
                    )
                })
                .collect()
        })
}

/// Per-system input settings view model.
#[derive(Clone)]
pub struct InputSettingsViewModel {
    editor: SettingsEditor,
    factory_id: Box<dyn SystemId>,
    display_name: &'static str,
    pub view: ReadOnlyObservableProperty<InputTabView>,
}

impl InputSettingsViewModel {
    pub fn new(editor: &SettingsEditor, factory: &Arc<dyn CoreFactory>) -> Self {
        let factory_id = factory.system_id();
        let display_name = factory.display_name();
        let current = editor.current();
        let initial = project_view(&current, factory.as_ref(), &current.capture_target);
        drop(current);
        let view = editor
            .projections()
            .register(factory.display_name(), initial, {
                let factory = Arc::clone(factory);
                move |state| project_view(state, factory.as_ref(), &state.capture_target)
            });
        Self {
            editor: editor.clone(),
            factory_id,
            display_name,
            view,
        }
    }

    pub fn system_id(&self) -> &dyn SystemId {
        self.factory_id.as_ref()
    }

    pub fn display_name(&self) -> &'static str {
        self.display_name
    }

    pub fn set_controller_slot(
        &self,
        slot: AttachmentId,
        profile_id: Option<&str>,
    ) -> Result<(), ViewModelError> {
        let factory_id = self.factory_id.clone_box();
        let profile_id = profile_id.map(|s| s.to_string());
        self.editor.transact(move |state| {
            let factory = state
                .registry
                .find_by_id(factory_id.as_ref())
                .cloned()
                .ok_or(ViewModelError::UnknownSystem(factory_id.to_string()))?;
            let input_factory = factory.input_system_factory();

            // Resolve the requested profile
            let profile: Option<Rc<dyn ControllerProfile>> = match &profile_id {
                Some(id) => Some(
                    input_factory
                        .resolve_controller(id)
                        .ok_or_else(|| ViewModelError::UnknownController(id.clone()))?,
                ),
                None => None,
            };

            // Validate the target slot exists
            input_factory
                .resolve_slot(slot.as_str())
                .ok_or(ViewModelError::UnknownSlot(slot.to_string()))?;

            let mut assignments =
                resolve_assignments(&persisted_pairs(state, factory.as_ref()), input_factory);

            // Ensure the target slot is present in assignments
            if !assignments.iter().any(|(s, _)| *s == slot) {
                assignments.push((slot, None));
            }

            // Apply the new slot (with multi-port conflict resolution)
            if let Some(ref p) = profile {
                clear_multi_port_conflicts(slot, p.as_ref(), &mut assignments);
            }
            if let Some(entry) = assignments.iter_mut().find(|(s, _)| *s == slot) {
                entry.1 = profile.clone();
            }

            // Convert back to string pairs and persist
            let pairs: Vec<(String, Option<String>)> = assignments
                .iter()
                .map(|(s, c)| {
                    (
                        s.to_string(),
                        c.as_ref().map(|p| p.profile_id().to_string()),
                    )
                })
                .collect();
            state
                .draft
                .app_state
                .controller_assignments
                .insert(factory.system_id(), pairs);
            Ok(())
        })
    }
}

fn project_view(
    state: &EditorState,
    factory: &dyn CoreFactory,
    capture_target: &Option<CaptureTarget>,
) -> InputTabView {
    let system_id = factory.system_id();
    let input_factory = factory.input_system_factory();
    let slots_descs = input_factory.slots();
    let controllers = input_factory.controllers();
    let language = state.draft.shared.general.language;

    let assignments = resolve_assignments(&persisted_pairs(state, factory), input_factory);

    // Build set of occupied slots (from multi-port controllers)
    let mut occupied = std::collections::HashSet::new();
    for (s, c_opt) in &assignments {
        let Some(profile) = c_opt else { continue };
        for ps in profile.port_sets() {
            if ps.ports.contains(s) {
                for &port in ps.ports.iter() {
                    occupied.insert(port);
                }
            }
        }
    }

    // Build slot views
    let slots: Vec<ControllerSlotView> = slots_descs
        .iter()
        .map(|slot_desc| {
            let profile_id = assignments
                .iter()
                .find(|(s, _)| *s == slot_desc.id)
                .and_then(|(_, c)| c.as_ref().map(|p| p.profile_id().to_string()));

            let mut choices: Vec<ChoiceView<Option<String>>> = vec![ChoiceView {
                value: None,
                label: ui_text(language, UiText::None).to_string(),
            }];

            choices.extend(
                controllers
                    .iter()
                    .filter(|c| {
                        c.port_sets()
                            .iter()
                            .any(|ps| ps.ports.first() == Some(&slot_desc.id))
                    })
                    .map(|c| ChoiceView {
                        value: Some(c.profile_id().to_string()),
                        label: c.label().to_string(),
                    }),
            );

            let occupied_by_other = occupied.contains(&slot_desc.id)
                && !assignments
                    .iter()
                    .any(|(s, c)| *s == slot_desc.id && c.is_some());

            ControllerSlotView {
                slot_id: slot_desc.id,
                label: slot_desc.label.to_string(),
                selected_profile_id: profile_id,
                choices,
                occupied_by_other_slot: occupied_by_other,
            }
        })
        .collect();

    // Build topology and key binding sections
    let topology: InputTopologyDescriptor = build_topology(&assignments, slots_descs);
    let sections: Vec<BindingSectionView> =
        keyboard_binding_sections(&topology, system_id.as_ref())
            .into_iter()
            .map(|section| {
                let rows: Vec<BindingRowView> = section
                    .bindings
                    .iter()
                    .map(|descriptor| {
                        let target = CaptureTarget::Binding {
                            system: descriptor.system.clone_box(),
                            attachment: descriptor.attachment.as_str().to_string(),
                            control: descriptor.control.as_str().to_string(),
                        };
                        let value = if capture_target.as_ref() == Some(&target) {
                            BindingValueView::Capturing(
                                ui_text(language, UiText::CapturePrompt).to_string(),
                            )
                        } else {
                            match current_binding_label(&state.draft, &target) {
                                Some(label) => BindingValueView::Bound(label.to_string()),
                                None => BindingValueView::Unbound(
                                    ui_text(language, UiText::Unbound).to_string(),
                                ),
                            }
                        };
                        BindingRowView {
                            target,
                            label: descriptor.control_label.to_string(),
                            value,
                        }
                    })
                    .collect();
                BindingSectionView {
                    label: section.attachment_label.to_string(),
                    rows,
                }
            })
            .collect();

    // Build conflict messages
    let conflicts: Vec<InputConflictView> =
        conflicting_keys(&state.draft.shared, &topology, system_id.as_ref())
            .into_iter()
            .map(|(key, labels)| InputConflictView {
                message: format!("{}: {}", key.label(), labels.join(", ")),
            })
            .collect();

    InputTabView {
        system_id: system_id.clone_box(),
        label: factory.display_name().to_string(),
        slots,
        sections,
        conflicts,
    }
}
