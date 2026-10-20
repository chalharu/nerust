//! Debugger message dispatch: one family per function plus the
//! shared action helpers. Pure bridge manipulation; the host drain
//! owns the emulator side.

use std::sync::atomic::Ordering;

use nerust_core_traits::debugger::{DEBUG_DUMP_PAGE_ROWS, DUMP_ROW_BYTES};
use nerust_gui_shell::debug::DebugAction;
use nerust_gui_viewmodel::debugger::{self as vm};
use nerust_settings_core::i18n::{self as ui_msg, UiText, text as ui_text};

use super::DebugMessage;
use crate::aux::bridge::DebugBridge;

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
            bridge.write().nav.set_mem_input(text);
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
            bridge.write().nav.set_dis_input(text);
            bridge.view_invalidated.store(true, Ordering::Release);
        }
        DebugMessage::DisGo => dis_go(bridge),
        DebugMessage::FollowPcToggle => {
            bridge.write().nav.follow();
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
            bridge.write().bm_input = text;
            bridge.view_invalidated.store(true, Ordering::Release);
        }
        DebugMessage::BookmarkAdd => bookmark_add(bridge),
        DebugMessage::BookmarkJump(addr) => navigate_to(bridge, addr),
        DebugMessage::BookmarkDelete(addr) => {
            bridge.write().bookmarks.retain(|(a, _)| *a != addr);
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
            bridge.write().watch = None;
            bridge.write().display.watch_value = None;
            bridge.view_invalidated.store(true, Ordering::Release);
        }
        DebugMessage::FreezeInputChanged(text) => {
            bridge.write().freeze_input = text;
            bridge.view_invalidated.store(true, Ordering::Release);
        }
        DebugMessage::FreezeSet => {
            let input = bridge.write().freeze_input.clone();
            apply_freeze(bridge, &input);
        }
        DebugMessage::EditFreezeSet => {
            let input = bridge.write().edit_input.trim().to_string();
            apply_freeze(bridge, &input);
        }
        DebugMessage::FreezeClear => {
            bridge.write().freeze = None;
            bridge.set_status(ui_text(bridge.language(), UiText::FreezeCleared).to_string());
        }
        _ => return Err(message),
    }
    Ok(())
}

fn update_staged_write(bridge: &DebugBridge, message: DebugMessage) -> Result<(), DebugMessage> {
    match message {
        DebugMessage::EditInputChanged(text) => {
            bridge.write().edit_input = text;
            bridge.view_invalidated.store(true, Ordering::Release);
        }
        DebugMessage::EditWrite => edit_write(bridge),
        DebugMessage::ConfirmWrite => {
            bridge.push(DebugAction::WriteConfirm);
            // Commit consumes the transaction; the input clears.
            bridge.write().edit_input = String::new();
        }
        DebugMessage::CancelWrite => {
            bridge.push(DebugAction::EditCancel);
            bridge.write().display.pending = None;
        }
        _ => return Err(message),
    }
    Ok(())
}

