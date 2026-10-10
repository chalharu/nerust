//! GTK debugger window: two-pane memory/disassembly probe plus
//! register, watch, freeze, and bookmark state.
//!
//! Same semantics as the tao window, same shell APIs, same view-model
//! formatters. GTK is synchronous, so there is no request queue: each
//! action runs [`run_debug_actions`] (the drain phase order) and then
//! rebuilds the widgets. Paused pull-only like tao; the window opens
//! paused and closing keeps the pause.

use std::{cell::RefCell, rc::Rc};

use gtk::{gdk, glib, prelude::*};
use nerust_core_traits::debugger::{DUMP_ROW_BYTES, SpaceId, StepUnit};
use nerust_gui_viewmodel::debugger::{self as vm, DisplayCache, NavState};

use super::State;

/// Rows per dump page; paging moves by a full page.
const DUMP_PAGE_ROWS: i32 = 8;
/// Bookmark cap (window-local, no file yet).
const BOOKMARK_CAP: usize = 8;

/// One action batch for [`run_debug_actions`]. Same phases as the tao
/// drain: execution, cancel/commit, stale-kill, prepare/stage, freeze,
/// snapshot. Order inside is load-bearing (kill runs before prepare).
#[derive(Debug, Clone)]
pub(crate) enum DebugAction {
    Refresh,
    Pause,
    Resume,
    TogglePause,
    StepFrame,
    StepInstr,
    MemNav,
    SelectRow(u32),
    /// Stage a parsed value into the shell-owned transaction.
    EditStage(u64),
    EditCancel,
    WriteConfirm,
}

/// Presentation state: movement, display data, and user lists.
/// Mirrors the tao bridge fields one to one.
pub(crate) struct DebuggerData {
    pub(crate) nav: NavState,
    pub(crate) spaces: Vec<(SpaceId, String, u32)>,
    pub(crate) display: DisplayCache,
    pub(crate) bookmarks: Vec<(u32, String)>,
    pub(crate) watch: Option<u32>,
    pub(crate) freeze: Option<(u32, u8)>,
    pub(crate) selected: Option<(u32, Option<u8>)>,
}

impl DebuggerData {
    fn new(session: &nerust_gui_shell::session::SessionHandle) -> Self {
        let spaces: Vec<(SpaceId, String, u32)> = session
            .debug_spaces()
            .iter()
            .map(|info| (info.id, info.name.to_string(), *info.range.start()))
            .collect();
        let start = spaces.first().map(|(_, _, start)| *start).unwrap_or(0);
        Self {
            nav: NavState::new(spaces.len(), start),
            spaces,
            display: DisplayCache::default(),
            bookmarks: Vec::new(),
            watch: None,
            freeze: None,
            selected: None,
        }
    }

    fn selected_space(&self) -> Option<SpaceId> {
        self.spaces.get(self.nav.space_idx()).map(|(id, _, _)| *id)
    }

    fn selected_name(&self) -> String {
        self.spaces
            .get(self.nav.space_idx())
            .map(|(_, name, _)| name.clone())
            .unwrap_or_default()
    }

    /// Displayed disassembly anchor: first visible line in follow-PC
    /// mode (the stored pin is stale there), the pin otherwise.
    fn anchor(&self) -> u32 {
        if self.nav.follow_pc() {
            self.display
                .disasm_lines
                .first()
                .map(|line| line.addr)
                .unwrap_or(0)
        } else {
            self.nav.dis_addr().unwrap_or(0)
        }
    }
}

