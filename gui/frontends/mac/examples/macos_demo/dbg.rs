//! Debugger panel: full gtk/tao panel set over the shared drain.
//!
//! Disassembly with follow/Go/Back/target-jump, registers with Set,
//! watch, memory dump with paging and an edit/confirm row, freeze,
//! bookmarks (combo + jump/delete), status line, and F6/F7/F9 keys.
//! Gestures dispatch `DebuggerEvent` into the shared `DebuggerModel`
//! reducer; non-empty actions run one in-flight drain, then the views
//! rebuild from the display cache.

use nerust_frontend_host::debugger::{DebuggerEvent, DebuggerModel};
use nerust_gui_shell::{
    debug::{DebugAction, DebugDrainInput, DrainFlight, DrainPoll},
    session::commands::SessionCommand,
};
use objc2::{MainThreadMarker, rc::Retained, sel};
use objc2_app_kit::{
    NSBackingStoreType, NSButton, NSComboBox, NSControlStateValueOn, NSTextField, NSTextView,
    NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize, NSString};

use super::app::anyobject;

pub struct DbgWin {
    pub window: Retained<NSWindow>,
    mtm: MainThreadMarker,
    delegate: Retained<super::app::AppDelegate>,
    text: Retained<NSTextView>,
    status: Retained<NSTextField>,
    dis_addr: Retained<NSTextField>,
    follow: Retained<NSButton>,
    mem_text: Retained<NSTextView>,
    mem_addr: Retained<NSTextField>,
    space_label: Retained<NSTextField>,
    reg_label: Retained<NSTextField>,
    reg_input: Retained<NSTextField>,
    reg_box: Retained<NSView>,
    watch_label: Retained<NSTextField>,
    freeze_label: Retained<NSTextField>,
    freeze_input: Retained<NSTextField>,
    bookmark_input: Retained<NSTextField>,
    bookmark_combo: Retained<NSComboBox>,
    edit_label: Retained<NSTextField>,
    edit_input: Retained<NSTextField>,
    model: DebuggerModel,
    flight: Option<DrainFlight>,
    prev_rows: Vec<nerust_core_traits::debugger::HexRow>,
}

fn text_view(mtm: MainThreadMarker, x: f64, y: f64, w: f64, h: f64) -> Retained<NSTextView> {
    let view: Retained<NSTextView> = NSTextView::initWithFrame(
        mtm.alloc::<NSTextView>(),
        NSRect::new(NSPoint::new(x, y), NSSize::new(w, h)),
    );
    view.setEditable(false);
    view
}

fn field(
    mtm: MainThreadMarker,
    x: f64,
    y: f64,
    w: f64,
    placeholder: &str,
) -> Retained<NSTextField> {
    let field: Retained<NSTextField> = NSTextField::initWithFrame(
        mtm.alloc::<NSTextField>(),
        NSRect::new(NSPoint::new(x, y), NSSize::new(w, 22.0)),
    );
    field.setPlaceholderString(Some(&NSString::from_str(placeholder)));
    field
}

fn label(mtm: MainThreadMarker, x: f64, y: f64, w: f64, text: &str) -> Retained<NSTextField> {
    let label: Retained<NSTextField> = NSTextField::initWithFrame(
        mtm.alloc::<NSTextField>(),
        NSRect::new(NSPoint::new(x, y), NSSize::new(w, 20.0)),
    );
    label.setStringValue(&NSString::from_str(text));
    label.setEditable(false);
    label.setBezeled(false);
    label.setDrawsBackground(false);
    label
}

fn button(
    mtm: MainThreadMarker,
    window: &NSWindow,
    delegate: &super::app::AppDelegate,
    x: f64,
    y: f64,
    title: &str,
    action: objc2::runtime::Sel,
) -> Retained<NSButton> {
    let btn: Retained<NSButton> = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str(title),
            Some(anyobject(delegate)),
            Some(action),
            mtm,
        )
    };
    btn.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(90.0, 24.0)));
    if let Some(view) = window.contentView() {
        view.addSubview(&btn);
    }
    btn
}

fn add(view: &NSView, child: &NSView) {
    view.addSubview(child);
}