fn update_registers(bridge: &DebugBridge, message: DebugMessage) -> Result<(), DebugMessage> {
    match message {
        DebugMessage::RegSelected(name) => {
            bridge.write().reg_target = Some(name);
            bridge.view_invalidated.store(true, Ordering::Release);
        }
        DebugMessage::RegInputChanged(text) => {
            bridge.write().reg_input = text;
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
    let language = bridge.language();
    let selected = bridge.write().selected.map(|(addr, _)| addr);
    match (selected, vm::parse_hex_addr(input).filter(|v| *v <= 0xFF)) {
        (Some(addr), Some(value)) => {
            bridge.write().freeze = Some((addr, value as u8));
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
    let mut state = bridge.write();
    if let Some(idx) = state.nav.cycle_space(delta) {
        let start = bridge.spaces()[idx].2;
        state.nav.set_space(idx, start);
        state.selected = None;
        drop(state);
        bridge.push(DebugAction::Refresh);
    }
}

fn mem_go(bridge: &DebugBridge) {
    let language = bridge.language();
    let input = bridge.write().nav.mem_input().to_string();
    match vm::parse_hex_addr(&input) {
        Some(addr) => {
            bridge.write().nav.go_mem(addr);
            bridge.push(DebugAction::MemNav);
        }
        None => bridge.set_status(ui_msg::parse_failed(language, &input)),
    }
}

fn mem_page(bridge: &DebugBridge, delta: i32) {
    bridge
        .write()
        .nav
        .page_mem(delta * DEBUG_DUMP_PAGE_ROWS as i32 * DUMP_ROW_BYTES as i32);
    bridge.push(DebugAction::MemNav);
}

fn dis_go(bridge: &DebugBridge) {
    let language = bridge.language();
    let input = bridge.write().nav.dis_input().to_string();
    match vm::parse_hex_addr(&input) {
        Some(addr) => navigate_to(bridge, addr),
        None => bridge.set_status(ui_msg::parse_failed(language, &input)),
    }
}

fn navigate_to(bridge: &DebugBridge, addr: u32) {
    let anchor = bridge.anchor();
    bridge.write().nav.navigate(anchor, addr);
    bridge.push(DebugAction::Refresh);
}

fn dis_back(bridge: &DebugBridge) {
    // Bind before branching: the guard drops at the semicolon, so
    // the arms below never re-lock the state they just released.
    let popped = bridge.write().nav.go_back();
    if popped.is_some() {
        bridge.push(DebugAction::Refresh);
    } else {
        bridge.set_status(ui_text(bridge.language(), UiText::BackEmpty).to_string());
    }
}

fn bookmark_add(bridge: &DebugBridge) {
    let label = bridge.write().bm_input.trim().to_string();
    // Anchor on the PC row when following, else the pin. The PC
    // row is snapshotted first and the guard dropped before
    // `anchor()` runs: it locks nav→display, so nesting it under
    // the display guard would invert the order and risk deadlock.
    let pc_addr = {
        bridge
            .write()
            .display
            .disasm_lines
            .iter()
            .find(|line| line.is_pc)
            .map(|line| line.addr)
    };
    let addr = pc_addr.unwrap_or_else(|| bridge.anchor());
    let full = {
        let mut state = bridge.write();
        if state.bookmarks.len() >= BOOKMARK_CAP {
            true
        } else {
            let n = state.bookmarks.len() + 1;
            state.bookmarks.push((
                addr,
                if label.is_empty() {
                    format!("BM{n}")
                } else {
                    label
                },
            ));
            state.bm_input.clear();
            false
        }
    };
    if full {
        bridge.set_status(ui_text(bridge.language(), UiText::BookmarkFull).to_string());
        return;
    }
    bridge.view_invalidated.store(true, Ordering::Release);
}

fn watch_set(bridge: &DebugBridge) {
    let selected = bridge.write().selected.map(|(addr, _)| addr);
    match selected {
        Some(addr) => {
            bridge.write().watch = Some(addr);
            bridge.push(DebugAction::Refresh);
        }
        None => {
            bridge.set_status(ui_text(bridge.language(), UiText::WatchNeedsSelection).to_string())
        }
    }
}

fn edit_write(bridge: &DebugBridge) {
    let input = bridge.write().edit_input.trim().to_string();
    let selected = bridge.write().selected;
    match (
        selected,
        vm::parse_hex_addr(&input).filter(|value| *value <= 0xFF),
    ) {
        (Some((addr, _)), Some(value)) => {
            // Re-select first: the drain stages fresh state, so Write
            // also works after a cancel empties the transaction.
            // The confirm row comes back from the drain, never
            // optimistically from here.
            bridge.push(DebugAction::SelectRow(addr));
            bridge.push(DebugAction::EditStage(value as u64));
        }
        (None, _) => {
            bridge.set_status(ui_text(bridge.language(), UiText::WriteNeedsSelection).to_string())
        }
        _ => bridge.set_status(ui_msg::parse_failed(bridge.language(), &input)),
    }
}

/// Register write: the target rides in bridge state (picked by row
/// click), the value parses as a full-width hex number; the backend
/// masks to the register width.
fn reg_set(bridge: &DebugBridge) {
    let input = bridge.write().reg_input.trim().to_string();
    let target = bridge.write().reg_target;
    match (target, vm::parse_hex_addr(&input)) {
        (Some(_), Some(value)) => {
            bridge.push(DebugAction::RegisterWrite(u64::from(value)));
        }
        (None, _) => bridge
            .set_status(ui_text(bridge.language(), UiText::RegisterNeedsSelection).to_string()),
        _ => bridge.set_status(ui_msg::parse_failed(bridge.language(), &input)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_core_traits::debugger::SpaceId;
    use nerust_gui_viewmodel::debugger::{DisplayCache, PendingWrite, StatusMessage};
    use nerust_settings_core::model::language::AppLanguage;

    fn test_bridge() -> DebugBridge {
        DebugBridge::new(
            DisplayCache::default(),
            vec![(SpaceId(0), "Memory".to_string(), 0)],
            0,
            AppLanguage::English,
        )
    }

    fn status_text(bridge: &DebugBridge) -> String {
        match &bridge.write().display.status {
            StatusMessage::Local(text) => text.clone(),
            other => panic!("expected local status, got {other:?}"),
        }
    }

    #[test]
    fn confirm_write_pushes_write_confirm_and_clears_input() {
        let bridge = test_bridge();
        bridge.write().edit_input = "42".to_string();
        update_message(&bridge, DebugMessage::ConfirmWrite);
        assert!(matches!(
            bridge.take_requests().as_slice(),
            [DebugAction::WriteConfirm]
        ));
        assert!(bridge.write().edit_input.is_empty());
    }

    #[test]
    fn cancel_write_pushes_edit_cancel_and_clears_pending() {
        let bridge = test_bridge();
        bridge.write().display.pending = Some(PendingWrite {
            addr: 0x100,
            old: Some(0xAB),
            value: 0x42,
        });
        update_message(&bridge, DebugMessage::CancelWrite);
        assert!(matches!(
            bridge.take_requests().as_slice(),
            [DebugAction::EditCancel]
        ));
        assert!(bridge.write().display.pending.is_none());
    }

    #[test]
    fn bookmark_add_defaults_label_and_falls_back_to_pin() {
        let bridge = test_bridge();
        update_message(&bridge, DebugMessage::BookmarkAdd);
        let marks = bridge.write().bookmarks.clone();
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].0, bridge.anchor());
        assert_eq!(marks[0].1, "BM1");
        assert!(bridge.write().bm_input.is_empty());
    }

    #[test]
    fn freeze_rejects_unparseable_byte_with_selection() {
        let bridge = test_bridge();
        bridge.write().selected = Some((0x100, Some(0xAB)));
        bridge.write().freeze_input = "ZZZ".to_string();
        update_message(&bridge, DebugMessage::FreezeSet);
        assert!(bridge.write().freeze.is_none());
        assert_eq!(
            status_text(&bridge),
            "freeze needs a selected row and hex byte"
        );
    }

    #[test]
    fn freeze_rejects_out_of_range_byte() {
        let bridge = test_bridge();
        bridge.write().selected = Some((0x100, Some(0xAB)));
        bridge.write().freeze_input = "1FF".to_string();
        update_message(&bridge, DebugMessage::FreezeSet);
        assert!(bridge.write().freeze.is_none());
        assert_eq!(
            status_text(&bridge),
            "freeze needs a selected row and hex byte"
        );
    }

    #[test]
    fn edit_write_rejects_bad_hex_without_queueing() {
        let bridge = test_bridge();
        bridge.write().selected = Some((0x100, Some(0xAB)));
        bridge.write().edit_input = "ZZZ".to_string();
        update_message(&bridge, DebugMessage::EditWrite);
        assert!(bridge.take_requests().is_empty());
        assert_eq!(status_text(&bridge), "parse failed: ZZZ");
    }

    #[test]
    fn cycle_space_without_spaces_pushes_nothing() {
        let bridge = DebugBridge::new(DisplayCache::default(), Vec::new(), 0, AppLanguage::English);
        bridge.write().selected = Some((0x100, Some(0xAB)));
        update_message(&bridge, DebugMessage::SpaceNext);
        update_message(&bridge, DebugMessage::SpacePrev);
        assert!(bridge.take_requests().is_empty());
        assert!(bridge.write().selected.is_some());
    }

    #[test]
    fn reg_set_accepts_full_width_hex() {
        let bridge = test_bridge();
        bridge.write().reg_target = Some("r0");
        bridge.write().reg_input = "1FF".to_string();
        update_message(&bridge, DebugMessage::RegSet);
        assert!(matches!(
            bridge.take_requests().as_slice(),
            [DebugAction::RegisterWrite(0x1FF)]
        ));
    }
}
