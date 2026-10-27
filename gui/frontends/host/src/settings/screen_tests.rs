use std::{cell::RefCell, rc::Rc, sync::Arc};

use nerust_core_traits::audio::AudioBackendRegistry;
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::registry::SystemRegistry;
use nerust_settings_core::{
    i18n::{UiText, text},
    model::{
        app_state::DesktopAppState,
        input::ShortcutAction,
        local::{HostBackendLocalSettings, NotificationPosition},
        shared::{DesktopSharedSettings, StoragePolicy},
    },
};

use super::*;
use crate::{
    binding::ViewDefinition,
    settings::{InputPageSection, InputTab, SettingsEvent as E},
};

type Id = SettingsControlId;
type Kind = SettingsControlKind;
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

fn apply(controller: &mut SettingsController, id: &Id, input: I) {
    for event in controller
        .screen()
        .bind(id, input.clone())
        .unwrap_or_else(|| panic!("rejected {id:?}: {input:?}"))
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
    controls: Vec<SettingsControl>,
    groups: Vec<SettingsLayoutId>,
    tabs: Vec<SettingsTab>,
}

impl SettingsVisitor for Inventory {
    fn begin_page(&mut self, page: &SettingsScreenPage) {
        self.controls.push(page.select.clone());
    }
    fn begin_tab(&mut self, tab: &SettingsTab) {
        self.controls.push(tab.select.clone());
        self.tabs.push(tab.clone());
    }
    fn begin_group(&mut self, group: &SettingsLayout) {
        self.groups.push(group.id.clone());
    }
    fn control(&mut self, control: &SettingsControl) {
        self.controls.push(control.clone());
    }
}

fn inventory(screen: &SettingsScreen) -> Inventory {
    let mut result = Inventory::default();
    screen.visit(&mut result);
    result
}

