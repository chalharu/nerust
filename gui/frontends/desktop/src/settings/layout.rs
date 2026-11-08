use super::{Editor, choice};
use nerust_gui_viewmodel::settings::dto::{BindingSectionView, BindingValueView};
#[cfg(test)]
use nerust_gui_viewmodel::settings::dto::{ChoiceView, choice_position};
use nerust_settings_core::{editor::CaptureTarget, i18n::UiText};
use panechord::{Binding, Choice, Command, Keyed, Store, Viewport};
use std::{cell::RefCell, rc::Rc};

pub(super) type Capture = Rc<dyn Fn(CaptureTarget) -> Command>;

#[derive(Clone)]
pub(super) struct ChoiceRow {
    pub label: Binding<String>,
    pub choices: Binding<Vec<Choice>>,
    pub selected: Binding<String>,
    pub enabled: Binding<bool>,
}

#[derive(Clone)]
pub(super) struct BindingRow {
    pub label: Binding<String>,
    pub value: Binding<String>,
    pub change_label: Binding<String>,
    pub clear_label: Binding<String>,
    pub capture: Command,
    pub clear: Command,
}

pub(super) trait BindingRowCommands {
    fn command_capture(&self, argument: Binding<String>) -> Command;
    fn command_clear(&self, argument: Binding<String>) -> Command;
}

impl BindingRowCommands for Store<BindingRow> {
    fn command_capture(&self, _: Binding<String>) -> Command {
        self.snapshot().capture.clone()
    }
    fn command_clear(&self, _: Binding<String>) -> Command {
        self.snapshot().clear.clone()
    }
}

#[derive(Clone)]
pub(super) struct BindingPage {
    pub label: Binding<String>,
    pub rows: Binding<Vec<Keyed<BindingRow>>>,
}

#[derive(Clone)]
pub(super) struct InputPage {
    pub label: String,
    pub slots: Vec<Keyed<ChoiceRow>>,
    pub has_slots: bool,
    pub sections: Binding<Vec<Keyed<BindingPage>>>,
    pub section: String,
    pub show_section_tabs: bool,
    pub single_section: Option<Keyed<BindingPage>>,
}

#[derive(Clone)]
pub(super) struct SystemPage {
    pub label: String,
    pub fields: Vec<Keyed<ChoiceRow>>,
    pub viewport: Viewport,
}

fn viewport(length: usize, row_height: f32) -> Viewport {
    Viewport {
        count: length.clamp(1, 256),
        row_height,
        ..Viewport::default()
    }
}

#[cfg(test)]
fn choice_row<T: Clone + Eq>(label: String, choices: &[ChoiceView<T>], selected: &T) -> ChoiceRow {
    ChoiceRow {
        label: Binding::constant(label),
        choices: Binding::constant(
            choices
                .iter()
                .enumerate()
                .map(|(index, choice)| Choice {
                    id: index.to_string(),
                    label: choice.label.clone(),
                })
                .collect(),
        ),
        selected: Binding::constant(
            choice_position(choices, selected)
                .map(|index| index.to_string())
                .unwrap_or_default(),
        ),
        enabled: Binding::constant(true),
    }
}

