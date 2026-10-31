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
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    ops::Deref,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

type MemoryTargets = BTreeMap<String, (nerust_core_traits::debugger::HexRow, bool)>;
type RegisterTargets = BTreeMap<String, (&'static str, u64)>;
pub(crate) type Begin = Rc<
    dyn Fn(
        &[nerust_gui_shell::debug::DebugAction],
        &nerust_gui_shell::debug::DebugDrainInput<'_>,
    ) -> nerust_gui_shell::debug::DrainFlight,
>;
type Action = Rc<dyn Fn(String)>;

struct Operations {
    begin: Begin,
    spaces: Vec<nerust_core_traits::debugger::SpaceInfo>,
    nav: NavState,
    display: DisplayCache,
    dispatch: crate::view::Dispatch,
    watch: Option<u32>,
    freeze: Option<(u32, u8)>,
    bookmarks: BTreeMap<String, (String, nerust_core_traits::debugger::SpaceId, u32)>,
    next_bookmark: u64,
    media_revision: u64,
    spaces_source: Rc<dyn Fn() -> Vec<nerust_core_traits::debugger::SpaceInfo>>,
}

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
    bookmark_selection: Option<TableSelection>,
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
    automatic: Cell<bool>,
    next_refresh: Cell<Instant>,
    selection: RefCell<Option<panechord::Subscription>>,
    operations: RefCell<Option<Operations>>,
    action: Rc<RefCell<Option<Action>>>,
    activation: RefCell<Option<panechord::Subscription>>,
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
                    bookmark_selection: None,
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
            automatic: Cell::new(false),
            next_refresh: Cell::new(Instant::now()),
            selection: RefCell::new(None),
            operations: RefCell::new(None),
            action: Rc::new(RefCell::new(None)),
            activation: RefCell::new(None),
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
        let weak = Rc::downgrade(&pane);
        let source = pane.model.bind(
            |model| {
                (
                    model.disassembly_selection.clone(),
                    model.bookmark_selection.clone(),
                )
            },
            None::<fn(&mut Model, _)>,
        );
        let activation = pane.model.watch(source, move |_, (disassembly, bookmark)| {
            let Some(pane) = weak.upgrade() else {
                return;
            };
            if disassembly.is_some_and(|selection| selection.column == 1) {
                pane.execute("disassembly_target");
            }
            if let Some(selection) = bookmark {
                pane.execute(if selection.column == 1 {
                    "bookmark_remove"
                } else {
                    "bookmark_go"
                });
            }
        });
        pane.activation.replace(Some(activation));
        pane
    }

    pub(crate) fn connect(
        self: &Rc<Self>,
        spaces: Vec<nerust_core_traits::debugger::SpaceInfo>,
        begin: Begin,
        dispatch: crate::view::Dispatch,
        media_revision: u64,
        spaces_source: Rc<dyn Fn() -> Vec<nerust_core_traits::debugger::SpaceInfo>>,
    ) {
        let start = spaces.first().map_or(0, |space| *space.range.start());
        self.operations.replace(Some(Operations {
            nav: NavState::new(spaces.len(), start),
            spaces,
            begin,
            display: DisplayCache::default(),
            dispatch,
            watch: None,
            freeze: None,
            bookmarks: BTreeMap::new(),
            next_bookmark: 0,
            media_revision,
            spaces_source,
        }));
        let weak = Rc::downgrade(self);
        self.action.replace(Some(Rc::new(move |action| {
            if let Some(pane) = weak.upgrade() {
                pane.execute(&action);
            }
        })));
    }

    fn execute(&self, action: &str) {
        if self.model.is_closed()
            || !self.model.snapshot().enabled
            || self.flight.borrow().is_some()
        {
            return;
        }
        let snapshot = self.model.snapshot();
        let result = self.begin_action(action, &snapshot);
        match result {
            Ok(Some(flight)) => {
                self.flight.replace(Some(flight));
                let _ = self.model.update(|model| model.enabled = false);
            }
            Ok(None) => {}
            Err(error) => {
                let _ = self.model.update(move |model| model.status = error);
            }
        }
    }

    fn begin_action(
        &self,
        action: &str,
        model: &Model,
    ) -> Result<Option<nerust_gui_shell::debug::DrainFlight>, String> {
        use nerust_gui_shell::debug::DebugAction;
        use nerust_gui_viewmodel::debugger::parse_hex_addr;
        let mut operations = self.operations.borrow_mut();
        let Some(operations) = operations.as_mut() else {
            return Ok(None);
        };
        let memory = || {
            model
                .memory_selection
                .as_ref()
                .and_then(|selection| selected_memory(selection, &model.memory_targets))
                .ok_or_else(|| text(model.language, UiText::SelectDumpRow).to_owned())
        };
        let register = model
            .register_marked
            .as_ref()
            .and_then(|selection| model.register_targets.get(&selection.row_key))
            .map(|(name, _)| *name);
        let hex = |input: &str| {
            let input = input.trim();
            let input = input
                .strip_prefix("0x")
                .or_else(|| input.strip_prefix('$'))
                .unwrap_or(input);
            u64::from_str_radix(input, 16).map_err(|_| "Invalid hexadecimal value".to_owned())
        };
        let current = operations
            .display
            .disasm_lines
            .first()
            .map_or(0, |line| line.addr);
        let mut actions = vec![DebugAction::Refresh];
        match action {
            "poll" => actions.clear(),
            "refresh" => {}
            "pause" => {
                (operations.dispatch)(crate::view::Intent::Pause(!operations.display.paused));
            }
            "step_frame" | "step_instruction" => {
                (operations.dispatch)(crate::view::Intent::Pause(true));
                actions = vec![if action == "step_frame" {
                    DebugAction::StepFrame
                } else {
                    DebugAction::StepInstr
                }];
            }
            "ppu" => {
                (operations.dispatch)(crate::view::Intent::Ppu);
                return Ok(None);
            }
            "follow_pc" => operations.nav.follow(),
            "back" => {
                operations.nav.go_back();
            }
            "disassembly_go" => {
                let addr = parse_hex_addr(&model.disasm_addr)
                    .ok_or_else(|| "Invalid hexadecimal address".to_owned())?;
                operations.nav.navigate(current, addr);
            }
            "disassembly_target" => {
                let selected = model
                    .disassembly_selection
                    .as_ref()
                    .ok_or_else(|| "Select an instruction".to_owned())?;
                let line = operations
                    .display
                    .disasm_lines
                    .iter()
                    .find(|line| format!("{:08X}", line.addr) == selected.row_key)
                    .ok_or_else(|| "Instruction selection is no longer available".to_owned())?;
                let addr = if selected.column == 1 {
                    line.target.unwrap_or(line.addr)
                } else {
                    line.addr
                };
                operations.nav.navigate(current, addr);
            }
            "space_previous" | "space_next" => {
                if let Some(index) =
                    operations
                        .nav
                        .cycle_space(if action == "space_previous" { -1 } else { 1 })
                {
                    let start = *operations.spaces[index].range.start();
                    operations.nav.set_space(index, start);
                    operations.watch = None;
                    operations.freeze = None;
                    operations.display.dump_rows.clear();
                    actions = vec![DebugAction::MemNav];
                }
            }
            "page_previous" | "page_next" => {
                let delta = (nerust_core_traits::debugger::DEBUG_DUMP_PAGE_ROWS * 16) as i32;
                operations.nav.page_mem(if action == "page_previous" {
                    -delta
                } else {
                    delta
                });
                actions = vec![DebugAction::MemNav];
            }
            "memory_go" => {
                let addr = parse_hex_addr(&model.memory_addr)
                    .ok_or_else(|| "Invalid hexadecimal address".to_owned())?;
                operations.nav.go_mem(addr);
                actions = vec![DebugAction::MemNav];
            }
            "watch" | "edit_watch" => operations.watch = Some(memory()?.0),
            "watch_clear" => operations.watch = None,
            "freeze" | "edit_freeze" => {
                let (addr, old) = memory()?;
                let value = if action == "edit_freeze" && model.freeze_value.trim().is_empty() {
                    old.ok_or_else(|| "Selected byte is unavailable".to_owned())?
                } else {
                    u8::try_from(hex(&model.freeze_value)?)
                        .map_err(|_| "Value must fit in one byte".to_owned())?
                };
                if !operations
                    .spaces
                    .get(operations.nav.space_idx())
                    .is_some_and(|space| {
                        space.access == nerust_core_traits::debugger::SpaceAccess::ReadWrite
                    })
                {
                    return Err("Selected address space is read-only".into());
                }
                operations.freeze = Some((addr, value));
            }
            "freeze_clear" => operations.freeze = None,
            "register_set" => {
                if register.is_none() {
                    return Err(text(model.language, UiText::PickRegister).to_owned());
                }
                actions = vec![DebugAction::RegisterWrite(hex(&model.register_value)?)];
            }
            "write" => {
                let (addr, old) = memory()?;
                if old.is_none() {
                    return Err("Selected byte is unavailable".into());
                }
                if !operations
                    .spaces
                    .get(operations.nav.space_idx())
                    .is_some_and(|space| {
                        space.access == nerust_core_traits::debugger::SpaceAccess::ReadWrite
                    })
                {
                    return Err("Selected address space is read-only".into());
                }
                let value = hex(&model.edit_value)?;
                if value > u64::from(u8::MAX) {
                    return Err("Value must fit in one byte".into());
                }
                actions = vec![DebugAction::SelectRow(addr), DebugAction::EditStage(value)];
            }
            "confirm" if model.pending => actions = vec![DebugAction::WriteConfirm],
            "cancel" => actions = vec![DebugAction::EditCancel],
            "bookmark_add" => {
                let addr = model
                    .disassembly_selection
                    .as_ref()
                    .and_then(|selected| {
                        operations
                            .display
                            .disasm_lines
                            .iter()
                            .find(|line| format!("{:08X}", line.addr) == selected.row_key)
                    })
                    .map_or(current, |line| line.addr);
                if operations.bookmarks.len() >= 256 {
                    return Err("Bookmark limit reached".into());
                }
                let label = if model.bookmark_label.trim().is_empty() {
                    format!("0x{addr:08X}")
                } else {
                    model.bookmark_label.trim().to_owned()
                };
                let space = operations
                    .spaces
                    .get(operations.nav.space_idx())
                    .ok_or_else(|| "No address space is available".to_owned())?
                    .id;
                let key = operations.next_bookmark.to_string();
                operations.next_bookmark = operations
                    .next_bookmark
                    .checked_add(1)
                    .ok_or_else(|| "Bookmark identity limit reached".to_owned())?;
                operations.bookmarks.insert(key, (label, space, addr));
                let table = bookmark_table(&operations.bookmarks);
                let _ = self.model.update(move |model| model.bookmarks = table);
                return Ok(None);
            }
            "bookmark_go" | "bookmark_remove" => {
                let selected = model
                    .bookmark_selection
                    .as_ref()
                    .ok_or_else(|| "Select a bookmark".to_owned())?;
                let (_, space, addr) = operations
                    .bookmarks
                    .get(&selected.row_key)
                    .ok_or_else(|| "Bookmark is no longer available".to_owned())?;
                let (space, addr) = (*space, *addr);
                if action == "bookmark_remove" {
                    operations.bookmarks.remove(&selected.row_key);
                    let table = bookmark_table(&operations.bookmarks);
                    let _ = self.model.update(move |model| {
                        model.bookmarks = table;
                        model.bookmark_selection = None;
                    });
                    return Ok(None);
                }
                let index = operations
                    .spaces
                    .iter()
                    .position(|candidate| candidate.id == space)
                    .ok_or_else(|| "Bookmark address space is no longer available".to_owned())?;
                operations.nav.set_space(index, addr);
                operations.nav.navigate(current, addr);
                operations.watch = None;
                operations.freeze = None;
                let _ = self.model.update(|model| model.bookmark_selection = None);
                actions = vec![DebugAction::MemNav];
            }
            _ => return Ok(None),
        }
        {
            let input = nerust_gui_shell::debug::DebugDrainInput {
                space: operations
                    .spaces
                    .get(operations.nav.space_idx())
                    .map(|space| space.id),
                mem_addr: operations.nav.mem_addr(),
                dis_addr: operations.nav.dis_addr(),
                watch: operations.watch,
                reg_target: register,
                freeze: operations.freeze,
                prev_dump_rows: &operations.display.dump_rows,
                need_images: false,
            };
            Ok(Some((operations.begin)(&actions, &input)))
        }
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
        self.action.borrow_mut().take();
        self.operations.borrow_mut().take();
    }

    pub(crate) fn poll(&self, language: AppLanguage, loaded: bool, media_revision: u64) {
        if self.model.is_closed() {
            self.cancel();
            return;
        }
        let reset = {
            let mut operations = self.operations.borrow_mut();
            if let Some(operations) = operations.as_mut() {
                if operations.media_revision != media_revision {
                    operations.media_revision = media_revision;
                    operations.spaces = (operations.spaces_source)();
                    let start = operations
                        .spaces
                        .first()
                        .map_or(0, |space| *space.range.start());
                    operations.nav = NavState::new(operations.spaces.len(), start);
                    operations.watch = None;
                    operations.freeze = None;
                    operations.bookmarks.clear();
                    operations.display = DisplayCache::default();
                    true
                } else {
                    false
                }
            } else {
                false
            }
        };
        if reset {
            self.flight.borrow_mut().take();
            self.automatic.set(false);
            self.refresh(&DisplayCache::default(), language);
            let _ = self.model.update(|model| {
                model.bookmarks = code_table([]);
                model.bookmark_selection = None;
                model.memory_selection = None;
                model.pending = false;
                model.register_value.clear();
                model.edit_value.clear();
                model.freeze_value.clear();
                model.enabled = false;
            });
        }
        if !loaded {
            let _ = self.model.update(|model| model.enabled = false);
            return;
        }
        if reset {
            let _ = self.model.update(|model| model.enabled = true);
            self.execute("refresh");
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
            let automatic = self.automatic.replace(false);
            self.next_refresh
                .set(Instant::now() + Duration::from_millis(250));
            self.refresh(&output.display, language);
            if let Some(operations) = self.operations.borrow_mut().as_mut() {
                operations.display = output.display;
                let memory_addr = operations.nav.mem_input().to_owned();
                let disasm_addr = operations.nav.dis_input().to_owned();
                let back = format!(
                    "{} ({})",
                    text(language, UiText::Back),
                    operations.nav.back_len()
                );
                let space = operations
                    .spaces
                    .get(operations.nav.space_idx())
                    .map(|space| space.name.to_owned())
                    .unwrap_or_default();
                let watch = operations
                    .watch
                    .map(|addr| {
                        format!(
                            "0x{addr:08X}: {}",
                            operations
                                .display
                                .watch_value
                                .map(|value| format!("{value:02X}"))
                                .unwrap_or_else(|| "??".into())
                        )
                    })
                    .unwrap_or_else(|| text(language, UiText::WatchNone).to_owned());
                let freeze = operations
                    .freeze
                    .map(|(addr, value)| format!("0x{addr:08X} = {value:02X}"))
                    .unwrap_or_else(|| text(language, UiText::FreezeNone).to_owned());
                let pin = operations.nav.dis_addr();
                let _ = self.model.update(move |model| {
                    if !automatic {
                        model.memory_addr = memory_addr;
                        model.disasm_addr = disasm_addr;
                    }
                    model.back = back;
                    model.space = space;
                    model.watch = watch;
                    model.freeze = freeze;
                    if let Some(addr) = pin {
                        model.captions.disassembly =
                            format!("{} (0x{addr:08X})", text(language, UiText::Disassembly));
                    }
                });
            }
            let connected = self.operations.borrow().is_some();
            let _ = self.model.update(move |model| model.enabled = connected);
        }
        let refresh = self.flight.borrow().is_none()
            && Instant::now() >= self.next_refresh.get()
            && self.operations.borrow().as_ref().is_some_and(|operations| {
                (!operations.display.paused || self.model.snapshot().language != language)
                    && !matches!(
                        operations.display.status,
                        nerust_gui_viewmodel::debugger::StatusMessage::OutcomeUnknown
                    )
            });
        if refresh {
            self.automatic.set(true);
            self.execute("poll");
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
            action: self.action.clone(),
        };
        let view = panechord::try_view!("ui/debugger.yaml", bindings, panechord)?;
        let pane = self.clone();
        view.on_close(Rc::new(move || {
            pane.cancel();
            let selection = pane.selection.borrow_mut().take();
            drop(selection);
            let activation = pane.activation.borrow_mut().take();
            drop(activation);
            pane.model.close();
        }));
        Ok(view)
    }
}

