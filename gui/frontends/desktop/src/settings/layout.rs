use super::{Editor, choice};
#[cfg(test)]
use nerust_gui_viewmodel::settings::dto::choice_position;
use nerust_gui_viewmodel::settings::{
    InputSettingsViewModel, ViewModelError,
    dto::{BindingRowView, BindingSectionView, BindingValueView, ChoiceView, GamepadTabView},
};
use nerust_settings_core::{
    editor::CaptureTarget,
    i18n::{I18n, UiText},
    labels::pad_label,
    model::input::DEFAULT_DEADZONE_PERCENT,
};
use panechord::{Binding, Choice, Command, Keyed, Store, Viewport};
use std::{cell::RefCell, rc::Rc};

pub(super) type Capture = Rc<dyn Fn(CaptureTarget) -> Command>;
pub(super) type Reset = Rc<dyn Fn(super::ResetScope) -> Command>;

#[derive(Clone)]
pub(super) struct ResetRow {
    pub label: Binding<String>,
    pub reset: Command,
}

pub(super) trait ResetRowCommands {
    fn command_reset(&self, _: Binding<String>) -> Command;
}

impl ResetRowCommands for Store<ResetRow> {
    fn command_reset(&self, _: Binding<String>) -> Command {
        self.snapshot().reset.clone()
    }
}

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
pub(super) struct CheckRow {
    pub label: Binding<String>,
    pub checked: Binding<bool>,
}

#[derive(Clone)]
pub(super) struct PlayerPage {
    pub label: String,
    pub radio: Option<Keyed<ChoiceRow>>,
    pub radio_visible: bool,
    pub note: Binding<String>,
    pub note_visible: bool,
    pub keyboard: Option<Keyed<CheckRow>>,
    pub slots: Vec<Keyed<ChoiceRow>>,
    pub resets: Vec<Keyed<ResetRow>>,
}

/// One General L2 tab (App | Video | Audio | Rumble & Motion). All four
/// share one template with visibility flags; the row bindings below are
/// built once in the presentation and cloned into each tab entry.
#[derive(Clone)]
pub(super) struct GeneralTab {
    pub label: String,
    pub show_app: bool,
    pub show_video: bool,
    pub show_audio: bool,
    pub show_rumble: bool,
    pub language: Option<Keyed<ChoiceRow>>,
    pub storage: Option<Keyed<ChoiceRow>>,
    pub directory: Binding<String>,
    pub directory_visible: Binding<bool>,
    pub fullscreen: Option<Keyed<CheckRow>>,
    pub vsync: Option<Keyed<CheckRow>>,
    pub scaling: Option<Keyed<ChoiceRow>>,
    pub muted: Option<Keyed<CheckRow>>,
    pub volume: Binding<f64>,
    pub volume_label: Binding<String>,
    pub latency: Binding<f64>,
    pub latency_label: Binding<String>,
    pub sample_rate: Option<Keyed<ChoiceRow>>,
    pub rumble_motion: Option<Keyed<CheckRow>>,
    pub rumble_enabled: Option<Keyed<CheckRow>>,
    pub rumble_strength: Binding<f64>,
    pub rumble_strength_label: Binding<String>,
    pub rumble_target: Option<Keyed<ChoiceRow>>,
    pub resets: Vec<Keyed<ResetRow>>,
}

#[derive(Clone)]
pub(super) struct GamepadBlock {
    pub label: Binding<String>,
    pub assignment: Binding<String>,
    pub deadzone: Option<Keyed<ChoiceRow>>,
    pub dpad: Option<Keyed<ChoiceRow>>,
    pub sections: Binding<Vec<Keyed<BindingPage>>>,
    pub note: Binding<String>,
    pub note_visible: Binding<bool>,
    pub resets: Vec<Keyed<ResetRow>>,
}