fn binding_page(
    section: Binding<Option<BindingSectionView>>,
    editor: &Rc<Editor>,
    capture: &Capture,
    clear: &Capture,
) -> BindingPage {
    let cache = Rc::new(RefCell::new(std::collections::BTreeMap::<
        String,
        Keyed<BindingRow>,
    >::new()));
    let source = section.clone();
    let capture = capture.clone();
    let clear = clear.clone();
    let executor = editor.executor.clone();
    // Shared by every row: one language-following binding each, cloned
    // into rows so Change/Clear localize with the dialog language.
    let change_label = editor.tr(UiText::Change);
    let clear_label = editor.tr(UiText::Clear);
    BindingPage {
        label: section
            .clone()
            .map(|section| section.map(|section| section.label).unwrap_or_default()),
        rows: section.map(move |section| {
            let mut cache = cache.borrow_mut();
            let rows = section.map(|section| section.rows).unwrap_or_default();
            let mut keys = std::collections::BTreeSet::new();
            let result = rows
                .iter()
                .map(|row| {
                    let key = target_key(&row.target);
                    keys.insert(key.clone());
                    cache
                        .entry(key.clone())
                        .or_insert_with(|| {
                            let label_target = row.target.clone();
                            let value_target = row.target.clone();
                            Keyed::new(
                                key,
                                Store::new(
                                    BindingRow {
                                        label: source.clone().map(move |section| {
                                            section
                                                .and_then(|section| {
                                                    section
                                                        .rows
                                                        .into_iter()
                                                        .find(|row| row.target == label_target)
                                                })
                                                .map(|row| row.label)
                                                .unwrap_or_default()
                                        }),
                                        value: source.clone().map(move |section| {
                                            section
                                                .and_then(|section| {
                                                    section
                                                        .rows
                                                        .into_iter()
                                                        .find(|row| row.target == value_target)
                                                })
                                                .map(|row| match row.value {
                                                    BindingValueView::Unbound(value)
                                                    | BindingValueView::Bound(value)
                                                    | BindingValueView::Capturing(value) => value,
                                                })
                                                .unwrap_or_default()
                                        }),
                                        capture: capture(row.target.clone()),
                                        clear: clear(row.target.clone()),
                                        change_label: change_label.clone(),
                                        clear_label: clear_label.clone(),
                                    },
                                    executor.clone(),
                                ),
                            )
                        })
                        .clone()
                })
                .collect();
            cache.retain(|key, _| keys.contains(key));
            result
        }),
    }
}

fn target_key(target: &CaptureTarget) -> String {
    let identity = match target {
        CaptureTarget::Shortcut(action) => format!("shortcut_{action:?}"),
        CaptureTarget::Binding {
            system,
            attachment,
            control,
        } => {
            let system = system.to_string();
            format!(
                "{}:{system}{}:{attachment}{}:{control}",
                system.len(),
                attachment.len(),
                control.len()
            )
        }
    };
    identity.bytes().map(|byte| format!("{byte:02x}")).collect()
}

fn section_key(section: &BindingSectionView, index: usize) -> String {
    match section.rows.first().map(|row| &row.target) {
        Some(CaptureTarget::Binding {
            system, attachment, ..
        }) => target_key(&CaptureTarget::Binding {
            system: system.clone_box(),
            attachment: attachment.clone(),
            control: String::new(),
        }),
        Some(CaptureTarget::Shortcut(_)) => "shortcuts".into(),
        None => format!("empty_{index}"),
    }
}

fn binding_sections(
    source: Binding<Vec<BindingSectionView>>,
    editor: &Rc<Editor>,
    capture: &Capture,
    clear: &Capture,
) -> Binding<Vec<Keyed<BindingPage>>> {
    let cache = Rc::new(RefCell::new(std::collections::BTreeMap::<
        String,
        Keyed<BindingPage>,
    >::new()));
    let read = source.clone();
    let editor = editor.clone();
    let capture = capture.clone();
    let clear = clear.clone();
    source.map(move |sections| {
        let mut cache = cache.borrow_mut();
        let mut keys = std::collections::BTreeSet::new();
        let pages = sections
            .iter()
            .enumerate()
            .map(|(index, section)| {
                let key = section_key(section, index);
                keys.insert(key.clone());
                cache
                    .entry(key.clone())
                    .or_insert_with(|| {
                        let lookup = key.clone();
                        let section = read.clone().map(move |sections| {
                            sections
                                .into_iter()
                                .enumerate()
                                .find(|(index, section)| section_key(section, *index) == lookup)
                                .map(|(_, section)| section)
                        });
                        Keyed::new(
                            key,
                            Store::new(
                                binding_page(section, &editor, &capture, &clear),
                                editor.executor.clone(),
                            ),
                        )
                    })
                    .clone()
            })
            .collect();
        cache.retain(|key, _| keys.contains(key));
        pages
    })
}

