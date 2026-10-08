// SPIKE (iteration 4, DO NOT MERGE): GTK write-probe window.
//
// Renders the same shared `spike_format_dump` text as the tao spike window
// (parity probe) with the same two-step confirm UX. Session access goes
// through `State` spike accessors. Deleted with the branch.

use std::{cell::RefCell, rc::Rc};

use gtk::prelude::*;
use nerust_core_traits::debugger::{SpaceAccess, SpaceId};

use super::State;

fn parse_hex_u32(s: &str) -> Option<u32> {
    u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok()
}

fn parse_hex_u64(s: &str) -> Option<u64> {
    u64::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok()
}

/// Open the throwaway write-probe window on `app`.
pub(crate) fn open_spike_window(state: &Rc<RefCell<State>>, app: &gtk::Application) {
    use nerust_gui_shell::session::access::FrontendSession as _;
    state.borrow_mut().pause();
    let (space, start, name) = match state.borrow().spike_rw_space() {
        Some(t) => t,
        None => {
            log::warn!("spike write (gtk): no ReadWrite space");
            return;
        }
    };
    let dump = state.borrow().spike_read(space, start);

    let window = gtk::ApplicationWindow::new(app);
    window.set_title(Some(&format!("Memory write (spike, GTK): {name}")));
    window.set_default_size(560, 520);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.set_margin_top(12);
    root.set_margin_bottom(12);
    root.set_margin_start(12);
    root.set_margin_end(12);

    let label = gtk::Label::new(Some("SPIKE write probe (same shared text as tao)"));
    let scrolled = gtk::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    let text_view = gtk::TextView::new();
    text_view.set_monospace(true);
    text_view.set_editable(false);
    text_view.buffer().set_text(&dump);
    scrolled.set_child(Some(&text_view));

    let grid = gtk::Grid::new();
    grid.set_column_spacing(8);
    let addr_entry = gtk::Entry::new();
    // 0x0010 sits in the second visible row (16 rows cover 0x0000-0x00FF).
    addr_entry.set_text("0010");
    let value_entry = gtk::Entry::new();
    value_entry.set_text("AB");
    let width_entry = gtk::Entry::new();
    width_entry.set_text("1");
    grid.attach(&gtk::Label::new(Some("addr")), 0, 0, 1, 1);
    grid.attach(&addr_entry, 1, 0, 1, 1);
    grid.attach(&gtk::Label::new(Some("value")), 2, 0, 1, 1);
    grid.attach(&value_entry, 3, 0, 1, 1);
    grid.attach(&gtk::Label::new(Some("width")), 4, 0, 1, 1);
    grid.attach(&width_entry, 5, 0, 1, 1);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let write_button = gtk::Button::with_label("Write");
    let refresh_button = gtk::Button::with_label("Refresh");
    buttons.append(&write_button);
    buttons.append(&refresh_button);

    let confirm_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let confirm_label = gtk::Label::new(None);
    let confirm_button = gtk::Button::with_label("Confirm");
    let cancel_button = gtk::Button::with_label("Cancel");
    confirm_box.append(&confirm_label);
    confirm_box.append(&confirm_button);
    confirm_box.append(&cancel_button);
    confirm_box.set_visible(false);

    let result_label = gtk::Label::new(Some("paused"));
    result_label.set_selectable(true);

    root.append(&label);
    root.append(&scrolled);
    root.append(&grid);
    root.append(&buttons);
    root.append(&confirm_box);
    root.append(&result_label);
    window.set_child(Some(&root));

    let pending: Rc<RefCell<Option<(u32, u8, u64)>>> = Rc::new(RefCell::new(None));
    {
        let pending = pending.clone();
        let confirm_box = confirm_box.clone();
        let confirm_label = confirm_label.clone();
        let result_label = result_label.clone();
        write_button.connect_clicked(move |_| {
            let addr = parse_hex_u32(&addr_entry.text());
            // Width passes through unchecked: `BadWidth` refusal is a
            // failure-display probe, so the core (not the client) rejects it.
            let width = parse_hex_u64(&width_entry.text()).and_then(|w| u8::try_from(w).ok());
            let value = parse_hex_u64(&value_entry.text());
            match (addr, width, value) {
                (Some(addr), Some(width), Some(value)) => {
                    *pending.borrow_mut() = Some((addr, width, value));
                    confirm_label
                        .set_text(&format!("Write {value:02X} to {addr:04X} width {width}?"));
                    confirm_box.set_visible(true);
                }
                (_, _, _) => result_label.set_text("parse failed: addr/width/value must be hex"),
            }
        });
    }
    {
        let pending = pending.clone();
        let confirm_box = confirm_box.clone();
        cancel_button.connect_clicked(move |_| {
            *pending.borrow_mut() = None;
            confirm_box.set_visible(false);
        });
    }
    {
        let state = state.clone();
        let text_view = text_view.clone();
        let result_label = result_label.clone();
        let confirm_box = confirm_box.clone();
        let pending = pending.clone();
        confirm_button.connect_clicked(move |_| {
            let Some((addr, width, value)) = pending.borrow_mut().take() else {
                return;
            };
            confirm_box.set_visible(false);
            let line = state.borrow().spike_write(space, addr, width, value);
            result_label.set_text(&line);
            let dump = state.borrow().spike_read(space, start);
            text_view.buffer().set_text(&dump);
        });
    }
    {
        let state = state.clone();
        let text_view = text_view.clone();
        refresh_button.connect_clicked(move |_| {
            let dump = state.borrow().spike_read(space, start);
            text_view.buffer().set_text(&dump);
        });
    }

    window.present();
}

impl State {
    /// SPIKE: first ReadWrite space as (id, range start, name).
    pub(crate) fn spike_rw_space(&self) -> Option<(SpaceId, u32, String)> {
        self.session
            .spike_memory_spaces()
            .into_iter()
            .find(|s| s.access == SpaceAccess::ReadWrite)
            .map(|s| {
                let start = *s.range.start();
                (s.id, start, s.name.to_string())
            })
    }

    /// SPIKE: shared-format dump text (same string as tao shows).
    pub(crate) fn spike_read(&self, space: SpaceId, addr: u32) -> String {
        self.session.spike_read_text(space, addr, 16)
    }

    /// SPIKE: confirmed write, result line for display.
    pub(crate) fn spike_write(&self, space: SpaceId, addr: u32, width: u8, value: u64) -> String {
        self.session.spike_write_memory(space, addr, width, value)
    }
}