/// Run one action batch against the session and publish a batched
/// snapshot into `data.display`. The status line lands in
/// `display.status` like the tao drain.
pub(crate) fn run_debug_actions(
    data: &mut DebuggerData,
    session: &nerust_gui_shell::session::SessionHandle,
    actions: &[DebugAction],
) {
    let mut status = String::new();
    let mut state_changing = false;
    for action in actions {
        let outcome = match action {
            DebugAction::Refresh | DebugAction::MemNav | DebugAction::SelectRow(_) => {
                state_changing = true;
                continue;
            }
            DebugAction::Pause => session.debug_pause(),
            DebugAction::Resume => session.debug_resume(),
            DebugAction::TogglePause => {
                if session.debug_paused() {
                    session.debug_resume()
                } else {
                    session.debug_pause()
                }
            }
            DebugAction::StepFrame => {
                state_changing = true;
                session.debug_step(StepUnit::Frame)
            }
            DebugAction::StepInstr => {
                state_changing = true;
                session.debug_step(StepUnit::Instruction)
            }
            // Transaction traffic never kills: it builds the
            // transaction this batch consumes.
            DebugAction::EditStage(_) | DebugAction::EditCancel | DebugAction::WriteConfirm => {
                continue;
            }
        };
        status = outcome;
    }
    // Cancel/commit first: cancel clears, commit writes and consumes.
    for action in actions {
        match action {
            DebugAction::EditCancel => session.debug_clear_write(),
            DebugAction::WriteConfirm => {
                status = session.debug_commit_write();
            }
            _ => {}
        }
    }
    // Kill staged-but-uncommitted values from previous batches first;
    // SelectRow prepares fresh state after this.
    if state_changing {
        session.debug_note_traffic(true);
    }
    // Row selection reads the old byte for the selection display.
    for action in actions {
        if let DebugAction::SelectRow(addr) = action {
            let space = data.selected_space();
            let old = space.and_then(|space| session.debug_read_byte(space, *addr));
            data.selected = Some((*addr, old));
            if let Some(space) = space {
                session.debug_prepare_write(space, *addr);
            }
        }
        if let DebugAction::EditStage(value) = action {
            session.debug_stage_write(*value);
        }
    }
    // Frozen values re-apply on every batch (silent by design).
    if let Some((addr, value)) = data.freeze
        && let Some(space) = data.selected_space()
    {
        let _ = session.debug_write_byte(space, addr, value);
    }
    let space = data.selected_space();
    let prev = data.display.dump_rows.clone();
    let snapshot = session.debug_snapshot(space, data.nav.mem_addr(), data.nav.dis_addr());
    let watch_value = data
        .watch
        .and_then(|addr| space.and_then(|s| session.debug_read_byte(s, addr)));
    let pending_text = session.debug_pending_text();
    data.display.regs = snapshot.regs;
    data.display.dump_rows = snapshot.dump_rows.clone();
    data.display.diff = vm::diff_rows(&prev, &snapshot.dump_rows);
    data.display.disasm_lines = snapshot.disasm_lines;
    data.display.panels = snapshot.panels;
    data.display.images = snapshot.images;
    data.display.watch_value = watch_value;
    data.display.pending_text = pending_text;
    data.display.status = if status.is_empty() {
        if session.debug_paused() {
            "paused".to_string()
        } else {
            "running".to_string()
        }
    } else {
        status
    };
}

pub(crate) type DebuggerWindow = Rc<RefCell<DebuggerWindowCore>>;

pub(crate) struct DebuggerWindowCore {
    window: gtk::ApplicationWindow,
    state: Rc<RefCell<State>>,
    data: DebuggerData,
    ppu_window: Option<super::ppu_window::PpuWindow>,
    status_label: gtk::Label,
    disasm_title: gtk::Label,
    disasm_list: gtk::ListBox,
    dis_input: gtk::Entry,
    back_button: gtk::Button,
    space_label: gtk::Label,
    mem_input: gtk::Entry,
    dump_list: gtk::ListBox,
    select_label: gtk::Label,
    edit_input: gtk::Entry,
    confirm_row: gtk::Box,
    confirm_label: gtk::Label,
    regs_label: gtk::Label,
    watch_label: gtk::Label,
    freeze_input: gtk::Entry,
    freeze_label: gtk::Label,
    bookmark_input: gtk::Entry,
    bookmark_list: gtk::ListBox,
    panels_label: gtk::Label,
}

fn section(title: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(title));
    label.set_xalign(0.0);
    label
}

fn static_row(text: &str) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_selectable(false);
    let label = gtk::Label::new(Some(text));
    label.add_css_class("monospace");
    label.set_xalign(0.0);
    row.set_child(Some(&label));
    row
}