pub(super) fn inputs(
    editor: &Rc<Editor>,
    capture: Capture,
    clear: Capture,
) -> Vec<Keyed<InputPage>> {
    let vm = &editor.vm;
    let executor = editor.executor.clone();
    let mut pages: Vec<_> = vm
        .inputs()
        .iter()
        .enumerate()
        .map(|(index, input)| {
            let value = input.view.get();
            let slots = value
                .slots
                .iter()
                .enumerate()
                .map(|(index, slot)| {
                    let slot_id = slot.slot_id;
                    let choice_vm = input.clone();
                    let selection = choice(
                        editor.binding(
                            input.view.clone(),
                            move |value| {
                                value
                                    .slots
                                    .iter()
                                    .find(|slot| slot.slot_id == slot_id)
                                    .map(|slot| slot.choices.clone())
                                    .unwrap_or_default()
                            },
                            None,
                        ),
                        editor.binding(
                            input.view.clone(),
                            move |value| {
                                value
                                    .slots
                                    .iter()
                                    .find(|slot| slot.slot_id == slot_id)
                                    .and_then(|slot| slot.selected_profile_id.clone())
                            },
                            Some(Rc::new(move |value: Option<String>| {
                                choice_vm.set_controller_slot(slot_id, value.as_deref())
                            })),
                        ),
                    );
                    Keyed::new(
                        index.to_string(),
                        Store::new(
                            ChoiceRow {
                                label: editor.binding(
                                    input.view.clone(),
                                    move |value| {
                                        value
                                            .slots
                                            .iter()
                                            .find(|slot| slot.slot_id == slot_id)
                                            .map(|slot| slot.label.clone())
                                            .unwrap_or_default()
                                    },
                                    None,
                                ),
                                choices: selection.items,
                                selected: selection.selected,
                                enabled: editor.binding(
                                    input.view.clone(),
                                    move |value| {
                                        value
                                            .slots
                                            .iter()
                                            .find(|slot| slot.slot_id == slot_id)
                                            .is_some_and(|slot| !slot.occupied_by_other_slot)
                                    },
                                    None,
                                ),
                            },
                            executor.clone(),
                        ),
                    )
                })
                .collect();
            let sections = binding_sections(
                editor.binding(input.view.clone(), |value| value.sections.clone(), None),
                editor,
                &capture,
                &clear,
            );
            Keyed::new(
                format!("system-{index}"),
                Store::new(
                    InputPage {
                        label: value.label,
                        slots,
                        has_slots: !value.slots.is_empty(),
                        sections,
                        section: String::new(),
                        show_section_tabs: true,
                        single_section: None,
                    },
                    executor.clone(),
                ),
            )
        })
        .collect();
    let shortcuts = vm.shortcuts.view.get();
    pages.push(Keyed::new(
        "shortcuts",
        Store::new(
            InputPage {
                label: shortcuts.label.clone(),
                slots: Vec::new(),
                has_slots: false,
                sections: Binding::constant(Vec::new()),
                section: String::new(),
                show_section_tabs: false,
                single_section: Some(Keyed::new(
                    "global",
                    Store::new(
                        binding_page(
                            editor.binding(
                                vm.shortcuts.view.clone(),
                                |value| Some(value.clone()),
                                None,
                            ),
                            editor,
                            &capture,
                            &clear,
                        ),
                        executor.clone(),
                    ),
                )),
            },
            executor,
        ),
    ));
    pages
}

