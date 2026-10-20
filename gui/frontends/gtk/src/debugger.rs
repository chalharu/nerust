//! GTK debugger window: same semantics, shell APIs, and view-model formatters as the tao twin.
//! Action batches run as async shell-drain flights polled on the GTK tick.

use std::{cell::RefCell, rc::Rc};

use gtk::{gdk, glib, prelude::*};
use nerust_core_traits::debugger::{DEBUG_DUMP_PAGE_ROWS, DUMP_ROW_BYTES, SpaceId};
use nerust_gui_shell::debug::{
    DebugAction, DebugDrainInput, DrainFlight, DrainPoll, LiveImagesFlight, LiveImagesPoll,
};
use nerust_gui_viewmodel::debugger::{self as vm, DisplayCache, NavState, StatusMessage};
use nerust_settings_core::i18n::{self as ui_msg, UiText, text};
use nerust_settings_core::model::language::AppLanguage;

use super::State;

/// Bookmark cap (window-local, no file yet).
const BOOKMARK_CAP: usize = 8;

/// Rebuild granularity for [`DebuggerWindowCore::rebuild_partial`].
/// Handlers pick the smallest scope covering their data change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum RebuildScope {
    /// Everything (execution, memory writes, full refreshes).
    #[default]
    All,
    /// Dump rows plus the rows they drive (edit row, state).
    Dump,
    /// Disassembly list (navigation moved the anchor).
    Disasm,
    /// Registers, watch, and freeze labels.
    State,
    /// Staged-write confirm row.
    Edit,
    /// Bookmark list.
    Bookmarks,
    /// Status and title labels only.
    Header,
}

/// Pure write-input resolution: selection plus a hex byte. Widget callbacks
/// stay thin; the rules are unit-tested below.
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
    /// In-flight async drain with its rebuild scope: `act_scoped` starts it,
    /// the tick poller finishes it. A newer batch supersedes the stored one.
    flight: Option<(DrainFlight, RebuildScope)>,
    /// In-flight async live-images fetch, same pattern for the 250ms
    /// PPU follow tick. At most one; a newer tick supersedes.
    live_flight: Option<LiveImagesFlight>,
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
    // Floor for hex inputs: sibling labels with hexpand would otherwise
    // squeeze the entry below its placeholder in either language.
    entry.set_width_chars(10);
    entry
}

fn action_button(label: &str) -> gtk::Button {
    gtk::Button::with_label(label)
}

