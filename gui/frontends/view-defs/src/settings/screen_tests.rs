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
use wakui::scene::control::{
    Container, Control, ControlInput, ControlScreen, ControlVisitor, GroupPolicy, Tab, policy,
};
use wakui::scene::leaf::{
    ActionLeaf, BindingLeaf, ChoiceLeaf, ListLeaf, NoteLeaf, NumberLeaf, TextLeaf, ToggleLeaf,
};

use super::*;
use crate::settings::{
    InputPageSection, InputTab, SettingsEvent as E, SettingsSystemId, screen::section_tab_id,
};
use nerust_gui_viewmodel::settings::dto::{BindingRowView, BindingSectionView};
use wakui::scene::inflate::{cell_key, row_key};

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

fn token(controller: &SettingsController, key: impl AsRef<str>) -> ControlId {
    controller.ids.borrow_mut().ids.id(key.as_ref().to_string())
}

fn sys_ns(system: &str) -> String {
    row_key("systems", system, 0)
}

fn input_ns(tab: &str) -> String {
    row_key("input_tabs", tab, 0)
}

fn sec_ns(input: &str, tab_id: &str, occurrence: usize) -> String {
    cell_key(&input_ns(input), &row_key("sections", tab_id, occurrence))
}

fn select_key(ns: &str) -> String {
    cell_key(ns, "select")
}

fn field_key(system: &str, field: &str) -> String {
    let rowns = cell_key(&sys_ns(system), &row_key("fields", field, 0));
    cell_key(&rowns, "cell")
}

fn slot_key(system: &str, slot: &str) -> String {
    let tabns = input_ns(system);
    let rowns = cell_key(&tabns, &row_key("slots", slot, 0));
    cell_key(&rowns, "cell")
}

