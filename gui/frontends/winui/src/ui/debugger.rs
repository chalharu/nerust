//! Debugger window: disassembly, registers, watch, memory, freeze,
//! bookmarks — the full gtk/tao panel set over the shared shell drain.
//!
//! Gestures dispatch `DebuggerEvent` into the shared `DebuggerModel`
//! reducer; non-empty action lists run through a single in-flight drain
//! (the win32-demo model), then the window rebuilds from the display
//! cache. Pause ownership follows the inspect-requires-pause rule.

use std::cell::RefCell;
use std::rc::Rc;

use nerust_frontend_host::debugger::{DebuggerEvent, DebuggerModel};
use nerust_gui_shell::debug::{DebugAction, DebugDrainInput, DrainFlight, DrainPoll};
use nerust_gui_shell::session::commands::SessionCommand;
use windows_core::{Interface, Result};

use crate::bindings::Microsoft::UI::Xaml::Controls::{
    Button, ColumnDefinition, Grid, ListView, Orientation, RowDefinition, ScrollViewer,
    SelectionChangedEventHandler, StackPanel, TextBlock, TextBox, ToggleSwitch,
};
use crate::bindings::Microsoft::UI::Xaml::{GridLengthHelper, RoutedEventHandler, Window};

use super::app::with_app;
use super::{box_str, h};

pub struct DebuggerWin {
    pub window: Window,
    pub(crate) model: DebuggerModel,
    flight: Option<DrainFlight>,
    // Controls rebuilt on demand; handles kept for input echo.
    dis_list: ListView,
    dis_addr: TextBox,
    follow_check: ToggleSwitch,
    status: TextBlock,
    mem_list: ListView,
    mem_addr: TextBox,
    space_label: TextBlock,
    reg_grid: Grid,
    reg_target_label: TextBlock,
    reg_input: TextBox,
    watch_label: TextBlock,
    freeze_label: TextBlock,
    freeze_input: TextBox,
    bookmark_input: TextBox,
    bookmark_list: ListView,
    edit_label: TextBlock,
    edit_input: TextBox,
    building: Rc<RefCell<bool>>,
}

fn cast_err(what: &str) -> windows_core::Error {
    windows_core::Error::new(windows_core::HRESULT(0x80004005u32 as i32), what)
}

