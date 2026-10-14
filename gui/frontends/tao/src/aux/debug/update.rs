//! Debugger message dispatch: one family per function plus the
//! shared action helpers. Pure bridge manipulation; the host drain
//! owns the emulator side.

use std::sync::atomic::Ordering;

use nerust_core_traits::debugger::{DEBUG_DUMP_PAGE_ROWS, DUMP_ROW_BYTES};
use nerust_gui_shell::debug::DebugAction;
use nerust_gui_viewmodel::debugger::{self as vm};
use nerust_settings_core::i18n::{self as ui_msg, UiText, text as ui_text};

use super::DebugMessage;
use crate::aux::bridge::{DebugBridge, lock_recovered};

/// Bookmark cap (window-local, no file yet).
const BOOKMARK_CAP: usize = 8;

/// First handler wins; the chain only routes. Messages travel by
/// value so payloads move without cloning; an unhandled variant is a
/// programmer error (every family ends in `Err`), surfaced as a
/// warning instead of a silent drop.
pub(super) fn update_message(bridge: &DebugBridge, message: DebugMessage) {
    let message = match update_toolbar(bridge, message) {
        Ok(()) => return,
        Err(message) => message,
    };
    let message = match update_memory(bridge, message) {
        Ok(()) => return,
        Err(message) => message,
    };
    let message = match update_disasm(bridge, message) {
        Ok(()) => return,
        Err(message) => message,
    };
    let message = match update_bookmarks(bridge, message) {
        Ok(()) => return,
        Err(message) => message,
    };
    let message = match update_watch_freeze(bridge, message) {
        Ok(()) => return,
        Err(message) => message,
    };
    let message = match update_staged_write(bridge, message) {
        Ok(()) => return,
        Err(message) => message,
    };
    let Err(unhandled) = update_registers(bridge, message) else {
        return;
    };
    log::warn!("unhandled debugger message: {unhandled:?}");
}

/// Message dispatch, one family per function so `update` stays a
/// flat router. Each returns whether it handled the message; bodies
/// are moved verbatim from the old monolithic match.
fn update_toolbar(bridge: &DebugBridge, message: DebugMessage) -> Result<(), DebugMessage> {
    match message {
        DebugMessage::RefreshPressed => bridge.push(DebugAction::Refresh),
        DebugMessage::SpacePrev => cycle_space(bridge, -1),
        DebugMessage::SpaceNext => cycle_space(bridge, 1),
        DebugMessage::TogglePausePressed => bridge.push(DebugAction::TogglePause),
        DebugMessage::StepFramePressed => bridge.push(DebugAction::StepFrame),
        DebugMessage::StepInstrPressed => bridge.push(DebugAction::StepInstr),
        DebugMessage::PpuOpenPressed => {
            bridge.open_ppu.store(true, Ordering::Release);
            bridge.push(DebugAction::Refresh);
        }
        _ => return Err(message),
    }
    Ok(())
}

fn update_memory(bridge: &DebugBridge, message: DebugMessage) -> Result<(), DebugMessage> {
    match message {
        DebugMessage::MemInputChanged(text) => {
            lock_recovered(&bridge.nav).set_mem_input(text);
            bridge.view_invalidated.store(true, Ordering::Release);
        }
        DebugMessage::MemGo => mem_go(bridge),
        DebugMessage::MemPage(delta) => mem_page(bridge, delta),
        DebugMessage::RowSelected(addr) => {
            bridge.push(DebugAction::SelectRow(addr));
        }
        _ => return Err(message),
    }
    Ok(())
}

fn update_disasm(bridge: &DebugBridge, message: DebugMessage) -> Result<(), DebugMessage> {
    match message {
        DebugMessage::DisInputChanged(text) => {
            lock_recovered(&bridge.nav).set_dis_input(text);
            bridge.view_invalidated.store(true, Ordering::Release);
        }
        DebugMessage::DisGo => dis_go(bridge),
        DebugMessage::FollowPcToggle => {
            lock_recovered(&bridge.nav).follow();
            bridge.push(DebugAction::Refresh);
        }
        DebugMessage::DisLineClicked(addr) => navigate_to(bridge, addr),
        DebugMessage::FollowTarget(addr) => navigate_to(bridge, addr),
        DebugMessage::DisBack => dis_back(bridge),
        _ => return Err(message),
    }
    Ok(())
}