fn binding_key_str(
    system: &str,
    sections: &[BindingSectionView],
    shortcuts: &[BindingRowView],
    target: &CaptureTarget,
) -> String {
    let id = crate::settings::screen::binding_key(target);
    // Sections first, then the shortcuts tab; identities are unique so the
    // per-parent engine occurrence always resolves to zero here.
    for (section_index, section) in sections.iter().enumerate() {
        if section.rows.iter().any(|row| &row.target == target) {
            let tabid = section_tab_id(system, section_index, &section.rows);
            let secns = sec_ns(system, &tabid, 0);
            let rowns = cell_key(&secns, &row_key("binding_rows", &id, 0));
            return cell_key(&rowns, "cell");
        }
    }
    if shortcuts.iter().any(|row| &row.target == target) {
        let tabns = input_ns("shortcuts");
        let rowns = cell_key(&tabns, &row_key("shortcut_rows", &id, 0));
        return cell_key(&rowns, "cell");
    }
    panic!("unknown binding target {target:?}");
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

fn apply(controller: &mut SettingsController, id: ControlId, input: I) {
    for event in controller
        .screen()
        .bind(&id, input.clone())
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
    controls: Vec<Control>,
    groups: Vec<ControlId>,
    tabs: Vec<Tab>,
}

impl ControlVisitor for Inventory {
    fn begin_page(&mut self, page: &wakui::scene::control::Page) {
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
        // List and tab cells surface only through explicit walk, mirroring
        // what native flavors do after rendering the host control.
        wakui::scene::control::walk_control(control, self);
    }
}

fn inventory(view: &SettingsScreen) -> Inventory {
    let mut result = Inventory::default();
    view.screen.visit(&mut result);
    result
}

fn control_for(view: &SettingsScreen, key: &str, controller: &SettingsController) -> Control {
    let id = token(controller, key);
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
    // Every expected key resolves to a live control.
    let mut expected: Vec<ControlId> = [
        "language",
        "storage-policy",
        "storage-dir",
        "storage-error",
        "fullscreen",
        "scaling",
        "vsync",
        "mute",
        "volume",
        "sample-rate",
        "latency",
        "info-enabled",
        "info-duration",
        "info-color",
        "warning-enabled",
        "warning-duration",
        "warning-color",
        "error-enabled",
        "error-presentation",
        "error-color",
        "status-enabled",
        "position",
        "status-position",
        "area-mode",
        "status-area-mode",
        "status-area-follows",
        "no-systems",
        "cancel",
        "submit",
    ]
    .into_iter()
    .map(|key| token(&controller, key))
    .collect();
    expected.push(token(
        &controller,
        cell_key(&input_ns("shortcuts"), "select"),
    ));
    let shortcut_rows = controller.vm.shortcuts.view.get().rows;
    expected.extend(shortcut_rows.iter().map(|row| {
        token(
            &controller,
            binding_key_str("shortcuts", &[], &shortcut_rows, &row.target),
        )
    }));
    for id in &expected {
        assert!(view.screen.control(*id).is_some(), "missing {id:?}");
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
            "language",
            I::Select(0),
            E::SetLanguage(g.language_choices[0].clone()),
        ),
        (
            "storage-policy",
            I::Select(0),
            E::SetStoragePolicy(g.storage_policy_choices[0].clone()),
        ),
        (
            "storage-dir",
            I::Text("/chosen".into()),
            E::SetStorageDirectory("/chosen".into()),
        ),
        ("storage-dir", I::Activate, E::BrowseStorageDirectory),
        (
            "fullscreen",
            I::Bool(true),
            E::ToggleFullscreenDefault(true),
        ),
        (
            "scaling",
            I::Select(1),
            E::SetScaling(v.scaling_choices[1].clone()),
        ),
        ("vsync", I::Bool(false), E::ToggleVsync(false)),
        ("mute", I::Bool(true), E::ToggleMute(true)),
        ("volume", I::Number(42), E::SetVolume(42)),
        (
            "sample-rate",
            I::Select(1),
            E::SetSampleRate(a.sample_rate_choices[1].clone()),
        ),
        ("latency", I::Number(99), E::SetLatency(99)),
        ("info-enabled", I::Bool(false), E::ToggleInfoEnabled(false)),
        ("info-duration", I::Number(2345), E::SetInfoDuration(2345)),
        (
            "info-color",
            I::Select(1),
            E::SetInfoColor(n.color_choices[1].clone()),
        ),
        (
            "warning-enabled",
            I::Bool(false),
            E::ToggleWarningEnabled(false),
        ),
        (
            "warning-duration",
            I::Number(4567),
            E::SetWarningDuration(4567),
        ),
        (
            "warning-color",
            I::Select(2),
            E::SetWarningColor(n.color_choices[2].clone()),
        ),
        (
            "error-enabled",
            I::Bool(false),
            E::ToggleErrorEnabled(false),
        ),
        (
            "error-presentation",
            I::Select(0),
            E::ToggleErrorModal(true),
        ),
        (
            "error-presentation",
            I::Select(1),
            E::ToggleErrorModal(false),
        ),
        (
            "error-color",
            I::Select(3),
            E::SetErrorColor(n.color_choices[3].clone()),
        ),
        (
            "status-enabled",
            I::Bool(false),
            E::ToggleStatusEnabled(false),
        ),
        (
            "position",
            I::Select(1),
            E::SetPosition(n.position_choices[1].clone()),
        ),
        (
            "status-position",
            I::Select(2),
            E::SetStatusPosition(n.position_choices[2].clone()),
        ),
        (
            "area-mode",
            I::Select(1),
            E::SetAreaMode(n.area_mode_choices[1].clone()),
        ),
        (
            "status-area-mode",
            I::Select(0),
            E::SetStatusAreaMode(n.area_mode_choices[0].clone()),
        ),
        ("cancel", I::Activate, E::Cancel),
        ("submit", I::Activate, E::Submit),
    ];
    let shortcuts_tab = cell_key(&input_ns("shortcuts"), "select");
    let mut cases = cases;
    cases.push((shortcuts_tab.as_str(), I::Activate, E::SelectInputTab(0)));
    for (key, input, event) in cases {
        let view = controller.screen();
        let id = token(&controller, key);
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
    let shortcut_rows = controller.vm.shortcuts.view.get().rows;
    for row in shortcut_rows.clone() {
        let view = controller.screen();
        let id = token(
            &controller,
            binding_key_str("", &[], &shortcut_rows, &row.target),
        );
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
    let volume_id = token(&controller, "volume");
    apply(&mut controller, volume_id, I::Number(42));
    assert_eq!(controller.vm.audio.view.get().volume_percent, 42);
    let duration_id = token(&controller, "info-duration");
    apply(&mut controller, duration_id, I::Number(2345));
    assert_eq!(
        controller.vm.notifications.view.get().info_duration_ms,
        2345
    );
}

/// Map a page-select control back to its page. Selects are minted from
/// page keys, so the mapping is the token helper in reverse.
fn token_page(controller: &SettingsController, select: ControlId) -> SettingsPage {
    for (page, key) in [
        (
            SettingsPage::General,
            crate::settings::screen::keys::PAGE_GENERAL,
        ),
        (
            SettingsPage::Input,
            crate::settings::screen::keys::PAGE_INPUT,
        ),
        (
            SettingsPage::Video,
            crate::settings::screen::keys::PAGE_VIDEO,
        ),
        (
            SettingsPage::Audio,
            crate::settings::screen::keys::PAGE_AUDIO,
        ),
        (
            SettingsPage::Notifications,
            crate::settings::screen::keys::PAGE_NOTIFICATIONS,
        ),
        (
            SettingsPage::System,
            crate::settings::screen::keys::PAGE_SYSTEM,
        ),
    ] {
        if token(controller, format!("{key}/select")) == select {
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
        } else if control.downcast::<ListLeaf>().is_some() {
            // List hosts carry no binding; every input is rejected.
            assert!(view.bind(&control.id, I::Activate).is_none());
            assert!(view.bind(&control.id, I::Select(0)).is_none());
        } else {
            panic!("uncovered leaf");
        }
    }
    assert!(
        controller
            .screen()
            .bind(&token(&controller, "fields/unknown#9/cell"), I::Select(0),)
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
        let id = token(&controller, field_key(&key.0, field.id.as_str()));
        for (index, choice) in field.choices.iter().enumerate() {
            events(
                view.bind(&id, I::Select(index)),
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
        apply(&mut controller, id, I::Select(index));
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
        let id = token(&controller, slot_key(&key.0, slot.slot_id.as_str()));
        let c = control_for(
            &controller.screen(),
            &slot_key(&key.0, slot.slot_id.as_str()),
            &controller,
        );
        let leaf = c.downcast::<ChoiceLeaf>().unwrap();
        assert_eq!(
            leaf.selected,
            slot.choices
                .iter()
                .position(|c| c.value == slot.selected_profile_id)
        );
        for (index, choice) in slot.choices.iter().enumerate() {
            if slot.occupied_by_other_slot {
                assert!(controller.screen().bind(&id, I::Select(index)).is_none());
            } else {
                events(
                    controller.screen().bind(&id, I::Select(index)),
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
    // Tab selects are minted from domain tab keys; map them back through
    // the same registry to recover the expected navigation events.
    // Candidates come from the original snapshots: later applies only
    // change selections, never the section structure under test.
    let mut candidates: Vec<(String, Vec<E>)> = Vec::new();
    let sys = key.0.clone();
    candidates.push((select_key(&sys_ns(&sys)), vec![E::SelectSystemTab(0)]));
    candidates.push((select_key(&input_ns(&sys)), vec![E::SelectInputTab(0)]));
    for (section_index, section) in input.sections.iter().enumerate() {
        let tabid = section_tab_id(&sys, section_index, &section.rows);
        candidates.push((
            select_key(&sec_ns(&sys, &tabid, 0)),
            vec![
                E::SelectInputTab(0),
                E::SelectInputSection(InputPageSection::Attachment(section_index)),
            ],
        ));
    }
    candidates.push((
        select_key(&input_ns("shortcuts")),
        vec![E::SelectInputTab(1)],
    ));
    let inv = inventory(&view);
    for tab in inv.tabs {
        // Plain tabs resolve through the registry; empty-attachment tabs
        // use content keys minted at build, so match those by label.
        let expected = candidates
            .iter()
            .find(|(select_key, _)| token(&controller, select_key) == tab.select.id)
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
        let id = token(
            &controller,
            binding_key_str(&sys, &input.sections, &[], &row.target),
        );
        events(
            view.bind(&id, I::Activate),
            vec![E::StartCapture(row.target.clone())],
        );
        events(
            view.bind(&id, I::CaptureClear),
            vec![E::ClearCapture(row.target.clone())],
        );
    }

    let slot = &input.slots[0];
    let id = token(&controller, slot_key(&key.0, slot.slot_id.as_str()));
    let none = slot.choices.iter().position(|c| c.value.is_none()).unwrap();
    controller.input_tab = Some(InputTab::Shortcuts);
    apply(&mut controller, id, I::Select(none));
    let refreshed = controller.screen();
    let selected = refreshed
        .screen
        .control(id)
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
}

#[test]
fn input_tab_kind_gates_list_visibility() {
    // Each input tab shares one template body; per-kind lists resolve
    // through row flags so system tabs never show shortcuts (and the
    // shortcuts tab never shows conflicts, slots, or sections).
    let controller = state(true);
    let view = controller.screen();
    let sys = controller.vm.inputs()[0].view.get().system_id.to_string();
    let list_key = |tab: &str, list: &str| cell_key(&input_ns(tab), list);
    for (tab, conflicts, slots, shortcuts) in [
        (sys.as_str(), true, true, false),
        ("shortcuts", false, false, true),
    ] {
        let conflicts_id = token(&controller, list_key(tab, "conflicts"));
        let slots_id = token(&controller, list_key(tab, "slots"));
        let shortcuts_id = token(&controller, list_key(tab, "shortcuts"));
        assert_eq!(
            view.screen.control(conflicts_id).unwrap().metadata.visible,
            conflicts,
            "conflicts list on {tab}"
        );
        assert_eq!(
            view.screen.control(slots_id).unwrap().metadata.visible,
            slots,
            "slots list on {tab}"
        );
        assert_eq!(
            view.screen.control(shortcuts_id).unwrap().metadata.visible,
            shortcuts,
            "shortcuts list on {tab}"
        );
        // Hidden lists reject input through the same metadata gate.
        if !conflicts {
            assert!(view.bind(&conflicts_id, I::Activate).is_none());
        }
        if !shortcuts {
            assert!(view.bind(&shortcuts_id, I::Activate).is_none());
        }
    }
    // Sections exist only under system tabs; the shortcuts tab template
    // projects an empty tab set that native hosts collapse.
    struct TabsInventory(Vec<(ControlId, usize)>);
    impl ControlVisitor for TabsInventory {
        fn begin_tabs(&mut self, tabs: &wakui::scene::control::Tabs) {
            self.0.push((tabs.id, tabs.tabs.len()));
        }
    }
    let mut tabs_inventory = TabsInventory(Vec::new());
    view.screen.visit(&mut tabs_inventory);
    let sections_id = |tab: &str| cell_key(&input_ns(tab), "tabs");
    let sections_len = |tab: &str| {
        tabs_inventory
            .0
            .iter()
            .find(|(id, _)| *id == token(&controller, sections_id(tab)))
            .map(|(_, len)| *len)
            .expect("sections tab set")
    };
    assert_eq!(
        sections_len(&sys),
        controller.vm.inputs()[0].view.get().sections.len(),
        "system tab sections"
    );
    assert_eq!(sections_len("shortcuts"), 0, "shortcuts tab sections");
    // List hosts carry the retired row-container gap for native bodies.
    for key in ["conflicts", "slots", "shortcuts"] {
        let id = token(&controller, list_key(&sys, key));
        assert_eq!(
            view.screen.control(id).unwrap().metadata.hints.spacing,
            Some(12.0),
            "list gap on {key}"
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
                let id = token(
                    &controller,
                    slot_key(&current.system_id.to_string(), occupied.slot_id.as_str()),
                );
                let screen = controller.screen();
                let c = screen.screen.control(id).unwrap();
                assert!(!c.metadata.enabled);
                assert!(c.metadata.help.is_some());
                assert!(screen.bind(&id, I::Select(0)).is_none());
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
        ("info-enabled", "info-duration", I::Number(2000)),
        ("warning-enabled", "warning-color", I::Select(0)),
        ("error-enabled", "error-presentation", I::Select(0)),
        ("status-enabled", "status-position", I::Select(0)),
    ] {
        let toggle_id = token(&controller, toggle);
        apply(&mut controller, toggle_id, I::Bool(false));
        assert!(
            controller
                .screen()
                .bind(&token(&controller, child), input)
                .is_none()
        );
        let toggle_id = token(&controller, toggle);
        apply(&mut controller, toggle_id, I::Bool(true));
    }
    let presentation_id = token(&controller, "error-presentation");
    apply(&mut controller, presentation_id, I::Select(0));
    assert!(
        !controller
            .screen()
            .screen
            .control(token(&controller, "error-color"))
            .unwrap()
            .metadata
            .visible
    );
    assert!(
        controller
            .screen()
            .bind(&token(&controller, "error-color"), I::Select(0))
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
            .control(token(&controller, "status-area-follows"))
            .unwrap()
            .metadata
            .visible
    );
    assert!(
        screen
            .bind(&token(&controller, "status-area-mode"), I::Select(0))
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
            .bind(&token(&controller, "submit"), I::Activate)
            .is_none()
    );
    assert_eq!(
        screen.screen.message.as_deref(),
        Some("native apply failed")
    );
    assert_eq!(controller.error_message, screen.screen.message);
    assert!(
        screen
            .bind(&token(&controller, "cancel"), I::Activate)
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
            .control(token(&controller, "storage-dir"))
            .unwrap()
            .metadata
            .visible
    );
    assert!(
        screen
            .bind(
                &token(&controller, "storage-dir"),
                I::Text("ignored".into())
            )
            .is_none()
    );
    assert!(
        screen
            .bind(&token(&controller, "storage-dir"), I::Activate)
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
        token(&controller, select_key(&input_ns("shortcuts")))
    );
    assert_eq!(controller.vm.shortcuts.view.get().rows.len(), 8);
    let target = CaptureTarget::Shortcut(ShortcutAction::TogglePause);
    let shortcut_rows = controller.vm.shortcuts.view.get().rows;
    let id = binding_key_str("", &[], &shortcut_rows, &target);
    let token_id = token(&controller, &id);
    assert!(before.bind(&token_id, I::Activate).is_some());
    assert!(before.bind_capture_key(Key::Space).is_none());
    apply(&mut controller, token_id, I::Activate);
    let capture = controller.screen();
    assert_eq!(capture.screen.capture.target, Some(token(&controller, &id)));
    assert_eq!(
        capture.screen.capture.prompt,
        text(AppLanguage::English, UiText::CapturePrompt)
    );
    let bound = capture.screen.control(token(&controller, &id)).unwrap();
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
    let clear_id = token(&controller, &id);
    apply(&mut controller, clear_id, I::CaptureClear);
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
            .control(token(&controller, "cancel"))
            .unwrap()
            .label,
        "キャンセル"
    );
    let dir = after
        .screen
        .control(token(&controller, "storage-dir"))
        .unwrap();
    let text_leaf = dir.downcast::<TextLeaf>().unwrap();
    assert_eq!(text_leaf.value, String::new());
    assert_eq!(text_leaf.placeholder, String::new());
    assert_eq!(text_leaf.browse_label.as_deref(), Some("参照"));
    let volume = after.screen.control(token(&controller, "volume")).unwrap();
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
            .control(token(&controller, "cancel"))
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
        fn begin_page(&mut self, p: &wakui::scene::control::Page) {
            self.push("page", p.id);
        }
        fn end_page(&mut self, p: &wakui::scene::control::Page) {
            self.pop("page", p.id);
        }
        fn begin_container(&mut self, c: &Container) {
            self.push("container", c.id);
        }
        fn end_container(&mut self, c: &Container) {
            self.pop("container", c.id);
        }
        fn begin_tabs(&mut self, t: &wakui::scene::control::Tabs) {
            self.push("tabs", t.id);
        }
        fn end_tabs(&mut self, t: &wakui::scene::control::Tabs) {
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
    let volume = token(&controller.borrow(), "volume");
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
