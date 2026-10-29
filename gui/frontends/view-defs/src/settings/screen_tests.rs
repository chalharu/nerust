use std::{cell::RefCell, rc::Rc, sync::Arc};

use nerust_core_traits::audio::AudioBackendRegistry;
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::registry::SystemRegistry;
use nerust_settings_core::{
    i18n::{UiText, text},
    model::language::AppLanguage,
    model::{
        app_state::DesktopAppState,
        input::ShortcutAction,
        local::{HostBackendLocalSettings, NotificationPosition},
        shared::{DesktopSharedSettings, StoragePolicy},
    },
};
use nerust_system_traits::factory::descriptor::SystemSettingsFieldId;
use wakui::control::{
    Container, Control, ControlInput, ControlScreen, ControlVisitor, GroupPolicy, Tab, policy,
};
use wakui::leaf::{
    ActionLeaf, BindingLeaf, ChoiceLeaf, NoteLeaf, NumberLeaf, TextLeaf, ToggleLeaf,
};

use super::*;
use crate::settings::{InputPageSection, InputTab, SettingsEvent as E};

type Id = SettingsControlId;
type I = ControlInput;

fn state(nes: bool) -> SettingsController {
    let mut shared = DesktopSharedSettings::default();
    shared.input.shortcuts.keyboard =
        nerust_settings_core::model::input::default_shortcut_bindings();
    SettingsController::new(
        &SettingsSnapshot {
            shared,
            local: HostBackendLocalSettings::default(),
            app_state: DesktopAppState::default(),
        },
        Arc::new(SystemRegistry::new(if nes {
            vec![nerust_nes_factory::NesFactory::boxed().into()]
        } else {
            vec![]
        })),
        Arc::new(AudioBackendRegistry::new()),
    )
}

fn token(controller: &SettingsController, key: Id) -> ControlId {
    controller.ids.borrow_mut().id(StableKey::Control(key))
}

fn prepare(controller: &mut SettingsController) {
    controller
        .vm
        .general
        .set_language(AppLanguage::English)
        .unwrap();
    controller
        .vm
        .general
        .set_storage_policy(StoragePolicy::CustomDirectory)
        .unwrap();
    controller.vm.notifications.set_info_enabled(true).unwrap();
    controller
        .vm
        .notifications
        .set_warning_enabled(true)
        .unwrap();
    controller.vm.notifications.set_error_enabled(true).unwrap();
    controller.vm.notifications.set_error_modal(false).unwrap();
    controller
        .vm
        .notifications
        .set_status_enabled(true)
        .unwrap();
    controller
        .vm
        .notifications
        .set_position(NotificationPosition::TopLeft)
        .unwrap();
    controller
        .vm
        .notifications
        .set_status_position(NotificationPosition::BottomLeft)
        .unwrap();
    controller.update(E::SetStorageDirectory("/tmp".into()));
}

fn events(actual: Option<Vec<E>>, expected: Vec<E>) {
    // SettingsEvent intentionally predates this model and has no PartialEq.
    assert_eq!(
        format!("{:?}", actual.expect("valid binding")),
        format!("{expected:?}")
    );
}

fn apply(controller: &mut SettingsController, key: &Id, input: I) {
    let id = token(controller, key.clone());
    for event in controller
        .screen()
        .bind(&id, input.clone())
        .unwrap_or_else(|| panic!("rejected {key:?}: {input:?}"))
    {
        controller.update(event);
    }
    assert!(
        controller.error_message.is_none(),
        "{:?}",
        controller.error_message
    );
}

#[derive(Default)]
struct Inventory {
    controls: Vec<Control>,
    groups: Vec<ControlId>,
    tabs: Vec<Tab>,
}

impl ControlVisitor for Inventory {
    fn begin_page(&mut self, page: &wakui::control::Page) {
        self.controls.push(page.select.clone());
    }
    fn begin_container(&mut self, container: &Container) {
        if policy::<GroupPolicy>(container).is_some() {
            self.groups.push(container.id);
        }
    }
    fn begin_tab(&mut self, tab: &Tab) {
        self.controls.push(tab.select.clone());
        self.tabs.push(tab.clone());
    }
    fn control(&mut self, control: &Control) {
        self.controls.push(control.clone());
    }
}

