//! GTK debugger window: two-pane memory/disassembly probe plus
//! register, watch, freeze, and bookmark state.
//!
//! Same semantics as the tao window, same shell APIs, same view-model
//! formatters. GTK is synchronous, so there is no request queue: each
//! action batch runs the shared shell drain
//! ([`nerust_gui_shell::debug::SessionHandle::debug_drain`]) and then
//! rebuilds the widgets. Paused pull-only like tao; the window opens
//! paused and closing keeps the pause.

use std::{cell::RefCell, rc::Rc};

use gtk::{gdk, glib, prelude::*};
use nerust_core_traits::debugger::{DEBUG_DUMP_PAGE_ROWS, DUMP_ROW_BYTES, SpaceId};
use nerust_gui_settings::language::AppLanguage;
use nerust_gui_shell::debug::{DebugAction, DebugDrainInput};
use nerust_gui_viewmodel::debugger::{self as vm, DisplayCache, NavState};
use nerust_settings_core::i18n::{self as ui_msg, UiText, text};

use super::State;

/// Bookmark cap (window-local, no file yet).
const BOOKMARK_CAP: usize = 8;

/// Pure write-input resolution: selection plus a hex byte. Widget
/// callbacks stay thin; the rules are unit-tested below (and the
/// drain owns the transaction, never the widgets).
fn resolve_write_input(
    language: AppLanguage,
    selected: Option<(u32, Option<u8>)>,
    input: &str,
) -> Result<(u32, u64), String> {
    let input = input.trim().to_string();
    match (
        selected,
        vm::parse_hex_addr(&input).filter(|value| *value <= 0xFF),
    ) {
        (Some((addr, _)), Some(value)) => Ok((addr, value as u64)),
        (None, _) => Err(text(language, UiText::WriteNeedsSelection).to_string()),
        _ => Err(ui_msg::parse_failed(language, &input)),
    }
}

/// Pure freeze-input resolution, same shape as the write path.
fn resolve_freeze_input(
    language: AppLanguage,
    selected: Option<(u32, Option<u8>)>,
    input: &str,
) -> Result<(u32, u8), String> {
    let input = input.trim().to_string();
    match (
        selected,
        vm::parse_hex_addr(&input).filter(|value| *value <= 0xFF),
    ) {
        (Some((addr, _)), Some(value)) => Ok((addr, value as u8)),
        _ => Err(text(language, UiText::FreezeNeedsSelection).to_string()),
    }
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
    pub(crate) reg_target: Option<&'static str>,
    /// UI language snapshot at open time.
    pub(crate) language: AppLanguage,
}