fn update_bookmarks(bridge: &DebugBridge, message: DebugMessage) -> Result<(), DebugMessage> {
    match message {
        DebugMessage::BookmarkInputChanged(text) => {
            *lock_recovered(&bridge.bm_input) = text;
            bridge.view_invalidated.store(true, Ordering::Release);
        }
        DebugMessage::BookmarkAdd => bookmark_add(bridge),
        DebugMessage::BookmarkJump(addr) => navigate_to(bridge, addr),
        DebugMessage::BookmarkDelete(addr) => {
            lock_recovered(&bridge.bookmarks).retain(|(a, _)| *a != addr);
            bridge.view_invalidated.store(true, Ordering::Release);
        }
        _ => return Err(message),
    }
    Ok(())
}

fn update_watch_freeze(bridge: &DebugBridge, message: DebugMessage) -> Result<(), DebugMessage> {
    match message {
        DebugMessage::WatchSet => watch_set(bridge),
        DebugMessage::WatchClear => {
            *lock_recovered(&bridge.watch) = None;
            lock_recovered(&bridge.display).watch_value = None;
            bridge.view_invalidated.store(true, Ordering::Release);
        }
        DebugMessage::FreezeInputChanged(text) => {
            *lock_recovered(&bridge.freeze_input) = text;
            bridge.view_invalidated.store(true, Ordering::Release);
        }
        DebugMessage::FreezeSet => {
            let input = lock_recovered(&bridge.freeze_input).clone();
            apply_freeze(bridge, &input);
        }
        DebugMessage::EditFreezeSet => {
            let input = lock_recovered(&bridge.edit_input).trim().to_string();
            apply_freeze(bridge, &input);
        }
        DebugMessage::FreezeClear => {
            *lock_recovered(&bridge.freeze) = None;
            bridge.set_status(ui_text(bridge.language, UiText::FreezeCleared).to_string());
        }
        _ => return Err(message),
    }
    Ok(())
}

fn update_staged_write(bridge: &DebugBridge, message: DebugMessage) -> Result<(), DebugMessage> {
    match message {
        DebugMessage::EditInputChanged(text) => {
            *lock_recovered(&bridge.edit_input) = text;
            bridge.view_invalidated.store(true, Ordering::Release);
        }
        DebugMessage::EditWrite => edit_write(bridge),
        DebugMessage::ConfirmWrite => {
            bridge.push(DebugAction::WriteConfirm);
            // Commit consumes the transaction; the input clears.
            *lock_recovered(&bridge.edit_input) = String::new();
        }
        DebugMessage::CancelWrite => {
            bridge.push(DebugAction::EditCancel);
            lock_recovered(&bridge.display).pending = None;
        }
        _ => return Err(message),
    }
    Ok(())
}

fn update_registers(bridge: &DebugBridge, message: DebugMessage) -> Result<(), DebugMessage> {
    match message {
        DebugMessage::RegSelected(name) => {
            *lock_recovered(&bridge.reg_target) = Some(name);
            bridge.view_invalidated.store(true, Ordering::Release);
        }
        DebugMessage::RegInputChanged(text) => {
            *lock_recovered(&bridge.reg_input) = text;
            bridge.view_invalidated.store(true, Ordering::Release);
        }
        DebugMessage::RegSet => reg_set(bridge),
        _ => return Err(message),
    }
    Ok(())
}

/// Shared freeze staging: selection plus a hex byte. The top
/// Freeze row and the edit-row Freeze button differ only in which
/// input they read; the rules live here, not in the widgets.
fn apply_freeze(bridge: &DebugBridge, input: &str) {
    let language = bridge.language;
    let selected = lock_recovered(&bridge.selected).map(|(addr, _)| addr);
    match (selected, vm::parse_hex_addr(input).filter(|v| *v <= 0xFF)) {
        (Some(addr), Some(value)) => {
            *lock_recovered(&bridge.freeze) = Some((addr, value as u8));
            bridge.set_status(ui_msg::frozen(language, addr, value as u8));
            bridge.push(DebugAction::Refresh);
        }
        _ => bridge.set_status(ui_text(language, UiText::FreezeNeedsSelection).to_string()),
    }
}

// Space stepper (not a dropdown): overlay menus are undrivable on
// headless Xvfb, and fixed spaces suit Prev/Next cycling. The
// dropdown decision is revisited with evidence in the design notes.
fn cycle_space(bridge: &DebugBridge, delta: i32) {
    let mut nav = lock_recovered(&bridge.nav);
    if let Some(idx) = nav.cycle_space(delta) {
        let start = bridge.spaces[idx].2;
        nav.set_space(idx, start);
        drop(nav);
        *lock_recovered(&bridge.selected) = None;
        bridge.push(DebugAction::Refresh);
    }
}