impl DbgWin {
    pub fn open(
        mtm: MainThreadMarker,
        host: &mut nerust_mac::MacHost,
        delegate: &Retained<super::app::AppDelegate>,
    ) -> Option<Self> {
        let frame = NSRect::new(NSPoint::new(760.0, 60.0), NSSize::new(700.0, 900.0));
        let window: Retained<NSWindow> = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                mtm.alloc::<NSWindow>(),
                frame,
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Miniaturizable
                    | NSWindowStyleMask::Resizable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        window.setTitle(&NSString::from_str("Debugger"));
        let content = window.contentView()?;
        // Toolbar.
        button(
            mtm,
            &window,
            delegate,
            8.0,
            866.0,
            "Pause",
            sel!(debugPause:),
        );
        button(
            mtm,
            &window,
            delegate,
            104.0,
            866.0,
            "Resume",
            sel!(debugResume:),
        );
        button(
            mtm,
            &window,
            delegate,
            200.0,
            866.0,
            "Step Frame",
            sel!(debugStepFrame:),
        );
        button(
            mtm,
            &window,
            delegate,
            296.0,
            866.0,
            "Step Instr",
            sel!(debugStep:),
        );
        button(
            mtm,
            &window,
            delegate,
            392.0,
            866.0,
            "Refresh",
            sel!(debugRefresh:),
        );
        button(
            mtm,
            &window,
            delegate,
            488.0,
            866.0,
            "PPU",
            sel!(togglePpu:),
        );
        let status = label(mtm, 8.0, 842.0, 684.0, "");
        add(&content, &status);
        // Disassembly nav.
        let follow: Retained<NSButton> = unsafe {
            NSButton::checkboxWithTitle_target_action(
                &NSString::from_str("Follow PC"),
                Some(anyobject(delegate)),
                Some(sel!(debugFollow:)),
                mtm,
            )
        };
        follow.setFrame(NSRect::new(
            NSPoint::new(8.0, 812.0),
            NSSize::new(110.0, 22.0),
        ));
        add(&content, &follow);
        let dis_addr = field(mtm, 124.0, 812.0, 120.0, "addr hex");
        add(&content, &dis_addr);
        button(
            mtm,
            &window,
            delegate,
            250.0,
            810.0,
            "Go",
            sel!(debugDisGo:),
        );
        button(
            mtm,
            &window,
            delegate,
            346.0,
            810.0,
            "Back",
            sel!(debugDisBack:),
        );
        button(
            mtm,
            &window,
            delegate,
            442.0,
            810.0,
            "Jump",
            sel!(debugJump:),
        );
        button(
            mtm,
            &window,
            delegate,
            538.0,
            810.0,
            "Select",
            sel!(debugSelectDis:),
        );
        // Disassembly list.
        let text = text_view(mtm, 8.0, 560.0, 684.0, 244.0);
        add(&content, &text);
        // Registers.
        let reg_label = label(mtm, 8.0, 532.0, 684.0, "Registers");
        add(&content, &reg_label);
        let reg_box: Retained<NSView> = NSView::initWithFrame(
            mtm.alloc::<NSView>(),
            NSRect::new(NSPoint::new(8.0, 470.0), NSSize::new(684.0, 56.0)),
        );
        add(&content, &reg_box);
        let reg_input = field(mtm, 8.0, 442.0, 120.0, "hex");
        add(&content, &reg_input);
        button(
            mtm,
            &window,
            delegate,
            134.0,
            440.0,
            "Set",
            sel!(debugRegSet:),
        );
        // Watch / freeze.
        let watch_label = label(mtm, 8.0, 414.0, 340.0, "Watch: none");
        add(&content, &watch_label);
        button(
            mtm,
            &window,
            delegate,
            352.0,
            412.0,
            "Watch",
            sel!(debugWatchSet:),
        );
        button(
            mtm,
            &window,
            delegate,
            448.0,
            412.0,
            "Clear",
            sel!(debugWatchClear:),
        );
        let freeze_label = label(mtm, 8.0, 384.0, 220.0, "Freeze: none");
        add(&content, &freeze_label);
        let freeze_input = field(mtm, 232.0, 384.0, 110.0, "hex byte");
        add(&content, &freeze_input);
        button(
            mtm,
            &window,
            delegate,
            352.0,
            382.0,
            "Freeze",
            sel!(debugFreezeSet:),
        );
        button(
            mtm,
            &window,
            delegate,
            448.0,
            382.0,
            "Clear",
            sel!(debugFreezeClear:),
        );
        // Memory nav.
        let space_label = label(mtm, 8.0, 354.0, 220.0, "Space");
        add(&content, &space_label);
        button(
            mtm,
            &window,
            delegate,
            232.0,
            352.0,
            "Prev",
            sel!(debugSpacePrev:),
        );
        button(
            mtm,
            &window,
            delegate,
            328.0,
            352.0,
            "Next",
            sel!(debugSpaceNext:),
        );
        button(
            mtm,
            &window,
            delegate,
            424.0,
            352.0,
            "Pg -",
            sel!(debugPagePrev:),
        );
        button(
            mtm,
            &window,
            delegate,
            520.0,
            352.0,
            "Pg +",
            sel!(debugPageNext:),
        );
        let mem_addr = field(mtm, 8.0, 324.0, 120.0, "addr hex");
        add(&content, &mem_addr);
        button(
            mtm,
            &window,
            delegate,
            134.0,
            322.0,
            "Go",
            sel!(debugMemGo:),
        );
        button(
            mtm,
            &window,
            delegate,
            230.0,
            322.0,
            "Select",
            sel!(debugSelectMem:),
        );
        // Memory dump.
        let mem_text = text_view(mtm, 8.0, 120.0, 684.0, 196.0);
        add(&content, &mem_text);
        // Edit row.
        let edit_label = label(mtm, 8.0, 94.0, 200.0, "Select dump row");
        add(&content, &edit_label);
        let edit_input = field(mtm, 212.0, 94.0, 110.0, "hex byte");
        add(&content, &edit_input);
        button(
            mtm,
            &window,
            delegate,
            328.0,
            92.0,
            "Write",
            sel!(debugEditWrite:),
        );
        button(
            mtm,
            &window,
            delegate,
            424.0,
            92.0,
            "Watch",
            sel!(debugWatchSet:),
        );
        button(
            mtm,
            &window,
            delegate,
            520.0,
            92.0,
            "Freeze",
            sel!(debugEditFreeze:),
        );
        // Bookmarks.
        let bookmark_input = field(mtm, 8.0, 62.0, 150.0, "label");
        add(&content, &bookmark_input);
        button(
            mtm,
            &window,
            delegate,
            164.0,
            60.0,
            "Add",
            sel!(debugBookmarkAdd:),
        );
        let bookmark_combo: Retained<NSComboBox> = NSComboBox::initWithFrame(
            mtm.alloc::<NSComboBox>(),
            NSRect::new(NSPoint::new(260.0, 62.0), NSSize::new(240.0, 24.0)),
        );
        add(&content, &bookmark_combo);
        button(
            mtm,
            &window,
            delegate,
            506.0,
            60.0,
            "Jump",
            sel!(debugBookmarkJump:),
        );
        button(
            mtm,
            &window,
            delegate,
            602.0,
            60.0,
            "Delete",
            sel!(debugBookmarkDelete:),
        );
        window.makeKeyAndOrderFront(None);
        let mut win = Self {
            window,
            mtm,
            delegate: delegate.clone(),
            text,
            status,
            dis_addr,
            follow,
            mem_text,
            mem_addr,
            space_label,
            reg_label,
            reg_input,
            reg_box,
            watch_label,
            freeze_label,
            freeze_input,
            bookmark_input,
            bookmark_combo,
            edit_label,
            edit_input,
            model: DebuggerModel::from_session(host.session()),
            flight: None,
            prev_rows: Vec::new(),
        };
        win.refresh();
        win.on_tick(host);
        Some(win)
    }