fn inventory(view: &SettingsScreen) -> Inventory {
    let mut result = Inventory::default();
    view.screen.visit(&mut result);
    result
}

fn control_for(view: &SettingsScreen, key: &Id, controller: &SettingsController) -> Control {
    let id = token(controller, key.clone());
    view.screen.control(id).unwrap().clone()
}

#[test]
fn binding_table_covers_tree_controls_exactly_once() {
    // The refactor relies on table/tree 1:1 correspondence: validation walks
    // the tree while events come from the table. Duplicates would mix them.
    for nes in [false, true] {
        let mut controller = state(nes);
        prepare(&mut controller);
        let view = controller.screen();
        let tree: Vec<ControlId> = inventory(&view).controls.iter().map(|c| c.id).collect();
        let mut table: Vec<ControlId> = Vec::new();
        view.bindings
            .0
            .iter()
            .for_each(|(id, _, _)| table.push(*id));
        assert_eq!(table.len(), tree.len(), "table/tree size mismatch");
        for id in &tree {
            assert_eq!(
                table.iter().filter(|entry| *entry == id).count(),
                1,
                "binding for {id:?}"
            );
        }
    }
}

#[test]
fn six_pages_have_complete_unique_inventory_and_shared_groups() {
    let mut controller = state(false);
    prepare(&mut controller);
    let view = controller.screen();
    assert_eq!(view.screen.pages.len(), 6);
    let selected = view.screen.pages.iter().filter(|p| p.selected).count();
    assert_eq!(selected, 1);
    let found = inventory(&view);
    for (index, c) in found.controls.iter().enumerate() {
        assert!(
            found.controls[..index].iter().all(|other| other.id != c.id),
            "duplicate {:?}",
            c.id
        );
        assert!(view.screen.control(c.id).is_some());
    }
    // Every expected domain key resolves to a live control.
    let mut expected = vec![
        Id::Language,
        Id::StoragePolicy,
        Id::StorageDirectory,
        Id::StorageError,
        Id::Fullscreen,
        Id::Scaling,
        Id::Vsync,
        Id::Mute,
        Id::Volume,
        Id::SampleRate,
        Id::Latency,
        Id::InfoEnabled,
        Id::InfoDuration,
        Id::InfoColor,
        Id::WarningEnabled,
        Id::WarningDuration,
        Id::WarningColor,
        Id::ErrorEnabled,
        Id::ErrorPresentation,
        Id::ErrorColor,
        Id::StatusEnabled,
        Id::Position,
        Id::StatusPosition,
        Id::AreaMode,
        Id::StatusAreaMode,
        Id::StatusAreaFollows,
        Id::NoSystems,
        Id::Cancel,
        Id::Submit,
        Id::Tab(SettingsTabId::Shortcuts),
    ];
    expected.extend(
        controller
            .vm
            .shortcuts
            .view
            .get()
            .rows
            .iter()
            .map(|r| Id::Binding(r.target.clone())),
    );
    for key in expected {
        let id = token(&controller, key.clone());
        assert!(view.screen.control(id).is_some(), "missing {key:?}");
    }
    // Pages resolve through their select controls.
    for page in 0..6 {
        let id = view.screen.pages[page].select.id;
        assert!(view.screen.control(id).is_some());
    }
    assert_eq!(found.groups.len(), 5);
    assert!(controller.validation_banner().can_submit);
}