fn mem_go(bridge: &DebugBridge) {
    let language = bridge.language;
    let input = lock_recovered(&bridge.nav).mem_input().to_string();
    match vm::parse_hex_addr(&input) {
        Some(addr) => {
            lock_recovered(&bridge.nav).go_mem(addr);
            bridge.push(DebugAction::MemNav);
        }
        None => bridge.set_status(ui_msg::parse_failed(language, &input)),
    }
}

fn mem_page(bridge: &DebugBridge, delta: i32) {
    lock_recovered(&bridge.nav)
        .page_mem(delta * DEBUG_DUMP_PAGE_ROWS as i32 * DUMP_ROW_BYTES as i32);
    bridge.push(DebugAction::MemNav);
}

fn dis_go(bridge: &DebugBridge) {
    let language = bridge.language;
    let input = lock_recovered(&bridge.nav).dis_input().to_string();
    match vm::parse_hex_addr(&input) {
        Some(addr) => navigate_to(bridge, addr),
        None => bridge.set_status(ui_msg::parse_failed(language, &input)),
    }
}

fn navigate_to(bridge: &DebugBridge, addr: u32) {
    let anchor = bridge.anchor();
    lock_recovered(&bridge.nav).navigate(anchor, addr);
    bridge.push(DebugAction::Refresh);
}

fn dis_back(bridge: &DebugBridge) {
    match lock_recovered(&bridge.nav).go_back() {
        Some(_) => bridge.push(DebugAction::Refresh),
        None => bridge.set_status(ui_text(bridge.language, UiText::BackEmpty).to_string()),
    }
}

fn bookmark_add(bridge: &DebugBridge) {
    let label = lock_recovered(&bridge.bm_input).trim().to_string();
    // Anchor on the PC row when following, else the pin. The PC
    // row is snapshotted first and the guard dropped before
    // `anchor()` runs: it locks nav→display, so nesting it under
    // the display guard would invert the order and risk deadlock.
    let pc_addr = {
        lock_recovered(&bridge.display)
            .disasm_lines
            .iter()
            .find(|line| line.is_pc)
            .map(|line| line.addr)
    };
    let addr = pc_addr.unwrap_or_else(|| bridge.anchor());
    let mut marks = lock_recovered(&bridge.bookmarks);
    if marks.len() >= BOOKMARK_CAP {
        bridge.set_status(ui_text(bridge.language, UiText::BookmarkFull).to_string());
        return;
    }
    let n = marks.len() + 1;
    marks.push((
        addr,
        if label.is_empty() {
            format!("BM{n}")
        } else {
            label
        },
    ));
    lock_recovered(&bridge.bm_input).clear();
    bridge.view_invalidated.store(true, Ordering::Release);
}

fn watch_set(bridge: &DebugBridge) {
    let selected = lock_recovered(&bridge.selected).map(|(addr, _)| addr);
    match selected {
        Some(addr) => {
            *lock_recovered(&bridge.watch) = Some(addr);
            bridge.push(DebugAction::Refresh);
        }
        None => {
            bridge.set_status(ui_text(bridge.language, UiText::WatchNeedsSelection).to_string())
        }
    }
}

fn edit_write(bridge: &DebugBridge) {
    let input = lock_recovered(&bridge.edit_input).trim().to_string();
    let selected = *lock_recovered(&bridge.selected);
    match (
        selected,
        vm::parse_hex_addr(&input).filter(|value| *value <= 0xFF),
    ) {
        (Some((addr, _)), Some(value)) => {
            // Re-select first: the drain prepares fresh state
            // (old byte) before staging, so Write also works
            // after a cancel (which empties the transaction).
            // The confirm row comes back authoritatively from
            // the drain (same as GTK): no optimistic text.
            bridge.push(DebugAction::SelectRow(addr));
            bridge.push(DebugAction::EditStage(value as u64));
        }
        (None, _) => {
            bridge.set_status(ui_text(bridge.language, UiText::WriteNeedsSelection).to_string())
        }
        _ => bridge.set_status(ui_msg::parse_failed(bridge.language, &input)),
    }
}

/// Register write: the target rides in bridge state (picked by row
/// click), the value parses as a full-width hex number; the backend
/// masks to the register width.
fn reg_set(bridge: &DebugBridge) {
    let input = lock_recovered(&bridge.reg_input).trim().to_string();
    let target = *lock_recovered(&bridge.reg_target);
    match (target, vm::parse_hex_addr(&input)) {
        (Some(_), Some(value)) => {
            bridge.push(DebugAction::RegisterWrite(u64::from(value)));
        }
        (None, _) => {
            bridge.set_status(ui_text(bridge.language, UiText::RegisterNeedsSelection).to_string())
        }
        _ => bridge.set_status(ui_msg::parse_failed(bridge.language, &input)),
    }
}
