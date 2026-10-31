use nerust_gui_viewmodel::debugger::format::format_dump_address;
use nerust_gui_viewmodel::debugger::{DisplayCache, format_disasm_line, format_register};
use nerust_settings_core::{
    i18n::{UiText, text},
    model::language::AppLanguage,
};
use panechord::{Binding, Command, Executor, Store, View};
use panechord::{
    Table, TableCell, TableColumn, TableRow, TableSelection, TextOverflow, TextRole, TextStyle,
};
use std::{cell::RefCell, ops::Deref, rc::Rc, sync::Arc};

#[derive(Clone)]
pub(crate) struct Projection {
    pub(crate) disassembly: Table,
    pub(crate) registers: [Table; 2],
    memory: Table,
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
        row_gap: 4.0,
    }
}

pub(crate) fn project(display: &DisplayCache) -> Projection {
    let midpoint = display.regs.len().div_ceil(2);
    let registers = [&display.regs[..midpoint], &display.regs[midpoint..]].map(|registers| {
        let mut table = code_table(registers.iter().map(|&(name, value)| {
            (
                name.bytes().map(|byte| format!("{byte:02X}")).collect(),
                format_register(name, value),
            )
        }));
        Arc::make_mut(&mut table.columns)[0].stretch = false;
        table
    });
    let disassembly = code_table(
        display
            .disasm_lines
            .iter()
            .map(|line| (format!("{:08X}", line.addr), format_disasm_line(line))),
    );
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
            row_gap: 4.0,
        },
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
    close: String,
}
impl Captions {
    fn new(language: AppLanguage) -> Self {
        let label = |key| text(language, key).to_owned();
        Self {
            title: label(UiText::Debugger),
            pause: label(UiText::Pause),
            step_frame: label(UiText::StepFrame),
            step_instruction: label(UiText::StepInstr),
            ppu: label(UiText::PpuViewer),
            disassembly: label(UiText::Disassembly),
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
            close: label(UiText::Close),
        }
    }
}

#[derive(Clone)]
struct Model {
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
}
impl Pane {
    pub(crate) fn new(language: AppLanguage, executor: Rc<dyn Executor>) -> Rc<Self> {
        let projection = project(&DisplayCache::default());
        let [registers_left, registers_right] = projection.registers;
        let label = |key| text(language, key).to_owned();
        Rc::new(Self {
            model: Store::new(
                Model {
                    captions: Captions::new(language),
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
                    memory_addr: String::new(),
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
        })
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
        let _ = self.model.update(move |model| {
            model.disassembly = projection.disassembly;
            model.disassembly_selection = projection.pc;
            model.registers_left = registers_left;
            model.registers_right = registers_right;
            model.register_style = projection.register_style;
            model.registers_visible = projection.registers_visible;
            model.memory = projection.memory;
            model.status = status;
        });
    }

    pub(crate) fn build(self: &Rc<Self>) -> Result<View, String> {
        let close = Rc::new(RefCell::new(None::<panechord::WeakView>));
        let bindings = Presentation {
            model: self.model.clone(),
            close: close.clone(),
        };
        let view = panechord::try_view!("ui/debugger.yaml", bindings, panechord)?;
        close.replace(Some(view.downgrade()));
        let pane = self.clone();
        view.on_close(Rc::new(move || {
            pane.cancel();
            pane.model.close();
        }));
        Ok(view)
    }
}

#[derive(Clone)]
struct Presentation {
    model: Store<Model>,
    close: Rc<RefCell<Option<panechord::WeakView>>>,
}
impl Deref for Presentation {
    type Target = Store<Model>;
    fn deref(&self) -> &Self::Target {
        &self.model
    }
}
impl Presentation {
    fn command_close(&self, argument: Binding<String>) -> Command {
        let close = self.close.clone();
        self.model.command(
            |_| true,
            move |_, _: String| {
                if let Some(view) = close
                    .borrow()
                    .as_ref()
                    .and_then(panechord::WeakView::upgrade)
                {
                    view.close();
                }
            },
            argument,
        )
    }
    fn command_action(&self, argument: Binding<String>) -> Command {
        self.model
            .command(|model| model.enabled, |_, _: String| {}, argument)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