pub(super) fn systems(editor: &Rc<Editor>) -> Vec<Keyed<SystemPage>> {
    let vm = &editor.vm;
    let executor = editor.executor.clone();
    vm.systems()
        .iter()
        .enumerate()
        .map(|(index, system)| {
            let value = system.view.get();
            let fields = value
                .fields
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    let field_id = field.id.clone();
                    let label_id = field_id.clone();
                    let read_id = field_id.clone();
                    let selected_id = field_id.clone();
                    let choice_vm = system.clone();
                    let selection = choice(
                        editor.binding(system.view.clone(), move |value| {
                            value.fields.iter().find(|field| field.id == read_id)
                                .map(|field| field.choices.clone()).unwrap_or_default()
                        }, None),
                        editor.binding(system.view.clone(), move |value| {
                            value.fields.iter().find(|field| field.id == selected_id)
                                .map(|field| field.selected.clone())
                                .unwrap_or_else(|| nerust_system_traits::factory::descriptor::SystemSettingsChoiceId(String::new().into()))
                        }, Some(Rc::new(move |value| choice_vm.set_choice(&field_id, &value)))),
                    );
                    Keyed::new(
                        index.to_string(),
                        Store::new(
                            ChoiceRow {
                                label: editor.binding(system.view.clone(), move |value| {
                                    value.fields.iter().find(|field| field.id == label_id)
                                        .map(|field| field.label.clone()).unwrap_or_default()
                                }, None),
                                choices: selection.items,
                                selected: selection.selected,
                                enabled: Binding::constant(true),
                            },
                            executor.clone(),
                        ),
                    )
                })
                .collect();
            Keyed::new(
                format!("system-{index}"),
                Store::new(
                    SystemPage {
                        label: value.label,
                        fields,
                        viewport: viewport(value.fields.len(), 46.0),
                    },
                    executor.clone(),
                ),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn choices_follow_vm_order_and_unknown_selection_stays_empty() {
        let choices = vec![
            ChoiceView {
                value: 7,
                label: "Seven".into(),
            },
            ChoiceView {
                value: 2,
                label: "Two".into(),
            },
        ];
        let row = choice_row("Field".into(), &choices, &2);
        assert_eq!(row.selected.get(), "1");
        assert_eq!(row.choices.get()[0].label, "Seven");
        assert!(
            choice_row("Field".into(), &choices, &9)
                .selected
                .get()
                .is_empty()
        );
        assert_eq!(viewport(0, 34.0).count, 1);
        assert_eq!(viewport(300, 34.0).count, 256);
    }

    struct TestExecutor;
    impl panechord::Executor for TestExecutor {
        fn spawn(&self, _: std::pin::Pin<Box<dyn std::future::Future<Output = ()>>>) {
            panic!("layout must not dispatch async work");
        }
    }

    /// Headless editor on the real adapter: the temp dir owner must stay
    /// alive so the isolated settings root outlives the dialog.
    fn headless_editor() -> (
        Rc<Editor>,
        crate::SessionAdapter,
        crate::session::test_fixture::TempDir,
    ) {
        use crate::session::test_fixture::loaded_adapter;
        use nerust_gui_application::SettingsPort;
        let (adapter, dir) = loaded_adapter();
        let editor = Editor::new(adapter.settings().clone(), &adapter, Rc::new(TestExecutor))
            .expect("editor builds on real factories");
        (editor, adapter, dir)
    }

    /// Enabled no-op command factory for layout-only tests: rows still
    /// build their bindings, and invoking the command runs its body.
    fn noop_capture(executor: &Rc<dyn panechord::Executor>) -> Capture {
        let flag = Store::new(String::new(), executor.clone());
        Rc::new(move |_: CaptureTarget| {
            flag.command(
                |_: &String| true,
                |_: &Store<String>, ()| {},
                Binding::constant(()),
            )
        })
    }

    /// System pages mirror the live registry: every factory system yields
    /// a labeled page whose rows track the view model selections.
    #[test]
    fn systems_pages_expose_live_choices_and_viewports() {
        let (editor, _adapter, _dir) = headless_editor();
        let pages = systems(&editor);
        assert!(!pages.is_empty(), "real registry exposes systems");
        for page in &pages {
            let snapshot = page.model.snapshot();
            assert!(!snapshot.label.is_empty());
            assert!(!snapshot.fields.is_empty());
            assert_eq!(snapshot.viewport.count, snapshot.fields.len().clamp(1, 256));
            for field in &snapshot.fields {
                let row = field.model.snapshot();
                assert!(!row.label.get().is_empty());
                assert!(!row.choices.get().is_empty());
                assert!(!row.selected.get().is_empty());
                assert!(row.enabled.get());
            }
        }
        // Flipping a choice on the view model moves the built selection.
        let (field_label, position) = {
            let registry = editor.vm.systems();
            let (system, field, other) = registry
                .iter()
                .flat_map(|system| {
                    let view = system.view.get();
                    view.fields
                        .into_iter()
                        .filter_map(|field| {
                            let id = field.id.clone();
                            field
                                .choices
                                .iter()
                                .find(|choice| choice.value != field.selected)
                                .map(|choice| (system, id, choice.value.clone()))
                        })
                        .collect::<Vec<_>>()
                })
                .next()
                .expect("a system choice has an alternative");
            system.set_choice(&field, &other).expect("choice applies");
            let expected = system
                .view
                .get()
                .fields
                .into_iter()
                .find(|candidate| candidate.id == field)
                .expect("field projects");
            (
                expected.label.clone(),
                choice_position(&expected.choices, &other)
                    .map(|index| index.to_string())
                    .unwrap_or_default(),
            )
        };
        let selected = systems(&editor)
            .iter()
            .flat_map(|page| page.model.snapshot().fields)
            .map(|row| row.model.snapshot())
            .find(|row| row.label.get() == field_label)
            .expect("flipped field builds a row")
            .selected
            .get();
        assert_eq!(selected, position);
    }

    /// Input pages carry slots plus section bindings; the shortcuts page
    /// collapses to one prebuilt section with no slot chrome.
    #[test]
    fn inputs_pages_include_shortcuts_and_live_sections() {
        let (editor, _adapter, _dir) = headless_editor();
        let executor: Rc<dyn panechord::Executor> = Rc::new(TestExecutor);
        let pages = inputs(&editor, noop_capture(&executor), noop_capture(&executor));
        assert_eq!(pages.len(), editor.vm.inputs().len() + 1);
        let shortcuts = pages.last().expect("shortcuts page closes the list");
        assert_eq!(shortcuts.key, "shortcuts");
        let page = shortcuts.model.snapshot();
        assert!(!page.label.is_empty());
        assert!(!page.has_slots);
        assert!(page.slots.is_empty());
        assert!(!page.show_section_tabs);
        let single = page
            .single_section
            .expect("shortcuts collapse to one section")
            .model
            .snapshot();
        assert!(!single.label.get().is_empty());
        assert!(!single.rows.get().is_empty());
        for (index, input) in editor.vm.inputs().iter().enumerate() {
            let view = input.view.get();
            let entry = pages
                .iter()
                .find(|page| page.key == format!("system-{index}"))
                .expect("system page builds");
            let snapshot = entry.model.snapshot();
            assert_eq!(snapshot.label, view.label);
            assert_eq!(snapshot.has_slots, !view.slots.is_empty());
            assert_eq!(snapshot.slots.len(), view.slots.len());
            assert_eq!(snapshot.sections.get().len(), view.sections.len());
            for (slot, view_slot) in snapshot.slots.iter().zip(view.slots.iter()) {
                let row = slot.model.snapshot();
                assert_eq!(row.label.get(), view_slot.label);
                assert_eq!(row.choices.get().len(), view_slot.choices.len());
                let expected = choice_position(&view_slot.choices, &view_slot.selected_profile_id)
                    .map(|index| index.to_string())
                    .unwrap_or_default();
                assert_eq!(row.selected.get(), expected);
                assert_eq!(row.enabled.get(), !view_slot.occupied_by_other_slot);
            }
        }
        // Writing a slot choice id routes through the profile assignment.
        let entry = pages
            .iter()
            .find(|page| page.key.starts_with("system-"))
            .expect("system page builds");
        let slot = entry
            .model
            .snapshot()
            .slots
            .into_iter()
            .next()
            .expect("system page exposes a slot");
        let row = slot.model.snapshot();
        let ids: Vec<String> = row
            .choices
            .get()
            .into_iter()
            .map(|choice| choice.id)
            .collect();
        let original = row.selected.get();
        let other = ids
            .iter()
            .find(|id| **id != original)
            .expect("slot has an alternative")
            .clone();
        assert!(row.selected.set(other.clone()));
        assert_eq!(row.selected.get(), other);
        assert!(row.selected.set(original.clone()));
        assert_eq!(row.selected.get(), original);
    }

    /// Binding rows localize their chrome live: switching the draft
    /// language re-renders Change/Clear without rebuilding the page.
    #[test]
    fn binding_rows_localize_chrome_with_draft_language() {
        use nerust_settings_core::i18n::text as ui_text;
        use nerust_settings_core::model::language::AppLanguage;
        let (editor, _adapter, _dir) = headless_editor();
        let executor: Rc<dyn panechord::Executor> = Rc::new(TestExecutor);
        let pages = inputs(&editor, noop_capture(&executor), noop_capture(&executor));
        let rows = pages
            .last()
            .expect("shortcuts page closes the list")
            .model
            .snapshot()
            .single_section
            .expect("shortcuts collapse to one section")
            .model
            .snapshot()
            .rows
            .get();
        let row = rows
            .first()
            .expect("shortcuts expose rows")
            .model
            .snapshot();
        assert_eq!(row.change_label.get(), "Change");
        assert_eq!(row.clear_label.get(), "Clear");
        assert!(!row.label.get().is_empty());
        assert!(!row.value.get().is_empty());
        editor
            .vm
            .general
            .set_language(AppLanguage::Japanese)
            .expect("language applies");
        assert_eq!(
            row.change_label.get(),
            ui_text(AppLanguage::Japanese, UiText::Change)
        );
        assert_eq!(
            row.clear_label.get(),
            ui_text(AppLanguage::Japanese, UiText::Clear)
        );
        // Row chrome delegates to the built capture/clear commands.
        let _ = row.capture.invoke();
        let _ = row.clear.invoke();
    }

    /// Binding row values re-project live: cleared shortcuts read back
    /// the unbound label and the capturing row reads back the prompt.
    #[test]
    fn binding_row_values_cover_unbound_and_capturing() {
        use nerust_settings_core::i18n::{UiText, text as ui_text};
        use nerust_settings_core::model::input::ShortcutAction;
        let (editor, _adapter, _dir) = headless_editor();
        let executor: Rc<dyn panechord::Executor> = Rc::new(TestExecutor);
        let pages = inputs(&editor, noop_capture(&executor), noop_capture(&executor));
        let rows = pages
            .last()
            .expect("shortcuts page closes the list")
            .model
            .snapshot()
            .single_section
            .expect("shortcuts collapse to one section")
            .model
            .snapshot()
            .rows
            .get();
        assert!(!rows.is_empty());
        for action in [
            ShortcutAction::TogglePause,
            ShortcutAction::SaveActiveSlot,
            ShortcutAction::SelectNextSlot,
            ShortcutAction::SelectPreviousSlot,
            ShortcutAction::LoadActiveSlot,
            ShortcutAction::ToggleFullscreen,
            ShortcutAction::Reset,
            ShortcutAction::OpenDebugger,
        ] {
            editor
                .vm
                .capture
                .clear_binding(&CaptureTarget::Shortcut(action))
                .expect("clear applies");
        }
        let language = editor.vm.general.view.get().language;
        let unbound = ui_text(language, UiText::Unbound).to_string();
        let values: Vec<String> = rows
            .iter()
            .map(|row| row.model.snapshot().value.get())
            .collect();
        assert!(values.iter().all(|value| *value == unbound));
        let target = CaptureTarget::Shortcut(ShortcutAction::TogglePause);
        editor
            .vm
            .capture
            .start_capture(target)
            .expect("capture starts");
        let values: Vec<String> = rows
            .iter()
            .map(|row| row.model.snapshot().value.get())
            .collect();
        assert!(values.contains(&editor.vm.capture.view.get().prompt));
        editor.vm.capture.cancel_capture();
    }

    /// Section builders key stably, tolerate missing sections, and prune
    /// pages the source drops — all without touching backend views.
    #[test]
    fn section_builders_key_stably_and_prune_removed_sections() {
        use nerust_gui_viewmodel::settings::dto::BindingRowView;
        use nerust_settings_core::model::input::ShortcutAction;
        let (editor, _adapter, _dir) = headless_editor();
        let executor: Rc<dyn panechord::Executor> = Rc::new(TestExecutor);
        let capture = noop_capture(&executor);
        let clear = noop_capture(&executor);
        let section = editor.vm.shortcuts.view.get();
        assert_eq!(section_key(&section, 0), "shortcuts");
        assert_eq!(
            section_key(
                &BindingSectionView {
                    label: String::new(),
                    rows: Vec::new(),
                },
                3
            ),
            "empty_3"
        );
        let toggles = target_key(&CaptureTarget::Shortcut(ShortcutAction::TogglePause));
        let reset = target_key(&CaptureTarget::Shortcut(ShortcutAction::Reset));
        assert!(!toggles.is_empty());
        assert_ne!(toggles, reset);
        // Binding targets fold system, attachment, and control into the key.
        if let Some(target) = editor
            .vm
            .inputs()
            .iter()
            .flat_map(|input| input.view.get().sections)
            .flat_map(|section| section.rows)
            .map(|row| row.target)
            .find(|target| matches!(target, CaptureTarget::Binding { .. }))
        {
            let key = target_key(&target);
            assert!(!key.is_empty());
            assert_ne!(key, toggles);
            let section = BindingSectionView {
                label: "Pads".into(),
                rows: vec![BindingRowView {
                    target,
                    label: "Button".into(),
                    value: BindingValueView::Bound("Space".into()),
                }],
            };
            assert!(!section_key(&section, 0).is_empty());
            assert_ne!(section_key(&section, 0), "shortcuts");
        }
        let page = binding_page(
            Binding::constant(Some(section.clone())),
            &editor,
            &capture,
            &clear,
        );
        assert_eq!(page.label.get(), section.label);
        assert_eq!(page.rows.get().len(), section.rows.len());
        let missing = binding_page(Binding::constant(None), &editor, &capture, &clear);
        assert!(missing.label.get().is_empty());
        assert!(missing.rows.get().is_empty());
        // Cached pages survive re-reads and drop sections the source removes.
        let source = Store::new(vec![section.clone()], executor.clone());
        let pages = binding_sections(
            source.bind(|sections| sections.clone(), None),
            &editor,
            &capture,
            &clear,
        );
        assert_eq!(pages.get().len(), 1);
        assert_eq!(pages.get().len(), 1);
        // Reading the cached page replays the section lookup by key.
        let inner = pages
            .get()
            .into_iter()
            .next()
            .expect("section page builds")
            .model
            .snapshot();
        assert_eq!(inner.label.get(), section.label);
        assert_eq!(inner.rows.get().len(), section.rows.len());
        let _ = source.update(|sections| sections.clear());
        assert!(pages.get().is_empty());
    }

    /// Row chrome delegates to the dialog commands: capture starts a
    /// session and surfaces its prompt; clear ends it and clears status.
    #[test]
    fn binding_row_commands_drive_capture_and_clear() {
        use super::super::Apply;
        use nerust_gui_runtime::settings::SettingsSnapshot;
        use nerust_settings_core::model::input::ShortcutAction;
        let (editor, _adapter, _dir) = headless_editor();
        let executor: Rc<dyn panechord::Executor> = Rc::new(TestExecutor);
        let apply: Apply = Rc::new(|_: SettingsSnapshot| Ok(()));
        let quit: Rc<dyn Fn()> = Rc::new(|| {});
        let slot = Rc::new(RefCell::new(None));
        let commands = editor.commands(&apply, quit, &slot);
        let target = CaptureTarget::Shortcut(ShortcutAction::TogglePause);
        let row = BindingRow {
            label: Binding::constant("Toggle pause".into()),
            value: Binding::constant("Space".into()),
            change_label: Binding::constant("Change".into()),
            clear_label: Binding::constant("Clear".into()),
            capture: (commands.capture)(target.clone()),
            clear: (commands.clear)(target.clone()),
        };
        let store = Store::new(row, executor);
        let argument = Binding::constant("row".into());
        let _ = store.command_capture(argument.clone()).invoke();
        assert!(editor.vm.capture.view.get().target.is_some());
        assert!(!editor.status.with(|status| status.clone()).is_empty());
        let _ = store.command_clear(argument).invoke();
        assert!(editor.vm.capture.view.get().target.is_none());
        assert!(editor.status.with(|status| status.clone()).is_empty());
    }
}