    pub fn on_tick(&mut self, _host: &mut nerust_mac::MacHost) {
        if let Some(flight) = self.flight.as_mut() {
            match flight.poll() {
                DrainPoll::Pending => {}
                DrainPoll::Done(output) => {
                    self.model.apply_drain(output.display, output.reselected);
                    self.prev_rows = self.model.display.dump_rows.clone();
                    self.refresh();
                    self.flight = None;
                }
            }
        }
    }

    fn act(&mut self, host: &mut nerust_mac::MacHost, actions: &[DebugAction]) {
        if actions.is_empty() {
            self.refresh();
            return;
        }
        let mut drain = Vec::new();
        for action in actions {
            match action {
                DebugAction::Pause => {
                    let _ = host.dispatch_cmd(SessionCommand::Pause);
                }
                DebugAction::Resume => {
                    let _ = host.dispatch_cmd(SessionCommand::Resume);
                }
                DebugAction::TogglePause => {
                    let paused = host.tick().paused;
                    let _ = host.dispatch_cmd(if paused {
                        SessionCommand::Resume
                    } else {
                        SessionCommand::Pause
                    });
                }
                other => drain.push(*other),
            }
        }
        if drain.is_empty() {
            self.refresh();
            return;
        }
        let prev = self.model.display.dump_rows.clone();
        let input = DebugDrainInput {
            space: self.model.selected_space(),
            mem_addr: self.model.anchor(),
            dis_addr: self.model.nav.dis_addr(),
            watch: self.model.watch,
            freeze: self.model.freeze,
            reg_target: self.model.reg_target,
            prev_dump_rows: &prev,
            need_images: false,
        };
        let _ = &prev;
        self.flight = Some(host.session().debug_drain_begin(&drain, &input));
    }