impl DebuggerWindowCore {
    /// Open the debugger: build widgets, then pause and refresh through the drain.
    /// Closing keeps the pause; resume stays explicit.
    pub(crate) fn open(
        app: &gtk::Application,
        // Independent toplevel like the tao twin: no transient-for parenting.
        // True dialogs (preferences, file/error) stay transient.
        _main_window: &gtk::ApplicationWindow,
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
        // Registers flow into two clickable columns, halving tall-list height.
        // Pure text split, no system branches. Row click picks the write target.
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
            flight: None,
            live_flight: None,
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
        // Minimum one full page: the right column's lists start empty, so
        // without a floor the panes minimum collapses to a stub.
        dis_scroll.set_size_request(-1, 260);
        // Classic scrollbars: overlay indicators hide whether the
        // disassembly, dump, and right column scroll.
        dis_scroll.set_overlay_scrolling(false);
        dis_scroll.set_child(Some(&disasm_list));
        // The drain fetches a 32-row scroll window, so the wheel scrolls natively;
        // paging stays on Prev/Next, cursor keys, and Go.

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

        // Memory section: full width below the panes. It stretches with the
        // window, so resizing grows viewports instead of stranding dead space.
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
        // The drain fetches a 32-row scroll window, so the wheel scrolls natively;
        // paging stays on Prev/Next, cursor keys, and Go.
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
        // Live PPU follow while the core runs. Images-only, never a full drain:
        // the paused-gated inspect refuses while running and would blank the lists.
        let weak = Rc::downgrade(&result);
        glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
            let Some(win) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            // Backstop for drain flights: poll here too so a missed watcher
            // can never strand a pending display.
            Self::poll_flight(&win);
            let live = win.borrow().window.is_visible() && win.borrow().ppu_window.is_some() && {
                let state = win.borrow().state.clone();
                let session = &state.borrow().session;
                session.tick_metrics().loaded && !session.tick_metrics().paused
            };
            if live {
                Self::live_tick(&win);
            }
            glib::ControlFlow::Continue
        });
        #[cfg(target_os = "macos")]
        crate::gdk_raw::guard_window_tabs(&window);
        window.present();
        result
    }

    /// Live-tick body: settle the in-flight fetch, then start a new one when following.
    /// Patches images and pause chrome only; never rebuilds widgets (scroll survives).
    fn live_tick(win: &DebuggerWindow) {
        if Self::poll_live_flight(win) {
            // A fetch is still crossing: wait for its poller instead
            // of stacking another one behind it.
            return;
        }
        let live = win.borrow().window.is_visible() && win.borrow().ppu_window.is_some() && {
            let state = win.borrow().state.clone();
            let session = &state.borrow().session;
            session.tick_metrics().loaded && !session.tick_metrics().paused
        };
        if !live {
            return;
        }
        let state = win.borrow().state.clone();
        let mut flight = state.borrow().session.debug_live_images_begin(true);
        match flight.poll() {
            LiveImagesPoll::Done(live) => Self::apply_live_images(win, live),
            LiveImagesPoll::Pending => {
                win.borrow_mut().live_flight = Some(flight);
                Self::watch_live_flight(win);
            }
        }
    }

    /// Short poller while a live fetch is pending, mirroring `watch_flight`.
    /// Applies within ~30ms of the reply instead of waiting for the 250ms tick.
    fn watch_live_flight(win: &DebuggerWindow) {
        let weak = Rc::downgrade(win);
        glib::timeout_add_local(std::time::Duration::from_millis(30), move || {
            let Some(win) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if Self::poll_live_flight(&win) {
                glib::ControlFlow::Continue
            } else {
                glib::ControlFlow::Break
            }
        });
    }

    /// Poll the stored live fetch once. Returns whether one is still pending.
    /// Orphaned flights settle harmlessly: the generation guard skips stale data.
    fn poll_live_flight(win: &DebuggerWindow) -> bool {
        let mut flight = match win.borrow_mut().live_flight.take() {
            Some(flight) => flight,
            None => return false,
        };
        match flight.poll() {
            LiveImagesPoll::Done(live) => {
                Self::apply_live_images(win, live);
                false
            }
            LiveImagesPoll::Pending => {
                win.borrow_mut().live_flight = Some(flight);
                true
            }
        }
    }

    /// Patch finished live images into the display. Images apply when newer
    /// regardless of pause; running chrome only applies while still running.
    fn apply_live_images(win: &DebuggerWindow, live: nerust_gui_shell::session::LiveImages) {
        let Some(batch) = live else {
            return;
        };
        let images = batch
            .images
            .into_iter()
            .map(|image| (image.label, image.width, image.height, image.rgba))
            .collect();
        let generation = batch.generation;
        let (language, running) = {
            let this = win.borrow();
            let running = {
                let state = this.state.clone();
                let session = &state.borrow().session;
                session.tick_metrics().loaded && !session.tick_metrics().paused
            };
            (this.data.language, running)
        };
        let mut this = win.borrow_mut();
        if generation == this.data.display.images_gen {
            return;
        }
        this.data.display.images = images;
        this.data.display.images_gen = generation;
        if running {
            this.data.display.paused = false;
            this.data.display.status = StatusMessage::Running;
            // Light chrome only: a full rebuild would yank the dump scroll.
            this.pause_button.set_label(text(language, UiText::Pause));
            this.status_label
                .set_label(&this.data.display.status.render(language));
        }
        if let Some(ppu) = this.ppu_window.as_ref() {
            let display = &this.data.display;
            ppu.borrow_mut()
                .sync(&display.images, display.images_gen, &display.panels);
        }
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
            match key_action(key) {
                Some(DebuggerKey::StepFrame) => Self::act(&clicked, &[DebugAction::StepFrame]),
                Some(DebuggerKey::StepInstr) => Self::act(&clicked, &[DebugAction::StepInstr]),
                Some(DebuggerKey::TogglePause) => Self::act(&clicked, &[DebugAction::TogglePause]),
                Some(DebuggerKey::PageMem(delta)) => Self::page_mem(&clicked, delta),
                None => (),
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

    /// Run actions through an async drain flight, then rebuild from the new display.
    /// Inputs are copied out first so the window borrow never spans the round-trip.
    fn act(win: &DebuggerWindow, actions: &[DebugAction]) {
        Self::act_scoped(win, actions, RebuildScope::All);
    }

    /// [`DebuggerWindowCore::act`] with a rebuild scope: the finished drain
    /// refreshes only the sections its data moved.
    fn act_scoped(win: &DebuggerWindow, actions: &[DebugAction], scope: RebuildScope) {
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
        };
        // Supersede: a newer batch replaces the stored flight; the
        // FIFO command order keeps the newest view correct.
        let mut flight = state.borrow().session.debug_drain_begin(actions, &input);
        match flight.poll() {
            DrainPoll::Done(output) => {
                Self::apply_scoped(win, *output, scope);
            }
            DrainPoll::Pending => {
                win.borrow_mut().flight = Some((flight, scope));
                Self::watch_flight(win);
            }
        }
    }

    /// Short poller while a flight is pending: applies the display within ~30ms.
    /// Dies with the window; extra pollers from rapid batches see no flight and stop.
    fn watch_flight(win: &DebuggerWindow) {
        let weak = Rc::downgrade(win);
        glib::timeout_add_local(std::time::Duration::from_millis(30), move || {
            let Some(win) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if Self::poll_flight(&win) {
                glib::ControlFlow::Continue
            } else {
                glib::ControlFlow::Break
            }
        });
    }

    /// Poll the stored flight once. Returns whether one is still pending: on
    /// completion the display applies, exactly like a synchronous drain.
    fn poll_flight(win: &DebuggerWindow) -> bool {
        let (mut flight, scope) = match win.borrow_mut().flight.take() {
            Some(stored) => stored,
            None => return false,
        };
        match flight.poll() {
            DrainPoll::Done(output) => {
                Self::apply_scoped(win, *output, scope);
                false
            }
            DrainPoll::Pending => {
                win.borrow_mut().flight = Some((flight, scope));
                true
            }
        }
    }

    /// Apply a finished drain: merge running sections, store the
    /// selection, and rebuild the scoped widgets from the new display.
    fn apply_scoped(
        win: &DebuggerWindow,
        output: nerust_gui_shell::debug::DebugDrainOutput,
        scope: RebuildScope,
    ) {
        {
            let mut this = win.borrow_mut();
            // Running drains come back section-degenerate (paused gate): restore
            // the last paused rows so Resume keeps inspectable state.
            let mut display = output.display;
            display.restore_paused_sections(&this.data.display);
            this.data.display = display;
            if let Some(selected) = output.reselected {
                this.data.selected = Some(selected);
            }
        }
        Self::rebuild_partial(win, scope);
    }

    /// Status-line-only refresh: parse errors and prompts never touch the lists,
    /// so only the header rebuilds.
    fn set_status(win: &DebuggerWindow, status: String) {
        // Frontend-local wording: stored verbatim, like the tao twin.
        win.borrow_mut().data.display.status = StatusMessage::Local(status);
        Self::rebuild_partial(win, RebuildScope::Header);
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
        // The PPU opens with whatever the last drain fetched, so refresh once
        // with the viewer present: images and panels populate immediately.
        Self::act(win, &[DebugAction::Refresh]);
    }

    fn mem_go(win: &DebuggerWindow) {
        let language = win.borrow().data.language;
        let input = win.borrow().mem_input.text().to_string();
        match vm::parse_hex_addr(&input) {
            Some(addr) => {
                win.borrow_mut().data.nav.go_mem(addr);
                Self::act_scoped(win, &[DebugAction::MemNav], RebuildScope::Dump);
            }
            None => Self::set_status(win, ui_msg::parse_failed(language, &input)),
        }
    }

    fn page_mem(win: &DebuggerWindow, delta: i32) {
        win.borrow_mut()
            .data
            .nav
            .page_mem(delta * DEBUG_DUMP_PAGE_ROWS as i32 * DUMP_ROW_BYTES as i32);
        Self::act_scoped(win, &[DebugAction::MemNav], RebuildScope::Dump);
    }

    fn cycle_space(win: &DebuggerWindow, delta: i32) {
        let stepped = win.borrow_mut().data.nav.cycle_space(delta);
        if let Some(idx) = stepped {
            let start = win.borrow().data.spaces[idx].2;
            let mut this = win.borrow_mut();
            this.data.nav.set_space(idx, start);
            this.data.selected = None;
            drop(this);
            Self::act_scoped(win, &[DebugAction::Refresh], RebuildScope::Dump);
        }
    }

    fn dis_go(win: &DebuggerWindow) {
        let language = win.borrow().data.language;
        let input = win.borrow().dis_input.text().to_string();
        match vm::parse_hex_addr(&input) {
            Some(addr) => {
                let anchor = win.borrow().data.anchor();
                win.borrow_mut().data.nav.navigate(anchor, addr);
                Self::act_scoped(win, &[DebugAction::Refresh], RebuildScope::Disasm);
            }
            None => Self::set_status(win, ui_msg::parse_failed(language, &input)),
        }
    }

    fn dis_back(win: &DebuggerWindow) {
        if win.borrow_mut().data.nav.go_back().is_some() {
            Self::act_scoped(win, &[DebugAction::Refresh], RebuildScope::Disasm);
        } else {
            let language = win.borrow().data.language;
            Self::set_status(win, text(language, UiText::BackEmpty).to_string());
        }
    }

    fn follow_target(win: &DebuggerWindow, addr: u32) {
        let anchor = win.borrow().data.anchor();
        win.borrow_mut().data.nav.navigate(anchor, addr);
        Self::act_scoped(win, &[DebugAction::Refresh], RebuildScope::Disasm);
    }

    fn dis_line_clicked(win: &DebuggerWindow, addr: u32) {
        let anchor = win.borrow().data.anchor();
        win.borrow_mut().data.nav.navigate(anchor, addr);
        Self::act_scoped(win, &[DebugAction::Refresh], RebuildScope::Disasm);
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
        Self::rebuild_partial(win, RebuildScope::Bookmarks);
    }

    fn bookmark_jump(win: &DebuggerWindow, addr: u32) {
        let anchor = win.borrow().data.anchor();
        win.borrow_mut().data.nav.navigate(anchor, addr);
        Self::act_scoped(win, &[DebugAction::Refresh], RebuildScope::Disasm);
    }

    fn bookmark_delete(win: &DebuggerWindow, addr: u32) {
        win.borrow_mut().data.bookmarks.retain(|(a, _)| *a != addr);
        Self::rebuild_partial(win, RebuildScope::Bookmarks);
    }

    fn row_selected(win: &DebuggerWindow, addr: u32) {
        Self::act_scoped(win, &[DebugAction::SelectRow(addr)], RebuildScope::Dump);
    }

    fn reg_selected(win: &DebuggerWindow, name: &'static str) {
        // Highlight-only: no fresh emu data needed, so rebuild without a drain
        // (a full drain would stall the main thread on timeout round-trips).
        win.borrow_mut().data.reg_target = Some(name);
        Self::rebuild_partial(win, RebuildScope::State);
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
                Self::act_scoped(
                    win,
                    &[DebugAction::RegisterWrite(u64::from(value))],
                    RebuildScope::State,
                );
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
                Self::act_scoped(win, &[DebugAction::Refresh], RebuildScope::State);
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
        Self::rebuild_partial(win, RebuildScope::State);
    }

    fn freeze_set(win: &DebuggerWindow) {
        let input = win.borrow().freeze_input.text().trim().to_string();
        Self::freeze_stage(win, &input);
    }

    /// Edit-row Freeze: reads the edit row's own input, so each of the row's
    /// three buttons reads what sits next to it.
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
                // Re-select first: the batch prepares fresh state before staging.
                // The confirm row and moved dump highlight render on completion.
                Self::act_scoped(
                    win,
                    &[DebugAction::SelectRow(addr), DebugAction::EditStage(value)],
                    RebuildScope::Dump,
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
        Self::act_scoped(win, &[DebugAction::EditCancel], RebuildScope::Edit);
    }

    /// Rebuild slice by slice. The header always refreshes; heavy lists rebuild
    /// only when their data moved, so highlight-only gestures skip ~700 creates.
    fn rebuild_partial(win: &DebuggerWindow, scope: RebuildScope) {
        let this = win.borrow();
        Self::rebuild_header(&this);
        match scope {
            RebuildScope::All => {
                Self::rebuild_disasm(win, &this);
                Self::rebuild_dump(win, &this);
                Self::rebuild_edit_row(&this);
                Self::rebuild_state(win, &this);
                Self::rebuild_bookmarks(win, &this);
            }
            RebuildScope::Dump => {
                Self::rebuild_dump(win, &this);
                Self::rebuild_edit_row(&this);
                Self::rebuild_state(win, &this);
            }
            RebuildScope::Disasm => {
                Self::rebuild_disasm(win, &this);
            }
            RebuildScope::State => {
                Self::rebuild_state(win, &this);
            }
            RebuildScope::Edit => {
                Self::rebuild_edit_row(&this);
            }
            RebuildScope::Bookmarks => {
                Self::rebuild_bookmarks(win, &this);
            }
            RebuildScope::Header => {}
        }

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
        this.status_label
            .set_label(&display.status.render(language));
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
                "{} @0x{:08X} {}",
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
        // Pinned anchor rows carry `@` like the tao viewer so a line click
        // reads back visibly even off-PC. Nothing pinned in follow-PC mode.
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
        // Byte-precise selection like the tao viewer: every byte is its own
        // button, so the write target reads exactly.
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
            let head = gtk::Button::with_label(&format!("{marker}0x{:08X}:", hex_row.addr));
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
            Some((addr, _)) => format!("{} 0x{addr:08X}", text(language, UiText::Write)),
            None => text(language, UiText::Write).to_string(),
        });
        match &display.pending {
            Some(pending) => {
                this.confirm_label.set_label(&pending.render(language));
                this.confirm_row.set_visible(true);
            }
            None => this.confirm_row.set_visible(false),
        }
    }

    fn rebuild_state(win: &DebuggerWindow, this: &DebuggerWindowCore) {
        let display = &this.data.display;
        let language = this.data.language;
        // Registers flow into two clickable columns: the kernel order is kept,
        // the view only halves the list. Lines format here from the snapshot.
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
            let jump = gtk::Button::with_label(&format!("{label} 0x{addr:08X}"));
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

/// Window-scoped key mapping: F6/F7/F9 execution, Up/Down dump paging.
/// Left/Right stay with the widgets; pure mapping shared by controller and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DebuggerKey {
    StepFrame,
    StepInstr,
    TogglePause,
    PageMem(i32),
}

