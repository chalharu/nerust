use super::*;
use nerust_gui_viewmodel::settings::dto::{BindingSectionView, BindingValueView};
use nerust_settings_core::editor::CaptureTarget;
use panechord::{Keyed, Viewport};

pub(super) type Capture = Rc<dyn Fn(CaptureTarget) -> Command>;

#[derive(Clone)]
pub(super) struct ChoiceRow {
    pub label: String,
    pub choices: Vec<Choice>,
    pub selected: String,
}

#[derive(Clone)]
pub(super) struct BindingRow {
    pub label: String,
    pub value: String,
    pub capture: Command,
}

pub(super) trait BindingRowCommands {
    fn command_capture(&self, argument: Binding<String>) -> Command;
}

impl BindingRowCommands for Store<BindingRow> {
    fn command_capture(&self, _: Binding<String>) -> Command {
        self.snapshot().capture.clone()
    }
}

#[derive(Clone)]
pub(super) struct BindingPage {
    pub label: String,
    pub rows: Vec<Keyed<BindingRow>>,
    pub viewport: Viewport,
}

#[derive(Clone)]
pub(super) struct InputPage {
    pub label: String,
    pub slots: Vec<Keyed<ChoiceRow>>,
    pub slot_viewport: Viewport,
    pub has_slots: bool,
    pub sections: Vec<Keyed<BindingPage>>,
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

fn choice_row<T: Clone + Eq>(label: String, choices: &[ChoiceView<T>], selected: &T) -> ChoiceRow {
    ChoiceRow {
        label,
        choices: choices
            .iter()
            .enumerate()
            .map(|(index, choice)| Choice {
                id: index.to_string(),
                label: choice.label.clone(),
            })
            .collect(),
        selected: choice_position(choices, selected)
            .map(|index| index.to_string())
            .unwrap_or_default(),
    }
}

fn binding_page(
    section: &BindingSectionView,
    executor: Rc<dyn Executor>,
    capture: &Capture,
) -> BindingPage {
    BindingPage {
        label: section.label.clone(),
        viewport: viewport(section.rows.len(), 34.0),
        rows: section
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let value = match &row.value {
                    BindingValueView::Unbound(value)
                    | BindingValueView::Bound(value)
                    | BindingValueView::Capturing(value) => value.clone(),
                };
                Keyed::new(
                    index.to_string(),
                    Store::new(
                        BindingRow {
                            label: row.label.clone(),
                            value,
                            capture: capture(row.target.clone()),
                        },
                        executor.clone(),
                    ),
                )
            })
            .collect(),
    }
}

pub(super) fn inputs(
    vm: &SettingsViewModel,
    executor: Rc<dyn Executor>,
    capture: Capture,
) -> Vec<Keyed<InputPage>> {
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
                    Keyed::new(
                        index.to_string(),
                        Store::new(
                            choice_row(
                                slot.label.clone(),
                                &slot.choices,
                                &slot.selected_profile_id,
                            ),
                            executor.clone(),
                        ),
                    )
                })
                .collect();
            let sections = value
                .sections
                .iter()
                .enumerate()
                .map(|(index, section)| {
                    Keyed::new(
                        index.to_string(),
                        Store::new(
                            binding_page(section, executor.clone(), &capture),
                            executor.clone(),
                        ),
                    )
                })
                .collect();
            Keyed::new(
                format!("system-{index}"),
                Store::new(
                    InputPage {
                        label: value.label,
                        slots,
                        slot_viewport: viewport(value.slots.len(), 46.0),
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
                slot_viewport: Viewport::default(),
                has_slots: false,
                sections: Vec::new(),
                section: String::new(),
                show_section_tabs: false,
                single_section: Some(Keyed::new(
                    "global",
                    Store::new(
                        binding_page(&shortcuts, executor.clone(), &capture),
                        executor.clone(),
                    ),
                )),
            },
            executor,
        ),
    ));
    pages
}

pub(super) fn systems(
    vm: &SettingsViewModel,
    executor: Rc<dyn Executor>,
) -> Vec<Keyed<SystemPage>> {
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
                    Keyed::new(
                        index.to_string(),
                        Store::new(
                            choice_row(field.label.clone(), &field.choices, &field.selected),
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
        assert_eq!(row.selected, "1");
        assert_eq!(row.choices[0].label, "Seven");
        assert!(choice_row("Field".into(), &choices, &9).selected.is_empty());
        assert_eq!(viewport(0, 34.0).count, 1);
        assert_eq!(viewport(300, 34.0).count, 256);
    }
}
