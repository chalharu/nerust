//! GTK debugger window: same semantics, shell APIs, and view-model formatters as the tao twin.
//! Action batches run as async shell-drain flights polled on the GTK tick.

use std::{cell::RefCell, ops::Deref, rc::Rc};

use gtk::{gdk, glib, prelude::*};
use wakui::aux::{AuxWindows, OpenRequest};
use wakui::debugger::{
    DebugHandle, DebugSnapshot, DebuggerEvent, DebuggerScope, RegRow, pin_marker,
};
use wakui::focus::AuxKind;
use wakui::window::{DebugContents, Window as HostWindow, WindowVisitor};

use super::State;

type RebuildScope = DebuggerScope;

pub(crate) type DebuggerWindow = Rc<RefCell<DebuggerWindowCore>>;

pub(crate) struct DebuggerWindowCore {
    window: gtk::ApplicationWindow,
    state: Rc<RefCell<State>>,
    handle: DebugHandle,
    viewers: AuxWindows<u64, super::ppu_window::PpuWindow>,
    widgets: DebuggerWidgets,
}

/// Native handles can be used after releasing the controller's RefCell borrow.
#[derive(Clone)]
pub(crate) struct DebuggerWidgets {
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

impl Deref for DebuggerWindowCore {
    type Target = DebuggerWidgets;

    fn deref(&self) -> &Self::Target {
        &self.widgets
    }
}

struct DebuggerView {
    snapshot: DebugSnapshot,
    widgets: DebuggerWidgets,
}

impl Deref for DebuggerView {
    type Target = DebuggerWidgets;