impl DebuggerData {
    fn new(session: &nerust_gui_shell::session::SessionHandle) -> Self {
        let spaces: Vec<(SpaceId, String, u32)> = session
            .debug_spaces()
            .iter()
            .map(|info| (info.id, info.name.to_string(), *info.range.start()))
            .collect();
        let start = spaces.first().map(|(_, _, start)| *start).unwrap_or(0);
        let language = session.settings_snapshot().shared.general.language;
        Self {
            nav: NavState::new(spaces.len(), start),
            spaces,
            display: DisplayCache::default(),
            bookmarks: Vec::new(),
            watch: None,
            freeze: None,
            selected: None,
            reg_target: None,
            language,
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

pub(crate) type DebuggerWindow = Rc<RefCell<DebuggerWindowCore>>;

pub(crate) struct DebuggerWindowCore {
    window: gtk::ApplicationWindow,
    state: Rc<RefCell<State>>,
    data: DebuggerData,
    ppu_window: Option<super::ppu_window::PpuWindow>,
    pause_button: gtk::Button,
    write_button: gtk::Button,
    regs_left: gtk::ListBox,
    regs_right: gtk::ListBox,
    reg_target_label: gtk::Label,
    reg_input: gtk::Entry,
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
    watch_label: gtk::Label,
    freeze_input: gtk::Entry,
    freeze_label: gtk::Label,
    bookmark_input: gtk::Entry,
    bookmark_list: gtk::ListBox,
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
    /// Open the debugger: build widgets, then pause and refresh
    /// through the shared drain. Closing keeps the pause; resume
    /// stays explicit.
    pub(crate) fn open(
        app: &gtk::Application,
        main_window: &gtk::ApplicationWindow,
        state: Rc<RefCell<State>>,
    ) -> DebuggerWindow {
        let data = DebuggerData::new(&state.borrow().session);
        let language = data.language;
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title(text(language, UiText::Debugger))
            .default_width(1100)
            // Taller than the tao twin: Adwaita rows need ~920px
            // to show the panes plus the dump viewport.
            .default_height(920)
            .build();
        window.set_transient_for(Some(main_window));

        let pause_button = action_button(text(language, UiText::Pause));
        let status_label = gtk::Label::new(None);
        status_label.set_xalign(0.0);
        let disasm_title = gtk::Label::new(Some(text(language, UiText::Disassembly)));
        disasm_title.set_xalign(0.0);
        let disasm_list = gtk::ListBox::new();
        let dis_input = entry(Some(text(language, UiText::AddrHex)));
        let back_button = action_button(&format!("{} (0)", text(language, UiText::Back)));
        let space_label = gtk::Label::new(None);
        space_label.set_xalign(0.0);
        space_label.set_hexpand(true);
        let mem_input = entry(Some(text(language, UiText::AddrHex)));
        let dump_list = gtk::ListBox::new();
        let select_label = gtk::Label::new(Some(text(language, UiText::SelectDumpRow)));
        select_label.set_xalign(0.0);
        let edit_input = entry(Some(text(language, UiText::HexByte)));
        let confirm_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let confirm_label = gtk::Label::new(None);
        confirm_label.set_xalign(0.0);
        confirm_label.set_hexpand(true);
        // Registers flow into two clickable columns like the tao
        // viewer so tall lists (GBA: 17) cost half the height. Pure
        // text split, no system branches. Row click picks the write
        // target; the Set row below writes through the drain.
        let regs_left = gtk::ListBox::new();
        regs_left.set_selection_mode(gtk::SelectionMode::None);
        let regs_right = gtk::ListBox::new();
        regs_right.set_selection_mode(gtk::SelectionMode::None);
        let reg_target_label = gtk::Label::new(Some(text(language, UiText::PickRegister)));
        reg_target_label.set_xalign(0.0);
        let reg_input = entry(Some(text(language, UiText::Hex)));
        let regs_row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        regs_row.append(&regs_left);
        regs_row.append(&regs_right);
        let watch_label = gtk::Label::new(Some(text(language, UiText::WatchNone)));
        watch_label.set_xalign(0.0);
        watch_label.set_hexpand(true);
        let freeze_input = entry(Some(text(language, UiText::HexByte)));
        let freeze_label = gtk::Label::new(Some(text(language, UiText::FreezeNone)));
        freeze_label.set_xalign(0.0);
        freeze_label.set_hexpand(true);
        let bookmark_input = entry(Some(text(language, UiText::Label)));
        let bookmark_list = gtk::ListBox::new();
        let write_button = action_button(text(language, UiText::Write));

        let result: DebuggerWindow = Rc::new(RefCell::new(Self {
            window: window.clone(),
            state: state.clone(),
            data,
            ppu_window: None,
            pause_button: pause_button.clone(),
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
            write_button: write_button.clone(),
            confirm_row: confirm_row.clone(),
            confirm_label: confirm_label.clone(),
            regs_left: regs_left.clone(),
            regs_right: regs_right.clone(),
            reg_target_label: reg_target_label.clone(),
            reg_input: reg_input.clone(),
            watch_label: watch_label.clone(),
            freeze_input: freeze_input.clone(),
            freeze_label: freeze_label.clone(),
            bookmark_input: bookmark_input.clone(),
            bookmark_list: bookmark_list.clone(),
        }));

        // Toolbar: a single Pause/Resume toggle plus stepping,
        // like tao. The label follows the drained pause state.
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        toolbar.append(&pause_button);
        {
            let win = result.clone();
            pause_button.connect_clicked(move |_| {
                Self::act(&win, &[DebugAction::TogglePause]);
            });
        }
        for (label, action) in [
            (text(language, UiText::StepFrame), DebugAction::StepFrame),
            (text(language, UiText::StepInstr), DebugAction::StepInstr),
        ] {
            let button = action_button(label);
            let win = result.clone();
            button.connect_clicked(move |_| Self::act(&win, std::slice::from_ref(&action)));
            toolbar.append(&button);
        }
        let ppu_button = action_button(text(language, UiText::PpuViewer));
        Self::wire_toolbar(&result, &ppu_button);
        toolbar.append(&ppu_button);

        // Disassembly header: title, Follow PC, addr entry, Go, Back.
        let dis_nav = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let follow_button = action_button(text(language, UiText::FollowPc));
        let go_button = action_button(text(language, UiText::Go));
        Self::wire_dis_nav(
            &result,
            &follow_button,
            &go_button,
            &dis_input,
            &back_button,
        );
        dis_nav.append(&disasm_title);
        dis_nav.append(&follow_button);
        dis_nav.append(&dis_input);
        dis_nav.append(&go_button);
        dis_nav.append(&back_button);

        let dis_scroll = gtk::ScrolledWindow::new();
        dis_scroll.set_vexpand(true);
        // Minimum one full page: the right column's lists
        // start empty (zero natural height), so without a floor the
        // panes minimum collapses and the list would show a stub.
        dis_scroll.set_size_request(-1, 260);
        // Classic scrollbars: overlay indicators hide whether the
        // disassembly, dump, and right column scroll.
        dis_scroll.set_overlay_scrolling(false);
        dis_scroll.set_child(Some(&disasm_list));
        // The drain fetches a 32-row scroll window, so the wheel
        // scrolls the viewport natively; paging stays on Prev/Next,
        // cursor keys, and Go.

        let left_col = gtk::Box::new(gtk::Orientation::Vertical, 12);
        left_col.set_hexpand(true);
        left_col.set_vexpand(true);
        let title = gtk::Label::new(Some(text(language, UiText::Debugger)));
        title.set_xalign(0.0);
        left_col.append(&title);
        left_col.append(&toolbar);
        left_col.append(&dis_nav);
        left_col.append(&dis_scroll);

        // Right state column (scrolls independently; PPU panels are long).
        let right_col = gtk::Box::new(gtk::Orientation::Vertical, 12);
        right_col.append(&status_label);
        right_col.append(&section(text(language, UiText::Registers)));
        right_col.append(&regs_row);
        let reg_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        reg_target_label.set_size_request(140, -1);
        let reg_set = action_button(text(language, UiText::Set));
        Self::wire_reg_row(&result, &reg_set, &reg_input);
        reg_row.append(&reg_target_label);
        reg_row.append(&reg_input);
        reg_row.append(&reg_set);
        right_col.append(&reg_row);
        right_col.append(&section(text(language, UiText::Watch)));
        let watch_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let watch_button = action_button(text(language, UiText::Watch));
        let watch_clear = action_button(text(language, UiText::Clear));
        watch_row.append(&watch_button);
        watch_row.append(&watch_label);
        watch_row.append(&watch_clear);
        right_col.append(&watch_row);
        right_col.append(&section(text(language, UiText::Freeze)));
        let freeze_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let freeze_button = action_button(text(language, UiText::Freeze));
        let freeze_clear = action_button(text(language, UiText::Clear));
        Self::wire_watch_freeze(
            &result,
            &watch_button,
            &watch_clear,
            &freeze_button,
            &freeze_clear,
        );
        freeze_row.append(&freeze_input);
        freeze_row.append(&freeze_button);
        freeze_row.append(&freeze_label);
        freeze_row.append(&freeze_clear);
        right_col.append(&freeze_row);
        right_col.append(&section(text(language, UiText::Bookmarks)));
        let bm_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        bookmark_input.set_hexpand(true);
        let bm_add = action_button(text(language, UiText::Add));
        Self::wire_bookmarks(&result, &bm_add, &bookmark_input);
        bm_row.append(&bookmark_input);
        bm_row.append(&bm_add);
        right_col.append(&bm_row);
        right_col.append(&bookmark_list);

        let panes = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        panes.set_vexpand(true);
        panes.append(&left_col);
        let right_scroll = gtk::ScrolledWindow::new();
        right_scroll.set_size_request(340, -1);
        right_scroll.set_overlay_scrolling(false);
        // Stretch to the panes height and scroll inside: tall state
        // (GBA: 17 registers) must not push Bookmarks out of view.
        right_scroll.set_vexpand(true);
        // Take the content's natural width: narrow rows (freeze,
        // bookmarks) must not clip behind the viewport.
        right_scroll.set_propagate_natural_width(true);
        right_scroll.set_child(Some(&right_col));
        panes.append(&right_scroll);

        // Memory section: full width below the panes. It stretches
        // with the window together with the panes, so resizing
        // grows the viewports instead of stranding dead space.
        let memory = gtk::Box::new(gtk::Orientation::Vertical, 8);
        memory.set_vexpand(true);
        memory.append(&section(text(language, UiText::Memory)));
        let space_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let space_title = gtk::Label::new(Some(text(language, UiText::Space)));
        space_title.set_size_request(150, -1);
        space_title.set_xalign(0.0);
        let space_prev = action_button(text(language, UiText::Prev));
        let space_next = action_button(text(language, UiText::Next));
        Self::wire_space_prev_next(&result, &space_prev, &space_next);
        space_row.append(&space_title);
        space_row.append(&space_prev);
        space_row.append(&space_label);
        space_row.append(&space_next);
        memory.append(&space_row);
        let mem_nav = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let page_title = gtk::Label::new(Some(text(language, UiText::Page)));
        page_title.set_size_request(150, -1);
        page_title.set_xalign(0.0);
        let mem_prev = action_button(text(language, UiText::Prev));
        let mem_next = action_button(text(language, UiText::Next));
        let mem_go = action_button(text(language, UiText::Go));
        Self::wire_mem_nav(&result, &mem_prev, &mem_next, &mem_go, &mem_input);
        mem_nav.append(&page_title);
        mem_nav.append(&mem_prev);
        mem_nav.append(&mem_next);
        mem_nav.append(&mem_input);
        mem_nav.append(&mem_go);
        memory.append(&mem_nav);
        let dump_scroll = gtk::ScrolledWindow::new();
        // Fixed share: the memory section stays on screen with the
        // panes above; the list viewport fits a full page.
        dump_scroll.set_size_request(-1, 300);
        dump_scroll.set_vexpand(true);
        dump_scroll.set_overlay_scrolling(false);
        dump_scroll.set_child(Some(&dump_list));
        // The drain fetches a 32-row scroll window, so the wheel
        // scrolls the viewport natively; paging stays on Prev/Next,
        // cursor keys, and Go.
        memory.append(&dump_scroll);

        // Edit row + confirm row + Refresh.
        let edit_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        select_label.set_size_request(240, -1);
        let edit_watch = action_button(text(language, UiText::Watch));
        let edit_freeze = action_button(text(language, UiText::Freeze));
        let confirm_button = action_button(text(language, UiText::Confirm));
        confirm_button.add_css_class("suggested-action");
        let cancel_button = action_button(text(language, UiText::Cancel));
        cancel_button.add_css_class("flat");
        let refresh_button = action_button(text(language, UiText::Refresh));
        Self::wire_edit_row(
            &result,
            &write_button,
            &edit_watch,
            &edit_freeze,
            &confirm_button,
            &cancel_button,
            &refresh_button,
        );
        edit_row.append(&select_label);
        edit_row.append(&edit_input);
        edit_row.append(&write_button);
        edit_row.append(&edit_watch);
        edit_row.append(&edit_freeze);
        memory.append(&edit_row);
        confirm_row.append(&confirm_label);
        confirm_row.append(&confirm_button);
        confirm_row.append(&cancel_button);
        memory.append(&confirm_row);
        memory.append(&refresh_button);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(16);
        content.set_margin_end(16);
        content.append(&panes);
        content.append(&memory);
        // No outer scroll: panes stretch, the dump list has a fixed
        // share, so the edit row and Refresh stay visible at 800px.
        window.set_child(Some(&content));

        // Window-scoped keys: F6/F7/F9 execution, Up/Down dump paging.
        // Left/Right stay with the widgets (caret movement).
        Self::wire_keys(&result, &window);

        window.connect_close_request(|window| {
            // Close keeps the pause; resume stays explicit.
            window.set_visible(false);
            glib::Propagation::Stop
        });

        Self::act(&result, &[DebugAction::Pause, DebugAction::Refresh]);
        window.present();
        result
    }

    /// One wiring helper per section: each connects a single
    /// pane's callbacks, so `open` stays construction plus calls.
    fn wire_toolbar(win: &DebuggerWindow, ppu_button: &gtk::Button) {
        let clicked = win.clone();
        ppu_button.connect_clicked(move |_| Self::open_ppu(&clicked));
    }

    fn wire_dis_nav(
        win: &DebuggerWindow,
        follow_button: &gtk::Button,
        go_button: &gtk::Button,
        dis_input: &gtk::Entry,
        back_button: &gtk::Button,
    ) {
        let clicked = win.clone();
        follow_button.connect_clicked(move |_| {
            clicked.borrow_mut().data.nav.follow();
            Self::act(&clicked, &[DebugAction::Refresh]);
        });
        let clicked = win.clone();
        go_button.connect_clicked(move |_| Self::dis_go(&clicked));
        let clicked = win.clone();
        dis_input.connect_activate(move |_| Self::dis_go(&clicked));
        let clicked = win.clone();
        back_button.connect_clicked(move |_| Self::dis_back(&clicked));
    }

    fn wire_watch_freeze(
        win: &DebuggerWindow,
        watch_button: &gtk::Button,
        watch_clear: &gtk::Button,
        freeze_button: &gtk::Button,
        freeze_clear: &gtk::Button,
    ) {
        let clicked = win.clone();
        watch_button.connect_clicked(move |_| Self::watch_set(&clicked));
        let clicked = win.clone();
        watch_clear.connect_clicked(move |_| Self::watch_clear(&clicked));
        let clicked = win.clone();
        freeze_button.connect_clicked(move |_| Self::freeze_set(&clicked));
        let clicked = win.clone();
        freeze_clear.connect_clicked(move |_| Self::freeze_clear(&clicked));
    }

    fn wire_bookmarks(win: &DebuggerWindow, bm_add: &gtk::Button, bookmark_input: &gtk::Entry) {
        let clicked = win.clone();
        bm_add.connect_clicked(move |_| Self::bookmark_add(&clicked));
        let clicked = win.clone();
        bookmark_input.connect_activate(move |_| Self::bookmark_add(&clicked));
    }

    fn wire_reg_row(win: &DebuggerWindow, reg_set: &gtk::Button, reg_input: &gtk::Entry) {
        let clicked = win.clone();
        reg_set.connect_clicked(move |_| Self::reg_set(&clicked));
        let clicked = win.clone();
        reg_input.connect_activate(move |_| Self::reg_set(&clicked));
    }

    fn wire_space_prev_next(
        win: &DebuggerWindow,
        space_prev: &gtk::Button,
        space_next: &gtk::Button,
    ) {
        let clicked = win.clone();
        space_prev.connect_clicked(move |_| Self::cycle_space(&clicked, -1));
        let clicked = win.clone();
        space_next.connect_clicked(move |_| Self::cycle_space(&clicked, 1));
    }

    fn wire_mem_nav(
        win: &DebuggerWindow,
        mem_prev: &gtk::Button,
        mem_next: &gtk::Button,
        mem_go: &gtk::Button,
        mem_input: &gtk::Entry,
    ) {
        let clicked = win.clone();
        mem_prev.connect_clicked(move |_| Self::page_mem(&clicked, -1));
        let clicked = win.clone();
        mem_next.connect_clicked(move |_| Self::page_mem(&clicked, 1));
        let clicked = win.clone();
        mem_go.connect_clicked(move |_| Self::mem_go(&clicked));
        let clicked = win.clone();
        mem_input.connect_activate(move |_| Self::mem_go(&clicked));
    }

    fn wire_edit_row(
        win: &DebuggerWindow,
        write_button: &gtk::Button,
        edit_watch: &gtk::Button,
        edit_freeze: &gtk::Button,
        confirm_button: &gtk::Button,
        cancel_button: &gtk::Button,
        refresh_button: &gtk::Button,
    ) {
        let clicked = win.clone();
        write_button.connect_clicked(move |_| Self::edit_write(&clicked));
        let clicked = win.clone();
        edit_watch.connect_clicked(move |_| Self::watch_set(&clicked));
        let clicked = win.clone();
        edit_freeze.connect_clicked(move |_| Self::edit_freeze_set(&clicked));
        let clicked = win.clone();
        confirm_button.connect_clicked(move |_| Self::confirm_write(&clicked));
        let clicked = win.clone();
        cancel_button.connect_clicked(move |_| Self::cancel_write(&clicked));
        let clicked = win.clone();
        refresh_button.connect_clicked(move |_| Self::act(&clicked, &[DebugAction::Refresh]));
    }

    fn wire_keys(win: &DebuggerWindow, window: &gtk::ApplicationWindow) {
        let keys = gtk::EventControllerKey::new();
        let clicked = win.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            match key {
                gdk::Key::F6 => Self::act(&clicked, &[DebugAction::StepFrame]),
                gdk::Key::F7 => Self::act(&clicked, &[DebugAction::StepInstr]),
                gdk::Key::F9 => Self::act(&clicked, &[DebugAction::TogglePause]),
                gdk::Key::Up => Self::page_mem(&clicked, -1),
                gdk::Key::Down => Self::page_mem(&clicked, 1),
                _ => (),
            }
            glib::Propagation::Proceed
        });
        window.add_controller(keys);
    }

    /// Present an existing window (idempotent reopen path).
    pub(crate) fn present(&self) {
        self.window.present();
    }

    /// Refresh an existing window (idempotent reopen path).
    pub(crate) fn refresh(win: &DebuggerWindow) {
        Self::act(win, &[DebugAction::Refresh]);
    }

    /// Run actions through the shared shell drain, then rebuild
    /// every widget from the new display. Images are fetched only
    /// while the PPU viewer is open. Inputs are copied out before
    /// the blocking drain so the window borrow never spans the
    /// thread round-trips.
    fn act(win: &DebuggerWindow, actions: &[DebugAction]) {
        let (
            state,
            space,
            mem_addr,
            dis_addr,
            watch,
            freeze,
            reg_target,
            prev_dump_rows,
            need_images,
            language,
        ) = {
            let this = win.borrow();
            (
                this.state.clone(),
                this.data.selected_space(),
                this.data.nav.mem_addr(),
                this.data.nav.dis_addr(),
                this.data.watch,
                this.data.freeze,
                this.data.reg_target,
                this.data.display.dump_rows.clone(),
                this.ppu_window.is_some(),
                this.data.language,
            )
        };
        let input = DebugDrainInput {
            space,
            mem_addr,
            dis_addr,
            watch,
            freeze,
            reg_target,
            prev_dump_rows: &prev_dump_rows,
            need_images,
            language,
        };
        let output = state.borrow().session.debug_drain(actions, &input);
        {
            let mut this = win.borrow_mut();
            this.data.display = output.display;
            if let Some(selected) = output.reselected {
                this.data.selected = Some(selected);
            }
        }
        Self::rebuild(win);
    }

    fn set_status(win: &DebuggerWindow, status: String) {
        win.borrow_mut().data.display.status = status;
        Self::rebuild(win);
    }

    fn open_ppu(win: &DebuggerWindow) {
        // Idempotent: a hidden viewer re-presents with focus instead
        // of duplicating (close hides, never destroys).
        if let Some(ppu) = win.borrow().ppu_window.clone() {
            ppu.borrow().present();
            return;
        }
        let app = win
            .borrow()
            .window
            .application()
            .expect("debugger has an application");
        let main = win.borrow().window.clone();
        let state = win.borrow().state.clone();
        let images = win.borrow().data.display.images.clone();
        let images_gen = win.borrow().data.display.images_gen;
        let panels = win.borrow().data.display.panels.clone();
        let language = win.borrow().data.language;
        let ppu = super::ppu_window::PpuWindowCore::open(
            &app, &main, state, images, images_gen, panels, language,
        );
        win.borrow_mut().ppu_window = Some(ppu);
        // The PPU opens with whatever the last drain fetched (empty
        // before any PPU was open), so refresh once with the viewer
        // present: images and panels populate immediately.
        Self::act(win, &[DebugAction::Refresh]);
    }

    fn mem_go(win: &DebuggerWindow) {
        let language = win.borrow().data.language;
        let input = win.borrow().mem_input.text().to_string();
        match vm::parse_hex_addr(&input) {
            Some(addr) => {
                win.borrow_mut().data.nav.go_mem(addr);
                Self::act(win, &[DebugAction::MemNav]);
            }
            None => Self::set_status(win, ui_msg::parse_failed(language, &input)),
        }
    }

    fn page_mem(win: &DebuggerWindow, delta: i32) {
        win.borrow_mut()
            .data
            .nav
            .page_mem(delta * DEBUG_DUMP_PAGE_ROWS as i32 * DUMP_ROW_BYTES as i32);
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
        let language = win.borrow().data.language;
        let input = win.borrow().dis_input.text().to_string();
        match vm::parse_hex_addr(&input) {
            Some(addr) => {
                let anchor = win.borrow().data.anchor();
                win.borrow_mut().data.nav.navigate(anchor, addr);
                Self::act(win, &[DebugAction::Refresh]);
            }
            None => Self::set_status(win, ui_msg::parse_failed(language, &input)),
        }
    }

    fn dis_back(win: &DebuggerWindow) {
        if win.borrow_mut().data.nav.go_back().is_some() {
            Self::act(win, &[DebugAction::Refresh]);
        } else {
            let language = win.borrow().data.language;
            Self::set_status(win, text(language, UiText::BackEmpty).to_string());
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
        let input = win.borrow().bookmark_input.text().trim().to_string();
        if win.borrow().data.bookmarks.len() >= BOOKMARK_CAP {
            let language = win.borrow().data.language;
            Self::set_status(win, text(language, UiText::BookmarkFull).to_string());
            return;
        }
        // Empty labels default to BM{n} like the tao viewer: Add
        // always does something visible.
        let n = win.borrow().data.bookmarks.len() + 1;
        let label = if input.is_empty() {
            format!("BM{n}")
        } else {
            input
        };
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

    fn reg_selected(win: &DebuggerWindow, name: &'static str) {
        win.borrow_mut().data.reg_target = Some(name);
        Self::act(win, &[DebugAction::Refresh]);
    }

    fn reg_set(win: &DebuggerWindow) {
        let (target, input) = {
            let this = win.borrow();
            (
                this.data.reg_target,
                this.reg_input.text().trim().to_string(),
            )
        };
        match (target, vm::parse_hex_addr(&input)) {
            (Some(_), Some(value)) => {
                Self::act(win, &[DebugAction::RegisterWrite(u64::from(value))]);
            }
            (None, _) => {
                let language = win.borrow().data.language;
                Self::set_status(
                    win,
                    text(language, UiText::RegisterNeedsSelection).to_string(),
                )
            }
            _ => {
                let language = win.borrow().data.language;
                Self::set_status(win, ui_msg::parse_failed(language, &input))
            }
        }
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
            None => {
                let language = win.borrow().data.language;
                Self::set_status(win, text(language, UiText::WatchNeedsSelection).to_string())
            }
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
        let input = win.borrow().freeze_input.text().trim().to_string();
        Self::freeze_stage(win, &input);
    }

    /// Edit-row Freeze: reads the edit row's own input, so each of
    /// the row's three buttons reads what sits next to it (Write and
    /// Watch already do; Freeze used to reach up to the top input).
    fn edit_freeze_set(win: &DebuggerWindow) {
        let (language, selected, input) = {
            let this = win.borrow();
            (
                this.data.language,
                this.data.selected,
                this.edit_input.text().trim().to_string(),
            )
        };
        match resolve_freeze_input(language, selected, &input) {
            Ok((addr, value)) => {
                win.borrow_mut().data.freeze = Some((addr, value));
                Self::set_status(win, ui_msg::frozen(language, addr, value));
                Self::act(win, &[DebugAction::Refresh]);
            }
            Err(status) => Self::set_status(win, status),
        }
    }

    fn freeze_stage(win: &DebuggerWindow, input: &str) {
        let language = win.borrow().data.language;
        let selected = win.borrow().data.selected;
        match resolve_freeze_input(language, selected, input) {
            Ok((addr, value)) => {
                win.borrow_mut().data.freeze = Some((addr, value));
                Self::set_status(win, ui_msg::frozen(language, addr, value));
                Self::act(win, &[DebugAction::Refresh]);
            }
            Err(status) => Self::set_status(win, status),
        }
    }

    fn freeze_clear(win: &DebuggerWindow) {
        win.borrow_mut().data.freeze = None;
        let language = win.borrow().data.language;
        Self::set_status(win, text(language, UiText::FreezeCleared).to_string());
    }

    fn edit_write(win: &DebuggerWindow) {
        let (language, selected, input) = {
            let this = win.borrow();
            (
                this.data.language,
                this.data.selected,
                this.edit_input.text().trim().to_string(),
            )
        };
        match resolve_write_input(language, selected, &input) {
            Ok((addr, value)) => {
                // Re-select first: the batch prepares fresh state
                // before staging, so Write also works after a cancel.
                // GTK runs batches synchronously, so the authoritative
                // confirm row is already in the display after act.
                Self::act(
                    win,
                    &[DebugAction::SelectRow(addr), DebugAction::EditStage(value)],
                );
            }
            Err(status) => Self::set_status(win, status),
        }
    }

    fn confirm_write(win: &DebuggerWindow) {
        Self::act(win, &[DebugAction::WriteConfirm]);
        // Commit consumes the transaction; the input clears. No
        // rebuild: act already rendered the committed state.
        win.borrow().edit_input.set_text("");
    }

    fn cancel_write(win: &DebuggerWindow) {
        Self::act(win, &[DebugAction::EditCancel]);
    }

    /// Rebuild every widget from `data`. Single refresh point; widget
    /// callbacks never touch the session directly.
    fn rebuild(win: &DebuggerWindow) {
        let this = win.borrow();
        Self::rebuild_header(&this);
        Self::rebuild_disasm(win, &this);
        Self::rebuild_dump(win, &this);
        Self::rebuild_edit_row(&this);
        Self::rebuild_state(win, &this);
        Self::rebuild_bookmarks(win, &this);

        // The PPU viewer follows the same refresh (panels included).
        if let Some(ppu) = this.ppu_window.as_ref() {
            let display = &this.data.display;
            ppu.borrow_mut()
                .sync(&display.images, display.images_gen, &display.panels);
        }
    }

    /// One rebuild helper per section; all read the borrowed core.
    fn rebuild_header(this: &DebuggerWindowCore) {
        let display = &this.data.display;
        let nav = &this.data.nav;
        let language = this.data.language;
        this.pause_button.set_label(if display.paused {
            text(language, UiText::Resume)
        } else {
            text(language, UiText::Pause)
        });
        this.status_label.set_label(&display.status);
        // The title names the anchor: clicks pin it visibly instead
        // of silently moving the window.
        let dis_title = if nav.follow_pc() {
            format!(
                "{} {}",
                text(language, UiText::Disassembly),
                text(language, UiText::FollowPcTag)
            )
        } else {
            format!(
                "{} @{:08X} {}",
                text(language, UiText::Disassembly),
                this.data.anchor(),
                text(language, UiText::FixedTag)
            )
        };
        this.disasm_title.set_label(&dis_title);
        this.back_button.set_label(&format!(
            "{} ({})",
            text(language, UiText::Back),
            nav.back_len()
        ));
        this.space_label.set_label(&this.data.selected_name());
        if this.mem_input.text() != nav.mem_input() {
            this.mem_input.set_text(nav.mem_input());
        }
        if this.dis_input.text() != nav.dis_input() {
            this.dis_input.set_text(nav.dis_input());
        }
    }

    fn rebuild_disasm(win: &DebuggerWindow, this: &DebuggerWindowCore) {
        let display = &this.data.display;
        let language = this.data.language;
        // Pinned anchor rows carry `@` like the tao viewer so a line
        // click reads back visibly even off-PC. Nothing is pinned in
        // follow-PC mode.
        let pinned = if this.data.nav.follow_pc() {
            None
        } else {
            Some(this.data.anchor())
        };
        clear_list(&this.disasm_list);
        if display.disasm_lines.is_empty() {
            this.disasm_list
                .append(&static_row(text(language, UiText::NoDisassembly)));
            this.disasm_list
                .append(&static_row(text(language, UiText::DisassemblyHint)));
        }
        for line in &display.disasm_lines {
            let row = gtk::ListBoxRow::new();
            row.set_selectable(false);
            let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            // The PC row carries a `>` marker like the dump
            // selection: the current position reads at a glance.
            let is_pinned = pinned == Some(line.addr);
            let marker = if line.is_pc {
                "> "
            } else if is_pinned {
                "@ "
            } else {
                "  "
            };
            let button =
                gtk::Button::with_label(&format!("{marker}{}", vm::format_disasm_line(line)));
            button.add_css_class("monospace");
            left_align(&button);
            button.set_hexpand(true);
            button.set_halign(gtk::Align::Fill);
            if line.is_pc || is_pinned {
                button.add_css_class("suggested-action");
            } else {
                // Text-look rows like the tao viewer: the frame
                // implies an action, but these only select.
                button.add_css_class("flat");
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
    }

    fn rebuild_dump(win: &DebuggerWindow, this: &DebuggerWindowCore) {
        let display = &this.data.display;
        clear_list(&this.dump_list);
        // Byte-precise selection like the tao viewer: every byte is
        // its own button, so the write target reads exactly (the
        // address head keeps the old row-base pick).
        for hex_row in &display.dump_rows {
            let selected_addr = this.data.selected.map(|(a, _)| a);
            // Containing-row match: a mid-row pick still lights its row.
            let row_hit = selected_addr
                .is_some_and(|a| a >= hex_row.addr && a < hex_row.addr + u32::from(hex_row.valid));
            let changed = display.diff.contains(&hex_row.addr);
            let marker = match (row_hit, changed) {
                (true, true) => ">*",
                (true, false) => "> ",
                (false, true) => " *",
                (false, false) => "  ",
            };
            let row = gtk::ListBoxRow::new();
            row.set_selectable(false);
            let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 2);
            let head = gtk::Button::with_label(&format!("{marker}{:08X}:", hex_row.addr));
            head.add_css_class("monospace");
            left_align(&head);
            if row_hit {
                head.add_css_class("suggested-action");
            } else {
                head.add_css_class("flat");
            }
            let base = hex_row.addr;
            let head_clicked = Rc::clone(win);
            head.connect_clicked(move |_| Self::row_selected(&head_clicked, base));
            hbox.append(&head);
            for i in 0..hex_row.valid {
                let addr = hex_row.addr + u32::from(i);
                let byte = hex_row.bytes[usize::from(i)];
                let button = gtk::Button::with_label(&format!("{byte:02X}"));
                button.add_css_class("monospace");
                if selected_addr == Some(addr) {
                    button.add_css_class("suggested-action");
                } else {
                    button.add_css_class("flat");
                }
                let clicked = Rc::clone(win);
                button.connect_clicked(move |_| Self::row_selected(&clicked, addr));
                hbox.append(&button);
            }
            row.set_child(Some(&hbox));
            this.dump_list.append(&row);
        }
    }

    fn rebuild_edit_row(this: &DebuggerWindowCore) {
        let display = &this.data.display;
        let language = this.data.language;
        let select_text = match this.data.selected {
            Some((addr, old)) => ui_msg::selected_row(language, addr, old),
            None => text(language, UiText::SelectDumpRow).to_string(),
        };
        this.select_label.set_label(&select_text);
        // The target rides on the button: writes always land on the
        // selected row, never on the page anchor.
        this.write_button.set_label(&match this.data.selected {
            Some((addr, _)) => format!("{} {addr:08X}", text(language, UiText::Write)),
            None => text(language, UiText::Write).to_string(),
        });
        match &display.pending_text {
            Some(pending) => {
                this.confirm_label.set_label(pending);
                this.confirm_row.set_visible(true);
            }
            None => this.confirm_row.set_visible(false),
        }
    }

    fn rebuild_state(win: &DebuggerWindow, this: &DebuggerWindowCore) {
        let display = &this.data.display;
        let language = this.data.language;
        // Registers flow into two clickable columns like the tao
        // viewer: the kernel order is kept, the view only halves
        // the list. Row click picks the write target; lines are
        // formatted here from the structured snapshot.
        let regs = &display.regs;
        let mid = regs.len().div_ceil(2).min(regs.len());
        Self::fill_reg_list(win, &this.regs_left, &regs[..mid], this.data.reg_target);
        Self::fill_reg_list(win, &this.regs_right, &regs[mid..], this.data.reg_target);
        this.reg_target_label
            .set_label(&match this.data.reg_target {
                Some(name) => format!("{} {name}", text(language, UiText::Set)),
                None => text(language, UiText::PickRegister).to_string(),
            });
        let watch_text = match (this.data.watch, display.watch_value) {
            (Some(addr), value) => ui_msg::watch_value(language, addr, value),
            (None, _) => text(language, UiText::WatchNone).to_string(),
        };
        this.watch_label.set_label(&watch_text);
        let freeze_text = match this.data.freeze {
            Some((addr, value)) => ui_msg::frozen(language, addr, value),
            None => text(language, UiText::FreezeNone).to_string(),
        };
        this.freeze_label.set_label(&freeze_text);
    }

    fn fill_reg_list(
        win: &DebuggerWindow,
        list: &gtk::ListBox,
        regs: &[(&'static str, u64)],
        target: Option<&'static str>,
    ) {
        clear_list(list);
        for &(name, value) in regs.iter() {
            let row = gtk::ListBoxRow::new();
            row.set_selectable(false);
            let button = gtk::Button::with_label(&vm::format_register(name, value));
            button.add_css_class("monospace");
            left_align(&button);
            button.set_hexpand(true);
            button.set_halign(gtk::Align::Fill);
            if Some(name) == target {
                button.add_css_class("suggested-action");
            } else {
                button.add_css_class("flat");
            }
            let clicked = Rc::clone(win);
            button.connect_clicked(move |_| Self::reg_selected(&clicked, name));
            row.set_child(Some(&button));
            list.append(&row);
        }
    }

    fn rebuild_bookmarks(win: &DebuggerWindow, this: &DebuggerWindowCore) {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_input_parses_hex_byte_with_selection() {
        use nerust_gui_settings::language::AppLanguage;
        assert_eq!(
            resolve_write_input(AppLanguage::English, Some((0x30, Some(0x00))), "42"),
            Ok((0x30, 0x42))
        );
        // Whitespace is harmless; values stay a single byte.
        assert_eq!(
            resolve_write_input(AppLanguage::English, Some((0x30, None)), "  ff "),
            Ok((0x30, 0xFF))
        );
    }

    #[test]
    fn write_input_rejects_missing_selection_and_bad_text() {
        use nerust_gui_settings::language::AppLanguage;
        assert_eq!(
            resolve_write_input(AppLanguage::English, None, "42"),
            Err("write needs a selection".to_string())
        );
        assert_eq!(
            resolve_write_input(AppLanguage::English, Some((0x30, Some(0x00))), "zz"),
            Err("parse failed: zz".to_string())
        );
        assert_eq!(
            resolve_write_input(AppLanguage::English, Some((0x30, Some(0x00))), "100"),
            Err("parse failed: 100".to_string())
        );
        assert_eq!(
            resolve_write_input(AppLanguage::Japanese, None, "42"),
            Err("書込には行選択が必要".to_string())
        );
    }

    #[test]
    fn freeze_input_needs_row_and_byte_together() {
        use nerust_gui_settings::language::AppLanguage;
        assert_eq!(
            resolve_freeze_input(AppLanguage::English, Some((0x10, Some(0x00))), "FF"),
            Ok((0x10, 0xFF))
        );
        assert_eq!(
            resolve_freeze_input(AppLanguage::English, None, "FF"),
            Err("freeze needs a selected row and hex byte".to_string())
        );
        assert_eq!(
            resolve_freeze_input(AppLanguage::English, Some((0x10, Some(0x00))), "1FF"),
            Err("freeze needs a selected row and hex byte".to_string())
        );
    }
}
