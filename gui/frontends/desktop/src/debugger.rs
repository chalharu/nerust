use nerust_gui_viewmodel::debugger::format::format_dump_address;
use nerust_gui_viewmodel::debugger::{DisplayCache, NavState, format_disasm_line, format_register};
use nerust_settings_core::{
    i18n::{UiText, text},
    model::language::AppLanguage,
};
use panechord::{Binding, Command, Executor, Store, View};
use panechord::{
    Table, TableCell, TableColumn, TableRow, TableSelection, TextOverflow, TextRole, TextStyle,
};
use std::{cell::RefCell, collections::BTreeMap, ops::Deref, rc::Rc, sync::Arc};

type MemoryTargets = BTreeMap<String, (nerust_core_traits::debugger::HexRow, bool)>;
type RegisterTargets = BTreeMap<String, (&'static str, u64)>;

fn register_key(name: &str) -> String {
    name.bytes().map(|byte| format!("{byte:02X}")).collect()
}

fn synchronize_registers(model: &mut Model) {
    let next = model
        .register_left_selection
        .as_ref()
        .filter(|selection| Some(*selection) != model.register_marked.as_ref())
        .or(model.register_right_selection.as_ref())
        .or(model.register_left_selection.as_ref())
        .filter(|selection| {
            selection.column == 0 && model.register_targets.contains_key(&selection.row_key)
        })
        .cloned();
    let selected = next
        .as_ref()
        .and_then(|selection| model.register_targets.get(&selection.row_key));
    model.reg_target = selected.map_or_else(
        || text(model.language, UiText::PickRegister).to_owned(),
        |(name, _)| format!("{} {name}", text(model.language, UiText::Set)),
    );
    if model.register_left_selection != next {
        model.register_left_selection = None;
    } else {
        model.register_right_selection = None;
    }
    model.register_marked = next;
}

fn selected_memory(
    selection: &TableSelection,
    targets: &MemoryTargets,
) -> Option<(u32, Option<u8>)> {
    let (row, _) = targets.get(&selection.row_key)?;
    let index = selection.column.saturating_sub(1);
    let byte = row.bytes.get(index)?;
    let addr = row.addr.checked_add(index as u32)?;
    Some((addr, (index < usize::from(row.valid)).then_some(*byte)))
}

#[derive(Clone)]
pub(crate) struct Projection {
    pub(crate) disassembly: Table,
    pub(crate) registers: [Table; 2],
    memory: Table,
    memory_addr: String,
    memory_targets: Arc<MemoryTargets>,
    register_targets: Arc<RegisterTargets>,
    pc: Option<TableSelection>,
    register_style: panechord::Style,
    registers_visible: bool,
}

fn code_table(rows: impl IntoIterator<Item = (String, String)>) -> Table {
    Table {
        columns: vec![TableColumn {
            title: String::new(),
            width: None,
            text: TextStyle {
                role: TextRole::Code,
                ..Default::default()
            },
            overflow: TextOverflow::Clip,
            stretch: true,
            hide_empty: false,
        }]
        .into(),
        rows: rows
            .into_iter()
            .map(|(key, text)| {
                Arc::new(TableRow {
                    key,
                    cells: vec![TableCell {
                        text,
                        foreground: None,
                    }],
                })
            })
            .collect::<Vec<_>>()
            .into(),
        empty_text: String::new(),
        column_gap: 0.0,
        row_gap: 0.0,
        row_inset: 2.0,
    }
}

pub(crate) fn project(display: &DisplayCache) -> Projection {
    let midpoint = display.regs.len().div_ceil(2);
    let registers = [&display.regs[..midpoint], &display.regs[midpoint..]].map(|registers| {
        let mut table = code_table(
            registers
                .iter()
                .map(|&(name, value)| (register_key(name), format_register(name, value))),
        );
        Arc::make_mut(&mut table.columns)[0].stretch = false;
        table
    });
    let mut disassembly = code_table(display.disasm_lines.iter().map(|line| {
        (
            format!("{:08X}", line.addr),
            format!(
                "{}{}",
                if line.is_pc { "> " } else { "  " },
                format_disasm_line(line)
            ),
        )
    }));
    let mut columns = disassembly.columns.to_vec();
    columns.push(TableColumn {
        title: String::new(),
        width: Some(40.0),
        text: TextStyle::default(),
        overflow: TextOverflow::Clip,
        stretch: false,
        hide_empty: true,
    });
    disassembly.columns = columns.into();
    disassembly.column_gap = 4.0;
    for (row, line) in Arc::make_mut(&mut disassembly.rows)
        .iter_mut()
        .zip(&display.disasm_lines)
    {
        Arc::make_mut(row).cells.push(TableCell {
            text: if line.target.is_some() {
                "\u{2192}".into()
            } else {
                String::new()
            },
            foreground: None,
        });
    }
    Projection {
        registers,
        disassembly,
        pc: display
            .disasm_lines
            .iter()
            .find(|line| line.is_pc)
            .map(|line| TableSelection {
                row_key: format!("{:08X}", line.addr),
                column: 0,
            }),
        memory: Table {
            columns: (0..17)
                .map(|_| TableColumn {
                    title: String::new(),
                    width: None,
                    text: TextStyle {
                        role: TextRole::Code,
                        ..Default::default()
                    },
                    overflow: TextOverflow::Clip,
                    stretch: false,
                    hide_empty: true,
                })
                .collect::<Vec<_>>()
                .into(),
            rows: display
                .dump_rows
                .iter()
                .map(|row| {
                    let mut cells = vec![TableCell {
                        text: format_dump_address(
                            row.addr,
                            false,
                            display.diff.contains(&row.addr),
                        ),
                        foreground: None,
                    }];
                    cells.extend(row.bytes.iter().enumerate().map(|(index, byte)| TableCell {
                        text: if index < usize::from(row.valid) {
                            format!("{byte:02X}")
                        } else {
                            String::new()
                        },
                        foreground: None,
                    }));
                    Arc::new(TableRow {
                        key: format!("{:08X}", row.addr),
                        cells,
                    })
                })
                .collect::<Vec<_>>()
                .into(),
            empty_text: String::new(),
            column_gap: 2.0,
            row_gap: 0.0,
            row_inset: 2.0,
        },
        memory_addr: NavState::new(0, display.dump_rows.first().map_or(0, |row| row.addr))
            .mem_input()
            .to_owned(),
        memory_targets: Arc::new(
            display
                .dump_rows
                .iter()
                .map(|row| {
                    (
                        format!("{:08X}", row.addr),
                        (*row, display.diff.contains(&row.addr)),
                    )
                })
                .collect(),
        ),
        register_targets: Arc::new(
            display
                .regs
                .iter()
                .map(|&(name, value)| (register_key(name), (name, value)))
                .collect(),
        ),
        register_style: panechord::Style {
            placement: panechord::Placement {
                height: Some((midpoint as f32 * 38.0).clamp(1.0, 4096.0)),
                ..Default::default()
            },
            ..Default::default()
        },
        registers_visible: midpoint > 0,
    }
}

#[derive(Clone)]
struct Captions {
    title: String,
    pause: String,
    step_frame: String,
    step_instruction: String,
    ppu: String,
    disassembly: String,
    follow_pc: String,
    addr: String,
    go: String,
    registers: String,
    set: String,
    hex: String,
    watch: String,
    clear: String,
    freeze: String,
    hex_byte: String,
    bookmarks: String,
    label: String,
    add: String,
    memory: String,
    space: String,
    page: String,
    previous: String,
    next: String,
    write: String,
    confirm: String,
    cancel: String,
    refresh: String,
}
impl Captions {
    fn new(language: AppLanguage, paused: bool) -> Self {
        let label = |key| text(language, key).to_owned();
        Self {
            title: label(UiText::Debugger),
            pause: label(if paused {
                UiText::Resume
            } else {
                UiText::Pause
            }),
            step_frame: label(UiText::StepFrame),
            step_instruction: label(UiText::StepInstr),
            ppu: label(UiText::PpuViewer),
            disassembly: format!(
                "{} {}",
                label(UiText::Disassembly),
                label(UiText::FollowPcTag)
            ),
            follow_pc: label(UiText::FollowPc),
            addr: label(UiText::AddrHex),
            go: label(UiText::Go),
            registers: label(UiText::Registers),
            set: label(UiText::Set),
            hex: label(UiText::Hex),
            watch: label(UiText::Watch),
            clear: label(UiText::Clear),
            freeze: label(UiText::Freeze),
            hex_byte: label(UiText::HexByte),
            bookmarks: label(UiText::Bookmarks),
            label: label(UiText::Label),
            add: label(UiText::Add),
            memory: label(UiText::Memory),
            space: label(UiText::Space),
            page: label(UiText::Page),
            previous: label(UiText::Prev),
            next: label(UiText::Next),
            write: label(UiText::Write),
            confirm: label(UiText::Confirm),
            cancel: label(UiText::Cancel),
            refresh: label(UiText::Refresh),
        }
    }
}

#[derive(Clone)]
struct Model {
    language: AppLanguage,
    projection_revision: u64,
    memory_targets: Arc<MemoryTargets>,
    memory_marked: Option<String>,
    register_targets: Arc<RegisterTargets>,
    register_marked: Option<TableSelection>,
    captions: Captions,
    disassembly: Table,
    disassembly_selection: Option<TableSelection>,
    register_left_selection: Option<TableSelection>,
    register_right_selection: Option<TableSelection>,
    memory_selection: Option<TableSelection>,
    registers_left: Table,
    registers_right: Table,
    register_style: panechord::Style,
    registers_visible: bool,
    memory: Table,
    bookmarks: Table,
    status: String,
    back: String,
    reg_target: String,
    selected: String,
    watch: String,
    freeze: String,
    space: String,
    disasm_addr: String,
    memory_addr: String,
    register_value: String,
    edit_value: String,
    freeze_value: String,
    bookmark_label: String,
    confirmation: String,
    pending: bool,
    enabled: bool,
}

pub(crate) struct Pane {
    model: Store<Model>,
    flight: RefCell<Option<nerust_gui_shell::debug::DrainFlight>>,
    selection: RefCell<Option<panechord::Subscription>>,
}
impl Pane {
    pub(crate) fn new(language: AppLanguage, executor: Rc<dyn Executor>) -> Rc<Self> {
        let projection = project(&DisplayCache::default());
        let [registers_left, registers_right] = projection.registers;
        let label = |key| text(language, key).to_owned();
        let pane = Rc::new(Self {
            model: Store::new(
                Model {
                    language,
                    projection_revision: 0,
                    memory_targets: projection.memory_targets,
                    memory_marked: None,
                    register_targets: projection.register_targets,
                    register_marked: None,
                    captions: Captions::new(language, false),
                    disassembly: projection.disassembly,
                    disassembly_selection: projection.pc,
                    register_left_selection: None,
                    register_right_selection: None,
                    memory_selection: None,
                    registers_left,
                    registers_right,
                    memory: projection.memory,
                    bookmarks: code_table([]),
                    register_style: projection.register_style,
                    registers_visible: projection.registers_visible,
                    status: label(UiText::NoCore),
                    back: format!("{} (0)", text(language, UiText::Back)),
                    reg_target: label(UiText::PickRegister),
                    selected: label(UiText::SelectDumpRow),
                    watch: label(UiText::WatchNone),
                    freeze: label(UiText::FreezeNone),
                    space: String::new(),
                    disasm_addr: String::new(),
                    memory_addr: projection.memory_addr,
                    register_value: String::new(),
                    edit_value: String::new(),
                    freeze_value: String::new(),
                    bookmark_label: String::new(),
                    confirmation: String::new(),
                    pending: false,
                    enabled: false,
                },
                executor,
            ),
            flight: RefCell::new(None),
            selection: RefCell::new(None),
        });
        let source = pane.model.bind(
            |model| {
                (
                    model.memory_selection.clone(),
                    model.register_left_selection.clone(),
                    model.register_right_selection.clone(),
                    model.language,
                    model.projection_revision,
                )
            },
            None::<fn(&mut Model, _)>,
        );
        let subscription = pane.model.watch(source, |store, _| {
            let _ = store.update(|model| {
                synchronize_registers(model);
                let selected = model
                    .memory_selection
                    .as_ref()
                    .and_then(|selection| selected_memory(selection, &model.memory_targets));
                model.selected = selected.map_or_else(
                    || text(model.language, UiText::SelectDumpRow).to_owned(),
                    |(addr, old)| {
                        nerust_settings_core::i18n::selected_row(model.language, addr, old)
                    },
                );
                model.captions.write = selected.map_or_else(
                    || text(model.language, UiText::Write).to_owned(),
                    |(addr, _)| format!("{} 0x{addr:08X}", text(model.language, UiText::Write)),
                );
                let next = selected.and(
                    model
                        .memory_selection
                        .as_ref()
                        .map(|selection| selection.row_key.clone()),
                );
                for key in [model.memory_marked.take(), next.clone()]
                    .into_iter()
                    .flatten()
                {
                    if let Some((row, changed)) = model.memory_targets.get(&key)
                        && let Some(index) = model.memory.rows.iter().position(|row| row.key == key)
                    {
                        let marker =
                            format_dump_address(row.addr, next.as_ref() == Some(&key), *changed);
                        if model.memory.rows[index].cells[0].text != marker {
                            Arc::make_mut(&mut Arc::make_mut(&mut model.memory.rows)[index])
                                .cells[0]
                                .text = marker;
                        }
                    }
                }
                model.memory_marked = next;
            });
        });
        pane.selection.replace(Some(subscription));
        pane
    }

    pub(crate) fn start(&self, space: String, flight: nerust_gui_shell::debug::DrainFlight) {
        if self.model.is_closed() {
            return;
        }
        self.flight.replace(Some(flight));
        let _ = self.model.update(move |model| model.space = space);
    }

    pub(crate) fn cancel(&self) {
        self.flight.borrow_mut().take();
    }

    pub(crate) fn poll(&self, language: AppLanguage) {
        if self.model.is_closed() {
            self.cancel();
            return;
        }
        let output = {
            let mut flight = self.flight.borrow_mut();
            match flight.as_mut().map(|flight| flight.poll()) {
                Some(nerust_gui_shell::debug::DrainPoll::Done(output)) => {
                    flight.take();
                    Some(*output)
                }
                _ => None,
            }
        };
        if let Some(output) = output {
            self.refresh(&output.display, language);
        }
    }

    pub(crate) fn refresh(&self, display: &DisplayCache, language: AppLanguage) {
        let projection = project(display);
        let [registers_left, registers_right] = projection.registers;
        let status = display.status.render(language);
        let captions = Captions::new(language, display.paused);
        let pending = display.pending.is_some();
        let confirmation = display
            .pending
            .map(|pending| pending.render(language))
            .unwrap_or_default();
        let _ = self.model.update(move |model| {
            model.language = language;
            model.projection_revision = model.projection_revision.wrapping_add(1);
            model.memory_targets = projection.memory_targets;
            model.register_targets = projection.register_targets;
            model.captions = captions;
            model.pending = pending;
            model.confirmation = confirmation;
            model.disassembly = projection.disassembly;
            model.disassembly_selection = projection.pc;
            model.registers_left = registers_left;
            model.registers_right = registers_right;
            if model
                .register_left_selection
                .as_ref()
                .is_some_and(|selection| selection.validate(&model.registers_left).is_err())
            {
                model.register_left_selection = None;
            }
            if model
                .register_right_selection
                .as_ref()
                .is_some_and(|selection| selection.validate(&model.registers_right).is_err())
            {
                model.register_right_selection = None;
            }
            model.register_style = projection.register_style;
            model.registers_visible = projection.registers_visible;
            model.memory = projection.memory;
            if model
                .memory_selection
                .as_ref()
                .is_some_and(|selection| selection.validate(&model.memory).is_err())
            {
                model.memory_selection = None;
            }
            model.memory_addr = projection.memory_addr;
            model.status = status;
        });
    }

    pub(crate) fn build(self: &Rc<Self>) -> Result<View, String> {
        let bindings = Presentation {
            model: self.model.clone(),
        };
        let view = panechord::try_view!("ui/debugger.yaml", bindings, panechord)?;
        let pane = self.clone();
        view.on_close(Rc::new(move || {
            pane.cancel();
            let selection = pane.selection.borrow_mut().take();
            drop(selection);
            pane.model.close();
        }));
        Ok(view)
    }
}

#[derive(Clone)]
struct Presentation {
    model: Store<Model>,
}
impl Deref for Presentation {
    type Target = Store<Model>;
    fn deref(&self) -> &Self::Target {
        &self.model
    }
}
impl Presentation {
    fn command_action(&self, argument: Binding<String>) -> Command {
        self.model
            .command(|model| model.enabled, |_, _: String| {}, argument)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoDispatch;
    impl Executor for NoDispatch {
        fn spawn(&self, _: std::pin::Pin<Box<dyn std::future::Future<Output = ()>>>) {
            panic!("selection projection must not dispatch async work");
        }
    }

    #[test]
    fn register_selection_is_exclusive_preserves_input_and_reprojects_language() {
        let pane = Pane::new(AppLanguage::English, Rc::new(NoDispatch));
        let display = DisplayCache {
            regs: vec![("First/Flags", 1), ("Second", 2), ("Third", 3)],
            ..Default::default()
        };
        pane.refresh(&display, AppLanguage::English);
        assert!(pane.model.update(|model| {
            model.register_value = "AB".into();
            model.register_left_selection = Some(TableSelection {
                row_key: register_key("First/Flags"),
                column: 0,
            });
        }));
        assert_eq!(pane.model.snapshot().reg_target, "Set First/Flags");
        assert!(pane.model.update(
            |model| model.register_right_selection = Some(TableSelection {
                row_key: register_key("Third"),
                column: 0
            })
        ));
        let model = pane.model.snapshot();
        assert!(model.register_left_selection.is_none());
        assert_eq!(model.reg_target, "Set Third");
        assert_eq!(model.register_value, "AB");
        assert!(pane.model.update(
            |model| model.register_left_selection = Some(TableSelection {
                row_key: register_key("Second"),
                column: 0
            })
        ));
        assert!(pane.model.snapshot().register_right_selection.is_none());
        pane.refresh(&display, AppLanguage::Japanese);
        assert_eq!(
            pane.model.snapshot().reg_target,
            format!("{} Second", text(AppLanguage::Japanese, UiText::Set))
        );
        assert_eq!(pane.model.snapshot().register_value, "AB");
        pane.refresh(&DisplayCache::default(), AppLanguage::English);
        assert!(pane.model.snapshot().register_marked.is_none());
        assert_eq!(
            pane.model.snapshot().reg_target,
            text(AppLanguage::English, UiText::PickRegister)
        );
        pane.model.close();
        assert!(!pane.model.update(
            |model| model.register_left_selection = Some(TableSelection {
                row_key: register_key("Second"),
                column: 0
            })
        ));
    }

    #[test]
    fn memory_selection_updates_marker_caption_and_clears_retired_targets() {
        let pane = Pane::new(AppLanguage::English, Rc::new(NoDispatch));
        let display = DisplayCache {
            dump_rows: vec![nerust_core_traits::debugger::HexRow {
                addr: 0x1000,
                valid: 2,
                bytes: [0xAB; 16],
            }],
            diff: vec![0x1000],
            ..Default::default()
        };
        pane.refresh(&display, AppLanguage::English);
        assert!(
            pane.model
                .update(|model| model.memory_selection = Some(TableSelection {
                    row_key: "00001000".into(),
                    column: 2
                }))
        );
        let model = pane.model.snapshot();
        assert_eq!(model.memory.rows[0].cells[0].text, ">*0x00001000:");
        assert_eq!(
            model.selected,
            nerust_settings_core::i18n::selected_row(AppLanguage::English, 0x1001, Some(0xAB))
        );
        assert_eq!(model.captions.write, "Write 0x00001001");
        pane.refresh(&display, AppLanguage::Japanese);
        assert_eq!(
            pane.model.snapshot().selected,
            nerust_settings_core::i18n::selected_row(AppLanguage::Japanese, 0x1001, Some(0xAB))
        );
        assert!(pane.model.update(|model| model.memory_selection = None));
        assert_eq!(
            pane.model.snapshot().memory.rows[0].cells[0].text,
            " *0x00001000:"
        );
        assert!(
            pane.model
                .update(|model| model.memory_selection = Some(TableSelection {
                    row_key: "00001000".into(),
                    column: 1
                }))
        );
        pane.refresh(&DisplayCache::default(), AppLanguage::English);
        assert!(pane.model.snapshot().memory_selection.is_none());
        assert_eq!(
            pane.model.snapshot().selected,
            text(AppLanguage::English, UiText::SelectDumpRow)
        );
    }

    #[test]
    fn memory_selection_resolves_structured_rows_not_display_text() {
        let row = nerust_core_traits::debugger::HexRow {
            addr: 0x1000,
            valid: 2,
            bytes: [0xAB; 16],
        };
        let targets = BTreeMap::from([("opaque-key".to_owned(), (row, false))]);
        assert_eq!(
            selected_memory(
                &TableSelection {
                    row_key: "opaque-key".into(),
                    column: 0
                },
                &targets
            ),
            Some((0x1000, Some(0xAB)))
        );
        assert_eq!(
            selected_memory(
                &TableSelection {
                    row_key: "opaque-key".into(),
                    column: 2
                },
                &targets
            ),
            Some((0x1001, Some(0xAB)))
        );
        assert_eq!(
            selected_memory(
                &TableSelection {
                    row_key: "opaque-key".into(),
                    column: 3
                },
                &targets
            ),
            Some((0x1002, None))
        );
        assert_eq!(
            selected_memory(
                &TableSelection {
                    row_key: "opaque-key".into(),
                    column: 99
                },
                &targets
            ),
            None
        );
    }

    #[test]
    fn captions_follow_pause_and_language_without_domain_actions() {
        assert_eq!(Captions::new(AppLanguage::English, true).pause, "Resume");
        assert_eq!(Captions::new(AppLanguage::English, false).pause, "Pause");
        assert!(
            Captions::new(AppLanguage::English, true)
                .disassembly
                .contains(text(AppLanguage::English, UiText::FollowPcTag))
        );
        assert_eq!(
            project(&DisplayCache::default()).memory_addr,
            NavState::new(0, 0).mem_input()
        );
    }

    #[test]
    fn projection_uses_structured_targets_and_master_row_geometry() {
        let mut display = DisplayCache::default();
        let line = nerust_core_traits::debugger::DisasmLine {
            addr: 0x100,
            bytes: [0; 4],
            len: 1,
            text: "operation".into(),
            is_pc: true,
            target: Some(0x200),
        };
        display.disasm_lines.push(line.clone());
        display
            .disasm_lines
            .push(nerust_core_traits::debugger::DisasmLine {
                addr: 0x101,
                is_pc: false,
                target: None,
                ..line
            });
        let projection = project(&display);
        assert_eq!(projection.disassembly.validate(), Ok(()));
        assert_eq!(projection.disassembly.column_gap, 4.0);
        assert_eq!(
            (
                projection.disassembly.row_inset,
                projection.disassembly.row_gap
            ),
            (2.0, 0.0)
        );
        assert_eq!(projection.disassembly.rows[0].cells[1].text, "\u{2192}");
        assert!(projection.disassembly.rows[1].cells[1].text.is_empty());
        assert_eq!(
            TableSelection {
                row_key: "00000100".into(),
                column: 1
            }
            .validate(&projection.disassembly),
            Ok(())
        );
        assert_eq!(
            TableSelection {
                row_key: "00000101".into(),
                column: 1
            }
            .validate(&projection.disassembly),
            Err("unknown table selection")
        );
    }

    #[test]
    fn projection_preserves_byte_cells_validity_and_change_markers() {
        let mut display = DisplayCache::default();
        display
            .dump_rows
            .push(nerust_core_traits::debugger::HexRow {
                addr: 0x1234,
                valid: 2,
                bytes: [0xAB; 16],
            });
        display.diff.push(0x1234);
        let table = project(&display).memory;
        assert_eq!(table.validate(), Ok(()));
        assert_eq!(table.columns.len(), 17);
        assert_eq!(table.rows[0].cells[0].text, " *0x00001234:");
        assert_eq!(table.rows[0].cells[1].text, "AB");
        assert_eq!(table.rows[0].cells[2].text, "AB");
        assert!(
            table.rows[0].cells[3..]
                .iter()
                .all(|cell| cell.text.is_empty())
        );
        assert_eq!(format_dump_address(0x1234, true, true), ">*0x00001234:");
        assert_eq!(format_dump_address(0x1234, true, false), "> 0x00001234:");
    }

    #[test]
    fn projection_preserves_register_order_and_shared_formatting() {
        let display = DisplayCache {
            regs: vec![("First/Flags", 1), ("Second", 2), ("Third", 3)],
            ..Default::default()
        };
        let projection = project(&display);
        assert_eq!(
            projection.registers[0].rows[0].cells[0].text,
            format_register("First/Flags", 1)
        );
        assert!(!projection.registers[0].rows[0].key.contains('/'));
        assert_eq!(projection.register_style.placement.height, Some(76.0));
        assert!(projection.registers_visible);
        assert_eq!(
            projection.registers[0].rows[1].cells[0].text,
            format_register("Second", 2)
        );
        assert_eq!(
            projection.registers[1].rows[0].cells[0].text,
            format_register("Third", 3)
        );
        for table in projection
            .registers
            .iter()
            .chain([&projection.disassembly, &projection.memory])
        {
            assert_eq!(table.validate(), Ok(()));
        }
    }
}