    fn deref(&self) -> &Self::Target {
        &self.widgets
    }
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
    /// Interpret the owned reservation only after the registry borrow ends.
    pub(crate) fn from_window(
        model: &HostWindow,
        app: &gtk::Application,
        main: &gtk::ApplicationWindow,
        state: Rc<RefCell<State>>,
    ) -> Option<DebuggerWindow> {
        struct Builder<'a> {
            app: &'a gtk::Application,
            main: &'a gtk::ApplicationWindow,
            state: Rc<RefCell<State>>,
            title: String,
            result: Option<DebuggerWindow>,
        }
        impl WindowVisitor for Builder<'_> {
            fn begin_window(&mut self, model: &HostWindow) {
                self.title.clone_from(&model.title);
            }
            fn visit_debug(&mut self, _contents: &DebugContents) {
                self.result = Some(DebuggerWindowCore::open_group(
                    self.app,
                    self.main,
                    self.state.clone(),
                    &self.title,
                ));
            }
        }
        let mut builder = Builder {
            app,
            main,
            state,
            title: String::new(),
            result: None,
        };
        model.visit(&mut builder);
        builder.result
    }

    #[cfg(test)]
    fn open(
        app: &gtk::Application,
        main: &gtk::ApplicationWindow,
        state: Rc<RefCell<State>>,
    ) -> DebuggerWindow {
        Self::from_window(&HostWindow::auxiliary(AuxKind::Debug), app, main, state)
            .expect("debug contents")
    }

    /// Open the debugger: build widgets, then pause and refresh through the drain.
    /// Closing keeps the pause; resume stays explicit.
    fn open_group(
        app: &gtk::Application,
        // Independent toplevel like the tao twin: no transient-for parenting.
        // True dialogs (preferences, file/error) stay transient.
        _main_window: &gtk::ApplicationWindow,
        state: Rc<RefCell<State>>,
        title: &str,
    ) -> DebuggerWindow {
        let handle = state.borrow_mut().host.open_debug();
        let chrome = state.borrow().host.debugger_chrome();
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title(title)
            .default_width(1100)
            // Taller than the tao twin: Adwaita rows need ~920px
            // to show the panes plus the dump viewport.
            .default_height(920)
            .build();

        let pause_button = action_button(chrome.pause.as_str());
        let status_label = gtk::Label::new(None);
        status_label.set_xalign(0.0);
        let disasm_title = gtk::Label::new(Some(chrome.disassembly.as_str()));
        disasm_title.set_xalign(0.0);
        let disasm_list = gtk::ListBox::new();
        let dis_input = entry(Some(chrome.addr_hex.as_str()));
        let back_button = action_button(&chrome.back_label(0));
        let space_label = gtk::Label::new(None);
        space_label.set_xalign(0.0);
        space_label.set_hexpand(true);
        let mem_input = entry(Some(chrome.addr_hex.as_str()));
        let dump_list = gtk::ListBox::new();
        let select_label = gtk::Label::new(Some(chrome.select_dump_row.as_str()));
        select_label.set_xalign(0.0);
        let edit_input = entry(Some(chrome.hex_byte.as_str()));
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
        let reg_target_label = gtk::Label::new(Some(chrome.pick_register.as_str()));
        reg_target_label.set_xalign(0.0);
        let reg_input = entry(Some(chrome.hex.as_str()));
        let regs_row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        regs_row.append(&regs_left);
        regs_row.append(&regs_right);
        let watch_label = gtk::Label::new(Some(chrome.watch_none.as_str()));
        watch_label.set_xalign(0.0);
        watch_label.set_hexpand(true);
        let freeze_input = entry(Some(chrome.hex_byte.as_str()));
        let freeze_label = gtk::Label::new(Some(chrome.freeze_none.as_str()));
        freeze_label.set_xalign(0.0);
        freeze_label.set_hexpand(true);
        let bookmark_input = entry(Some(chrome.label.as_str()));
        let bookmark_list = gtk::ListBox::new();
        let write_button = action_button(chrome.write.as_str());

        let result: DebuggerWindow = Rc::new(RefCell::new(Self {
            window: window.clone(),
            state: state.clone(),
            handle,
            viewers: Default::default(),
            widgets: DebuggerWidgets {
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
            },
        }));

        // Toolbar: a single Pause/Resume toggle plus stepping,
        // like tao. The label follows the drained pause state.
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        toolbar.append(&pause_button);
        {
            let win = result.clone();
            pause_button.connect_clicked(move |_| {
                Self::dispatch(&win, DebuggerEvent::TogglePausePressed);
            });
        }
        for (label, action) in [
            (chrome.step_frame.as_str(), DebuggerEvent::StepFramePressed),
            (chrome.step_instr.as_str(), DebuggerEvent::StepInstrPressed),
        ] {
            let button = action_button(label);
            let win = result.clone();
            button.connect_clicked(move |_| Self::dispatch(&win, action.clone()));
            toolbar.append(&button);
        }
        let ppu_button = action_button(chrome.ppu_viewer.as_str());
        Self::wire_toolbar(&result, &ppu_button);
        toolbar.append(&ppu_button);

        // Disassembly header: title, Follow PC, addr entry, Go, Back.
        let dis_nav = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let follow_button = action_button(chrome.follow_pc.as_str());
        let go_button = action_button(chrome.go.as_str());
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
        let title = gtk::Label::new(Some(chrome.debugger.as_str()));
        title.set_xalign(0.0);
        left_col.append(&title);
        left_col.append(&toolbar);
        left_col.append(&dis_nav);
        left_col.append(&dis_scroll);

        // Right state column (scrolls independently; PPU panels are long).
        let right_col = gtk::Box::new(gtk::Orientation::Vertical, 12);
        right_col.append(&status_label);
        right_col.append(&section(chrome.registers.as_str()));
        right_col.append(&regs_row);
        let reg_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        reg_target_label.set_size_request(140, -1);
        let reg_set = action_button(chrome.set.as_str());
        Self::wire_reg_row(&result, &reg_set, &reg_input);
        reg_row.append(&reg_target_label);
        reg_row.append(&reg_input);
        reg_row.append(&reg_set);
        right_col.append(&reg_row);
        right_col.append(&section(chrome.watch.as_str()));
        let watch_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let watch_button = action_button(chrome.watch.as_str());
        let watch_clear = action_button(chrome.clear.as_str());
        watch_row.append(&watch_button);
        watch_row.append(&watch_label);
        watch_row.append(&watch_clear);
        right_col.append(&watch_row);
        right_col.append(&section(chrome.freeze.as_str()));
        let freeze_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let freeze_button = action_button(chrome.freeze.as_str());
        let freeze_clear = action_button(chrome.clear.as_str());
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
        right_col.append(&section(chrome.bookmarks.as_str()));
        let bm_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        bookmark_input.set_hexpand(true);
        let bm_add = action_button(chrome.add.as_str());
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
        memory.append(&section(chrome.memory.as_str()));
        let space_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let space_title = gtk::Label::new(Some(chrome.space.as_str()));
        space_title.set_size_request(150, -1);
        space_title.set_xalign(0.0);
        let space_prev = action_button(chrome.prev.as_str());
        let space_next = action_button(chrome.next.as_str());
        Self::wire_space_prev_next(&result, &space_prev, &space_next);
        space_row.append(&space_title);
        space_row.append(&space_prev);
        space_row.append(&space_label);
        space_row.append(&space_next);
        memory.append(&space_row);
        let mem_nav = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let page_title = gtk::Label::new(Some(chrome.page.as_str()));
        page_title.set_size_request(150, -1);
        page_title.set_xalign(0.0);
        let mem_prev = action_button(chrome.prev.as_str());
        let mem_next = action_button(chrome.next.as_str());
        let mem_go = action_button(chrome.go.as_str());
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
        let edit_watch = action_button(chrome.watch.as_str());
        let edit_freeze = action_button(chrome.freeze.as_str());
        let confirm_button = action_button(chrome.confirm.as_str());
        confirm_button.add_css_class("suggested-action");
        let cancel_button = action_button(chrome.cancel.as_str());
        cancel_button.add_css_class("flat");
        let refresh_button = action_button(chrome.refresh.as_str());
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
        Self::open_refresh(&result);
        // Live PPU follow while the core runs. Images-only, never a full drain:
        // the paused-gated inspect refuses while running and would blank the lists.
        let weak = Rc::downgrade(&result);
        glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
            let Some(win) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            // External destruction leaves the Rc alive but the surface
            // dead: any touch below queries a destroyed native window
            // (fatal X error), so stop instead.
            if win.borrow().is_gone() {
                return glib::ControlFlow::Break;
            }
            // Backstop for drain flights: poll here too so a missed watcher
            // can never strand a pending display.
            Self::poll_flight(&win);
            Self::live_tick(&win);
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
        let (state, handle, ppu_open) = {
            let this = win.borrow();
            (
                this.state.clone(),
                this.handle,
                this.viewers.get(AuxKind::Ppu).is_some(),
            )
        };
        let visible = win.borrow().window.is_visible();
        state.borrow_mut().host.debug_live_tick(handle, ppu_open);
        let effects = state
            .borrow_mut()
            .host
            .debug_poll_live(handle, visible, ppu_open);
        let pending = effects.pending;
        Self::present_live_effects(win, &effects);
        if pending {
            Self::watch_live_flight(win);
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
            if win.borrow().is_gone() {
                return glib::ControlFlow::Break;
            }
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
        let (state, handle, ppu_open) = {
            let this = win.borrow();
            (
                this.state.clone(),
                this.handle,
                this.viewers.get(AuxKind::Ppu).is_some(),
            )
        };
        let visible = win.borrow().window.is_visible();
        let effects = state
            .borrow_mut()
            .host
            .debug_poll_live(handle, visible, ppu_open);
        let pending = effects.pending;
        Self::present_live_effects(win, &effects);
        pending
    }

    fn present_live_effects(win: &DebuggerWindow, effects: &wakui::debugger::DebuggerLiveEffects) {
        if !effects.changed {
            return;
        }
        let widgets = win.borrow().widgets.clone();
        if effects.running {
            // Images and chrome only: never touch the scrolling lists or inputs.
            let (state, handle) = {
                let this = win.borrow();
                (this.state.clone(), this.handle)
            };
            if let Some(snapshot) = state.borrow().host.debug_snapshot(handle) {
                widgets.status_label.set_label(&snapshot.rows.status_text);
            }
        }
        Self::sync_ppu(win);
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
            Self::dispatch(&clicked, DebuggerEvent::FollowPcToggle);
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
        refresh_button
            .connect_clicked(move |_| Self::dispatch(&clicked, DebuggerEvent::RefreshPressed));
    }

    fn wire_keys(win: &DebuggerWindow, window: &gtk::ApplicationWindow) {
        let keys = gtk::EventControllerKey::new();
        let clicked = win.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            match key_action(key) {
                Some(DebuggerKey::StepFrame) => {
                    Self::dispatch(&clicked, DebuggerEvent::StepFramePressed)
                }
                Some(DebuggerKey::StepInstr) => {
                    Self::dispatch(&clicked, DebuggerEvent::StepInstrPressed)
                }
                Some(DebuggerKey::TogglePause) => {
                    Self::dispatch(&clicked, DebuggerEvent::TogglePausePressed)
                }
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

    pub(crate) fn debug_handle(&self) -> DebugHandle {
        self.handle
    }

    /// Clone the native handle without invoking GTK presentation or lifecycle calls.
    pub(crate) fn native_window(&self) -> gtk::ApplicationWindow {
        self.window.clone()
    }

    fn ppu_window(&self) -> Option<super::ppu_window::PpuWindow> {
        self.viewers.get(AuxKind::Ppu).cloned()
    }

    /// True when the native surface is gone (external destruction skips
    /// close-request and `destroy` never fires). Callers must check before
    /// any touch: GTK queries on a dead surface are fatal X errors.
    pub(crate) fn is_gone(&self) -> bool {
        super::surface_state::surface_gone(&self.window)
    }

    /// Refresh an existing window (idempotent reopen path).
    pub(crate) fn refresh(win: &DebuggerWindow) {
        Self::dispatch(win, DebuggerEvent::RefreshPressed);
    }

    /// Every native gesture enters here, including local-only operations.
    /// Check replacement BEFORE reading inputs or reducing the old gesture.
    fn dispatch(win: &DebuggerWindow, event: DebuggerEvent) {
        if Self::reset_replaced_epoch(win) {
            let widgets = win.borrow().widgets.clone();
            for entry in [
                &widgets.bookmark_input,
                &widgets.freeze_input,
                &widgets.edit_input,
                &widgets.reg_input,
            ] {
                entry.set_text("");
            }
            Self::refresh_all(win);
            return;
        }
        // Echo on gesture rather than changed: typing is entirely local, and
        // programmatic set_text cannot feed back into the reducer during rebuild.
        let widgets = win.borrow().widgets.clone();
        let inputs = [
            DebuggerEvent::MemInputChanged(widgets.mem_input.text().to_string()),
            DebuggerEvent::DisInputChanged(widgets.dis_input.text().to_string()),
            DebuggerEvent::BookmarkInputChanged(widgets.bookmark_input.text().to_string()),
            DebuggerEvent::FreezeInputChanged(widgets.freeze_input.text().to_string()),
            DebuggerEvent::EditInputChanged(widgets.edit_input.text().to_string()),
            DebuggerEvent::RegInputChanged(widgets.reg_input.text().to_string()),
        ];
        let clear_bookmark = matches!(event, DebuggerEvent::BookmarkAdd);
        let clear_edit = matches!(event, DebuggerEvent::ConfirmWrite);
        let was_pending = {
            let this = win.borrow();
            this.state.borrow().host.debug_drain_pending(this.handle)
        };
        let update = {
            let this = win.borrow();
            let need_images = this.viewers.get(AuxKind::Ppu).is_some();
            let handle = this.handle;
            let state = this.state.clone();
            drop(this);
            state.borrow_mut().host.debug_gesture(
                handle,
                inputs.into_iter().collect(),
                event,
                need_images,
            )
        };
        // Consumed text is reflected immediately, not on a later worker reply
        // which could otherwise overwrite newer typing.
        if clear_bookmark || clear_edit {
            let (state, handle) = {
                let this = win.borrow();
                (this.state.clone(), this.handle)
            };
            if let Some(snapshot) = state.borrow().host.debug_snapshot(handle) {
                if clear_bookmark {
                    widgets.bookmark_input.set_text(&snapshot.inputs.bm_input);
                }
                if clear_edit {
                    widgets.edit_input.set_text(&snapshot.inputs.edit_input);
                }
            }
        }
        if update.open_ppu {
            // The common update already requests Refresh. Create first so that
            // the one drain sees need_images; native creation never drains itself.
            Self::create_ppu(win);
        }
        if let Some(scope) = update.scope {
            Self::rebuild_partial(win, scope);
        }
        if Self::poll_flight(win) && !was_pending {
            Self::watch_flight(win);
        }
    }

    fn reset_replaced_epoch(win: &DebuggerWindow) -> bool {
        let (state, handle) = {
            let this = win.borrow();
            (this.state.clone(), this.handle)
        };
        state.borrow_mut().host.debug_reset_if_replaced(handle)
    }

    /// Refresh through an async drain flight, then rebuild from the new display.
    /// Inputs are copied out first so the window borrow never spans the round-trip.
    fn refresh_all(win: &DebuggerWindow) {
        Self::refresh_scope(win, RebuildScope::All);
    }

    /// Gesture-side open port (Pause + Refresh) without naming shell actions.
    fn open_refresh(win: &DebuggerWindow) {
        let (state, handle, need_images, was_pending) = {
            let this = win.borrow();
            let state = this.state.clone();
            let was_pending = state.borrow().host.debug_drain_pending(this.handle);
            (
                state,
                this.handle,
                this.viewers.get(AuxKind::Ppu).is_some(),
                was_pending,
            )
        };
        state
            .borrow_mut()
            .host
            .debug_open_refresh(handle, need_images);
        if Self::poll_flight(win) && !was_pending {
            Self::watch_flight(win);
        }
    }

    /// [`DebuggerWindowCore::refresh_all`] with a rebuild scope: the finished
    /// drain refreshes only the sections its data moved.
    fn refresh_scope(win: &DebuggerWindow, scope: RebuildScope) {
        let (state, handle, need_images, was_pending) = {
            let this = win.borrow();
            let state = this.state.clone();
            let was_pending = state.borrow().host.debug_drain_pending(this.handle);
            (
                state,
                this.handle,
                this.viewers.get(AuxKind::Ppu).is_some(),
                was_pending,
            )
        };
        state
            .borrow_mut()
            .host
            .debug_refresh(handle, scope, need_images);
        if Self::poll_flight(win) && !was_pending {
            Self::watch_flight(win);
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
            if win.borrow().is_gone() {
                return glib::ControlFlow::Break;
            }
            if Self::poll_flight(&win) {
                glib::ControlFlow::Continue
            } else {
                glib::ControlFlow::Break
            }
        });
    }

    /// Poll the stored flight once. Returns whether one is still pending: on
    /// completion the app snapshot refreshes, exactly like a synchronous drain.
    fn poll_flight(win: &DebuggerWindow) -> bool {
        let (state, handle, ppu_open) = {
            let this = win.borrow();
            (
                this.state.clone(),
                this.handle,
                this.viewers.get(AuxKind::Ppu).is_some(),
            )
        };
        let wake = state.borrow_mut().host.debug_poll(handle, ppu_open);
        // The app owns the bridge now: rebuild from the fresh snapshot and
        // refresh the viewer, mirroring the scoped-drain contract.
        Self::rebuild_all(win);
        Self::sync_ppu(win);
        wake.flight_pending
    }

    fn open_ppu(win: &DebuggerWindow) {
        Self::dispatch(win, DebuggerEvent::PpuOpenPressed);
    }

    fn create_ppu(win: &DebuggerWindow) {
        // Close hides, so keep its ticket. External destruction can skip
        // lifecycle signals: retire the dead ticket before reserving again.
        let existing = {
            let this = win.borrow();
            this.viewers.route_id(AuxKind::Ppu).zip(this.ppu_window())
        };
        if let Some((id, ppu)) = existing
            && super::surface_state::surface_gone(&ppu.borrow().window)
        {
            let removed = win.borrow_mut().viewers.remove_id(id);
            drop(removed);
        }
        // Reserve before native construction/presentation can reenter us.
        let request = win.borrow_mut().viewers.request_open(AuxKind::Ppu);
        let id = match request {
            OpenRequest::Create(id) => id,
            OpenRequest::Present(_) => {
                let ppu = win.borrow().ppu_window();
                if let Some(ppu) = ppu {
                    ppu.borrow().present();
                }
                return;
            }
            OpenRequest::Pending(_) => return,
        };
        let app = win
            .borrow()
            .window
            .application()
            .expect("debugger has an application");
        let main = win.borrow().window.clone();
        let (state, handle) = {
            let this = win.borrow();
            (this.state.clone(), this.handle)
        };
        let host = state.borrow();
        let images = host.host.debug_pictures(handle);
        let images_gen = host.host.debug_generation(handle);
        let panels = host.host.debug_panels(handle);
        let chrome = host.host.ppu_chrome();
        drop(host);
        let model = win.borrow().viewers.window(id).cloned();
        let ppu = model.and_then(|model| {
            super::ppu_window::PpuWindowCore::from_window(
                &model, &app, &main, state, handle, images, images_gen, panels, chrome,
            )
        });
        let Some(ppu) = ppu else {
            win.borrow_mut().viewers.cancel_open(id);
            return;
        };
        let completed = win.borrow_mut().viewers.complete_open(id, None, ppu);
        if let Err(ppu) = completed {
            // Destruction can emit callbacks: never retain the registry borrow.
            let native = ppu.borrow().window.clone();
            native.destroy();
        }
    }

    fn mem_go(win: &DebuggerWindow) {
        Self::dispatch(win, DebuggerEvent::MemGo);
    }

    fn page_mem(win: &DebuggerWindow, delta: i32) {
        Self::dispatch(win, DebuggerEvent::MemPage(delta));
    }

    fn cycle_space(win: &DebuggerWindow, delta: i32) {
        Self::dispatch(
            win,
            if delta < 0 {
                DebuggerEvent::SpacePrev
            } else {
                DebuggerEvent::SpaceNext
            },
        );
    }

    fn dis_go(win: &DebuggerWindow) {
        Self::dispatch(win, DebuggerEvent::DisGo);
    }

    fn dis_back(win: &DebuggerWindow) {
        Self::dispatch(win, DebuggerEvent::DisBack);
    }

    fn follow_target(win: &DebuggerWindow, addr: u32) {
        Self::dispatch(win, DebuggerEvent::FollowTarget(addr));
    }

    fn dis_line_clicked(win: &DebuggerWindow, addr: u32) {
        Self::dispatch(win, DebuggerEvent::DisLineClicked(addr));
    }

    fn bookmark_add(win: &DebuggerWindow) {
        Self::dispatch(win, DebuggerEvent::BookmarkAdd);
    }

    fn bookmark_jump(win: &DebuggerWindow, addr: u32) {
        Self::dispatch(win, DebuggerEvent::BookmarkJump(addr));
    }

    fn bookmark_delete(win: &DebuggerWindow, addr: u32) {
        Self::dispatch(win, DebuggerEvent::BookmarkDelete(addr));
    }

    fn row_selected(win: &DebuggerWindow, addr: u32) {
        Self::dispatch(win, DebuggerEvent::RowSelected(addr));
    }

    fn reg_selected(win: &DebuggerWindow, name: &'static str) {
        Self::dispatch(win, DebuggerEvent::RegSelected(name));
    }

    fn reg_set(win: &DebuggerWindow) {
        Self::dispatch(win, DebuggerEvent::RegSet);
    }

    fn watch_set(win: &DebuggerWindow) {
        Self::dispatch(win, DebuggerEvent::WatchSet);
    }

    fn watch_clear(win: &DebuggerWindow) {
        Self::dispatch(win, DebuggerEvent::WatchClear);
    }

    fn freeze_set(win: &DebuggerWindow) {
        Self::dispatch(win, DebuggerEvent::FreezeSet);
    }

    /// Edit-row Freeze: reads the edit row's own input, so each of the row's
    /// three buttons reads what sits next to it.
    fn edit_freeze_set(win: &DebuggerWindow) {
        Self::dispatch(win, DebuggerEvent::EditFreezeSet);
    }

    fn freeze_clear(win: &DebuggerWindow) {
        Self::dispatch(win, DebuggerEvent::FreezeClear);
    }

    fn edit_write(win: &DebuggerWindow) {
        Self::dispatch(win, DebuggerEvent::EditWrite);
    }

    fn confirm_write(win: &DebuggerWindow) {
        Self::dispatch(win, DebuggerEvent::ConfirmWrite);
    }

    fn cancel_write(win: &DebuggerWindow) {
        Self::dispatch(win, DebuggerEvent::CancelWrite);
    }

    /// Rebuild slice by slice. The header always refreshes; heavy lists rebuild
    /// only when their data moved, so highlight-only gestures skip ~700 creates.
    fn rebuild_partial(win: &DebuggerWindow, scope: RebuildScope) {
        // Native property notifications and child removal may reenter. Own all
        // presentation data before touching GTK; never clone image buffers here.
        let this = match Self::view_snapshot(win) {
            Some(view) => view,
            None => return,
        };
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

        Self::sync_ppu(win);
    }

    fn sync_ppu(win: &DebuggerWindow) {
        let (ppu, images, generation, panels) = {
            let this = win.borrow();
            let Some(ppu) = this.ppu_window() else {
                return;
            };
            let state = this.state.clone();
            let handle = this.handle;
            let host = state.borrow();
            (
                ppu,
                host.host.debug_pictures(handle),
                host.host.debug_generation(handle),
                host.host.debug_panels(handle),
            )
        };
        let native = ppu.borrow().window.clone();
        if !super::surface_state::surface_gone(&native) {
            ppu.borrow_mut()
                .sync(&images, generation, panels.as_deref());
        }
    }

    /// Snapshot helper: the app owns the bridge; native code renders copies.
    fn view_snapshot(win: &DebuggerWindow) -> Option<DebuggerView> {
        let (state, handle, widgets) = {
            let this = win.borrow();
            (this.state.clone(), this.handle, this.widgets.clone())
        };
        state
            .borrow()
            .host
            .debug_snapshot(handle)
            .map(|snapshot| DebuggerView { snapshot, widgets })
    }

    /// Full rebuild from the latest app snapshot.
    fn rebuild_all(win: &DebuggerWindow) {
        let Some(this) = Self::view_snapshot(win) else {
            return;
        };
        Self::rebuild_header(&this);
        Self::rebuild_disasm(win, &this);
        Self::rebuild_dump(win, &this);
        Self::rebuild_edit_row(&this);
        Self::rebuild_state(win, &this);
        Self::rebuild_bookmarks(win, &this);
        Self::sync_ppu(win);
    }

    /// Rebuild helpers use an owned snapshot, never a borrowed controller.
    fn rebuild_header(this: &DebuggerView) {
        let rows = &this.snapshot.rows;
        let inputs = &this.snapshot.inputs;
        let chrome = &this.snapshot.chrome;
        this.pause_button.set_label(chrome.pause_label(rows.paused));
        this.status_label.set_label(&rows.status_text);
        // The title names the anchor: clicks pin it visibly instead
        // of silently moving the window.
        let anchor = if inputs.follow_pc {
            rows.disasm.first().map(|line| line.addr).unwrap_or(0)
        } else {
            inputs.dis_addr.unwrap_or(0)
        };
        let dis_title = chrome.disassembly_title(inputs.follow_pc, anchor);
        this.disasm_title.set_label(&dis_title);
        this.back_button
            .set_label(&chrome.back_label(inputs.back_len));
        this.space_label.set_label(&this.snapshot.space_name);
        if this.mem_input.text() != inputs.mem_input {
            this.mem_input.set_text(&inputs.mem_input);
        }
        if this.dis_input.text() != inputs.dis_input {
            this.dis_input.set_text(&inputs.dis_input);
        }
    }

    fn rebuild_disasm(win: &DebuggerWindow, this: &DebuggerView) {
        let rows = &this.snapshot.rows;
        let inputs = &this.snapshot.inputs;
        let chrome = &this.snapshot.chrome;
        // Pinned anchor rows carry `@` like the tao viewer so a line click
        // reads back visibly even off-PC. Nothing pinned in follow-PC mode.
        let pinned = if inputs.follow_pc {
            None
        } else {
            Some(inputs.dis_addr.unwrap_or(0))
        };
        clear_list(&this.disasm_list);
        if rows.disasm.is_empty() {
            this.disasm_list.append(&static_row(&chrome.no_disassembly));
            this.disasm_list
                .append(&static_row(&chrome.disassembly_hint));
        }
        for line in &rows.disasm {
            let row = gtk::ListBoxRow::new();
            row.set_selectable(false);
            let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            // The PC row carries a `>` marker like the dump
            // selection: the current position reads at a glance.
            let is_pinned = pinned == Some(line.addr);
            let marker = pin_marker(line.is_pc, is_pinned);
            let button = gtk::Button::with_label(&format!("{marker}{}", line.text));
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

    fn rebuild_dump(win: &DebuggerWindow, this: &DebuggerView) {
        let rows = &this.snapshot.rows;
        let selected = this.snapshot.selected;
        clear_list(&this.dump_list);
        // Byte-precise selection like the tao viewer: every byte is its own
        // button, so the write target reads exactly.
        for hex_row in &rows.dump {
            let selected_addr = selected.map(|(a, _)| a);
            // Containing-row match: a mid-row pick still lights its row.
            let row_end = hex_row.addr + hex_row.bytes.len() as u32;
            let row_hit = selected_addr.is_some_and(|a| a >= hex_row.addr && a < row_end);
            let changed = rows.changed_addrs.contains(&hex_row.addr);
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
            for (i, byte) in hex_row.bytes.iter().enumerate() {
                let addr = hex_row.addr + i as u32;
                let byte = *byte;
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

    fn rebuild_edit_row(this: &DebuggerView) {
        let rows = &this.snapshot.rows;
        let chrome = &this.snapshot.chrome;
        this.select_label.set_label(&rows.selected_line);
        // The target rides on the button: writes always land on the
        // selected row, never on the page anchor.
        this.write_button
            .set_label(&chrome.write_target_label(this.snapshot.selected));
        match &rows.pending_text {
            Some(line) => {
                this.confirm_label.set_label(line);
                this.confirm_row.set_visible(true);
            }
            None => this.confirm_row.set_visible(false),
        }
    }

    fn rebuild_state(win: &DebuggerWindow, this: &DebuggerView) {
        let rows = &this.snapshot.rows;
        let chrome = &this.snapshot.chrome;
        // Registers flow into two clickable columns: the kernel order is kept,
        // the view only halves the list. Lines arrive pre-formatted.
        let regs = &rows.regs;
        let mid = regs.len().div_ceil(2).min(regs.len());
        Self::fill_reg_list(win, &this.regs_left, &regs[..mid], this.snapshot.reg_target);
        Self::fill_reg_list(
            win,
            &this.regs_right,
            &regs[mid..],
            this.snapshot.reg_target,
        );
        this.reg_target_label
            .set_label(&chrome.set_target_label(this.snapshot.reg_target));
        this.watch_label.set_label(&rows.watch_text);
        this.freeze_label.set_label(&rows.freeze_text);
    }

    fn fill_reg_list(
        win: &DebuggerWindow,
        list: &gtk::ListBox,
        regs: &[RegRow],
        target: Option<&'static str>,
    ) {
        clear_list(list);
        for row_data in regs.iter() {
            let name = row_data.name;
            let row = gtk::ListBoxRow::new();
            row.set_selectable(false);
            let button = gtk::Button::with_label(&row_data.text);
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

    fn rebuild_bookmarks(win: &DebuggerWindow, this: &DebuggerView) {
        clear_list(&this.bookmark_list);
        for (addr, label) in &this.snapshot.inputs.bookmarks {
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
    use nerust_frontend_viewdefs::viewport::testkit::NullFactory;

    #[derive(Debug)]
    struct MockRomLoader;

    impl nerust_gui_shell::load::RomLoader for MockRomLoader {
        fn load_rom(
            &mut self,
            _path: &std::path::Path,
            _target: &mut dyn nerust_gui_shell::load::RomLoadTarget,
        ) -> Result<(), nerust_gui_shell::load::RomLoaderError> {
            Err(nerust_gui_shell::load::RomLoaderError::from_session(
                nerust_gui_shell::session::SessionError::NoCore,
            ))
        }
    }

    pub(super) fn harness() -> Option<(
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
            gpu_factory: Rc::new(NullFactory {
                fail_create: true,
                ..Default::default()
            }),
            registry,
            rom_loader: Box::new(MockRomLoader),
            audio_registry: audio,
        };
        let state = Rc::new(RefCell::new(
            crate::State::new_for_tests(ctx).expect("test state builds"),
        ));
        Some((app, main, state))
    }

    fn snapshot_of(win: &DebuggerWindow) -> DebugSnapshot {
        let (state, handle) = {
            let this = win.borrow();
            (this.state.clone(), this.handle)
        };
        state
            .borrow()
            .host
            .debug_snapshot(handle)
            .expect("snapshot")
    }

    fn state_drain_pending(win: &DebuggerWindow) -> bool {
        let (state, handle) = {
            let this = win.borrow();
            (this.state.clone(), this.handle)
        };
        state.borrow().host.debug_drain_pending(handle)
    }

    fn poll_settle(win: &DebuggerWindow) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while DebuggerWindowCore::poll_flight(win) {
            assert!(
                std::time::Instant::now() < deadline,
                "worker did not settle"
            );
            std::thread::yield_now();
        }
    }

    /// Canned NES loader: headless tests that need a loaded core.
    #[derive(Debug, Default)]
    struct NesRomLoader;

    impl nerust_gui_shell::load::RomLoader for NesRomLoader {
        fn load_rom(
            &mut self,
            _path: &std::path::Path,
            target: &mut dyn nerust_gui_shell::load::RomLoadTarget,
        ) -> Result<(), nerust_gui_shell::load::RomLoaderError> {
            use nerust_system_traits::factory::settings::FactorySettingsView;
            let factory = nerust_nes_factory::NesFactory::boxed();
            let view = FactorySettingsView {
                language: Default::default(),
                system_config: factory
                    .as_system_defaults()
                    .unwrap()
                    .default_system_settings(),
            };
            let request = factory
                .resolve_load_request(&view, factory.default_load_options())
                .unwrap();
            target
                .set_active_system(factory.system_id().as_ref())
                .unwrap();
            let mut rom = vec![0u8; 16 + 16384 + 8192];
            rom[..6].copy_from_slice(&[b'N', b'E', b'S', 0x1a, 1, 1]);
            rom[16..19].copy_from_slice(&[0x4c, 0, 0x80]);
            for vector in [0x3ffa, 0x3ffc, 0x3ffe] {
                rom[16 + vector..18 + vector].copy_from_slice(&[0x00, 0x80]);
            }
            target.load_resolved(
                nerust_system_traits::factory::load::MediaObject::new(None, rom),
                request,
            )
        }
    }

    /// NES-capable harness for tests that need a loaded core.
    fn nes_harness() -> Option<(
        gtk::Application,
        gtk::ApplicationWindow,
        Rc<RefCell<crate::State>>,
    )> {
        use std::sync::Arc;
        if !crate::test_support::ui_test_available() {
            return None;
        }
        let app = gtk::Application::builder()
            .application_id("org.nerust.debugger-nes-test")
            .build();
        let main = gtk::ApplicationWindow::builder().application(&app).build();
        let registry = Arc::new(nerust_gui_shell::registry::SystemRegistry::new(vec![
            nerust_nes_factory::NesFactory::boxed().into(),
        ]));
        let audio = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
        let ctx = nerust_gui_shell::context::FrontendContext {
            gpu_factory: Rc::new(NullFactory {
                fail_create: true,
                ..Default::default()
            }),
            registry,
            rom_loader: Box::new(NesRomLoader),
            audio_registry: audio,
        };
        let state = Rc::new(RefCell::new(
            crate::State::new_for_tests(ctx).expect("test state builds"),
        ));
        state
            .borrow_mut()
            .load_path(std::path::Path::new("test.nes"))
            .expect("canned NES loads");
        assert!(state.borrow().host.tick_loaded());
        Some((app, main, state))
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
    fn worker_replacement_rejects_stale_ui_and_resumes_fresh() {
        use wakui::pause::PauseOwner;
        let Some((_app, _main, state)) = nes_harness() else {
            return;
        };
        let app2 = gtk::Application::builder()
            .application_id("org.nerust.debugger-replacement-test")
            .build();
        let main2 = gtk::ApplicationWindow::builder().application(&app2).build();
        let win = DebuggerWindowCore::from_window(
            &HostWindow::auxiliary(AuxKind::Debug),
            &app2,
            &main2,
            state.clone(),
        )
        .expect("debug contents");
        poll_settle(&win);
        // Pause through the gesture path, like the toolbar button.
        DebuggerWindowCore::dispatch(&win, DebuggerEvent::TogglePausePressed);
        poll_settle(&win);
        assert!(state.borrow().host.tick_paused());
        // Reload replaces the session epoch: the next gesture first
        // retires the old queue and rebuilds rather than pausing stale state.
        state
            .borrow_mut()
            .load_path(std::path::Path::new("test.nes"))
            .expect("reload replaces the epoch");
        let handle = win.borrow().handle;
        assert!(state.borrow_mut().host.debug_reset_if_replaced(handle));
        assert!(!state.borrow().host.tick_paused());
        // Fresh gestures run against the replacement session.
        DebuggerWindowCore::dispatch(&win, DebuggerEvent::TogglePausePressed);
        poll_settle(&win);
        assert!(state.borrow().host.tick_paused());
        state
            .borrow_mut()
            .host
            .set_pause_owner(PauseOwner::Modal, false);
        let _ = win;
    }

    /// Open a debugger window for widget tests. Each test uses its own
    /// application: a second Application fights the first for registration.
    fn open_test_window() -> Option<(gtk::Application, gtk::ApplicationWindow, DebuggerWindow)> {
        let (app, main, state) = harness()?;
        let win = DebuggerWindowCore::open(&app, &main, state);
        Some((app, main, win))
    }

    #[test]
    fn widget_destroyed_window_reports_gone() {
        let Some((_app, _main, win)) = open_test_window() else {
            eprintln!("SKIP: debugger widget test needs a display");
            return;
        };
        assert!(!win.borrow().is_gone());
        // External destroy disposes the widget without signals; the
        // helper must report it so reopen paths rebuild.
        win.borrow().window.destroy();
        assert!(win.borrow().is_gone());
    }

    #[test]
    fn widget_debugger_open_constructs() {
        let Some((_app, _main, win)) = open_test_window() else {
            eprintln!("SKIP: debugger widget test needs a display");
            return;
        };
        // No core reads as paused: the toggle offers Resume.
        assert_eq!(win.borrow().pause_button.label().as_deref(), Some("Resume"));
        let snapshot = snapshot_of(&win);
        assert!(snapshot.rows.paused);
        assert!(!snapshot.rows.status_text.is_empty());
        assert!(snapshot.space_name.is_empty());
        assert!(!snapshot.has_spaces);
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
        // Typing updates inputs immediately; worker traffic starts on drain.
        win.borrow().mem_input.set_text("100");
        DebuggerWindowCore::mem_go(&win);
        assert_eq!(snapshot_of(&win).inputs.mem_input, "100");
        assert_eq!(snapshot_of(&win).inputs.mem_addr, Some(0x100));
        win.borrow().mem_input.set_text("zz");
        DebuggerWindowCore::mem_go(&win);
        // Invalid hex reports loudly after the drain round-trips.
        poll_settle(&win);
        assert!(snapshot_of(&win).rows.status_text.contains("parse failed"));
        DebuggerWindowCore::page_mem(&win, 1);
        DebuggerWindowCore::page_mem(&win, -1);
        // Disassembly navigation pins, follows, and backs out.
        win.borrow().dis_input.set_text("300");
        DebuggerWindowCore::dis_go(&win);
        assert_eq!(snapshot_of(&win).inputs.dis_addr, Some(0x300));
        win.borrow().dis_input.set_text("zz");
        DebuggerWindowCore::dis_go(&win);
        DebuggerWindowCore::dis_back(&win);
        DebuggerWindowCore::dis_line_clicked(&win, 0x302);
        DebuggerWindowCore::follow_target(&win, 0x400);
        DebuggerWindowCore::dis_back(&win);
        DebuggerWindowCore::dis_back(&win);
        // Bookmarks anchor, jump, and delete through the app session.
        win.borrow().bookmark_input.set_text("");
        DebuggerWindowCore::bookmark_add(&win);
        assert_eq!(snapshot_of(&win).inputs.bookmarks.len(), 1);
        let anchor = snapshot_of(&win).inputs.bookmarks[0].0;
        DebuggerWindowCore::bookmark_jump(&win, anchor);
        DebuggerWindowCore::bookmark_delete(&win, anchor);
        assert!(snapshot_of(&win).inputs.bookmarks.is_empty());
    }

    #[test]
    fn widget_drive_selection_and_writes() {
        let Some((_app, _main, win)) = open_test_window() else {
            eprintln!("SKIP: debugger widget test needs a display");
            return;
        };
        // Row click selects, then watch/freeze ride the target.
        DebuggerWindowCore::row_selected(&win, 0x10);
        assert_eq!(snapshot_of(&win).selected, Some((0x10, None)));
        DebuggerWindowCore::watch_set(&win);
        assert_eq!(snapshot_of(&win).inputs.watch, Some(0x10));
        DebuggerWindowCore::watch_clear(&win);
        assert!(snapshot_of(&win).inputs.watch.is_none());
        win.borrow().freeze_input.set_text("FF");
        DebuggerWindowCore::freeze_set(&win);
        assert_eq!(snapshot_of(&win).inputs.freeze, Some((0x10, 0xFF)));
        win.borrow().edit_input.set_text("00");
        DebuggerWindowCore::edit_freeze_set(&win);
        assert_eq!(snapshot_of(&win).inputs.freeze, Some((0x10, 0x00)));
        DebuggerWindowCore::freeze_clear(&win);
        assert!(snapshot_of(&win).inputs.freeze.is_none());
        assert!(
            snapshot_of(&win)
                .rows
                .freeze_text
                .contains(snapshot_of(&win).chrome.freeze_none.as_str())
        );
        // Write stages through the drain, then commits loudly
        // without a core; cancel clears the optimistic text.
        win.borrow().edit_input.set_text("42");
        DebuggerWindowCore::edit_write(&win);
        poll_settle(&win);
        assert!(snapshot_of(&win).rows.pending_text.is_some());
        DebuggerWindowCore::confirm_write(&win);
        poll_settle(&win);
        assert_eq!(win.borrow().edit_input.text().as_str(), "");
        win.borrow().edit_input.set_text("42");
        DebuggerWindowCore::edit_write(&win);
        DebuggerWindowCore::cancel_write(&win);
        poll_settle(&win);
        assert!(snapshot_of(&win).rows.pending_text.is_none());
        // Register pick then Set pushes one register write.
        DebuggerWindowCore::reg_selected(&win, "r0");
        assert_eq!(snapshot_of(&win).reg_target, Some("r0"));
        win.borrow().reg_input.set_text("7");
        DebuggerWindowCore::reg_set(&win);
        poll_settle(&win);
        assert!(
            !snapshot_of(&win)
                .rows
                .status_text
                .contains("needs a register")
        );
        win.borrow().reg_input.set_text("zz");
        DebuggerWindowCore::reg_set(&win);
        poll_settle(&win);
        assert!(snapshot_of(&win).rows.status_text.contains("parse failed"));
    }

    #[test]
    fn widget_common_events_typing_scopes_and_reentrant_rebuild() {
        let Some((_app, _main, win)) = open_test_window() else {
            eprintln!("SKIP: debugger widget test needs a display");
            return;
        };
        DebuggerWindowCore::rebuild_partial(&win, RebuildScope::All);
        let widgets = win.borrow().widgets.clone();
        let dump_row = widgets.dump_list.first_child().unwrap();
        let dis_row = widgets.disasm_list.first_child().unwrap();
        let scroll = widgets
            .dump_list
            .ancestor(gtk::ScrolledWindow::static_type())
            .unwrap()
            .downcast::<gtk::ScrolledWindow>()
            .unwrap();
        let adjustment = scroll.vadjustment();
        adjustment.configure(48.0, 0.0, 1000.0, 1.0, 100.0, 100.0);

        // These synchronous GTK notifications must permit reentrant mutation,
        // including notifications emitted by the controller's own set_text.
        let notifications = Rc::new(std::cell::Cell::new(0));
        for entry in [
            &widgets.mem_input,
            &widgets.dis_input,
            &widgets.bookmark_input,
            &widgets.freeze_input,
            &widgets.edit_input,
            &widgets.reg_input,
        ] {
            let weak = Rc::downgrade(&win);
            let count = notifications.clone();
            entry.connect_changed(move |_| {
                assert!(weak.upgrade().unwrap().try_borrow_mut().is_ok());
                count.set(count.get() + 1);
            });
        }
        let weak = Rc::downgrade(&win);
        widgets.status_label.connect_label_notify(move |_| {
            assert!(weak.upgrade().unwrap().try_borrow_mut().is_ok());
        });
        for (entry, input) in [
            (&widgets.mem_input, "zz"),
            (&widgets.dis_input, "500"),
            (&widgets.bookmark_input, "native"),
            (&widgets.freeze_input, "AA"),
            (&widgets.edit_input, "42"),
            (&widgets.reg_input, "7"),
        ] {
            entry.set_text(input);
            assert!(
                !state_drain_pending(&win),
                "typing must not start worker traffic"
            );
        }
        // Actual activate signal -> shared parse status. Only the header
        // rebuilds on a local error: list identity and scroll hold still.
        widgets.mem_input.emit_activate();
        poll_settle(&win);
        assert!(snapshot_of(&win).rows.status_text.contains("parse failed"));
        assert_eq!(widgets.dump_list.first_child().unwrap(), dump_row);
        assert_eq!(widgets.disasm_list.first_child().unwrap(), dis_row);
        assert_eq!(adjustment.value(), 48.0);

        // Real byte-row button -> common SelectRow, then real watch/freeze and
        // production write buttons exercise the shared ordered action batches.
        let row = widgets.dump_list.row_at_index(0).unwrap();
        let bytes = row.child().unwrap().downcast::<gtk::Box>().unwrap();
        let select = bytes
            .first_child()
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap();
        select.emit_clicked();
        assert_eq!(snapshot_of(&win).selected, Some((0x100, None)));
        let watch = action_button("Watch");
        let watch_clear = action_button("Clear watch");
        let freeze = action_button("Freeze");
        let freeze_clear = action_button("Clear freeze");
        DebuggerWindowCore::wire_watch_freeze(&win, &watch, &watch_clear, &freeze, &freeze_clear);
        watch.emit_clicked();
        assert_eq!(snapshot_of(&win).inputs.watch, Some(0x100));
        freeze.emit_clicked();
        assert_eq!(snapshot_of(&win).inputs.freeze, Some((0x100, 0xaa)));
        widgets.write_button.emit_clicked();
        poll_settle(&win);
        let pending = snapshot_of(&win).rows.pending_text.expect("staged write");
        assert!(pending.contains("0x100"));
        assert!(widgets.confirm_row.is_visible());
        let confirm = widgets
            .confirm_label
            .next_sibling()
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap();
        confirm.emit_clicked();
        poll_settle(&win);
        assert!(widgets.edit_input.text().is_empty());
        assert_eq!(snapshot_of(&win).inputs.edit_input, String::new());
        watch_clear.emit_clicked();
        freeze_clear.emit_clicked();
        assert!(snapshot_of(&win).inputs.watch.is_none());
        assert!(snapshot_of(&win).inputs.freeze.is_none());
        assert!(!state_drain_pending(&win));

        widgets.mem_input.set_text("100");
        widgets.mem_input.emit_activate();
        poll_settle(&win);
        assert_eq!(snapshot_of(&win).inputs.mem_addr, Some(0x100));
        assert_eq!(widgets.disasm_list.first_child().unwrap(), dis_row);
        widgets.dis_input.emit_activate();
        poll_settle(&win);
        assert_eq!(snapshot_of(&win).inputs.dis_addr, Some(0x500));
        let add = action_button("Add");
        DebuggerWindowCore::wire_bookmarks(&win, &add, &widgets.bookmark_input);
        add.emit_clicked();
        assert_eq!(snapshot_of(&win).inputs.bookmarks.len(), 1);
        assert!(widgets.bookmark_input.text().is_empty());
        assert!(!state_drain_pending(&win));
        assert!(notifications.get() >= 8);
    }

    #[test]
    fn widget_shared_bookmarks_use_pin_and_dedup() {
        let Some((_app, _main, win)) = open_test_window() else {
            eprintln!("SKIP: debugger widget test needs a display");
            return;
        };
        // Without a core there is no PC row: bookmarks anchor on the
        // disassembly pin instead, and repeat anchors dedup.
        let widgets = win.borrow().widgets.clone();
        widgets.bookmark_input.set_text("empty");
        widgets.bookmark_input.emit_activate();
        poll_settle(&win);
        assert_eq!(snapshot_of(&win).inputs.bookmarks.len(), 1);
        for addr in 0x600..0x606 {
            DebuggerWindowCore::dis_line_clicked(&win, addr);
            widgets.bookmark_input.set_text("pin");
            widgets.bookmark_input.emit_activate();
        }
        poll_settle(&win);
        assert!(snapshot_of(&win).inputs.bookmarks.len() <= 8);
        widgets.bookmark_input.set_text("duplicate at cap");
        widgets.bookmark_input.emit_activate();
        poll_settle(&win);
        assert!(snapshot_of(&win).inputs.bookmarks.len() <= 8);
        assert!(!state_drain_pending(&win));
    }

    #[test]
    fn widget_common_live_and_running_drain_preserve_inspection() {
        let Some((_app, _main, win)) = open_test_window() else {
            eprintln!("SKIP: debugger widget test needs a display");
            return;
        };
        DebuggerWindowCore::rebuild_partial(&win, RebuildScope::All);
        let widgets = win.borrow().widgets.clone();
        let dump_row = widgets.dump_list.first_child().unwrap();
        let dis_row = widgets.disasm_list.first_child().unwrap();
        widgets.mem_input.set_text("unfinished input");
        // Live ticks settle harmlessly without a viewer: no images land,
        // inputs and list identity hold still.
        DebuggerWindowCore::live_tick(&win);
        assert_eq!(widgets.mem_input.text().as_str(), "unfinished input");
        assert_eq!(widgets.dump_list.first_child().unwrap(), dump_row);
        assert_eq!(widgets.disasm_list.first_child().unwrap(), dis_row);
        assert!(!DebuggerWindowCore::poll_live_flight(&win));
        // A fresh drain round-trips through the worker: the pause chrome
        // follows the coreless status while lists keep their identity.
        DebuggerWindowCore::refresh(&win);
        poll_settle(&win);
        assert_eq!(widgets.mem_input.text().as_str(), "unfinished input");
        assert_eq!(widgets.dump_list.first_child().unwrap(), dump_row);
        assert_eq!(widgets.disasm_list.first_child().unwrap(), dis_row);
        assert_eq!(
            widgets.pause_button.label().as_deref(),
            Some("Resume"),
            "coreless drain reads paused"
        );
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
        assert!(win.borrow().ppu_window().is_none());
        DebuggerWindowCore::open_ppu(&win);
        assert!(win.borrow().ppu_window().is_some());
        DebuggerWindowCore::open_ppu(&win);
        // Same independence as the debugger above.
        let ppu = win.borrow().ppu_window().expect("ppu open");
        assert!(ppu.borrow().window.transient_for().is_none());
    }

    #[test]
    fn widget_ppu_hidden_reuses_ticket_and_dead_reopen_rejects_stale_removal() {
        let Some((_app, _main, win)) = open_test_window() else {
            eprintln!("SKIP: debugger widget test needs a display");
            return;
        };
        assert_eq!(win.borrow().native_window(), win.borrow().window);
        DebuggerWindowCore::open_ppu(&win);
        let old = win.borrow().viewers.route_id(AuxKind::Ppu).unwrap();
        let ppu = win.borrow().ppu_window().unwrap();
        let native = ppu.borrow().window.clone();
        native.close();
        assert!(!native.is_visible());
        assert!(!super::super::surface_state::surface_gone(&native));
        DebuggerWindowCore::open_ppu(&win);
        assert_eq!(win.borrow().viewers.route_id(AuxKind::Ppu), Some(old));
        assert!(Rc::ptr_eq(&ppu, &win.borrow().ppu_window().unwrap()));
        assert!(native.is_visible());

        native.destroy();
        assert!(super::super::surface_state::surface_gone(&native));
        DebuggerWindowCore::open_ppu(&win);
        let current = win.borrow().viewers.route_id(AuxKind::Ppu).unwrap();
        let replacement = win.borrow().ppu_window().unwrap();
        assert_ne!(current, old);
        assert!(!Rc::ptr_eq(&ppu, &replacement));
        let removed = win.borrow_mut().viewers.remove_id(old);
        assert!(removed.is_none());
        assert_eq!(win.borrow().viewers.route_id(AuxKind::Ppu), Some(current));
        assert!(Rc::ptr_eq(
            &replacement,
            &win.borrow().ppu_window().unwrap()
        ));
        assert!(replacement.borrow().window.is_visible());
    }

    #[test]
    fn widget_ppu_native_open_reentry_is_pending() {
        let Some((app, _main, win)) = open_test_window() else {
            eprintln!("SKIP: debugger widget test needs a display");
            return;
        };
        // Application membership signals require startup, unlike the other
        // widget-only tests which can present unregistered windows.
        app.register(None::<&gio::Cancellable>)
            .expect("test application registers");
        let added = Rc::new(std::cell::Cell::new(0));
        let count = added.clone();
        let opening = win.clone();
        let signal = app.connect_window_added(move |_, _| {
            count.set(count.get() + 1);
            let id = opening.borrow().viewers.route_id(AuxKind::Ppu).unwrap();
            assert_eq!(
                opening.borrow_mut().viewers.request_open(AuxKind::Ppu),
                OpenRequest::Pending(id)
            );
            DebuggerWindowCore::open_ppu(&opening);
        });
        DebuggerWindowCore::open_ppu(&win);
        app.disconnect(signal);
        assert_eq!(added.get(), 1);
        assert!(win.borrow().ppu_window().is_some());
    }

    #[test]
    fn widget_ppu_stale_native_open_is_destroyed_outside_registry_borrow() {
        let Some((app, _main, win)) = open_test_window() else {
            eprintln!("SKIP: debugger widget test needs a display");
            return;
        };
        app.register(None::<&gio::Cancellable>)
            .expect("test application registers");
        let stale_native = Rc::new(RefCell::new(None));
        let captured = stale_native.clone();
        let opening = win.clone();
        let signal = app.connect_window_added(move |_, native| {
            if captured.borrow().is_some() {
                return;
            }
            *captured.borrow_mut() = Some(native.clone());
            let id = opening.borrow().viewers.route_id(AuxKind::Ppu).unwrap();
            assert!(opening.borrow_mut().viewers.cancel_open(id));
            DebuggerWindowCore::open_ppu(&opening);
        });
        // Removing the rejected native window must permit reentrant mutation.
        let removing = win.clone();
        let removed = app.connect_window_removed(move |_, _| {
            assert!(removing.try_borrow_mut().is_ok());
        });
        DebuggerWindowCore::open_ppu(&win);
        app.disconnect(signal);
        app.disconnect(removed);
        let stale = stale_native.borrow().clone().unwrap();
        assert!(super::super::surface_state::surface_gone(&stale));
        let current = win.borrow().ppu_window().unwrap();
        assert!(current.borrow().window.is_visible());
        assert_ne!(current.borrow().window.upcast_ref::<gtk::Window>(), &stale);
    }
}