#[test]
fn all_static_bindings_emit_the_exact_typed_event() {
    let mut controller = state(false);
    prepare(&mut controller);
    let g = controller.vm.general.view.get();
    let v = controller.vm.video.view.get();
    let a = controller.vm.audio.view.get();
    let n = controller.vm.notifications.view.get();
    let cases = vec![
        (
            Id::Language,
            I::Select(0),
            E::SetLanguage(g.language_choices[0].clone()),
        ),
        (
            Id::StoragePolicy,
            I::Select(0),
            E::SetStoragePolicy(g.storage_policy_choices[0].clone()),
        ),
        (
            Id::StorageDirectory,
            I::Text("/chosen".into()),
            E::SetStorageDirectory("/chosen".into()),
        ),
        (Id::StorageDirectory, I::Activate, E::BrowseStorageDirectory),
        (
            Id::Fullscreen,
            I::Bool(true),
            E::ToggleFullscreenDefault(true),
        ),
        (
            Id::Scaling,
            I::Select(1),
            E::SetScaling(v.scaling_choices[1].clone()),
        ),
        (Id::Vsync, I::Bool(false), E::ToggleVsync(false)),
        (Id::Mute, I::Bool(true), E::ToggleMute(true)),
        (Id::Volume, I::Number(42), E::SetVolume(42)),
        (
            Id::SampleRate,
            I::Select(1),
            E::SetSampleRate(a.sample_rate_choices[1].clone()),
        ),
        (Id::Latency, I::Number(99), E::SetLatency(99)),
        (Id::InfoEnabled, I::Bool(false), E::ToggleInfoEnabled(false)),
        (Id::InfoDuration, I::Number(2345), E::SetInfoDuration(2345)),
        (
            Id::InfoColor,
            I::Select(1),
            E::SetInfoColor(n.color_choices[1].clone()),
        ),
        (
            Id::WarningEnabled,
            I::Bool(false),
            E::ToggleWarningEnabled(false),
        ),
        (
            Id::WarningDuration,
            I::Number(4567),
            E::SetWarningDuration(4567),
        ),
        (
            Id::WarningColor,
            I::Select(2),
            E::SetWarningColor(n.color_choices[2].clone()),
        ),
        (
            Id::ErrorEnabled,
            I::Bool(false),
            E::ToggleErrorEnabled(false),
        ),
        (
            Id::ErrorPresentation,
            I::Select(0),
            E::ToggleErrorModal(true),
        ),
        (
            Id::ErrorPresentation,
            I::Select(1),
            E::ToggleErrorModal(false),
        ),
        (
            Id::ErrorColor,
            I::Select(3),
            E::SetErrorColor(n.color_choices[3].clone()),
        ),
        (
            Id::StatusEnabled,
            I::Bool(false),
            E::ToggleStatusEnabled(false),
        ),
        (
            Id::Position,
            I::Select(1),
            E::SetPosition(n.position_choices[1].clone()),
        ),
        (
            Id::StatusPosition,
            I::Select(2),
            E::SetStatusPosition(n.position_choices[2].clone()),
        ),
        (
            Id::AreaMode,
            I::Select(1),
            E::SetAreaMode(n.area_mode_choices[1].clone()),
        ),
        (
            Id::StatusAreaMode,
            I::Select(0),
            E::SetStatusAreaMode(n.area_mode_choices[0].clone()),
        ),
        (Id::Cancel, I::Activate, E::Cancel),
        (Id::Submit, I::Activate, E::Submit),
        (
            Id::Tab(SettingsTabId::Shortcuts),
            I::Activate,
            E::SelectInputTab(0),
        ),
    ];
    for (key, input, event) in cases {
        let view = controller.screen();
        let id = token(&controller, key.clone());
        events(view.bind(&id, input), vec![event]);
    }
    {
        let view = controller.screen();
        for page in &view.screen.pages {
            events(
                view.bind(&page.select.id, I::Activate),
                vec![E::SelectPage(token_page(&controller, page.select.id))],
            );
        }
    }
    for row in controller.vm.shortcuts.view.get().rows.clone() {
        let view = controller.screen();
        let id = token(&controller, Id::Binding(row.target.clone()));
        events(
            view.bind(&id, I::Activate),
            vec![E::StartCapture(row.target.clone())],
        );
        events(
            view.bind(&id, I::CaptureClear),
            vec![E::ClearCapture(row.target.clone())],
        );
        assert!(view.bind(&id, I::CaptureKey(Key::Space)).is_none());
    }
    assert_eq!(
        controller.vm.audio.view.get(),
        a,
        "building/binding never mutates VM"
    );
    apply(&mut controller, &Id::Volume, I::Number(42));
    assert_eq!(controller.vm.audio.view.get().volume_percent, 42);
    apply(&mut controller, &Id::InfoDuration, I::Number(2345));
    assert_eq!(
        controller.vm.notifications.view.get().info_duration_ms,
        2345
    );
}