impl DebuggerWin {
    pub fn open(host: &mut nerust_win::WinHost) -> Option<Self> {
        let flow = (|| -> Result<Self> {
            let window = Window::new()?;
            window.SetTitle(&h("Debugger"))?;
            let model = DebuggerModel::from_session(host.session());

            let toolbar = StackPanel::new()?;
            toolbar.SetOrientation(Orientation::Horizontal)?;
            let pause = Button::new()?;
            pause.SetContent(&box_str("Pause")?)?;
            pause.Click(&RoutedEventHandler::new(|_, _| {
                gesture(DebuggerEvent::TogglePausePressed);
                Ok(())
            }))?;
            let step_frame = Button::new()?;
            step_frame.SetContent(&box_str("Step Frame")?)?;
            step_frame.Click(&RoutedEventHandler::new(|_, _| {
                gesture(DebuggerEvent::StepFramePressed);
                Ok(())
            }))?;
            let step_instr = Button::new()?;
            step_instr.SetContent(&box_str("Step Instr")?)?;
            step_instr.Click(&RoutedEventHandler::new(|_, _| {
                gesture(DebuggerEvent::StepInstrPressed);
                Ok(())
            }))?;
            let ppu = Button::new()?;
            ppu.SetContent(&box_str("PPU Viewer")?)?;
            ppu.Click(&RoutedEventHandler::new(|_, _| {
                super::app::with_app(|app| app.open_ppu());
                Ok(())
            }))?;
            let refresh = Button::new()?;
            refresh.SetContent(&box_str("Refresh")?)?;
            refresh.Click(&RoutedEventHandler::new(|_, _| {
                gesture(DebuggerEvent::RefreshPressed);
                Ok(())
            }))?;
            for button in [&pause, &step_frame, &step_instr, &ppu, &refresh] {
                toolbar.Children()?.Append(button)?;
            }

            let status = TextBlock::new()?;

            let follow_check = ToggleSwitch::new()?;
            follow_check.SetHeader(&box_str("Follow PC")?)?;
            follow_check.SetIsOn(true)?;
            follow_check.Toggled(&RoutedEventHandler::new(|_, _| {
                gesture(DebuggerEvent::FollowPcToggle);
                Ok(())
            }))?;
            let dis_addr = TextBox::new()?;
            dis_addr.SetPlaceholderText(&h("addr hex"))?;
            let go = Button::new()?;
            go.SetContent(&box_str("Go")?)?;
            go.Click(&RoutedEventHandler::new(|_, _| {
                gesture(DebuggerEvent::DisGo);
                Ok(())
            }))?;
            let back = Button::new()?;
            back.SetContent(&box_str("Back")?)?;
            back.Click(&RoutedEventHandler::new(|_, _| {
                gesture(DebuggerEvent::DisBack);
                Ok(())
            }))?;
            let jump = Button::new()?;
            jump.SetContent(&box_str("Jump")?)?;
            jump.Click(&RoutedEventHandler::new(|_, _| {
                with_app(|app| app.debug_follow_selected());
                Ok(())
            }))?;
            let dis_nav = StackPanel::new()?;
            dis_nav.SetOrientation(Orientation::Horizontal)?;
            dis_nav.Children()?.Append(&follow_check)?;
            dis_nav.Children()?.Append(&dis_addr)?;
            dis_nav.Children()?.Append(&go)?;
            dis_nav.Children()?.Append(&back)?;
            dis_nav.Children()?.Append(&jump)?;

            let dis_list = ListView::new()?;

            let space_label = TextBlock::new()?;
            let mem_addr = TextBox::new()?;
            mem_addr.SetPlaceholderText(&h("addr hex"))?;
            let mem_nav = StackPanel::new()?;
            mem_nav.SetOrientation(Orientation::Horizontal)?;
            let space_prev = Button::new()?;
            space_prev.SetContent(&box_str("Prev")?)?;
            space_prev.Click(&RoutedEventHandler::new(|_, _| {
                gesture(DebuggerEvent::SpacePrev);
                Ok(())
            }))?;
            let space_next = Button::new()?;
            space_next.SetContent(&box_str("Next")?)?;
            space_next.Click(&RoutedEventHandler::new(|_, _| {
                gesture(DebuggerEvent::SpaceNext);
                Ok(())
            }))?;
            let mem_prev = Button::new()?;
            mem_prev.SetContent(&box_str("Page -")?)?;
            mem_prev.Click(&RoutedEventHandler::new(|_, _| {
                gesture(DebuggerEvent::MemPage(-1));
                Ok(())
            }))?;
            let mem_next = Button::new()?;
            mem_next.SetContent(&box_str("Page +")?)?;
            mem_next.Click(&RoutedEventHandler::new(|_, _| {
                gesture(DebuggerEvent::MemPage(1));
                Ok(())
            }))?;
            let mem_go = Button::new()?;
            mem_go.SetContent(&box_str("Go")?)?;
            mem_go.Click(&RoutedEventHandler::new(|_, _| {
                gesture(DebuggerEvent::MemGo);
                Ok(())
            }))?;
            for button in [&space_prev, &space_next, &mem_prev, &mem_next, &mem_go] {
                mem_nav.Children()?.Append(button)?;
            }
            mem_nav.Children()?.Append(&mem_addr)?;

            let mem_list = ListView::new()?;

            let reg_grid = Grid::new()?;
            let reg_target_label = TextBlock::new()?;
            let reg_input = TextBox::new()?;
            reg_input.SetPlaceholderText(&h("hex"))?;
            let reg_set = Button::new()?;
            reg_set.SetContent(&box_str("Set")?)?;
            reg_set.Click(&RoutedEventHandler::new(|_, _| {
                gesture(DebuggerEvent::RegSet);
                Ok(())
            }))?;

            let watch_label = TextBlock::new()?;
            let watch_btn = Button::new()?;
            watch_btn.SetContent(&box_str("Watch")?)?;
            watch_btn.Click(&RoutedEventHandler::new(|_, _| {
                gesture(DebuggerEvent::WatchSet);
                Ok(())
            }))?;
            let watch_clear = Button::new()?;
            watch_clear.SetContent(&box_str("Clear")?)?;
            watch_clear.Click(&RoutedEventHandler::new(|_, _| {
                gesture(DebuggerEvent::WatchClear);
                Ok(())
            }))?;

            let freeze_label = TextBlock::new()?;
            let freeze_input = TextBox::new()?;
            freeze_input.SetPlaceholderText(&h("hex byte"))?;
            let freeze_btn = Button::new()?;
            freeze_btn.SetContent(&box_str("Freeze")?)?;
            freeze_btn.Click(&RoutedEventHandler::new(|_, _| {
                gesture(DebuggerEvent::FreezeSet);
                Ok(())
            }))?;
            let freeze_clear = Button::new()?;
            freeze_clear.SetContent(&box_str("Clear")?)?;
            freeze_clear.Click(&RoutedEventHandler::new(|_, _| {
                gesture(DebuggerEvent::FreezeClear);
                Ok(())
            }))?;

            let bookmark_input = TextBox::new()?;
            bookmark_input.SetPlaceholderText(&h("label"))?;
            let bookmark_add = Button::new()?;
            bookmark_add.SetContent(&box_str("Add")?)?;
            bookmark_add.Click(&RoutedEventHandler::new(|_, _| {
                gesture(DebuggerEvent::BookmarkAdd);
                Ok(())
            }))?;
            let bookmark_list = ListView::new()?;

            let edit_label = TextBlock::new()?;
            let edit_input = TextBox::new()?;
            edit_input.SetPlaceholderText(&h("hex byte"))?;
            let edit_row = StackPanel::new()?;
            edit_row.SetOrientation(Orientation::Horizontal)?;
            let edit_write = Button::new()?;
            edit_write.SetContent(&box_str("Write")?)?;
            edit_write.Click(&RoutedEventHandler::new(|_, _| {
                gesture(DebuggerEvent::EditWrite);
                Ok(())
            }))?;
            let edit_watch = Button::new()?;
            edit_watch.SetContent(&box_str("Watch")?)?;
            edit_watch.Click(&RoutedEventHandler::new(|_, _| {
                gesture(DebuggerEvent::WatchSet);
                Ok(())
            }))?;
            let edit_freeze = Button::new()?;
            edit_freeze.SetContent(&box_str("Freeze")?)?;
            edit_freeze.Click(&RoutedEventHandler::new(|_, _| {
                gesture(DebuggerEvent::EditFreezeSet);
                Ok(())
            }))?;
            let edit_confirm = Button::new()?;
            edit_confirm.SetContent(&box_str("Confirm")?)?;
            edit_confirm.Click(&RoutedEventHandler::new(|_, _| {
                gesture(DebuggerEvent::ConfirmWrite);
                Ok(())
            }))?;
            let edit_cancel = Button::new()?;
            edit_cancel.SetContent(&box_str("Cancel")?)?;
            edit_cancel.Click(&RoutedEventHandler::new(|_, _| {
                gesture(DebuggerEvent::CancelWrite);
                Ok(())
            }))?;
            for button in [
                &edit_write,
                &edit_watch,
                &edit_freeze,
                &edit_confirm,
                &edit_cancel,
            ] {
                edit_row.Children()?.Append(button)?;
            }
            // Selecting a disassembly row pins the anchor; the Jump
            // button follows its branch target.
            dis_list.SelectionChanged(&SelectionChangedEventHandler::new(|sender, _| {
                let list: ListView = sender
                    .as_ref()
                    .and_then(|sender| sender.cast().ok())
                    .ok_or_else(|| cast_err("dis list cast"))?;
                let index = list.SelectedIndex()?;
                super::app::with_app(|app| app.debug_select_dis_row(index));
                Ok(())
            }))?;
            // Single click pins the memory row anchor for editing.
            mem_list.SelectionChanged(&SelectionChangedEventHandler::new(|sender, _| {
                let list: ListView = sender
                    .as_ref()
                    .and_then(|sender| sender.cast().ok())
                    .ok_or_else(|| cast_err("mem list cast"))?;
                let index = list.SelectedIndex()?;
                super::app::with_app(|app| app.debug_select_row(index));
                Ok(())
            }))?;

            let root = ScrollViewer::new()?;
            let stack = StackPanel::new()?;
            stack.SetOrientation(Orientation::Vertical)?;
            let items = stack.Children()?;
            items.Append(&toolbar)?;
            items.Append(&status)?;
            items.Append(&dis_nav)?;
            items.Append(&dis_list)?;
            items.Append(&space_label)?;
            items.Append(&mem_nav)?;
            items.Append(&mem_list)?;
            items.Append(&reg_grid)?;
            items.Append(&reg_target_label)?;
            items.Append(&reg_input)?;
            items.Append(&reg_set)?;
            items.Append(&watch_label)?;
            items.Append(&watch_btn)?;
            items.Append(&watch_clear)?;
            items.Append(&freeze_label)?;
            items.Append(&freeze_input)?;
            items.Append(&freeze_btn)?;
            items.Append(&freeze_clear)?;
            items.Append(&bookmark_input)?;
            items.Append(&bookmark_add)?;
            items.Append(&bookmark_list)?;
            items.Append(&edit_label)?;
            items.Append(&edit_input)?;
            items.Append(&edit_row)?;
            root.SetContent(&stack)?;
            window.SetContent(&root)?;
            window.Activate()?;

            let mut win = Self {
                window,
                model,
                flight: None,
                dis_list,
                dis_addr,
                follow_check,
                status,
                mem_list,
                mem_addr,
                space_label,
                reg_grid,
                reg_target_label,
                reg_input,
                watch_label,
                freeze_label,
                freeze_input,
                bookmark_input,
                bookmark_list,
                edit_label,
                edit_input,
                building: Rc::new(RefCell::new(false)),
            };
            let _ = win.refresh_all();
            Ok(win)
        })();
        match flow {
            Ok(mut win) => {
                win.act(host, &[DebugAction::Refresh]);
                Some(win)
            }
            Err(error) => {
                log::error!("winui debugger open failed: {error}");
                None
            }
        }
    }