fn clear_list(list: &gtk::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

/// Left-align a button label (GTK centers by default; debugger rows
/// read like the tao monospace columns).
fn left_align(button: &gtk::Button) {
    if let Some(child) = button.first_child()
        && let Ok(label) = child.downcast::<gtk::Label>()
    {
        label.set_xalign(0.0);
    }
}

fn entry(placeholder: Option<&str>) -> gtk::Entry {
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(placeholder);
    entry
}

fn action_button(label: &str) -> gtk::Button {
    gtk::Button::with_label(label)
}

impl DebuggerWindowCore {
    /// Open the debugger: pin pause, build widgets, refresh, present.
    /// Closing keeps the pause; resume stays explicit.
    pub(crate) fn open(
        app: &gtk::Application,
        main_window: &gtk::ApplicationWindow,
        state: Rc<RefCell<State>>,
    ) -> DebuggerWindow {
        state.borrow().session.debug_pause();
        let data = DebuggerData::new(&state.borrow().session);
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("Debugger")
            .default_width(1100)
            .default_height(800)
            .build();
        window.set_transient_for(Some(main_window));

        let status_label = gtk::Label::new(None);
        status_label.set_xalign(0.0);
        let disasm_title = gtk::Label::new(Some("Disassembly"));
        disasm_title.set_xalign(0.0);
        let disasm_list = gtk::ListBox::new();
        let dis_input = entry(Some("addr hex"));
        let back_button = action_button("Back (0)");
        let space_label = gtk::Label::new(None);
        space_label.set_xalign(0.0);
        space_label.set_hexpand(true);
        let mem_input = entry(Some("addr hex"));
        let dump_list = gtk::ListBox::new();
        let select_label = gtk::Label::new(Some("Select a dump row"));
        select_label.set_xalign(0.0);
        let edit_input = entry(Some("hex byte"));
        let confirm_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let confirm_label = gtk::Label::new(None);
        confirm_label.set_xalign(0.0);
        confirm_label.set_hexpand(true);
        let regs_label = gtk::Label::new(None);
        regs_label.add_css_class("monospace");
        regs_label.set_xalign(0.0);
        let watch_label = gtk::Label::new(Some("watch: none"));
        watch_label.set_xalign(0.0);
        watch_label.set_hexpand(true);
        let freeze_input = entry(Some("hex byte"));
        let freeze_label = gtk::Label::new(Some("freeze: none"));
        freeze_label.set_xalign(0.0);
        freeze_label.set_hexpand(true);
        let bookmark_input = entry(Some("label"));
        let bookmark_list = gtk::ListBox::new();
        let panels_label = gtk::Label::new(None);
        panels_label.add_css_class("monospace");
        panels_label.set_xalign(0.0);

        let result: DebuggerWindow = Rc::new(RefCell::new(Self {
            window: window.clone(),
            state: state.clone(),
            data,
            ppu_window: None,
            status_label: status_label.clone(),
            disasm_title: disasm_title.clone(),
            disasm_list: disasm_list.clone(),
            dis_input: dis_input.clone(),
            back_button: back_button.clone(),
            space_label: space_label.clone(),
            mem_input: mem_input.clone(),
            dump_list: dump_list.clone(),
            select_label: select_label.clone(),
            edit_input: edit_input.clone(),
            confirm_row: confirm_row.clone(),
            confirm_label: confirm_label.clone(),
            regs_label: regs_label.clone(),
            watch_label: watch_label.clone(),
            freeze_input: freeze_input.clone(),
            freeze_label: freeze_label.clone(),
            bookmark_input: bookmark_input.clone(),
            bookmark_list: bookmark_list.clone(),
            panels_label: panels_label.clone(),
        }));

        // Toolbar: execution above disassembly, like tao.
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        for (label, action) in [
            ("Pause", DebugAction::Pause),
            ("Resume", DebugAction::Resume),
            ("Step Frame", DebugAction::StepFrame),
            ("Step Instr", DebugAction::StepInstr),
        ] {
            let button = action_button(label);
            let win = result.clone();
            button.connect_clicked(move |_| Self::act(&win, std::slice::from_ref(&action)));
            toolbar.append(&button);
        }
        let ppu_button = action_button("PPU Viewer");
        {
            let win = result.clone();
            ppu_button.connect_clicked(move |_| Self::open_ppu(&win));
        }
        toolbar.append(&ppu_button);

        // Disassembly header: title, Follow PC, addr entry, Go, Back.
        let dis_nav = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let follow_button = action_button("Follow PC");
        {
            let win = result.clone();
            follow_button.connect_clicked(move |_| {
                win.borrow_mut().data.nav.follow();
                Self::act(&win, &[DebugAction::Refresh]);
            });
        }
        let go_button = action_button("Go");
        {
            let win = result.clone();
            go_button.connect_clicked(move |_| Self::dis_go(&win));
            let win = result.clone();
            dis_input.connect_activate(move |_| Self::dis_go(&win));
        }
        {
            let win = result.clone();
            back_button.connect_clicked(move |_| Self::dis_back(&win));
        }
        dis_nav.append(&disasm_title);
        dis_nav.append(&follow_button);
        dis_nav.append(&dis_input);
        dis_nav.append(&go_button);
        dis_nav.append(&back_button);

        let dis_scroll = gtk::ScrolledWindow::new();
        dis_scroll.set_vexpand(true);
        dis_scroll.set_child(Some(&disasm_list));

        let left_col = gtk::Box::new(gtk::Orientation::Vertical, 12);
        left_col.set_hexpand(true);
        left_col.set_vexpand(true);
        let title = gtk::Label::new(Some("Debugger"));
        title.set_xalign(0.0);
        left_col.append(&title);
        left_col.append(&toolbar);
        left_col.append(&dis_nav);
        left_col.append(&dis_scroll);

        // Right state column (scrolls independently; PPU panels are long).
        let right_col = gtk::Box::new(gtk::Orientation::Vertical, 12);
        right_col.append(&status_label);
        right_col.append(&section("Registers"));
        right_col.append(&regs_label);
        right_col.append(&section("Watch"));
        let watch_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let watch_button = action_button("Watch");
        {
            let win = result.clone();
            watch_button.connect_clicked(move |_| Self::watch_set(&win));
        }
        let watch_clear = action_button("Clear");
        {
            let win = result.clone();
            watch_clear.connect_clicked(move |_| Self::watch_clear(&win));
        }
        watch_row.append(&watch_button);
        watch_row.append(&watch_label);
        watch_row.append(&watch_clear);
        right_col.append(&watch_row);
        right_col.append(&section("Freeze"));
        let freeze_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let freeze_button = action_button("Freeze");
        {
            let win = result.clone();
            freeze_button.connect_clicked(move |_| Self::freeze_set(&win));
        }
        let freeze_clear = action_button("Clear");
        {
            let win = result.clone();
            freeze_clear.connect_clicked(move |_| Self::freeze_clear(&win));
        }
        freeze_row.append(&freeze_input);
        freeze_row.append(&freeze_button);
        freeze_row.append(&freeze_label);
        freeze_row.append(&freeze_clear);
        right_col.append(&freeze_row);
        right_col.append(&section("Bookmarks"));
        let bm_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        bookmark_input.set_hexpand(true);
        let bm_add = action_button("Add");
        {
            let win = result.clone();
            bm_add.connect_clicked(move |_| Self::bookmark_add(&win));
            let win = result.clone();
            bookmark_input.connect_activate(move |_| Self::bookmark_add(&win));
        }
        bm_row.append(&bookmark_input);
        bm_row.append(&bm_add);
        right_col.append(&bm_row);
        right_col.append(&bookmark_list);
        right_col.append(&section("Panels"));
        right_col.append(&panels_label);

        let panes = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        panes.set_vexpand(true);
        panes.append(&left_col);
        let right_scroll = gtk::ScrolledWindow::new();
        right_scroll.set_size_request(340, -1);
        right_scroll.set_child(Some(&right_col));
        panes.append(&right_scroll);

        // Memory section: full width below the panes.
        let memory = gtk::Box::new(gtk::Orientation::Vertical, 10);
        memory.append(&section("Memory"));
        let space_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let space_title = gtk::Label::new(Some("space"));
        space_title.set_size_request(150, -1);
        space_title.set_xalign(0.0);
        let space_prev = action_button("Prev");
        {
            let win = result.clone();
            space_prev.connect_clicked(move |_| Self::cycle_space(&win, -1));
        }
        let space_next = action_button("Next");
        {
            let win = result.clone();
            space_next.connect_clicked(move |_| Self::cycle_space(&win, 1));
        }
        space_row.append(&space_title);
        space_row.append(&space_prev);
        space_row.append(&space_label);
        space_row.append(&space_next);
        memory.append(&space_row);
        let mem_nav = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let mem_prev = action_button("Prev");
        {
            let win = result.clone();
            mem_prev.connect_clicked(move |_| Self::page_mem(&win, -1));
        }
        let mem_next = action_button("Next");
        {
            let win = result.clone();
            mem_next.connect_clicked(move |_| Self::page_mem(&win, 1));
        }
        let mem_go = action_button("Go");
        {
            let win = result.clone();
            mem_go.connect_clicked(move |_| Self::mem_go(&win));
            let win = result.clone();
            mem_input.connect_activate(move |_| Self::mem_go(&win));
        }
        mem_nav.append(&mem_prev);
        mem_nav.append(&mem_next);
        mem_nav.append(&mem_input);
        mem_nav.append(&mem_go);
        memory.append(&mem_nav);
        let dump_scroll = gtk::ScrolledWindow::new();
        // Fixed share: the memory section stays on screen with the
        // panes above; the list scrolls inside its 5-row window.
        dump_scroll.set_size_request(-1, 200);
        dump_scroll.set_child(Some(&dump_list));
        memory.append(&dump_scroll);

        // Edit row + confirm row + Refresh.
        let edit_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        select_label.set_size_request(240, -1);
        let write_button = action_button("Write");
        {
            let win = result.clone();
            write_button.connect_clicked(move |_| Self::edit_write(&win));
        }
        let edit_watch = action_button("Watch");
        {
            let win = result.clone();
            edit_watch.connect_clicked(move |_| Self::watch_set(&win));
        }
        let edit_freeze = action_button("Freeze");
        {
            let win = result.clone();
            edit_freeze.connect_clicked(move |_| Self::freeze_set(&win));
        }
        edit_row.append(&select_label);
        edit_row.append(&edit_input);
        edit_row.append(&write_button);
        edit_row.append(&edit_watch);
        edit_row.append(&edit_freeze);
        memory.append(&edit_row);
        let confirm_button = action_button("Confirm");
        confirm_button.add_css_class("suggested-action");
        {
            let win = result.clone();
            confirm_button.connect_clicked(move |_| Self::confirm_write(&win));
        }
        let cancel_button = action_button("Cancel");
        cancel_button.add_css_class("flat");
        {
            let win = result.clone();
            cancel_button.connect_clicked(move |_| Self::cancel_write(&win));
        }
        confirm_row.append(&confirm_label);
        confirm_row.append(&confirm_button);
        confirm_row.append(&cancel_button);
        memory.append(&confirm_row);
        let refresh_button = action_button("Refresh");
        {
            let win = result.clone();
            refresh_button.connect_clicked(move |_| Self::act(&win, &[DebugAction::Refresh]));
        }
        memory.append(&refresh_button);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
        content.set_margin_top(16);
        content.set_margin_bottom(16);
        content.set_margin_start(16);
        content.set_margin_end(16);
        content.append(&panes);
        content.append(&memory);
        // No outer scroll: panes stretch, the dump list has a fixed
        // share, so the edit row and Refresh stay visible at 800px.
        window.set_child(Some(&content));

        // Window-scoped keys: F6/F7/F9 execution, Up/Down dump paging.
        // Left/Right stay with the widgets (caret movement).
        let keys = gtk::EventControllerKey::new();
        {
            let win = result.clone();
            keys.connect_key_pressed(move |_, key, _, _| {
                match key {
                    gdk::Key::F6 => Self::act(&win, &[DebugAction::StepFrame]),
                    gdk::Key::F7 => Self::act(&win, &[DebugAction::StepInstr]),
                    gdk::Key::F9 => Self::act(&win, &[DebugAction::TogglePause]),
                    gdk::Key::Up => Self::page_mem(&win, -1),
                    gdk::Key::Down => Self::page_mem(&win, 1),
                    _ => (),
                }
                glib::Propagation::Proceed
            });
        }
        window.add_controller(keys);

        window.connect_close_request(|window| {
            // Close keeps the pause; resume stays explicit.
            window.set_visible(false);
            glib::Propagation::Stop
        });

        Self::act(&result, &[DebugAction::Refresh]);
        window.present();
        result
    }

    /// Present an existing window (idempotent reopen path).
    pub(crate) fn present(&self) {
        self.window.present();
    }

    /// Refresh an existing window (idempotent reopen path).
    pub(crate) fn refresh(win: &DebuggerWindow) {
        Self::act(win, &[DebugAction::Refresh]);
    }

    /// Run actions, then rebuild every widget from the new display.
    fn act(win: &DebuggerWindow, actions: &[DebugAction]) {
        let session = win.borrow().state.clone();
        let session = session.borrow();
        run_debug_actions(&mut win.borrow_mut().data, &session.session, actions);
        Self::rebuild(win);
    }

    fn set_status(win: &DebuggerWindow, status: String) {
        win.borrow_mut().data.display.status = status;
        Self::rebuild(win);
    }

    fn open_ppu(win: &DebuggerWindow) {
        let app = win
            .borrow()
            .window
            .application()
            .expect("debugger has an application");
        let main = win.borrow().window.clone();
        let state = win.borrow().state.clone();
        let images = win.borrow().data.display.images.clone();
        let ppu = super::ppu_window::PpuWindowCore::open(&app, &main, state, images);
        win.borrow_mut().ppu_window = Some(ppu);
    }

    fn mem_go(win: &DebuggerWindow) {
        let input = win.borrow().mem_input.text().to_string();
        match vm::parse_hex_addr(&input) {
            Some(addr) => {
                win.borrow_mut().data.nav.go_mem(addr);
                Self::act(win, &[DebugAction::MemNav]);
            }
            None => Self::set_status(win, format!("parse failed: {input}")),
        }
    }

    fn page_mem(win: &DebuggerWindow, delta: i32) {
        win.borrow_mut()
            .data
            .nav
            .page_mem(delta * DUMP_PAGE_ROWS * DUMP_ROW_BYTES as i32);
        Self::act(win, &[DebugAction::MemNav]);
    }

    fn cycle_space(win: &DebuggerWindow, delta: i32) {
        let stepped = win.borrow_mut().data.nav.cycle_space(delta);
        if let Some(idx) = stepped {
            let start = win.borrow().data.spaces[idx].2;
            let mut this = win.borrow_mut();
            this.data.nav.set_space(idx, start);
            this.data.selected = None;
            drop(this);
            Self::act(win, &[DebugAction::Refresh]);
        }
    }

    fn dis_go(win: &DebuggerWindow) {
        let input = win.borrow().dis_input.text().to_string();
        match vm::parse_hex_addr(&input) {
            Some(addr) => {
                let anchor = win.borrow().data.anchor();
                win.borrow_mut().data.nav.navigate(anchor, addr);
                Self::act(win, &[DebugAction::Refresh]);
            }
            None => Self::set_status(win, format!("parse failed: {input}")),
        }
    }

    fn dis_back(win: &DebuggerWindow) {
        if win.borrow_mut().data.nav.go_back().is_some() {
            Self::act(win, &[DebugAction::Refresh]);
        } else {
            Self::set_status(win, "back stack is empty".to_string());
        }
    }

    fn follow_target(win: &DebuggerWindow, addr: u32) {
        let anchor = win.borrow().data.anchor();
        win.borrow_mut().data.nav.navigate(anchor, addr);
        Self::act(win, &[DebugAction::Refresh]);
    }

    fn dis_line_clicked(win: &DebuggerWindow, addr: u32) {
        let anchor = win.borrow().data.anchor();
        win.borrow_mut().data.nav.navigate(anchor, addr);
        Self::act(win, &[DebugAction::Refresh]);
    }

    fn bookmark_add(win: &DebuggerWindow) {
        let label = win.borrow().bookmark_input.text().trim().to_string();
        if label.is_empty() {
            return;
        }
        if win.borrow().data.bookmarks.len() >= BOOKMARK_CAP {
            Self::set_status(win, "bookmark list full".to_string());
            return;
        }
        let addr = win.borrow().data.anchor();
        {
            let mut this = win.borrow_mut();
            if !this.data.bookmarks.iter().any(|(a, _)| *a == addr) {
                this.data.bookmarks.push((addr, label));
            }
            this.bookmark_input.set_text("");
        }
        Self::rebuild(win);
    }

    fn bookmark_jump(win: &DebuggerWindow, addr: u32) {
        let anchor = win.borrow().data.anchor();
        win.borrow_mut().data.nav.navigate(anchor, addr);
        Self::act(win, &[DebugAction::Refresh]);
    }

    fn bookmark_delete(win: &DebuggerWindow, addr: u32) {
        win.borrow_mut().data.bookmarks.retain(|(a, _)| *a != addr);
        Self::rebuild(win);
    }

    fn row_selected(win: &DebuggerWindow, addr: u32) {
        Self::act(win, &[DebugAction::SelectRow(addr)]);
    }

    fn watch_set(win: &DebuggerWindow) {
        // Copy out first: matching on the borrowed guard while taking
        // a mutable borrow below panics (RefCell already borrowed).
        let selected = win.borrow().data.selected.map(|(addr, _)| addr);
        match selected {
            Some(addr) => {
                win.borrow_mut().data.watch = Some(addr);
                Self::act(win, &[DebugAction::Refresh]);
            }
            None => Self::set_status(win, "watch needs a selection".to_string()),
        }
    }

    fn watch_clear(win: &DebuggerWindow) {
        let mut this = win.borrow_mut();
        this.data.watch = None;
        this.data.display.watch_value = None;
        drop(this);
        Self::rebuild(win);
    }

    fn freeze_set(win: &DebuggerWindow) {
        let selected = win.borrow().data.selected.map(|(addr, _)| addr);
        let input = win.borrow().freeze_input.text().trim().to_string();
        match (selected, vm::parse_hex_addr(&input).filter(|v| *v <= 0xFF)) {
            (Some(addr), Some(value)) => {
                win.borrow_mut().data.freeze = Some((addr, value as u8));
                Self::set_status(win, format!("frozen {addr:08X}={value:02X}"));
                Self::act(win, &[DebugAction::Refresh]);
            }
            _ => Self::set_status(win, "freeze needs a selected row and hex byte".to_string()),
        }
    }

    fn freeze_clear(win: &DebuggerWindow) {
        win.borrow_mut().data.freeze = None;
        Self::set_status(win, "freeze cleared".to_string());
    }

    fn edit_write(win: &DebuggerWindow) {
        let selected = win.borrow().data.selected;
        let input = win.borrow().edit_input.text().trim().to_string();
        match (
            selected,
            vm::parse_hex_addr(&input).filter(|value| *value <= 0xFF),
        ) {
            (Some((addr, _)), Some(value)) => {
                // Re-select first: the batch prepares fresh state
                // before staging, so Write also works after a cancel.
                // GTK runs batches synchronously, so the authoritative
                // confirm row is already in the display after act.
                Self::act(
                    win,
                    &[
                        DebugAction::SelectRow(addr),
                        DebugAction::EditStage(value as u64),
                    ],
                );
            }
            (None, _) => Self::set_status(win, "write needs a selection".to_string()),
            _ => Self::set_status(win, format!("parse failed: {input}")),
        }
    }

    fn confirm_write(win: &DebuggerWindow) {
        Self::act(win, &[DebugAction::WriteConfirm]);
        // Commit consumes the transaction; the input clears.
        win.borrow().edit_input.set_text("");
        Self::rebuild(win);
    }

    fn cancel_write(win: &DebuggerWindow) {
        Self::act(win, &[DebugAction::EditCancel]);
    }

    /// Rebuild every widget from `data`. Single refresh point; widget
    /// callbacks never touch the session directly.
    fn rebuild(win: &DebuggerWindow) {
        let this = win.borrow();
        let display = &this.data.display;
        let nav = &this.data.nav;
        this.status_label.set_label(&display.status);
        this.disasm_title.set_label(if nav.follow_pc() {
            "Disassembly (follow PC)"
        } else {
            "Disassembly (fixed)"
        });
        this.back_button
            .set_label(&format!("Back ({})", nav.back_len()));
        this.space_label.set_label(&this.data.selected_name());
        if this.mem_input.text() != nav.mem_input() {
            this.mem_input.set_text(nav.mem_input());
        }
        if this.dis_input.text() != nav.dis_input() {
            this.dis_input.set_text(nav.dis_input());
        }

        clear_list(&this.disasm_list);
        if display.disasm_lines.is_empty() {
            this.disasm_list.append(&static_row("(no disassembly)"));
        }
        for line in &display.disasm_lines {
            let row = gtk::ListBoxRow::new();
            row.set_selectable(false);
            let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            let button = gtk::Button::with_label(&vm::format_disasm_line(line));
            button.add_css_class("monospace");
            left_align(&button);
            button.set_hexpand(true);
            button.set_halign(gtk::Align::Fill);
            if line.is_pc {
                button.add_css_class("suggested-action");
            }
            let addr = line.addr;
            let clicked = Rc::clone(win);
            button.connect_clicked(move |_| Self::dis_line_clicked(&clicked, addr));
            hbox.append(&button);
            if let Some(target) = line.target {
                let follow = gtk::Button::with_label("→");
                follow.set_size_request(40, -1);
                let followed = Rc::clone(win);
                follow.connect_clicked(move |_| Self::follow_target(&followed, target));
                hbox.append(&follow);
            }
            row.set_child(Some(&hbox));
            this.disasm_list.append(&row);
        }

        clear_list(&this.dump_list);
        for (addr, line) in &display.dump_rows {
            let is_selected = this.data.selected.map(|(a, _)| a) == Some(*addr);
            let changed = display.diff.contains(addr);
            let marker = match (is_selected, changed) {
                (true, true) => ">*",
                (true, false) => "> ",
                (false, true) => " *",
                (false, false) => "  ",
            };
            let row = gtk::ListBoxRow::new();
            row.set_selectable(false);
            let button = gtk::Button::with_label(&format!("{marker}{line}"));
            button.add_css_class("monospace");
            left_align(&button);
            button.set_hexpand(true);
            button.set_halign(gtk::Align::Fill);
            if is_selected {
                button.add_css_class("suggested-action");
            }
            let addr = *addr;
            let clicked = Rc::clone(win);
            button.connect_clicked(move |_| Self::row_selected(&clicked, addr));
            row.set_child(Some(&button));
            this.dump_list.append(&row);
        }

        let select_text = match this.data.selected {
            Some((addr, Some(old))) => format!("Selected {addr:08X} (was {old:02X})"),
            Some((addr, None)) => format!("Selected {addr:08X} (was ??)"),
            None => "Select a dump row".to_string(),
        };
        this.select_label.set_label(&select_text);
        match &display.pending_text {
            Some(pending) => {
                this.confirm_label.set_label(pending);
                this.confirm_row.set_visible(true);
            }
            None => this.confirm_row.set_visible(false),
        }

        this.regs_label.set_label(&display.regs);
        let watch_text = match (this.data.watch, display.watch_value) {
            (Some(addr), Some(value)) => format!("watch {addr:08X} = {value:02X}"),
            (Some(addr), None) => format!("watch {addr:08X} = ??"),
            (None, _) => "watch: none".to_string(),
        };
        this.watch_label.set_label(&watch_text);
        let freeze_text = match this.data.freeze {
            Some((addr, value)) => format!("frozen {addr:08X}={value:02X}"),
            None => "freeze: none".to_string(),
        };
        this.freeze_label.set_label(&freeze_text);

        clear_list(&this.bookmark_list);
        for (addr, label) in &this.data.bookmarks {
            let row = gtk::ListBoxRow::new();
            row.set_selectable(false);
            let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            let jump = gtk::Button::with_label(&format!("{label} {addr:08X}"));
            jump.add_css_class("monospace");
            jump.add_css_class("flat");
            let addr = *addr;
            let jumped = Rc::clone(win);
            jump.connect_clicked(move |_| Self::bookmark_jump(&jumped, addr));
            let delete = gtk::Button::with_label("×");
            let deleted = Rc::clone(win);
            delete.connect_clicked(move |_| Self::bookmark_delete(&deleted, addr));
            hbox.append(&jump);
            hbox.append(&delete);
            row.set_child(Some(&hbox));
            this.bookmark_list.append(&row);
        }
        this.panels_label.set_label(&display.panels);

        // The PPU viewer follows the same refresh.
        if let Some(ppu) = this.ppu_window.as_ref() {
            ppu.borrow_mut().sync(&display.images);
        }
    }
}