/// Map a page-select control back to its page. Selects are minted from
/// page keys, so the mapping is the token helper in reverse.
fn token_page(controller: &SettingsController, select: ControlId) -> SettingsPage {
    for page in [
        SettingsPage::General,
        SettingsPage::Input,
        SettingsPage::Video,
        SettingsPage::Audio,
        SettingsPage::Notifications,
        SettingsPage::System,
    ] {
        if token(controller, Id::Page(page)) == select {
            return page;
        }
    }
    panic!("unknown page select");
}

#[test]
fn all_controls_reject_wrong_kind_bad_indices_and_numeric_overflow() {
    let mut controller = state(true);
    prepare(&mut controller);
    let view = controller.screen();
    for control in inventory(&view).controls {
        assert!(view.bind(&control.id, I::CaptureKey(Key::Space)).is_none());
        if control.downcast::<ToggleLeaf>().is_some() {
            assert!(view.bind(&control.id, I::Select(0)).is_none());
        } else if let Some(leaf) = control.downcast::<ChoiceLeaf>() {
            assert!(view.bind(&control.id, I::Bool(true)).is_none());
            assert!(
                view.bind(&control.id, I::Select(leaf.choices.len()))
                    .is_none()
            );
            assert!(view.bind(&control.id, I::Select(usize::MAX)).is_none());
        } else if let Some(leaf) = control.downcast::<NumberLeaf>() {
            assert!(view.bind(&control.id, I::Text("12".into())).is_none());
            assert!(view.bind(&control.id, I::Number(leaf.max + 1)).is_none());
            assert!(view.bind(&control.id, I::Number(u32::MAX)).is_none());
            if leaf.min > 0 {
                assert!(view.bind(&control.id, I::Number(leaf.min - 1)).is_none());
            }
            assert!(view.bind(&control.id, I::Number(leaf.min)).is_some());
            assert!(view.bind(&control.id, I::Number(leaf.max)).is_some());
        } else if control.downcast::<TextLeaf>().is_some() {
            assert!(view.bind(&control.id, I::Number(0)).is_none());
        } else if control.downcast::<BindingLeaf>().is_some()
            || control.downcast::<ActionLeaf>().is_some()
        {
            assert!(view.bind(&control.id, I::Bool(true)).is_none());
        } else if control.downcast::<NoteLeaf>().is_some() {
            assert!(view.bind(&control.id, I::Activate).is_none());
        } else {
            panic!("uncovered leaf");
        }
    }
    assert!(
        controller
            .screen()
            .bind(
                &token(
                    &controller,
                    Id::SystemField {
                        system: SettingsSystemId("unknown".into()),
                        field: SystemSettingsFieldId("unknown".into()),
                    },
                ),
                I::Select(0),
            )
            .is_none()
    );
}