fn bookmark_table(
    bookmarks: &BTreeMap<String, (String, nerust_core_traits::debugger::SpaceId, u32)>,
) -> Table {
    let mut table = code_table(
        bookmarks
            .iter()
            .map(|(key, (label, _, addr))| (key.clone(), format!("{label} 0x{addr:08X}"))),
    );
    let mut columns = table.columns.to_vec();
    columns.push(TableColumn {
        title: String::new(),
        width: Some(40.0),
        text: TextStyle::default(),
        overflow: TextOverflow::Clip,
        stretch: false,
        hide_empty: false,
    });
    table.columns = columns.into();
    for row in Arc::make_mut(&mut table.rows) {
        Arc::make_mut(row).cells.push(TableCell {
            text: "\u{00D7}".into(),
            foreground: None,
        });
    }
    table
}

#[derive(Clone)]
struct Presentation {
    model: Store<Model>,
    action: Rc<RefCell<Option<Action>>>,
}
impl Deref for Presentation {
    type Target = Store<Model>;
    fn deref(&self) -> &Self::Target {
        &self.model
    }
}
impl Presentation {
    fn command_action(&self, argument: Binding<String>) -> Command {
        let name = argument.get();
        let action = self.action.clone();
        self.model.command(
            move |model| {
                model.enabled
                    && match name.as_str() {
                        "confirm" | "cancel" => model.pending,
                        "register_set" => model.register_marked.is_some(),
                        "write" | "watch" | "edit_watch" | "freeze" | "edit_freeze" => {
                            model.memory_selection.is_some()
                        }
                        "refresh" | "pause" | "step_frame" | "step_instruction" | "ppu"
                        | "follow_pc" | "back" | "disassembly_go" | "space_previous"
                        | "space_next" | "page_previous" | "page_next" | "memory_go"
                        | "watch_clear" | "freeze_clear" | "bookmark_add" => true,
                        _ => false,
                    }
            },
            move |_, name: String| {
                let dispatch = action.borrow().clone();
                if let Some(dispatch) = dispatch {
                    dispatch(name);
                }
            },
            argument,
        )
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