    /// Poll the in-flight drain; called from the main tick.
    pub fn poll(&mut self, _host: &mut nerust_win::WinHost) {
        if let Some(flight) = self.flight.as_mut()
            && let DrainPoll::Done(output) = flight.poll()
        {
            self.model.apply_drain(output.display, output.reselected);
            let _ = self.refresh_all();
            self.flight = None;
        }
    }

    fn refresh_all(&mut self) -> Result<()> {
        self.building.replace(true);
        let display = &self.model.display;
        let paused = if display.paused { "PAUSED" } else { "RUNNING" };
        let _ = self.status.SetText(&h(&format!(
            "{paused} | {} disasm lines",
            display.disasm_lines.len()
        )));
        // Disassembly.
        if self
            .dis_list
            .Items()
            .and_then(|items| items.Clear())
            .is_ok()
        {
            for line in display.disasm_lines.iter().take(256) {
                let bytes: String = line.bytes[..line.len as usize]
                    .iter()
                    .map(|b| format!("{b:02X}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                let mark = if line.is_pc { ">" } else { " " };
                let text = format!("{mark}{:08X}: {:<12} {}", line.addr, bytes, line.text);
                if let Ok(items) = self.dis_list.Items()
                    && let Ok(item) = box_str(&text)
                    && items.Append(&item).is_err()
                {
                    break;
                }
            }
        }
        let _ = self
            .follow_check
            .SetIsOn(self.model.nav.dis_addr().is_none());
        let _ = self
            .space_label
            .SetText(&h(&format!("Space: {}", self.model.selected_name())));
        // Registers.
        if self
            .reg_grid
            .Children()
            .and_then(|items| items.Clear())
            .is_ok()
        {
            self.reg_grid.ColumnDefinitions()?.Clear()?;
            for (index, (name, value)) in display.regs.iter().enumerate() {
                let button = Button::new()?;
                button.SetContent(&box_str(&format!("{name}: {value:02X}"))?)?;
                let reg_name = *name;
                button.Click(&RoutedEventHandler::new(move |_, _| {
                    gesture(DebuggerEvent::RegSelected(reg_name));
                    Ok(())
                }))?;
                Grid::SetColumn(&button, (index % 2) as i32)?;
                Grid::SetRow(&button, (index / 2) as i32)?;
                self.reg_grid.Children()?.Append(&button)?;
            }
            while self.reg_grid.ColumnDefinitions()?.Size()? < 2 {
                let def = ColumnDefinition::new()?;
                self.reg_grid.ColumnDefinitions()?.Append(&def)?;
            }
            while self.reg_grid.RowDefinitions()?.Size()? < display.regs.len().div_ceil(2) as u32 {
                let def = RowDefinition::new()?;
                def.SetHeight(GridLengthHelper::Auto()?)?;
                self.reg_grid.RowDefinitions()?.Append(&def)?;
            }
        }
        let target = self
            .model
            .reg_target
            .map(|name| format!("Set {name}"))
            .unwrap_or_else(|| "Pick a register".to_string());
        let _ = self.reg_target_label.SetText(&h(&target));
        // Watch / freeze.
        let watch = match (self.model.watch, display.watch_value) {
            (Some(addr), Some(value)) => format!("Watch: {addr:08X} = {value:02X}"),
            (Some(addr), None) => format!("Watch: {addr:08X} = --"),
            (None, _) => "Watch: none".to_string(),
        };
        let _ = self.watch_label.SetText(&h(&watch));
        let freeze = match self.model.freeze {
            Some((addr, value)) => format!("Freeze: {addr:08X} = {value:02X}"),
            None => "Freeze: none".to_string(),
        };
        let _ = self.freeze_label.SetText(&h(&freeze));
        // Memory dump.
        if self
            .mem_list
            .Items()
            .and_then(|items| items.Clear())
            .is_ok()
        {
            for row in display.dump_rows.iter() {
                let bytes: String = row.bytes[..row.valid as usize]
                    .iter()
                    .map(|b| format!("{b:02X}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                let text = format!("{:08X}: {bytes}", row.addr);
                if let Ok(items) = self.mem_list.Items()
                    && let Ok(item) = box_str(&text)
                    && items.Append(&item).is_err()
                {
                    break;
                }
            }
        }
        // Bookmarks.
        if self
            .bookmark_list
            .Items()
            .and_then(|items| items.Clear())
            .is_ok()
        {
            for (addr, label) in self.model.bookmarks.clone() {
                let text = format!("{label} 0x{addr:08X}");
                if let Ok(items) = self.bookmark_list.Items()
                    && let Ok(item) = box_str(&text)
                    && items.Append(&item).is_err()
                {
                    break;
                }
            }
        }
        // Edit row.
        let edit = match self.model.selected {
            Some((addr, _)) => format!("Edit {addr:08X}"),
            None => "Select dump row".to_string(),
        };
        let _ = self.edit_label.SetText(&h(&edit));
        self.building.replace(false);
        Ok(())
    }

    pub(crate) fn echo_inputs(&mut self) {
        let mem = self
            .mem_addr
            .Text()
            .map(|text| text.to_string())
            .unwrap_or_default();
        let dis = self
            .dis_addr
            .Text()
            .map(|text| text.to_string())
            .unwrap_or_default();
        let bookmark = self
            .bookmark_input
            .Text()
            .map(|text| text.to_string())
            .unwrap_or_default();
        let freeze = self
            .freeze_input
            .Text()
            .map(|text| text.to_string())
            .unwrap_or_default();
        let edit = self
            .edit_input
            .Text()
            .map(|text| text.to_string())
            .unwrap_or_default();
        let reg = self
            .reg_input
            .Text()
            .map(|text| text.to_string())
            .unwrap_or_default();
        self.model.dispatch(DebuggerEvent::MemInputChanged(mem));
        self.model.dispatch(DebuggerEvent::DisInputChanged(dis));
        self.model
            .dispatch(DebuggerEvent::BookmarkInputChanged(bookmark));
        self.model
            .dispatch(DebuggerEvent::FreezeInputChanged(freeze));
        self.model.dispatch(DebuggerEvent::EditInputChanged(edit));
        self.model.dispatch(DebuggerEvent::RegInputChanged(reg));
    }

    pub(crate) fn selected_dis_row(&self) -> Option<usize> {
        let index = self.dis_list.SelectedIndex().ok()?;
        (index >= 0).then_some(index as usize)
    }

    pub(crate) fn act(&mut self, host: &mut nerust_win::WinHost, actions: &[DebugAction]) {
        if actions.is_empty() {
            let _ = self.refresh_all();
            return;
        }
        // Pause-family actions run on the host first (inspect requires
        // pause); the drain only sees emulator actions.
        let mut drain_actions = Vec::new();
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
                other => drain_actions.push(*other),
            }
        }
        if drain_actions.is_empty() {
            let _ = self.refresh_all();
            return;
        }
        let input = DebugDrainInput {
            space: self.model.selected_space(),
            mem_addr: self.model.anchor(),
            dis_addr: self.model.nav.dis_addr(),
            watch: self.model.watch,
            freeze: self.model.freeze,
            reg_target: self.model.reg_target,
            prev_dump_rows: &[],
            need_images: false,
        };
        let prev = self.model.display.dump_rows.clone();
        let input = DebugDrainInput {
            prev_dump_rows: &prev,
            ..input
        };
        let _ = &prev;
        self.flight = Some(host.session().debug_drain_begin(&drain_actions, &input));
    }
}

/// Route a gesture through the reducer, then drain when actions exist.
fn gesture(event: DebuggerEvent) {
    super::app::with_app(|app| app.debug_gesture(event));
}