#[test]
fn dynamic_nes_descriptors_slots_and_sections_have_context_and_stable_identity() {
    let mut controller = state(true);
    let view = controller.screen();
    let input = controller.vm.inputs()[0].view.get();
    let system = controller.vm.systems()[0].view.get();
    let key = SettingsSystemId(system.system_id.to_string());
    assert!(!system.fields.is_empty());
    assert!(!input.sections.is_empty());
    for field in &system.fields {
        let id = Id::SystemField {
            system: key.clone(),
            field: field.id.clone(),
        };
        for (index, choice) in field.choices.iter().enumerate() {
            events(
                view.bind(&token(&controller, id.clone()), I::Select(index)),
                vec![
                    E::SelectSystemTab(0),
                    E::SetSystemChoice(field.id.as_str().into(), choice.clone()),
                ],
            );
        }
        controller.system_tab_index = Some(999);
        let index = field
            .choices
            .iter()
            .position(|c| c.value != field.selected)
            .unwrap();
        apply(&mut controller, &id, I::Select(index));
        assert_eq!(controller.system_tab_index, Some(0));
        assert_eq!(
            controller.vm.systems()[0]
                .view
                .get()
                .fields
                .iter()
                .find(|f| f.id == field.id)
                .unwrap()
                .selected,
            field.choices[index].value
        );
    }
    for slot in &input.slots {
        let id = Id::ControllerSlot {
            system: key.clone(),
            slot: slot.slot_id,
        };
        let c = control_for(&controller.screen(), &id, &controller);
        let leaf = c.downcast::<ChoiceLeaf>().unwrap();
        assert_eq!(
            leaf.selected,
            slot.choices
                .iter()
                .position(|c| c.value == slot.selected_profile_id)
        );
        for (index, choice) in slot.choices.iter().enumerate() {
            if slot.occupied_by_other_slot {
                assert!(
                    controller
                        .screen()
                        .bind(&token(&controller, id.clone()), I::Select(index))
                        .is_none()
                );
            } else {
                events(
                    controller
                        .screen()
                        .bind(&token(&controller, id.clone()), I::Select(index)),
                    vec![
                        E::SelectInputTab(0),
                        E::SetControllerSlot {
                            slot: slot.slot_id,
                            controller_id: choice.value.clone(),
                        },
                    ],
                );
            }
        }
    }
    let slot = &input.slots[0];
    let id = Id::ControllerSlot {
        system: key.clone(),
        slot: slot.slot_id,
    };
    let none = slot.choices.iter().position(|c| c.value.is_none()).unwrap();
    controller.input_tab = Some(InputTab::Shortcuts);
    apply(&mut controller, &id, I::Select(none));
    let refreshed = controller.screen();
    let selected = refreshed
        .screen
        .control(token(&controller, id.clone()))
        .unwrap()
        .downcast::<ChoiceLeaf>()
        .unwrap()
        .selected;
    assert_eq!(selected, Some(none), "selected None is not no selection");
    assert!(
        controller.vm.inputs()[0].view.get().slots[0]
            .selected_profile_id
            .is_none()
    );
    // Tab selects are minted from domain tab keys; map them back through
    // the same registry to recover the expected navigation events.
    // Candidates come from the original snapshots: later applies only
    // change selections, never the section structure under test.
    let mut candidates: Vec<(SettingsTabId, Vec<E>)> = Vec::new();
    candidates.push((
        SettingsTabId::System(key.clone()),
        vec![E::SelectSystemTab(0)],
    ));
    candidates.push((
        SettingsTabId::Input(key.clone()),
        vec![E::SelectInputTab(0)],
    ));
    for (section_index, section) in input.sections.iter().enumerate() {
        let domain = section
            .rows
            .iter()
            .find_map(|r| match &r.target {
                CaptureTarget::Binding { attachment, .. } => Some(SettingsTabId::Attachment {
                    system: key.clone(),
                    attachment: attachment.clone(),
                }),
                _ => None,
            })
            .unwrap_or_else(|| SettingsTabId::EmptyAttachment {
                system: key.clone(),
                index: section_index,
            });
        candidates.push((
            domain,
            vec![
                E::SelectInputTab(0),
                E::SelectInputSection(InputPageSection::Attachment(section_index)),
            ],
        ));
    }
    candidates.push((SettingsTabId::Shortcuts, vec![E::SelectInputTab(1)]));
    let inv = inventory(&view);
    for tab in inv.tabs {
        // Plain tabs resolve through the registry; empty-attachment tabs
        // use content keys minted at build, so match those by label.
        let expected = candidates
            .iter()
            .filter(|(domain, _)| !matches!(domain, SettingsTabId::EmptyAttachment { .. }))
            .find(|(domain, _)| token(&controller, Id::Tab(domain.clone())) == tab.select.id)
            .map(|(_, events)| events.clone())
            .or_else(|| {
                let sections: Vec<(usize, usize, String)> = controller
                    .vm
                    .inputs()
                    .iter()
                    .enumerate()
                    .flat_map(|(index, vm)| {
                        vm.view
                            .get()
                            .sections
                            .iter()
                            .enumerate()
                            .map(|(section_index, section)| {
                                (index, section_index, section.label.clone())
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect();
                sections
                    .into_iter()
                    .find(|(_, _, label)| *label == tab.label)
                    .map(|(index, section_index, _)| {
                        vec![
                            E::SelectInputTab(index),
                            E::SelectInputSection(InputPageSection::Attachment(section_index)),
                        ]
                    })
            })
            .unwrap_or_else(|| panic!("unknown tab {:?}", tab.label));
        events(view.bind(&tab.select.id, I::Activate), expected);
    }
    // Row binds resolve against the original projection: later applies
    // only change selections, and the snapshot still owns its bindings.
    for row in input.sections.iter().flat_map(|s| &s.rows) {
        let id = Id::Binding(row.target.clone());
        events(
            view.bind(&token(&controller, id.clone()), I::Activate),
            vec![E::StartCapture(row.target.clone())],
        );
        events(
            view.bind(&token(&controller, id), I::CaptureClear),
            vec![E::ClearCapture(row.target.clone())],
        );
    }
}

#[test]
fn occupied_multiport_controller_is_disabled_and_topology_changes_are_scoped() {
    let controller = state(true);
    let initial = controller.screen();
    let original_ids = inventory(&initial)
        .controls
        .into_iter()
        .map(|c| c.id)
        .collect::<Vec<_>>();
    let view = controller.vm.inputs()[0].view.get();
    let mut found = false;
    'slots: for slot in &view.slots {
        for profile in slot.choices.iter().filter_map(|c| c.value.as_deref()) {
            controller.vm.inputs()[0]
                .set_controller_slot(slot.slot_id, Some(profile))
                .unwrap();
            let current = controller.vm.inputs()[0].view.get();
            if let Some(occupied) = current.slots.iter().find(|s| s.occupied_by_other_slot) {
                let id = Id::ControllerSlot {
                    system: SettingsSystemId(current.system_id.to_string()),
                    slot: occupied.slot_id,
                };
                let screen = controller.screen();
                let c = screen
                    .screen
                    .control(token(&controller, id.clone()))
                    .unwrap();
                assert!(!c.metadata.enabled);
                assert!(c.metadata.help.is_some());
                assert!(
                    screen
                        .bind(&token(&controller, id.clone()), I::Select(0))
                        .is_none()
                );
                for token_id in original_ids.iter() {
                    assert!(
                        screen.screen.control(*token_id).is_some(),
                        "unrelated identity changed: {token_id:?}"
                    );
                }
                found = true;
                break 'slots;
            }
        }
    }
    assert!(
        found,
        "actual NES registry must exercise multiport occupancy"
    );
}

#[test]
fn host_owns_enabled_visibility_validation_and_native_error_policy() {
    let mut controller = state(false);
    prepare(&mut controller);
    for (toggle, child, input) in [
        (Id::InfoEnabled, Id::InfoDuration, I::Number(2000)),
        (Id::WarningEnabled, Id::WarningColor, I::Select(0)),
        (Id::ErrorEnabled, Id::ErrorPresentation, I::Select(0)),
        (Id::StatusEnabled, Id::StatusPosition, I::Select(0)),
    ] {
        apply(&mut controller, &toggle, I::Bool(false));
        assert!(
            controller
                .screen()
                .bind(&token(&controller, child.clone()), input)
                .is_none()
        );
        apply(&mut controller, &toggle, I::Bool(true));
    }
    apply(&mut controller, &Id::ErrorPresentation, I::Select(0));
    assert!(
        !controller
            .screen()
            .screen
            .control(token(&controller, Id::ErrorColor))
            .unwrap()
            .metadata
            .visible
    );
    assert!(
        controller
            .screen()
            .bind(&token(&controller, Id::ErrorColor), I::Select(0))
            .is_none()
    );
    controller
        .vm
        .notifications
        .set_status_position(NotificationPosition::TopRight)
        .unwrap();
    let screen = controller.screen();
    assert!(
        screen
            .screen
            .control(token(&controller, Id::StatusAreaFollows))
            .unwrap()
            .metadata
            .visible
    );
    assert!(
        screen
            .bind(&token(&controller, Id::StatusAreaMode), I::Select(0))
            .is_none()
    );
    controller.update(E::SetStorageDirectory(String::new()));
    controller.error_message = Some("native apply failed".into());
    let screen = controller.screen();
    assert!(!controller.validation_banner().can_submit);
    assert!(controller.validation_banner().storage.is_some());
    assert!(controller.validation_banner().headline.is_some());
    assert!(
        screen
            .bind(&token(&controller, Id::Submit), I::Activate)
            .is_none()
    );
    assert_eq!(
        screen.screen.message.as_deref(),
        Some("native apply failed")
    );
    assert_eq!(controller.error_message, screen.screen.message);
    assert!(
        screen
            .bind(&token(&controller, Id::Cancel), I::Activate)
            .is_some()
    );
    controller
        .vm
        .general
        .set_storage_policy(StoragePolicy::Sidecar)
        .unwrap();
    let screen = controller.screen();
    assert!(
        !screen
            .screen
            .control(token(&controller, Id::StorageDirectory))
            .unwrap()
            .metadata
            .visible
    );
    assert!(
        screen
            .bind(
                &token(&controller, Id::StorageDirectory),
                I::Text("ignored".into())
            )
            .is_none()
    );
    assert!(
        screen
            .bind(&token(&controller, Id::StorageDirectory), I::Activate)
            .is_none()
    );
}

#[test]
fn empty_registry_shortcuts_capture_and_localized_refresh_are_owned() {
    let mut controller = state(false);
    controller
        .vm
        .general
        .set_language(AppLanguage::English)
        .unwrap();
    let before = controller.screen();
    let rev = controller.vm.revision.get();
    let _ = controller.screen();
    assert_eq!(controller.vm.revision.get(), rev);
    assert_eq!(controller.input_tab, None);
    let inv = inventory(&before);
    assert_eq!(inv.tabs.len(), 1);
    assert!(inv.tabs[0].selected);
    assert_eq!(
        inv.tabs[0].select.id,
        token(&controller, Id::Tab(SettingsTabId::Shortcuts))
    );
    assert_eq!(controller.vm.shortcuts.view.get().rows.len(), 8);
    let target = CaptureTarget::Shortcut(ShortcutAction::TogglePause);
    let id = Id::Binding(target.clone());
    let token_id = token(&controller, id.clone());
    assert!(before.bind(&token_id, I::Activate).is_some());
    assert!(before.bind_capture_key(Key::Space).is_none());
    apply(&mut controller, &id, I::Activate);
    let capture = controller.screen();
    assert_eq!(
        capture.screen.capture.target,
        Some(token(&controller, id.clone()))
    );
    assert_eq!(
        capture.screen.capture.prompt,
        text(AppLanguage::English, UiText::CapturePrompt)
    );
    let bound = capture
        .screen
        .control(token(&controller, id.clone()))
        .unwrap();
    let leaf = bound.downcast::<BindingLeaf>().unwrap();
    assert!(leaf.capturing);
    events(
        capture.bind(&token(&controller, id.clone()), I::CaptureKey(Key::Space)),
        vec![E::CaptureKey(Key::Space)],
    );
    for e in capture.bind_capture_key(Key::Space).unwrap() {
        controller.update(e);
    }
    assert!(controller.screen().screen.capture.target.is_none());
    let bound_screen = controller.screen();
    let leaf = bound_screen
        .screen
        .control(token(&controller, id.clone()))
        .unwrap()
        .downcast::<BindingLeaf>()
        .unwrap();
    assert!(!leaf.capturing);
    assert!(leaf.text.contains("Space"));
    apply(&mut controller, &id, I::CaptureClear);
    controller
        .vm
        .general
        .set_language(AppLanguage::Japanese)
        .unwrap();
    controller.vm.audio.set_volume(37).unwrap();
    let after = controller.screen();
    assert_eq!(
        after.screen.title,
        text(AppLanguage::Japanese, UiText::Preferences)
    );
    for (before_page, after_page) in before.screen.pages.iter().zip(after.screen.pages.iter()) {
        assert_ne!(before_page.label, after_page.label);
    }
    assert_eq!(
        after
            .screen
            .control(token(&controller, Id::Cancel))
            .unwrap()
            .label,
        "キャンセル"
    );
    let dir = after
        .screen
        .control(token(&controller, Id::StorageDirectory))
        .unwrap();
    let text_leaf = dir.downcast::<TextLeaf>().unwrap();
    assert_eq!(text_leaf.value, String::new());
    assert_eq!(text_leaf.placeholder, String::new());
    assert_eq!(text_leaf.browse_label.as_deref(), Some("参照"));
    let volume = after
        .screen
        .control(token(&controller, Id::Volume))
        .unwrap();
    assert_eq!(volume.downcast::<NumberLeaf>().unwrap().value, 37);
    let binding_leaf = after
        .screen
        .control(token(&controller, id.clone()))
        .unwrap()
        .downcast::<BindingLeaf>()
        .unwrap();
    assert!(!binding_leaf.capturing);
    assert_eq!(binding_leaf.text, "未割当");
    assert_eq!(binding_leaf.change_label, "変更");
    assert_eq!(binding_leaf.clear_label, "解除");
    assert_eq!(
        inventory(&before)
            .controls
            .iter()
            .map(|c| c.id)
            .collect::<Vec<_>>(),
        inventory(&after)
            .controls
            .iter()
            .map(|c| c.id)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        before
            .screen
            .control(token(&controller, Id::Cancel))
            .unwrap()
            .label,
        "Cancel",
        "old snapshot remains owned"
    );
}

#[test]
fn object_safe_visitor_is_balanced_and_reenters_without_controller_borrows() {
    struct Visitor {
        controller: Rc<RefCell<SettingsController>>,
        stack: Vec<String>,
        visited: usize,
    }
    impl Visitor {
        fn push(&mut self, kind: &str, id: impl std::fmt::Debug) {
            self.stack.push(format!("{kind}:{id:?}"));
        }
        fn pop(&mut self, kind: &str, id: impl std::fmt::Debug) {
            assert_eq!(self.stack.pop().unwrap(), format!("{kind}:{id:?}"));
        }
    }
    impl ControlVisitor for Visitor {
        fn begin_screen(&mut self, _: &ControlScreen) {
            self.push("screen", ());
        }
        fn end_screen(&mut self, _: &ControlScreen) {
            self.pop("screen", ());
        }
        fn begin_page(&mut self, p: &wakui::control::Page) {
            self.push("page", p.id);
        }
        fn end_page(&mut self, p: &wakui::control::Page) {
            self.pop("page", p.id);
        }
        fn begin_container(&mut self, c: &Container) {
            self.push("container", c.id);
        }
        fn end_container(&mut self, c: &Container) {
            self.pop("container", c.id);
        }
        fn begin_tabs(&mut self, t: &wakui::control::Tabs) {
            self.push("tabs", t.id);
        }
        fn end_tabs(&mut self, t: &wakui::control::Tabs) {
            self.pop("tabs", t.id);
        }
        fn begin_tab(&mut self, t: &Tab) {
            self.push("tab", t.id);
        }
        fn end_tab(&mut self, t: &Tab) {
            self.pop("tab", t.id);
        }
        fn control(&mut self, _: &Control) {
            self.visited += 1;
            if self.visited == 1 {
                self.controller.borrow_mut().update(E::SetVolume(12));
                let nested = self.controller.borrow().screen();
                assert!(!inventory(&nested).controls.is_empty());
            }
        }
    }
    let controller = Rc::new(RefCell::new(state(true)));
    let view = controller.borrow().screen();
    let mut visitor = Visitor {
        controller: controller.clone(),
        stack: vec![],
        visited: 0,
    };
    let dynamic: &mut dyn ControlVisitor = &mut visitor;
    view.screen.visit(dynamic);
    assert!(visitor.stack.is_empty());
    assert!(visitor.visited > 30);
    assert_eq!(controller.borrow().vm.audio.view.get().volume_percent, 12);
    let volume = token(&controller.borrow(), Id::Volume);
    assert_eq!(
        view.screen
            .control(volume)
            .unwrap()
            .downcast::<NumberLeaf>()
            .unwrap()
            .value,
        100
    );
}