#[derive(Clone)]
pub(super) struct SystemPage {
    pub label: String,
    pub fields: Vec<Keyed<ChoiceRow>>,
    pub viewport: Viewport,
    pub key_sections: Binding<Vec<Keyed<BindingPage>>>,
    pub resets: Vec<Keyed<ResetRow>>,
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

fn hex_bytes(text: &str) -> String {
    text.bytes().map(|byte| format!("{byte:02x}")).collect()
}

fn target_key(target: &CaptureTarget) -> String {
    if let Some(action) = target.action() {
        return hex_bytes(&format!("shortcut_{action:?}"));
    }
    // Bindings and per-pad overrides share the length-prefixed identity;
    // overrides prefix the pad. Shortcuts return above.
    let system = target
        .system_id()
        .map(|system| system.to_string())
        .unwrap_or_default();
    let attachment = target.attachment().unwrap_or_default();
    let control = target.control().unwrap_or_default();
    let identity = match target.pad() {
        Some(pad) => format!(
            "pad{pad}_{}:{system}{}:{attachment}{}:{control}",
            system.len(),
            attachment.len(),
            control.len()
        ),
        None => format!(
            "{}:{system}{}:{attachment}{}:{control}",
            system.len(),
            attachment.len(),
            control.len()
        ),
    };
    hex_bytes(&identity)
}

fn section_key(section: &BindingSectionView, index: usize) -> String {
    let Some(target) = section.rows.first().map(|row| &row.target) else {
        return format!("empty_{index}");
    };
    if target.action().is_some() {
        return "shortcuts".into();
    }
    if target.pad().is_some() {
        return format!("padbindings_{index}");
    }
    // Section identity folds system + attachment (not control): rows of
    // one attachment share the page. Same shape as `target_key` with an
    // empty control, so same-length attachments (player1/player2) keep
    // distinct keys and the backend never rejects the update.
    let system = target
        .system_id()
        .map(|system| system.to_string())
        .unwrap_or_default();
    let attachment = target.attachment().unwrap_or_default();
    let identity = format!(
        "{}:{system}{}:{attachment}0:",
        system.len(),
        attachment.len(),
    );
    hex_bytes(&identity)
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

/// One global player page: the gamepad radio for this port plus the
/// controller-type slot choices of the machines that actually have a
/// slot on this port. Keyboard tables live on the machine pages and
/// pad mappings on the Gamepads page, so this page owns no binding lists.
pub(super) fn controllers_pages(
    editor: &Rc<Editor>,
    reset: &Reset,
    pads: &[String],
) -> Vec<Keyed<PlayerPage>> {
    let vm = &editor.vm;
    let executor = editor.executor.clone();
    let tick = editor.binding(vm.revision.clone(), Clone::clone, None);
    let reset_label = editor.tr(UiText::ResetScreen);
    (0..2)
        .map(|port| {
            let label = editor
                .tr(if port == 0 {
                    UiText::PlayerOne
                } else {
                    UiText::PlayerTwo
                })
                .get();
            let radio = gamepad_radio(editor, port, &tick, pads);
            let radio_visible = !pads.is_empty();
            // One note line, switching with enumeration: without pads it
            // names the missing hardware, otherwise it points at the
            // Gamepads tab (deadzone and mappings live there).
            let note = if radio_visible {
                editor.tr(UiText::DescGamepadTab)
            } else {
                editor.tr(UiText::DescNoGamepads)
            };
            let keyboard = keyboard_toggle(editor, port);
            let slots = slot_rows_for_port(editor, port);
            let resets = vec![Keyed::new(
                "reset".to_string(),
                Store::new(
                    ResetRow {
                        label: reset_label.clone(),
                        reset: reset(super::ResetScope::Player(port)),
                    },
                    executor.clone(),
                ),
            )];
            Keyed::new(
                format!("player-{port}"),
                Store::new(
                    PlayerPage {
                        label,
                        radio: Some(radio),
                        radio_visible,
                        note,
                        note_visible: true,
                        keyboard: Some(keyboard),
                        slots,
                        resets,
                    },
                    executor.clone(),
                ),
            )
        })
        .collect()
}

/// Built-in keyboard membership toggle for one player port. Routing is
/// enforced at rebuild time, so flipping this takes effect on apply;
/// keyboards share ports freely, so the other port keeps working.
fn keyboard_toggle(editor: &Rc<Editor>, port: usize) -> Keyed<CheckRow> {
    use nerust_input_traits::BUILTIN_KEYBOARD_MEMBER;
    use nerust_settings_core::model::app_state::{PORT_PLAYER_1, PORT_PLAYER_2};

    let vm = &editor.vm;
    let port_id = if port == 0 {
        PORT_PLAYER_1
    } else {
        PORT_PLAYER_2
    }
    .to_string();
    let read_vm = vm.clone();
    let write_vm = vm.clone();
    let write_port = port_id.clone();
    let checked = editor.binding(
        vm.revision.clone(),
        move |_| {
            read_vm
                .snapshot()
                .app_state
                .port_devices
                .get(&port_id)
                .is_some_and(|members| members.iter().any(|m| m == BUILTIN_KEYBOARD_MEMBER))
        },
        Some(Rc::new(move |enabled: bool| {
            write_vm.restore_scope(&write_vm.snapshot(), |draft, _| {
                let members = draft
                    .app_state
                    .port_devices
                    .entry(write_port.clone())
                    .or_default();
                if enabled {
                    if !members.iter().any(|m| m == BUILTIN_KEYBOARD_MEMBER) {
                        members.push(BUILTIN_KEYBOARD_MEMBER.to_string());
                    }
                } else {
                    members.retain(|member| member != BUILTIN_KEYBOARD_MEMBER);
                }
            })
        })),
    );
    Keyed::new(
        format!("keyboard-{port}"),
        Store::new(
            CheckRow {
                label: editor.tr(UiText::BuiltInKeyboard),
                checked,
            },
            editor.executor.clone(),
        ),
    )
}

/// Controller-type slot rows for one player port: every machine slot
/// whose resolved port matches. Ports are global, so the same slot
/// never renders on two player pages.
fn slot_rows_for_port(editor: &Rc<Editor>, port: usize) -> Vec<Keyed<ChoiceRow>> {
    let vm = &editor.vm;
    let executor = editor.executor.clone();
    vm.inputs()
        .iter()
        .enumerate()
        .flat_map(|(system_index, input)| {
            let value = input.view.get();
            value
                .slots
                .iter()
                .filter(|slot| slot.port == Some(port))
                .enumerate()
                .map(|(slot_index, slot)| {
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
                        format!("system-{system_index}-slot-{slot_index}"),
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
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Gamepad radio for one player port: Not assigned plus one entry per
/// enumerated pad (live index as the value). The assignment writes
/// through the settings table with rival clearing; unassigning clears
/// every same-port entry. Disconnected ranks read as unassigned until
/// they reconnect (stable identity arrives with the backend).
fn gamepad_radio(
    editor: &Rc<Editor>,
    port: usize,
    tick: &Binding<u64>,
    pads: &[String],
) -> Keyed<ChoiceRow> {
    let vm = &editor.vm;
    let count = pads.len();
    let lang = vm.general.clone();
    let items: Binding<Vec<ChoiceView<Option<usize>>>> = tick.clone().map(move |_| {
        let i18n = I18n::new(crate::display_language(lang.view.get().language));
        let mut out = vec![ChoiceView {
            value: None,
            label: i18n.text(UiText::NotAssigned),
        }];
        for index in 0..count {
            out.push(ChoiceView {
                value: Some(index),
                label: pad_label(i18n, index),
            });
        }
        out
    });
    let gamepads_read = vm.gamepads.clone();
    let selected: Binding<Option<usize>> = editor.binding(
        vm.revision.clone(),
        move |_| {
            let table = &gamepads_read.view.get().pads;
            table
                .iter()
                .position(|pad| pad.port == Some(port))
                .filter(|rank| *rank < count)
        },
        None,
    );
    let gamepads_write = vm.gamepads.clone();
    let selected_string: Binding<String> = selected.convert(
        move |value| {
            value
                .map(|index| index.to_string())
                .unwrap_or_else(|| "none".to_string())
        },
        move |id: String| -> Result<Option<usize>, String> {
            if id == "none" {
                let len = gamepads_write.view.get().pads.len();
                for index in 0..len {
                    if gamepads_write.view.get().pads[index].port == Some(port) {
                        gamepads_write
                            .set_pad_port(index, None)
                            .map_err(|error| error.to_string())?;
                    }
                }
                Ok(None)
            } else if let Ok(pad) = id.parse::<usize>() {
                gamepads_write
                    .set_pad_port(pad, Some(port))
                    .map(|_| Some(pad))
                    .map_err(|error| error.to_string())
            } else {
                // Ids come from our own items list; unreachable.
                Ok(None)
            }
        },
    );
    let selection = super::ChoiceBinding {
        items: items.map(|views| {
            views
                .into_iter()
                .map(|view| Choice {
                    id: view
                        .value
                        .map(|index| index.to_string())
                        .unwrap_or_else(|| "none".to_string()),
                    label: view.label,
                })
                .collect()
        }),
        selected: selected_string,
    };
    Keyed::new(
        format!("radio-{port}"),
        Store::new(
            ChoiceRow {
                label: editor.tr(UiText::GamepadDevice),
                choices: selection.items,
                selected: selection.selected,
                enabled: Binding::constant(true),
            },
            editor.executor.clone(),
        ),
    )
}

/// Pad blocks for the Gamepads page: one block per enumerated pad.
/// Assignment is display-only here (the Controllers radio owns it);
/// deadzone, D-pad source, and per-system mappings edit in place.
/// Unassigned pads show the pick-a-player note instead of mappings.
pub(super) fn gamepads_blocks(
    editor: &Rc<Editor>,
    capture: &Capture,
    clear: &Capture,
    reset: &Reset,
    pads: &[String],
) -> Vec<Keyed<GamepadBlock>> {
    let vm = &editor.vm;
    let executor = editor.executor.clone();
    let tick = editor.binding(vm.revision.clone(), Clone::clone, None);
    let reset_label = editor.tr(UiText::ResetScreen);
    let no_player_note = editor.tr(UiText::DescControllersTab);
    pads.iter()
        .enumerate()
        .map(|(pad, _)| {
            let lang = vm.general.clone();
            let label = tick.clone().map(move |_| {
                pad_label(
                    I18n::new(crate::display_language(lang.view.get().language)),
                    pad,
                )
            });
            let gamepads = vm.gamepads.clone();
            let lang = vm.general.clone();
            let assignment = tick.clone().map(move |_| {
                let i18n = I18n::new(crate::display_language(lang.view.get().language));
                match gamepads.view.get().pads.get(pad).and_then(|row| row.port) {
                    Some(0) => i18n.text(UiText::PlayerOne),
                    Some(1) => i18n.text(UiText::PlayerTwo),
                    _ => i18n.text(UiText::NotAssigned),
                }
            });
            let deadzone = pad_deadzone_row(editor, pad, &tick);
            let dpad = pad_dpad_row(editor, pad, &tick);
            let sections = pad_mapping_sections(editor, pad, &tick, capture, clear);
            let gamepads = vm.gamepads.clone();
            let note_visible = tick.clone().map(move |_| {
                gamepads
                    .view
                    .get()
                    .pads
                    .get(pad)
                    .and_then(|row| row.port)
                    .is_none()
            });
            let resets = vec![Keyed::new(
                "reset".to_string(),
                Store::new(
                    ResetRow {
                        label: reset_label.clone(),
                        reset: reset(super::ResetScope::Pad(pad)),
                    },
                    executor.clone(),
                ),
            )];
            Keyed::new(
                format!("pad-{pad}"),
                Store::new(
                    GamepadBlock {
                        label,
                        assignment,
                        deadzone: Some(deadzone),
                        dpad: Some(dpad),
                        sections,
                        note: no_player_note.clone(),
                        note_visible,
                        resets,
                    },
                    executor.clone(),
                ),
            )
        })
        .collect()
}

/// Deadzone choice row for one enumerated pad. The choices ride the
/// tab-level list; the value falls back to the default past the end of
/// the persisted table (writes grow the table on demand).
fn pad_deadzone_row(editor: &Rc<Editor>, pad: usize, tick: &Binding<u64>) -> Keyed<ChoiceRow> {
    let vm = &editor.vm;
    let deadzone_choices = vm.gamepads.view.get().deadzone_choices.clone();
    let zone_items = Binding::constant(deadzone_choices);
    let zones = vm.gamepads.clone();
    let zone_selected = editor.binding(
        vm.gamepads.view.clone(),
        move |value: &GamepadTabView| {
            value
                .pads
                .iter()
                .find(|row| row.index == pad)
                .map(|row| row.deadzone)
                .unwrap_or(DEFAULT_DEADZONE_PERCENT)
        },
        Some(Rc::new(move |percent: u8| zones.set_deadzone(pad, percent))
            as Rc<dyn Fn(u8) -> Result<(), ViewModelError>>),
    );
    let selection = choice(zone_items, zone_selected);
    let lang = vm.general.clone();
    Keyed::new(
        format!("pad-{pad}-deadzone"),
        Store::new(
            ChoiceRow {
                label: tick.clone().map(move |_| {
                    format!(
                        "{} deadzone",
                        pad_label(
                            I18n::new(crate::display_language(lang.view.get().language)),
                            pad
                        )
                    )
                }),
                choices: selection.items,
                selected: selection.selected,
                enabled: Binding::constant(true),
            },
            editor.executor.clone(),
        ),
    )
}

/// D-pad source toggle as an On/Off choice row (the Desktop dialog has
/// no switch widget; choice rows already carry change tracking).
fn pad_dpad_row(editor: &Rc<Editor>, pad: usize, tick: &Binding<u64>) -> Keyed<ChoiceRow> {
    let vm = &editor.vm;
    let lang = vm.general.clone();
    let items: Binding<Vec<ChoiceView<bool>>> = tick.clone().map(move |_| {
        let i18n = I18n::new(crate::display_language(lang.view.get().language));
        vec![
            ChoiceView {
                value: false,
                label: i18n.text(UiText::Off),
            },
            ChoiceView {
                value: true,
                label: i18n.text(UiText::On),
            },
        ]
    });
    let pads = vm.gamepads.clone();
    let selected = editor.binding(
        vm.gamepads.view.clone(),
        move |value: &GamepadTabView| {
            value
                .pads
                .iter()
                .find(|row| row.index == pad)
                .map(|row| row.dpad_enabled)
                .unwrap_or(true)
        },
        Some(
            Rc::new(move |enabled: bool| pads.set_dpad_enabled(pad, enabled))
                as Rc<dyn Fn(bool) -> Result<(), ViewModelError>>,
        ),
    );
    let selection = choice(items, selected);
    let lang = vm.general.clone();
    Keyed::new(
        format!("pad-{pad}-dpad"),
        Store::new(
            ChoiceRow {
                label: tick.clone().map(move |_| {
                    I18n::new(crate::display_language(lang.view.get().language))
                        .text(UiText::DpadSource)
                }),
                choices: selection.items,
                selected: selection.selected,
                enabled: Binding::constant(true),
            },
            editor.executor.clone(),
        ),
    )
}

/// Per-system pad mapping sections for one pad: one section per machine,
/// rows scoped to the pad's own player (unassigned pads map nothing).
/// The physical hat stays fixed (its switch lives with the pad
/// settings), so dpad rows never render here.
fn pad_mapping_sections(
    editor: &Rc<Editor>,
    pad: usize,
    tick: &Binding<u64>,
    capture: &Capture,
    clear: &Capture,
) -> Binding<Vec<Keyed<BindingPage>>> {
    let vm = &editor.vm;
    let inputs: Vec<_> = vm.inputs().to_vec();
    let gamepads = vm.gamepads.clone();
    let sections = tick.clone().map(move |_| {
        let slot = gamepads.view.get().pads.get(pad).and_then(|row| row.port);
        let mut out = Vec::new();
        for input in &inputs {
            if let Some(section) = pad_section_for_input(input, pad, slot) {
                out.push(section);
            }
        }
        out
    });
    binding_sections(sections, editor, capture, clear)
}

/// One machine's pad mapping section for one pad: rows scoped to the
/// pad's own player (unassigned pads map nothing). `None` keeps the
/// machine off the block. The physical hat stays fixed (its switch
/// lives with the pad settings), so dpad rows never render here.
fn pad_section_for_input(
    input: &InputSettingsViewModel,
    pad: usize,
    slot: Option<usize>,
) -> Option<BindingSectionView> {
    let value = input.view.get();
    let slot = slot?;
    // Collect the section's targets first so values resolve under one
    // borrow; rows render from the zipped results.
    let mut pending: Vec<(
        CaptureTarget,
        Option<nerust_input_traits::StandardButton>,
        String,
    )> = Vec::new();
    for binding in value.sections.iter().flat_map(|section| &section.rows) {
        if matches!(
            binding.pad_default,
            Some(
                nerust_input_traits::StandardButton::DpadUp
                    | nerust_input_traits::StandardButton::DpadDown
                    | nerust_input_traits::StandardButton::DpadLeft
                    | nerust_input_traits::StandardButton::DpadRight
            )
        ) {
            continue;
        }
        let (Some(attachment), Some(control)) =
            (binding.target.attachment(), binding.target.control())
        else {
            continue;
        };
        if binding.port.is_some_and(|port| port != slot) {
            continue;
        }
        pending.push((
            CaptureTarget::PadBinding {
                system: value.system_id.clone_box(),
                pad,
                attachment: attachment.to_string(),
                control: control.to_string(),
            },
            binding.pad_default,
            binding.label.clone(),
        ));
    }
    let values = input.pad_row_values(
        &pending
            .iter()
            .map(|(target, default, _)| (target.clone(), *default))
            .collect::<Vec<_>>(),
    );
    let mut section_rows: Vec<BindingRowView> = Vec::new();
    for ((target, _, label), value) in pending.into_iter().zip(values) {
        let text = match &value {
            BindingValueView::Unbound(label)
            | BindingValueView::Bound(label)
            | BindingValueView::Capturing(label) => label.clone(),
        };
        section_rows.push(BindingRowView {
            target,
            label,
            port: Some(slot),
            value: BindingValueView::Bound(text),
            pad_default: None,
        });
    }
    (!section_rows.is_empty()).then(|| BindingSectionView {
        label: value.label.clone(),
        rows: section_rows,
    })
}

/// Keyboard-only binding sections for one machine tab: the view's own
/// sections, never the per-pad appendages (those live on Gamepads).
fn keyboard_sections(
    editor: &Rc<Editor>,
    input: &InputSettingsViewModel,
    tick: &Binding<u64>,
    capture: &Capture,
    clear: &Capture,
) -> Binding<Vec<Keyed<BindingPage>>> {
    let view = input.view.clone();
    let sections = tick.clone().map(move |_| view.get().sections.clone());
    binding_sections(sections, editor, capture, clear)
}

/// Single global shortcuts section, refreshed on every transact so
/// captures and language switches re-project without a rebuild.
/// A one-item Repeat (not ContentControl) keeps the established
/// item-template path; the reset button rides a sibling Repeat.
pub(super) fn shortcuts_page(
    editor: &Rc<Editor>,
    capture: &Capture,
    clear: &Capture,
) -> Binding<Vec<Keyed<BindingPage>>> {
    let vm = &editor.vm;
    let executor = editor.executor.clone();
    let tick = editor.binding(vm.revision.clone(), Clone::clone, None);
    let shortcuts = vm.shortcuts.clone();
    let section = tick.map(move |_| Some(shortcuts.view.get()));
    let section_inner = section.clone();
    let editor = editor.clone();
    let capture = capture.clone();
    let clear = clear.clone();
    section.map(move |_| {
        vec![Keyed::new(
            "global".to_string(),
            Store::new(
                binding_page(section_inner.clone(), &editor, &capture, &clear),
                executor.clone(),
            ),
        )]
    })
}

pub(super) fn systems(
    editor: &Rc<Editor>,
    capture: &Capture,
    clear: &Capture,
    reset: &Reset,
) -> Vec<Keyed<SystemPage>> {
    let vm = &editor.vm;
    let executor = editor.executor.clone();
    let tick = editor.binding(vm.revision.clone(), Clone::clone, None);
    let reset_label = editor.tr(UiText::ResetScreen);
    // The two resets on one machine page share the base label; the keys
    // one names its scope so the pair stays distinct.
    let reset_keys_label = reset_label
        .clone()
        .zip(&editor.tr(UiText::KeyMapping))
        .map(|(reset, section)| format!("{reset} ({section})"));
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
            let system_id = system.system_id().to_string();
            let resets = [
                (
                    "reset-fields",
                    super::ResetScope::SystemFields(system_id.clone()),
                    reset_label.clone(),
                ),
                (
                    "reset-keys",
                    super::ResetScope::SystemKeys(system_id),
                    reset_keys_label.clone(),
                ),
            ]
            .into_iter()
            .map(|(key, scope, label)| {
                Keyed::new(
                    key.to_string(),
                    Store::new(
                        ResetRow {
                            label,
                            reset: reset(scope),
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
                        key_sections: keyboard_sections(
                            editor,
                            vm.inputs()
                                .iter()
                                .find(|input| {
                                    input.system_id().to_string()
                                        == system.system_id().to_string()
                                })
                                .expect("input view exists for its own system"),
                            &tick,
                            capture,
                            clear,
                        ),
                        resets,
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

    /// Enabled no-op reset factory for layout-only tests.
    fn noop_reset(executor: &Rc<dyn panechord::Executor>) -> Reset {
        let flag = Store::new(String::new(), executor.clone());
        Rc::new(move |_: crate::settings::ResetScope| {
            flag.command(
                |_: &String| true,
                |_: &Store<String>, ()| {},
                Binding::constant(()),
            )
        })
    }

    fn test_pads() -> Vec<String> {
        vec!["pad-a".to_string(), "pad-b".to_string()]
    }

    /// Gamepads blocks render one block per enumerated pad (deadzone,
    /// D-pad source, per-system mappings), and writing a row routes
    /// through the gamepad view model. Pre-backend enumeration is empty.
    #[test]
    fn gamepads_blocks_list_per_pad_settings() {
        let (editor, _adapter, _dir) = headless_editor();
        let executor: Rc<dyn panechord::Executor> = Rc::new(TestExecutor);
        let capture = noop_capture(&executor);
        let clear = noop_capture(&executor);
        let reset = noop_reset(&executor);
        // No backend pads enumerated: no blocks, like the live dialog.
        assert!(
            gamepads_blocks(&editor, &capture, &clear, &reset, &[]).is_empty(),
            "no enumerated pads means no blocks"
        );
        let blocks = gamepads_blocks(&editor, &capture, &clear, &reset, &test_pads());
        assert_eq!(blocks.len(), 2, "one block per enumerated pad");
        let snapshot = blocks[0].model.snapshot();
        assert!(!snapshot.assignment.get().is_empty());
        assert_eq!(snapshot.label.get(), "Gamepad 1");
        assert_eq!(
            snapshot.deadzone.as_ref().expect("deadzone row builds").key,
            "pad-0-deadzone"
        );
        assert_eq!(
            snapshot.dpad.as_ref().expect("dpad row builds").key,
            "pad-0-dpad"
        );
        assert!(!snapshot.resets.is_empty(), "pad blocks carry a reset");
        // Assigned pads map instead of noting; unassigning shows the note.
        assert!(
            !snapshot.note_visible.get(),
            "assigned pads map instead of noting"
        );
        editor
            .vm
            .gamepads
            .set_pad_port(0, None)
            .expect("unassign applies");
        assert!(
            gamepads_blocks(&editor, &capture, &clear, &reset, &test_pads())[0]
                .model
                .snapshot()
                .note_visible
                .get(),
            "unassigned pads show the pick-a-player note"
        );
        editor
            .vm
            .gamepads
            .set_pad_port(0, Some(0))
            .expect("port restores");
        // Deadzone writes route through the view model and grow the table.
        let zone = snapshot
            .deadzone
            .as_ref()
            .expect("deadzone row builds")
            .model
            .snapshot();
        assert!(zone.selected.set("2".to_string()));
        assert_eq!(editor.vm.gamepads.view.get().pads[0].deadzone, 10);
    }

    /// Player pages carry the gamepad radio plus the machines' slots for
    /// their own port only. The radio is hidden while nothing is
    /// enumerated, and assigning one player parks rivals on the other.
    #[test]
    fn controllers_pages_scope_slots_and_radio_to_their_port() {
        let (editor, _adapter, _dir) = headless_editor();
        let executor: Rc<dyn panechord::Executor> = Rc::new(TestExecutor);
        let reset = noop_reset(&executor);
        let pages = controllers_pages(&editor, &reset, &[]);
        assert_eq!(pages.len(), 2, "one page per global port");
        for (index, page) in pages.iter().enumerate() {
            let snapshot = page.model.snapshot();
            assert_eq!(snapshot.label, format!("Player {}", index + 1));
            assert!(
                !snapshot.radio_visible,
                "radio hides with no enumerated pads"
            );
            assert!(
                !snapshot.note.get().is_empty(),
                "unconnected ports explain themselves"
            );
            assert!(
                !snapshot.slots.is_empty(),
                "player pages keep their machine slots"
            );
            assert_eq!(snapshot.resets.len(), 1, "one reset per player page");
        }
        // With enumerated pads the radio lists Not assigned plus each pad.
        // The default draft parks pad 0 on player 1.
        let pages = controllers_pages(&editor, &reset, &test_pads());
        let radio = pages[0]
            .model
            .snapshot()
            .radio
            .as_ref()
            .expect("radio builds")
            .model
            .snapshot();
        assert_eq!(radio.choices.get().len(), 3, "none plus two pads");
        assert_eq!(radio.selected.get(), "0", "pad 0 drives player 1");
        // Assigning pad 0 to player 2 clears its player-1 slot: the radio
        // re-reads as unassigned on this page.
        editor
            .vm
            .gamepads
            .set_pad_port(0, Some(1))
            .expect("port applies");
        let pages = controllers_pages(&editor, &reset, &test_pads());
        let selected: Vec<String> = pages
            .iter()
            .map(|page| {
                page.model
                    .snapshot()
                    .radio
                    .as_ref()
                    .expect("radio builds")
                    .model
                    .snapshot()
                    .selected
                    .get()
            })
            .collect();
        assert_eq!(selected, vec!["none".to_string(), "0".to_string()]);
        editor
            .vm
            .gamepads
            .set_pad_port(0, Some(0))
            .expect("port restores");
    }

    /// Player pages carry a built-in keyboard toggle bound to port
    /// membership: unchecking parks the keyboard from that port (routing
    /// picks it up on apply), and a player reset restores the default.
    #[test]
    fn controllers_pages_keyboard_toggle_flips_port_membership() {
        use super::super::{Apply, ResetScope};
        use nerust_gui_runtime::settings::SettingsSnapshot;
        let (editor, _adapter, _dir) = headless_editor();
        let apply: Apply = Rc::new(|_: SettingsSnapshot| Ok(()));
        let quit: Rc<dyn Fn()> = Rc::new(|| {});
        let slot = Rc::new(RefCell::new(None));
        let commands = editor.commands(&apply, quit, &slot);
        let reset = &commands.reset;
        let keyboard = |pages: &[Keyed<PlayerPage>], index: usize| {
            pages[index]
                .model
                .snapshot()
                .keyboard
                .as_ref()
                .expect("keyboard toggle builds")
                .model
                .snapshot()
        };
        let pages = controllers_pages(&editor, reset, &test_pads());
        assert!(keyboard(&pages, 0).checked.get(), "builtin drives p1");
        assert!(keyboard(&pages, 1).checked.get(), "builtin drives p2");
        assert!(!keyboard(&pages, 0).label.get().is_empty());
        // Uncheck on player 1: membership drops only there.
        assert!(keyboard(&pages, 0).checked.set(false));
        let pages = controllers_pages(&editor, reset, &test_pads());
        assert!(!keyboard(&pages, 0).checked.get(), "p1 unchecks");
        assert!(keyboard(&pages, 1).checked.get(), "p2 keeps sharing");
        // A player-scoped reset restores the default membership.
        let _ = (reset.clone())(ResetScope::Player(0)).invoke();
        let pages = controllers_pages(&editor, reset, &test_pads());
        assert!(keyboard(&pages, 0).checked.get(), "reset restores p1");
    }

    /// System pages mirror the live registry: every factory system yields
    /// a labeled page whose rows track the view model selections, plus
    /// key-mapping sections and per-scope resets.
    #[test]
    fn systems_pages_expose_live_choices_and_viewports() {
        let (editor, _adapter, _dir) = headless_editor();
        let executor: Rc<dyn panechord::Executor> = Rc::new(TestExecutor);
        let capture = noop_capture(&executor);
        let clear = noop_capture(&executor);
        let reset = noop_reset(&executor);
        let pages = systems(&editor, &capture, &clear, &reset);
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
            assert!(
                !snapshot.key_sections.get().is_empty(),
                "machine pages carry their key tables"
            );
            assert_eq!(snapshot.resets.len(), 2, "fields and keys reset separately");
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
        let selected = systems(&editor, &capture, &clear, &reset)
            .iter()
            .flat_map(|page| page.model.snapshot().fields)
            .map(|row| row.model.snapshot())
            .find(|row| row.label.get() == field_label)
            .expect("flipped field builds a row")
            .selected
            .get();
        assert_eq!(selected, position);
    }

    /// Player pages carry the radio plus slots scoped to their own port;
    /// machine pages carry fields plus keyboard-only key tables, and the
    /// shortcuts page is one global section. Key tables never carry the
    /// per-pad appendages (those live on the Gamepads page).
    #[test]
    fn controllers_and_machine_pages_split_slots_and_key_tables() {
        let (editor, _adapter, _dir) = headless_editor();
        let executor: Rc<dyn panechord::Executor> = Rc::new(TestExecutor);
        let capture = noop_capture(&executor);
        let clear = noop_capture(&executor);
        let reset = noop_reset(&executor);
        let players = controllers_pages(&editor, &reset, &test_pads());
        assert_eq!(players.len(), 2, "one page per global port");
        // Every machine slot lands on exactly one player page.
        let mut slot_total = 0;
        for page in &players {
            let snapshot = page.model.snapshot();
            for slot in &snapshot.slots {
                let row = slot.model.snapshot();
                assert!(!row.label.get().is_empty());
                assert!(!row.choices.get().is_empty());
                assert!(!row.selected.get().is_empty());
                slot_total += 1;
            }
        }
        let expected: usize = editor
            .vm
            .inputs()
            .iter()
            .map(|input| {
                input
                    .view
                    .get()
                    .slots
                    .iter()
                    .filter(|slot| slot.port.is_some())
                    .count()
            })
            .sum();
        assert_eq!(
            slot_total, expected,
            "slots split across ports without loss"
        );
        // Machine pages keep keyboard-only sections (no pad appendages).
        let machines = systems(&editor, &capture, &clear, &reset);
        assert!(!machines.is_empty(), "real registry exposes systems");
        for page in &machines {
            let snapshot = page.model.snapshot();
            for section in snapshot.key_sections.get().iter() {
                assert!(
                    !section.key.starts_with("padbindings_"),
                    "machine pages never carry pad appendages: {}",
                    section.key
                );
            }
        }
        // Shortcuts stay a single global section with rows.
        let section = shortcuts_page(&editor, &capture, &clear);
        assert!(
            !section
                .get()
                .into_iter()
                .next()
                .expect("shortcuts section builds")
                .model
                .snapshot()
                .rows
                .get()
                .is_empty()
        );
        // Player radio writes route through the port assignment.
        let slot = players
            .iter()
            .flat_map(|page| page.model.snapshot().slots)
            .map(|slot| slot.model.snapshot())
            .find(|row| row.choices.get().len() > 1)
            .expect("a player slot has an alternative");
        let ids: Vec<String> = slot
            .choices
            .get()
            .into_iter()
            .map(|choice| choice.id)
            .collect();
        let original = slot.selected.get();
        let other = ids
            .iter()
            .find(|id| **id != original)
            .expect("slot has an alternative")
            .clone();
        assert!(slot.selected.set(other.clone()));
        assert_eq!(slot.selected.get(), other);
        assert!(slot.selected.set(original.clone()));
        assert_eq!(slot.selected.get(), original);
    }

    /// Binding rows localize their chrome live: switching the draft
    /// language re-renders Change/Clear without rebuilding the page.
    #[test]
    fn binding_rows_localize_chrome_with_draft_language() {
        use nerust_settings_core::i18n::I18n;
        use nerust_settings_core::model::language::AppLanguage;
        let (editor, _adapter, _dir) = headless_editor();
        let executor: Rc<dyn panechord::Executor> = Rc::new(TestExecutor);
        let capture = noop_capture(&executor);
        let clear = noop_capture(&executor);
        let page = shortcuts_page(&editor, &capture, &clear);
        let rows = page
            .get()
            .into_iter()
            .next()
            .expect("shortcuts section builds")
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
        let ja = I18n::new(AppLanguage::Japanese);
        assert_eq!(row.change_label.get(), ja.text(UiText::Change));
        assert_eq!(row.clear_label.get(), ja.text(UiText::Clear));
        // Row chrome delegates to the built capture/clear commands.
        let _ = row.capture.invoke();
        let _ = row.clear.invoke();
    }

    /// Binding row values re-project live: cleared shortcuts read back
    /// the unbound label and the capturing row reads back the prompt.
    #[test]
    fn binding_row_values_cover_unbound_and_capturing() {
        use nerust_settings_core::i18n::{I18n, UiText};
        use nerust_settings_core::model::input::ShortcutAction;
        let (editor, _adapter, _dir) = headless_editor();
        let executor: Rc<dyn panechord::Executor> = Rc::new(TestExecutor);
        let capture = noop_capture(&executor);
        let clear = noop_capture(&executor);
        let page = shortcuts_page(&editor, &capture, &clear);
        let rows = page
            .get()
            .into_iter()
            .next()
            .expect("shortcuts section builds")
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
        let unbound = I18n::new(crate::display_language(
            editor.vm.general.view.get().language,
        ))
        .text(UiText::Unbound);
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
                    port: None,
                    value: BindingValueView::Bound("Space".into()),
                    pad_default: None,
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

    /// Section keys must distinguish same-length attachments: the key used
    /// to fold attachment length only, so player1/player2 (both 22 chars
    /// on NES) collided and the backend rejected the whole TabItems update
    /// ("duplicate Tab page key"), blanking the tab.
    #[test]
    fn section_keys_distinguish_same_length_attachments() {
        use nerust_gui_viewmodel::settings::dto::BindingRowView;
        use nerust_settings_core::editor::CaptureTarget;
        let (editor, _adapter, _dir) = headless_editor();
        let executor: Rc<dyn panechord::Executor> = Rc::new(TestExecutor);
        let section = editor
            .vm
            .inputs()
            .iter()
            .flat_map(|input| input.view.get().sections)
            .find(|section| !section.rows.is_empty())
            .expect("a keyboard section exists");
        let variant = |attachment: &str| {
            let mut section = section.clone();
            for row in &mut section.rows {
                let BindingRowView { target, .. } = row;
                let (system, control) = match target {
                    CaptureTarget::Binding {
                        system, control, ..
                    } => ((*system).clone(), (*control).clone()),
                    other => panic!("keyboard row has unexpected target: {other:?}"),
                };
                *target = CaptureTarget::Binding {
                    system,
                    attachment: attachment.into(),
                    control,
                };
            }
            section.label = attachment.to_string();
            section
        };
        // Same length, different values: keys must differ.
        let first = variant("aa");
        let second = variant("bb");
        assert_ne!(section_key(&first, 0), section_key(&second, 1));
        let source = Store::new(vec![first, second], executor.clone());
        let pages = binding_sections(
            source.bind(|sections| sections.clone(), None),
            &editor,
            &noop_capture(&executor),
            &noop_capture(&executor),
        );
        let keys: Vec<String> = pages.get().into_iter().map(|page| page.key).collect();
        assert_eq!(keys.len(), 2);
        assert_ne!(keys[0], keys[1]);
    }

    /// Page and section keys across every dynamic TabItems stay unique
    /// under the default fixture: a single duplicate blanks the whole
    /// tab with "duplicate Tab page key".
    #[test]
    fn tab_item_keys_are_unique_across_pages_and_sections() {
        let (editor, _adapter, _dir) = headless_editor();
        let executor: Rc<dyn panechord::Executor> = Rc::new(TestExecutor);
        let capture = noop_capture(&executor);
        let clear = noop_capture(&executor);
        let reset = noop_reset(&executor);
        let mut seen_pages = std::collections::BTreeSet::new();
        for page in &systems(&editor, &capture, &clear, &reset) {
            assert!(
                seen_pages.insert(page.key.clone()),
                "duplicate machine page key {}",
                page.key
            );
            let sections = page.model.snapshot().key_sections.get();
            let mut seen_sections = std::collections::BTreeSet::new();
            for section in &sections {
                assert!(
                    seen_sections.insert(section.key.clone()),
                    "duplicate section key {} in page {}",
                    section.key,
                    page.key
                );
            }
        }
        for page in &controllers_pages(&editor, &reset, &test_pads()) {
            assert!(
                seen_pages.insert(page.key.clone()),
                "duplicate player page key {}",
                page.key
            );
        }
        for block in &gamepads_blocks(&editor, &capture, &clear, &reset, &test_pads()) {
            assert!(
                seen_pages.insert(block.key.clone()),
                "duplicate gamepad block key {}",
                block.key
            );
            let sections = block.model.snapshot().sections.get();
            let mut seen_sections = std::collections::BTreeSet::new();
            for section in &sections {
                assert!(
                    seen_sections.insert(section.key.clone()),
                    "duplicate section key {} in block {}",
                    section.key,
                    block.key
                );
            }
        }
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