    fn echo_inputs(&mut self) {
        let mem = self.mem_addr.stringValue().to_string();
        let dis = self.dis_addr.stringValue().to_string();
        let bookmark = self.bookmark_input.stringValue().to_string();
        let freeze = self.freeze_input.stringValue().to_string();
        let edit = self.edit_input.stringValue().to_string();
        let reg = self.reg_input.stringValue().to_string();
        self.model.dispatch(DebuggerEvent::MemInputChanged(mem));
        self.model.dispatch(DebuggerEvent::DisInputChanged(dis));
        self.model
            .dispatch(DebuggerEvent::BookmarkInputChanged(bookmark));
        self.model
            .dispatch(DebuggerEvent::FreezeInputChanged(freeze));
        self.model.dispatch(DebuggerEvent::EditInputChanged(edit));
        self.model.dispatch(DebuggerEvent::RegInputChanged(reg));
    }

    pub fn gesture(&mut self, host: &mut nerust_mac::MacHost, event: DebuggerEvent) {
        self.echo_inputs();
        let update = self.model.dispatch(event);
        if update.open_ppu {
            super::app::with_app(|app| app.open_ppu());
        }
        let actions = update.actions.clone();
        self.act(host, &actions);
    }

    /// Selected disassembly row index from the text selection.
    fn selected_dis_row(&self) -> Option<usize> {
        selected_row(&self.text)
    }

    /// Selected memory row index from the text selection.
    fn selected_mem_row(&self) -> Option<usize> {
        selected_row(&self.mem_text)
    }

    pub fn follow_selected(&mut self, host: &mut nerust_mac::MacHost) {
        if let Some(index) = self.selected_dis_row()
            && let Some(line) = self.model.display.disasm_lines.get(index)
            && let Some(target) = line.target
        {
            self.gesture(host, DebuggerEvent::FollowTarget(target));
        }
    }

    pub fn select_dis_row(&mut self, host: &mut nerust_mac::MacHost) {
        if let Some(index) = self.selected_dis_row()
            && let Some(line) = self.model.display.disasm_lines.get(index)
        {
            let addr = line.addr;
            self.gesture(host, DebuggerEvent::DisLineClicked(addr));
        }
    }

    pub fn select_mem_row(&mut self, host: &mut nerust_mac::MacHost) {
        if let Some(index) = self.selected_mem_row()
            && let Some(row) = self.model.display.dump_rows.get(index)
        {
            let addr = row.addr;
            self.gesture(host, DebuggerEvent::RowSelected(addr));
        }
    }

    pub fn step(&mut self, host: &mut nerust_mac::MacHost) {
        self.gesture(host, DebuggerEvent::StepInstrPressed);
    }

    pub fn bookmark_jump(&mut self, host: &mut nerust_mac::MacHost) {
        let index = self.bookmark_combo.indexOfSelectedItem();
        if index >= 0
            && let Some((addr, _)) = self.model.bookmarks.get(index as usize)
        {
            let addr = *addr;
            self.gesture(host, DebuggerEvent::BookmarkJump(addr));
        }
    }

    pub fn bookmark_delete(&mut self, host: &mut nerust_mac::MacHost) {
        let index = self.bookmark_combo.indexOfSelectedItem();
        if index >= 0
            && let Some((addr, _)) = self.model.bookmarks.get(index as usize)
        {
            let addr = *addr;
            self.gesture(host, DebuggerEvent::BookmarkDelete(addr));
        }
    }