fn key_action(key: gdk::Key) -> Option<DebuggerKey> {
    match key {
        gdk::Key::F6 => Some(DebuggerKey::StepFrame),
        gdk::Key::F7 => Some(DebuggerKey::StepInstr),
        gdk::Key::F9 => Some(DebuggerKey::TogglePause),
        gdk::Key::Up => Some(DebuggerKey::PageMem(-1)),
        gdk::Key::Down => Some(DebuggerKey::PageMem(1)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_input_parses_hex_byte_with_selection() {
        use nerust_settings_core::model::language::AppLanguage;
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
        use nerust_settings_core::model::language::AppLanguage;
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
    fn key_action_maps_window_shortcuts() {
        assert_eq!(key_action(gdk::Key::F6), Some(DebuggerKey::StepFrame));
        assert_eq!(key_action(gdk::Key::F7), Some(DebuggerKey::StepInstr));
        assert_eq!(key_action(gdk::Key::F9), Some(DebuggerKey::TogglePause));
        assert_eq!(key_action(gdk::Key::Up), Some(DebuggerKey::PageMem(-1)));
        assert_eq!(key_action(gdk::Key::Down), Some(DebuggerKey::PageMem(1)));
        assert_eq!(key_action(gdk::Key::Left), None);
        assert_eq!(key_action(gdk::Key::a), None);
    }

    #[test]
    fn freeze_input_needs_row_and_byte_together() {
        use nerust_settings_core::model::language::AppLanguage;
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

    #[derive(Debug)]
    struct MockGpuFactory;

    impl nerust_render_traits::renderer::GpuFactory for MockGpuFactory {
        fn create_renderer(
            &self,
            _config: &nerust_render_traits::renderer::RendererConfig,
            _display: raw_window_handle::RawDisplayHandle,
        ) -> Result<
            Box<dyn nerust_render_traits::renderer::GpuRenderer>,
            nerust_render_traits::renderer::RendererError,
        > {
            Err(nerust_render_traits::renderer::RendererError::new(
                "test",
                Box::new(std::io::Error::other("no renderer in tests")),
            ))
        }
    }

    #[derive(Debug)]
    struct MockRomLoader;

    impl nerust_gui_shell::load::RomLoader for MockRomLoader {
        fn load_rom(
            &mut self,
            _path: &std::path::Path,
            _target: &mut dyn nerust_gui_shell::load::RomLoadTarget,
        ) -> Result<(), nerust_gui_shell::load::RomLoaderError> {
            Err(nerust_gui_shell::load::RomLoaderError::Load(
                "no ROM in tests".to_string(),
            ))
        }
    }

    fn harness() -> Option<(
        gtk::Application,
        gtk::ApplicationWindow,
        Rc<RefCell<crate::State>>,
    )> {
        use std::sync::Arc;
        if !crate::test_support::ui_test_available() {
            return None;
        }
        let app = gtk::Application::builder()
            .application_id("org.nerust.debugger-test")
            .build();
        let main = gtk::ApplicationWindow::builder().application(&app).build();
        let registry = Arc::new(nerust_gui_shell::registry::SystemRegistry::new(Vec::new()));
        let audio = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
        let ctx = nerust_gui_shell::context::FrontendContext {
            gpu_factory: Rc::new(MockGpuFactory),
            registry,
            rom_loader: Box::new(MockRomLoader),
            audio_registry: audio,
        };
        let state = Rc::new(RefCell::new(crate::State::new(ctx)));
        Some((app, main, state))
    }

    fn synthetic_display() -> DisplayCache {
        use nerust_core_traits::debugger::{
            CellValue, ColumnKind, DebugPanel, DisasmLine, HexRow, PanelColumn, PanelRow,
        };
        static TEST_COLUMNS: [PanelColumn; 1] = [PanelColumn {
            id: "h",
            label_id: "H",
            kind: ColumnKind::Hex { digits: 2 },
        }];
        DisplayCache {
            regs: vec![("cpsr", 0x3F), ("r0", 1), ("r1", 2)],
            dump_rows: vec![
                HexRow {
                    addr: 0x100,
                    valid: 16,
                    bytes: [0xAB; 16],
                },
                HexRow {
                    addr: 0x110,
                    valid: 3,
                    bytes: [1, 2, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                },
            ],
            diff: vec![0x100],
            disasm_lines: vec![
                DisasmLine {
                    addr: 0x300,
                    bytes: [0xF0, 0xBC, 0, 0],
                    len: 2,
                    text: "POP {r4, r5, r6, r7}".to_string(),
                    is_pc: true,
                    target: None,
                },
                DisasmLine {
                    addr: 0x302,
                    bytes: [0x02, 0xBC, 0, 0],
                    len: 2,
                    text: "POP {r1}".to_string(),
                    is_pc: false,
                    target: Some(0x400),
                },
            ],
            panels: vec![DebugPanel {
                id: "p",
                label_id: "P",
                columns: &TEST_COLUMNS,
                rows: vec![PanelRow {
                    key: "row",
                    cells: vec![CellValue::U64(0xAB)],
                }],
            }],
            images: vec![("Tiles".to_string(), 2, 1, vec![0xFF; 8])],
            images_gen: 3,
            watch_value: Some(0xAB),
            paused: true,
            reg_names: vec!["cpsr", "r0", "r1"],
            status: StatusMessage::Paused,
            pending: Some(vm::PendingWrite {
                addr: 0x100,
                old: Some(0xAB),
                value: 0x42,
            }),
        }
    }

    /// Open a debugger window for widget tests. Each test uses its own
    /// application: a second Application fights the first for registration.
    fn open_test_window() -> Option<(gtk::Application, gtk::ApplicationWindow, DebuggerWindow)> {
        let (app, main, state) = harness()?;
        let win = DebuggerWindowCore::open(&app, &main, state);
        Some((app, main, win))
    }

    #[test]
    fn widget_harness_builds_app_state() {
        let Some((_app, _main, _state)) = harness() else {
            eprintln!("SKIP: debugger widget test needs a display");
            return;
        };
    }

    #[test]
    fn widget_debugger_open_constructs() {
        let Some((_app, _main, win)) = open_test_window() else {
            eprintln!("SKIP: debugger widget test needs a display");
            return;
        };
        // No core reads as paused: the toggle offers Resume.
        assert_eq!(win.borrow().pause_button.label().as_deref(), Some("Resume"));
        assert!(win.borrow().data.selected_space().is_none());
        assert_eq!(win.borrow().data.selected_name(), "");
        assert_eq!(win.borrow().data.anchor(), 0);
        // Independent toplevel like the tao twin: no transient-for parenting,
        // so the debugger never drags along with the main window.
        assert!(win.borrow().window.transient_for().is_none());
    }

    #[test]
    fn widget_drive_actions() {
        let Some((_app, _main, win)) = open_test_window() else {
            eprintln!("SKIP: debugger widget test needs a display");
            return;
        };
        // One mock space so selection can prepare without a core:
        // the drain still refuses loudly at the core boundary.
        win.borrow_mut().data.spaces = vec![(SpaceId(0), "Mock".to_string(), 0)];
        assert!(win.borrow().data.selected_space().is_some());
        assert_eq!(win.borrow().data.selected_name(), "Mock");
        // Memory navigation resolves valid hex, rejects the rest.
        win.borrow().mem_input.set_text("100");
        DebuggerWindowCore::mem_go(&win);
        assert_eq!(win.borrow().data.nav.mem_addr(), 0x100);
        win.borrow().mem_input.set_text("zz");
        DebuggerWindowCore::mem_go(&win);
        assert!(matches!(
            win.borrow().data.display.status,
            StatusMessage::Local(ref text) if text.contains("parse failed")
        ));
        DebuggerWindowCore::page_mem(&win, 1);
        DebuggerWindowCore::page_mem(&win, -1);
        // The stepper cycles the single mock space.
        DebuggerWindowCore::cycle_space(&win, 1);
        DebuggerWindowCore::cycle_space(&win, -1);
        // Disassembly navigation pins, follows, and backs out.
        win.borrow().dis_input.set_text("300");
        DebuggerWindowCore::dis_go(&win);
        assert!(!win.borrow().data.nav.follow_pc());
        win.borrow().dis_input.set_text("zz");
        DebuggerWindowCore::dis_go(&win);
        DebuggerWindowCore::dis_back(&win);
        DebuggerWindowCore::dis_line_clicked(&win, 0x302);
        DebuggerWindowCore::follow_target(&win, 0x400);
        DebuggerWindowCore::dis_back(&win);
        DebuggerWindowCore::dis_back(&win);
        // Bookmarks default empty labels, fill to the cap, jump, delete.
        win.borrow().bookmark_input.set_text("");
        DebuggerWindowCore::bookmark_add(&win);
        assert_eq!(win.borrow().data.bookmarks.len(), 1);
        for i in 1..9u32 {
            DebuggerWindowCore::dis_line_clicked(&win, 0x300 + i * 0x10);
            DebuggerWindowCore::bookmark_add(&win);
        }
        assert_eq!(win.borrow().data.bookmarks.len(), 8);
        DebuggerWindowCore::dis_line_clicked(&win, 0x500);
        DebuggerWindowCore::bookmark_add(&win);
        assert_eq!(win.borrow().data.bookmarks.len(), 8);
        DebuggerWindowCore::bookmark_jump(&win, 0x310);
        DebuggerWindowCore::bookmark_delete(&win, 0x310);
        assert_eq!(win.borrow().data.bookmarks.len(), 7);
    }

    #[test]
    fn widget_drive_selection_and_writes() {
        let Some((_app, _main, win)) = open_test_window() else {
            eprintln!("SKIP: debugger widget test needs a display");
            return;
        };
        win.borrow_mut().data.spaces = vec![(SpaceId(0), "Mock".to_string(), 0)];
        // Row click selects, then watch/freeze/edit ride the target.
        DebuggerWindowCore::row_selected(&win, 0x10);
        assert_eq!(win.borrow().data.selected, Some((0x10, None)));
        DebuggerWindowCore::watch_set(&win);
        assert_eq!(win.borrow().data.watch, Some(0x10));
        DebuggerWindowCore::watch_clear(&win);
        assert!(win.borrow().data.watch.is_none());
        win.borrow_mut().data.selected = None;
        DebuggerWindowCore::watch_set(&win);
        win.borrow_mut().data.selected = Some((0x10, Some(0)));
        win.borrow().freeze_input.set_text("FF");
        DebuggerWindowCore::freeze_set(&win);
        assert_eq!(win.borrow().data.freeze, Some((0x10, 0xFF)));
        win.borrow().edit_input.set_text("00");
        DebuggerWindowCore::edit_freeze_set(&win);
        assert_eq!(win.borrow().data.freeze, Some((0x10, 0x00)));
        win.borrow().freeze_input.set_text("1FF");
        DebuggerWindowCore::freeze_set(&win);
        win.borrow_mut().data.selected = None;
        DebuggerWindowCore::freeze_set(&win);
        DebuggerWindowCore::freeze_clear(&win);
        assert!(win.borrow().data.freeze.is_none());
        // Write stages SelectRow plus EditStage, then commits loudly
        // without a core; cancel clears the optimistic text.
        win.borrow_mut().data.selected = Some((0x10, Some(0)));
        win.borrow().edit_input.set_text("42");
        DebuggerWindowCore::edit_write(&win);
        assert!(win.borrow().data.display.pending.is_some());
        DebuggerWindowCore::confirm_write(&win);
        assert_eq!(win.borrow().data.display.status, StatusMessage::NoCore);
        assert_eq!(win.borrow().edit_input.text().as_str(), "");
        DebuggerWindowCore::edit_write(&win);
        DebuggerWindowCore::cancel_write(&win);
        assert!(win.borrow().data.display.pending.is_none());
        win.borrow_mut().data.selected = None;
        DebuggerWindowCore::edit_write(&win);
        win.borrow().edit_input.set_text("zz");
        win.borrow_mut().data.selected = Some((0x10, Some(0)));
        DebuggerWindowCore::edit_write(&win);
        // Register pick then Set pushes one register write.
        DebuggerWindowCore::reg_selected(&win, "r0");
        assert_eq!(win.borrow().data.reg_target, Some("r0"));
        win.borrow().reg_input.set_text("7");
        DebuggerWindowCore::reg_set(&win);
        assert_eq!(win.borrow().data.display.status, StatusMessage::NoCore);
        win.borrow().reg_input.set_text("zz");
        DebuggerWindowCore::reg_set(&win);
        win.borrow_mut().data.reg_target = None;
        DebuggerWindowCore::reg_set(&win);
    }

    #[test]
    fn widget_close_request_hides_and_represents() {
        let Some((_app, _main, win)) = open_test_window() else {
            eprintln!("SKIP: debugger widget test needs a display");
            return;
        };
        // A real window-manager close drives delete-event (not external X destroy):
        // the handler hides and the idempotent reopen re-presents it.
        win.borrow().window.close();
        assert!(!win.borrow().window.is_visible());
        win.borrow().present();
        assert!(win.borrow().window.is_visible());
    }

    #[test]
    fn widget_ppu_opens_and_represents() {
        let Some((_app, _main, win)) = open_test_window() else {
            eprintln!("SKIP: debugger widget test needs a display");
            return;
        };
        // The viewer opens once, then re-presents with focus.
        assert!(win.borrow().ppu_window.is_none());
        DebuggerWindowCore::open_ppu(&win);
        assert!(win.borrow().ppu_window.is_some());
        DebuggerWindowCore::open_ppu(&win);
        // Same independence as the debugger above.
        let ppu = win.borrow().ppu_window.clone().expect("ppu open");
        assert!(ppu.borrow().window.transient_for().is_none());
    }

    #[test]
    fn widget_rebuild_renders_synthetic_display() {
        let Some((_app, _main, win)) = open_test_window() else {
            eprintln!("SKIP: debugger widget test needs a display");
            return;
        };
        {
            let mut this = win.borrow_mut();
            this.data.display = synthetic_display();
            this.data.selected = Some((0x102, Some(9)));
            this.data.watch = Some(0x100);
            this.data.freeze = Some((0x100, 0xFF));
            this.data.reg_target = Some("r0");
            this.data.bookmarks = vec![(0x300, "BM1".to_string())];
            this.data.nav.navigate(0x300, 0x302);
        }
        DebuggerWindowCore::rebuild_partial(&win, RebuildScope::All);
        assert!(win.borrow().disasm_title.label().contains("@0x00000302"));
        assert!(
            win.borrow()
                .back_button
                .label()
                .as_deref()
                .unwrap_or("")
                .contains("(1)")
        );
        assert!(win.borrow().reg_target_label.label().contains("r0"));
        assert!(win.borrow().watch_label.label().contains("0x00000100"));
        assert!(win.borrow().freeze_label.label().contains("0x00000100"));
        assert!(win.borrow().select_label.label().contains("0x00000102"));
        // The viewer follows the rebuild with images and panels.
        DebuggerWindowCore::open_ppu(&win);
        assert!(win.borrow().ppu_window.is_some());
    }

    #[test]
    fn widget_preferences_dialog_builds() {
        let Some((_app, main, state)) = harness() else {
            eprintln!("SKIP: preferences widget test needs a display");
            return;
        };
        // Builds the full dialog (settings snapshot drives every
        // section); closing runs the callback without applying.
        super::super::preferences::present_preferences_dialog(&main, state, || {});
    }

    #[test]
    fn rebuild_scope_defaults_to_full_refresh() {
        assert_eq!(super::RebuildScope::default(), super::RebuildScope::All);
        assert_eq!(super::BOOKMARK_CAP, 8);
    }

    #[test]
    fn write_input_trims_and_rejects_empty() {
        use nerust_settings_core::model::language::AppLanguage;
        assert_eq!(
            super::resolve_write_input(AppLanguage::English, Some((0x10, None)), "  AB  "),
            Ok((0x10, 0xAB))
        );
        assert_eq!(
            super::resolve_write_input(AppLanguage::English, Some((0x10, None)), ""),
            Err("parse failed: ".to_string())
        );
        // Uppercase hex and boundary byte both pass.
        assert_eq!(
            super::resolve_write_input(AppLanguage::English, Some((0x10, None)), "FF"),
            Ok((0x10, 0xFF))
        );
    }

    #[test]
    fn freeze_input_trims_whitespace() {
        use nerust_settings_core::model::language::AppLanguage;
        assert_eq!(
            super::resolve_freeze_input(AppLanguage::English, Some((0x10, Some(0x00))), " 0a "),
            Ok((0x10, 0x0A))
        );
    }
}
