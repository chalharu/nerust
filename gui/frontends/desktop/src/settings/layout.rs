use super::*;
use nerust_gui_viewmodel::settings::dto::{BindingSectionView, BindingValueView};
use nerust_settings_core::editor::CaptureTarget;
use panechord::{Keyed, Viewport};

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
}