    pub fn select_reg(&mut self, host: &mut nerust_mac::MacHost, index: isize) {
        if let Some((name, _)) = self.model.display.regs.get(index as usize) {
            let name = *name;
            self.gesture(host, DebuggerEvent::RegSelected(name));
        }
    }

    fn refresh(&mut self) {
        let mut text = String::new();
        for line in self.model.display.disasm_lines.iter().take(200) {
            let bytes: String = line.bytes[..line.len as usize]
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(" ");
            let mark = if line.is_pc { ">" } else { " " };
            text.push_str(&format!(
                "{mark}{:08X}: {:<12} {}\n",
                line.addr, bytes, line.text
            ));
        }
        self.text.setString(&NSString::from_str(&text));
        let state = if self.model.display.paused {
            "PAUSED"
        } else {
            "RUNNING"
        };
        self.status.setStringValue(&NSString::from_str(&format!(
            "{state} | {} disasm lines",
            self.model.display.disasm_lines.len()
        )));
        let follow = self.model.nav.dis_addr().is_none();
        self.follow
            .setState(if follow { NSControlStateValueOn } else { 0 });
        self.space_label
            .setStringValue(&NSString::from_str(&format!(
                "Space: {}",
                self.model.selected_name()
            )));
        // Registers as buttons; selection sets the write target.
        self.reg_box.setSubviews(&NSArray::from_retained_slice(&[]));
        for (index, (name, value)) in self.model.display.regs.iter().enumerate() {
            let btn: Retained<NSButton> = unsafe {
                NSButton::buttonWithTitle_target_action(
                    &NSString::from_str(&format!("{name}: {value:02X}")),
                    Some(anyobject(&self.delegate)),
                    Some(sel!(debugRegSelect:)),
                    self.mtm,
                )
            };
            btn.setTag(index as isize);
            btn.setFrame(NSRect::new(
                NSPoint::new(
                    8.0 + (index % 4) as f64 * 170.0,
                    28.0 - (index / 4) as f64 * 28.0,
                ),
                NSSize::new(160.0, 24.0),
            ));
            self.reg_box.addSubview(&btn);
        }
        let target = self
            .model
            .reg_target
            .map(|name| format!("Set {name}"))
            .unwrap_or_else(|| "Pick a register".to_string());
        self.reg_label.setStringValue(&NSString::from_str(&target));
        let watch = match (self.model.watch, self.model.display.watch_value) {
            (Some(addr), Some(value)) => format!("Watch: {addr:08X} = {value:02X}"),
            (Some(addr), None) => format!("Watch: {addr:08X} = --"),
            (None, _) => "Watch: none".to_string(),
        };
        self.watch_label.setStringValue(&NSString::from_str(&watch));
        let freeze = match self.model.freeze {
            Some((addr, value)) => format!("Freeze: {addr:08X} = {value:02X}"),
            None => "Freeze: none".to_string(),
        };
        self.freeze_label
            .setStringValue(&NSString::from_str(&freeze));
        let mut dump = String::new();
        for row in self.model.display.dump_rows.iter() {
            let bytes: String = row.bytes[..row.valid as usize]
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(" ");
            dump.push_str(&format!("{:08X}: {bytes}\n", row.addr));
        }
        self.mem_text.setString(&NSString::from_str(&dump));
        let edit = match self.model.selected {
            Some((addr, _)) => match &self.model.display.pending {
                Some(pending) => format!(
                    "Confirm write {:08X} = {:02X}?",
                    pending.addr, pending.value as u8
                ),
                None => format!("Edit {addr:08X}"),
            },
            None => "Select dump row".to_string(),
        };
        self.edit_label.setStringValue(&NSString::from_str(&edit));
        // Bookmarks combo.
        self.bookmark_combo.removeAllItems();
        for (addr, label) in self.model.bookmarks.clone() {
            unsafe {
                self.bookmark_combo
                    .addItemWithObjectValue(&NSString::from_str(&format!("{label} @{addr:08X}")));
            }
        }
        if !self.model.bookmarks.is_empty() {
            self.bookmark_combo.selectItemAtIndex(0);
        }
    }
}

fn selected_row(text: &NSTextView) -> Option<usize> {
    let range = text.selectedRange();
    let string = text.string().to_string();
    let at = range.location.min(string.len());
    Some(string[..at].chars().filter(|c| *c == '\n').count())
}