#[test]
fn binding_table_covers_tree_controls_exactly_once() {
    // The refactor relies on table/tree 1:1 correspondence: validation walks
    // the tree while events come from the table. Duplicates would mix them.
    for nes in [false, true] {
        let mut controller = state(nes);
        prepare(&mut controller);
        let screen = controller.screen();
        let tree: Vec<Id> = inventory(&screen)
            .controls
            .iter()
            .map(|c| c.id.clone())
            .collect();
        let mut table: Vec<Id> = Vec::new();
        screen
            .bindings
            .0
            .iter()
            .for_each(|(id, _, _)| table.push(id.clone()));
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
    let screen = controller.screen();
    assert_eq!(
        screen.pages.iter().map(|p| p.id).collect::<Vec<_>>(),
        [
            SettingsPage::General,
            SettingsPage::Input,
            SettingsPage::Video,
            SettingsPage::Audio,
            SettingsPage::Notifications,
            SettingsPage::System,
        ]
    );
    assert_eq!(screen.pages.iter().filter(|p| p.selected).count(), 1);
    let inventory = inventory(&screen);
    for (index, c) in inventory.controls.iter().enumerate() {
        assert!(
            inventory.controls[..index]
                .iter()
                .all(|other| other.id != c.id),
            "duplicate {:?}",
            c.id
        );
        assert!(screen.control(&c.id).is_some());
    }
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
    expected.extend(screen.pages.iter().map(|p| Id::Page(p.id)));
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
    assert_eq!(inventory.controls.len(), expected.len());
    for id in expected {
        assert!(screen.control(&id).is_some(), "missing {id:?}");
    }
    assert_eq!(
        inventory.groups,
        [
            SettingsLayoutId::Notification(NotificationGroup::Info),
            SettingsLayoutId::Notification(NotificationGroup::Warning),
            SettingsLayoutId::Notification(NotificationGroup::Error),
            SettingsLayoutId::Notification(NotificationGroup::Display),
            SettingsLayoutId::Notification(NotificationGroup::Status),
        ]
    );
    let SettingsNode::Group(info) = &screen.pages[4].body.children[0] else {
        panic!()
    };
    let SettingsNode::Group(warning) = &screen.pages[4].body.children[1] else {
        panic!()
    };
    assert_eq!(info.children.len(), warning.children.len());
    assert_eq!(info.indent, warning.indent);
    assert!(info.header.is_some() && warning.header.is_some());
    assert!(screen.validation.can_submit);
}

#[test]
fn all_static_bindings_emit_the_exact_typed_event() {
    let mut controller = state(false);
    prepare(&mut controller);
    let screen = controller.screen();
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
    for (id, input, event) in cases {
        events(screen.bind(&id, input), vec![event]);
    }
    for page in &screen.pages {
        events(
            screen.bind(&page.select.id, I::Activate),
            vec![E::SelectPage(page.id)],
        );
    }
    for row in &controller.vm.shortcuts.view.get().rows {
        let id = Id::Binding(row.target.clone());
        events(
            screen.bind(&id, I::Activate),
            vec![E::StartCapture(row.target.clone())],
        );
        events(
            screen.bind(&id, I::CaptureClear),
            vec![E::ClearCapture(row.target.clone())],
        );
        assert!(screen.bind(&id, I::CaptureKey(Key::Space)).is_none());
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

#[test]
fn all_controls_reject_wrong_kind_bad_indices_and_numeric_overflow() {
    let mut controller = state(true);
    prepare(&mut controller);
    let screen = controller.screen();
    for control in inventory(&screen).controls {
        assert!(
            screen
                .bind(&control.id, I::CaptureKey(Key::Space))
                .is_none()
        );
        match control.kind {
            Kind::Toggle { .. } => assert!(screen.bind(&control.id, I::Select(0)).is_none()),
            Kind::Choice { ref choices, .. } => {
                assert!(screen.bind(&control.id, I::Bool(true)).is_none());
                assert!(screen.bind(&control.id, I::Select(choices.len())).is_none());
                assert!(screen.bind(&control.id, I::Select(usize::MAX)).is_none());
            }
            Kind::Number { min, max, .. } => {
                assert!(screen.bind(&control.id, I::Text("12".into())).is_none());
                assert!(screen.bind(&control.id, I::Number(max + 1)).is_none());
                assert!(screen.bind(&control.id, I::Number(u32::MAX)).is_none());
                if min > 0 {
                    assert!(screen.bind(&control.id, I::Number(min - 1)).is_none());
                }
                assert!(screen.bind(&control.id, I::Number(min)).is_some());
                assert!(screen.bind(&control.id, I::Number(max)).is_some());
            }
            Kind::Text { .. } => assert!(screen.bind(&control.id, I::Number(0)).is_none()),
            Kind::Binding { .. } | Kind::Action => {
                assert!(screen.bind(&control.id, I::Bool(true)).is_none())
            }
            Kind::Note { .. } => assert!(screen.bind(&control.id, I::Activate).is_none()),
        }
    }
    assert!(
        screen
            .bind(
                &Id::SystemField {
                    system: SettingsSystemId("unknown".into()),
                    field: SystemSettingsFieldId("unknown".into()),
                },
                I::Select(0)
            )
            .is_none()
    );
}

#[test]
fn dynamic_nes_descriptors_slots_and_sections_have_context_and_stable_identity() {
    let mut controller = state(true);
    let screen = controller.screen();
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
                screen.bind(&id, I::Select(index)),
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
        let c = screen.control(&id).unwrap();
        let Kind::Choice { selected, .. } = c.kind else {
            panic!()
        };
        assert_eq!(
            selected,
            slot.choices
                .iter()
                .position(|c| c.value == slot.selected_profile_id)
        );
        for (index, choice) in slot.choices.iter().enumerate() {
            if slot.occupied_by_other_slot {
                assert!(screen.bind(&id, I::Select(index)).is_none());
            } else {
                events(
                    screen.bind(&id, I::Select(index)),
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
    let Kind::Choice { selected, .. } = refreshed.control(&id).unwrap().kind else {
        panic!()
    };
    assert_eq!(selected, Some(none), "selected None is not no selection");
    assert!(
        controller.vm.inputs()[0].view.get().slots[0]
            .selected_profile_id
            .is_none()
    );
    let inv = inventory(&screen);
    for tab in inv.tabs {
        let expected = match &tab.id {
            SettingsTabId::System(_) => vec![E::SelectSystemTab(0)],
            SettingsTabId::Input(_) => vec![E::SelectInputTab(0)],
            SettingsTabId::Shortcuts => vec![E::SelectInputTab(1)],
            SettingsTabId::Attachment { .. } | SettingsTabId::EmptyAttachment { .. } => {
                let i = input
                    .sections
                    .iter()
                    .position(|s| s.label == tab.label)
                    .unwrap();
                vec![
                    E::SelectInputTab(0),
                    E::SelectInputSection(InputPageSection::Attachment(i)),
                ]
            }
        };
        events(screen.bind(&tab.select.id, I::Activate), expected);
    }
    for row in input.sections.iter().flat_map(|s| &s.rows) {
        let id = Id::Binding(row.target.clone());
        events(
            screen.bind(&id, I::Activate),
            vec![E::StartCapture(row.target.clone())],
        );
        events(
            screen.bind(&id, I::CaptureClear),
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
                let c = screen.control(&id).unwrap();
                assert!(!c.metadata.enabled);
                assert!(c.metadata.help.is_some());
                assert!(screen.bind(&id, I::Select(0)).is_none());
                for id in original_ids
                    .iter()
                    .filter(|id| !matches!(id, Id::Binding(_)))
                {
                    if let Id::Tab(SettingsTabId::Attachment { .. }) = id {
                        continue;
                    }
                    assert!(
                        screen.control(id).is_some(),
                        "unrelated identity changed: {id:?}"
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
        assert!(controller.screen().bind(&child, input).is_none());
        apply(&mut controller, &toggle, I::Bool(true));
    }
    apply(&mut controller, &Id::ErrorPresentation, I::Select(0));
    assert!(
        !controller
            .screen()
            .control(&Id::ErrorColor)
            .unwrap()
            .metadata
            .visible
    );
    assert!(
        controller
            .screen()
            .bind(&Id::ErrorColor, I::Select(0))
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
            .control(&Id::StatusAreaFollows)
            .unwrap()
            .metadata
            .visible
    );
    assert!(screen.bind(&Id::StatusAreaMode, I::Select(0)).is_none());
    controller.update(E::SetStorageDirectory(String::new()));
    controller.error_message = Some("native apply failed".into());
    let screen = controller.screen();
    assert!(!screen.validation.can_submit);
    assert!(screen.validation.storage.is_some());
    assert!(screen.validation.headline.is_some());
    assert!(screen.bind(&Id::Submit, I::Activate).is_none());
    assert_eq!(screen.message.as_deref(), Some("native apply failed"));
    assert_eq!(screen.error_message, screen.message);
    assert!(screen.bind(&Id::Cancel, I::Activate).is_some());
    controller
        .vm
        .general
        .set_storage_policy(StoragePolicy::Sidecar)
        .unwrap();
    let screen = controller.screen();
    assert!(
        !screen
            .control(&Id::StorageDirectory)
            .unwrap()
            .metadata
            .visible
    );
    assert!(
        screen
            .bind(&Id::StorageDirectory, I::Text("ignored".into()))
            .is_none()
    );
    assert!(screen.bind(&Id::StorageDirectory, I::Activate).is_none());
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
    assert_eq!(inv.tabs[0].id, SettingsTabId::Shortcuts);
    assert_eq!(controller.vm.shortcuts.view.get().rows.len(), 8);
    let target = CaptureTarget::Shortcut(ShortcutAction::TogglePause);
    let id = Id::Binding(target.clone());
    assert!(before.bind_capture_key(Key::Space).is_none());
    apply(&mut controller, &id, I::Activate);
    let capture = controller.screen();
    assert_eq!(capture.capture.target, Some(target.clone()));
    assert_eq!(
        capture.capture.prompt,
        text(AppLanguage::English, UiText::CapturePrompt)
    );
    let Kind::Binding { value, .. } = &capture.control(&id).unwrap().kind else {
        panic!()
    };
    assert!(matches!(value, BindingValueView::Capturing(_)));
    events(
        capture.bind(&id, I::CaptureKey(Key::Space)),
        vec![E::CaptureKey(Key::Space)],
    );
    for e in capture.bind_capture_key(Key::Space).unwrap() {
        controller.update(e);
    }
    assert!(controller.screen().capture.target.is_none());
    let bound = controller.screen();
    assert!(matches!(
        &bound.control(&id).unwrap().kind,
        Kind::Binding {
            value: BindingValueView::Bound(_),
            ..
        }
    ));
    apply(&mut controller, &id, I::CaptureClear);
    controller
        .vm
        .general
        .set_language(AppLanguage::Japanese)
        .unwrap();
    controller.vm.audio.set_volume(37).unwrap();
    let after = controller.screen();
    assert_eq!(
        after.title,
        text(AppLanguage::Japanese, UiText::Preferences)
    );
    for page in &after.pages {
        assert_ne!(
            before.pages.iter().find(|p| p.id == page.id).unwrap().label,
            page.label
        );
    }
    assert_eq!(after.control(&Id::Cancel).unwrap().label, "キャンセル");
    assert_eq!(
        after.control(&Id::StorageDirectory).unwrap().kind,
        Kind::Text {
            value: String::new(),
            placeholder: String::new(),
            browse_label: Some("参照".into())
        }
    );
    assert!(matches!(
        after.control(&Id::Volume).unwrap().kind,
        Kind::Number { value: 37, .. }
    ));
    assert!(
        matches!(&after.control(&id).unwrap().kind, Kind::Binding { value: BindingValueView::Unbound(s), change_label, clear_label } if s == "未割当" && change_label == "変更" && clear_label == "解除")
    );
    assert_eq!(
        inventory(&before)
            .controls
            .iter()
            .map(|c| &c.id)
            .collect::<Vec<_>>(),
        inventory(&after)
            .controls
            .iter()
            .map(|c| &c.id)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        before.control(&Id::Cancel).unwrap().label,
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
    impl SettingsVisitor for Visitor {
        fn begin_screen(&mut self, _: &SettingsScreen) {
            self.push("screen", ());
        }
        fn end_screen(&mut self, _: &SettingsScreen) {
            self.pop("screen", ());
        }
        fn begin_page(&mut self, p: &SettingsScreenPage) {
            self.push("page", p.id);
        }
        fn end_page(&mut self, p: &SettingsScreenPage) {
            self.pop("page", p.id);
        }
        fn begin_row(&mut self, l: &SettingsLayout) {
            self.push("row", &l.id);
        }
        fn end_row(&mut self, l: &SettingsLayout) {
            self.pop("row", &l.id);
        }
        fn begin_column(&mut self, l: &SettingsLayout) {
            self.push("column", &l.id);
        }
        fn end_column(&mut self, l: &SettingsLayout) {
            self.pop("column", &l.id);
        }
        fn begin_group(&mut self, l: &SettingsLayout) {
            self.push("group", &l.id);
        }
        fn end_group(&mut self, l: &SettingsLayout) {
            self.pop("group", &l.id);
        }
        fn begin_tabs(&mut self, t: &SettingsTabs) {
            self.push("tabs", &t.id);
        }
        fn end_tabs(&mut self, t: &SettingsTabs) {
            self.pop("tabs", &t.id);
        }
        fn begin_tab(&mut self, t: &SettingsTab) {
            self.push("tab", &t.id);
        }
        fn end_tab(&mut self, t: &SettingsTab) {
            self.pop("tab", &t.id);
        }
        fn control(&mut self, _: &SettingsControl) {
            self.visited += 1;
            if self.visited == 1 {
                self.controller.borrow_mut().update(E::SetVolume(12));
                let nested = self.controller.borrow().screen();
                assert!(!inventory(&nested).controls.is_empty());
            }
        }
    }
    let controller = Rc::new(RefCell::new(state(true)));
    let screen = controller.borrow().screen();
    let mut visitor = Visitor {
        controller: controller.clone(),
        stack: vec![],
        visited: 0,
    };
    let dynamic: &mut dyn SettingsVisitor = &mut visitor;
    screen.visit(dynamic);
    assert!(visitor.stack.is_empty());
    assert!(visitor.visited > 30);
    assert_eq!(controller.borrow().vm.audio.view.get().volume_percent, 12);
    assert!(matches!(
        screen.control(&Id::Volume).unwrap().kind,
        Kind::Number { value: 100, .. }
    ));
}
