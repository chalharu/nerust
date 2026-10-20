use super::views::*;
use super::*;
use nerust_core_traits::debugger::HexRow;
use nerust_gui_viewmodel::debugger::{self as vm, StatusMessage};

fn freeze_bridge() -> Arc<DebugBridge> {
    let bridge = Arc::new(DebugBridge::new(
        DisplayCache::default(),
        Vec::new(),
        0,
        AppLanguage::English,
    ));
    bridge.write().selected = Some((0x0200_0010, Some(0x00)));
    bridge.write().freeze_input = "FF".to_string();
    bridge.write().edit_input = "AA".to_string();
    bridge
}

fn drive(bridge: &Arc<DebugBridge>, message: DebugMessage) {
    let program = DebugProgram {
        bridge: Arc::clone(bridge),
    };
    let mut state = DebugState {
        bridge: Arc::clone(bridge),
    };
    let _ = program.update(&mut state, message);
}

#[test]
fn input_echo_never_queues_drain_traffic() {
    // Input-echo messages re-render locally; never queue emu-thread round-trips.
    let bridge = full_bridge();
    for message in [
        DebugMessage::MemInputChanged("0x100".to_string()),
        DebugMessage::DisInputChanged("0x200".to_string()),
        DebugMessage::BookmarkInputChanged("tag".to_string()),
        DebugMessage::FreezeInputChanged("FF".to_string()),
        DebugMessage::EditInputChanged("42".to_string()),
        DebugMessage::RegInputChanged("7".to_string()),
        DebugMessage::RegSelected("r0"),
        DebugMessage::WatchClear,
        DebugMessage::BookmarkDelete(0x999),
    ] {
        drive(&bridge, message);
    }
    assert!(
        bridge.outbox_len() == 0,
        "typing never blocks on the emu thread"
    );
    assert!(bridge.view_invalidated.load(Ordering::Acquire));
}

#[test]
fn edit_freeze_set_reads_edit_input() {
    let bridge = freeze_bridge();
    drive(&bridge, DebugMessage::EditFreezeSet);
    assert_eq!(bridge.write().freeze, Some((0x0200_0010, 0xAA)));
    assert_eq!(
        bridge.write().display.status,
        StatusMessage::Local("frozen 0x02000010=AA".to_string())
    );
}

#[test]
fn freeze_set_reads_top_input() {
    let bridge = freeze_bridge();
    drive(&bridge, DebugMessage::FreezeSet);
    assert_eq!(bridge.write().freeze, Some((0x0200_0010, 0xFF)));
    assert_eq!(
        bridge.write().display.status,
        StatusMessage::Local("frozen 0x02000010=FF".to_string())
    );
}

fn full_bridge() -> Arc<DebugBridge> {
    use nerust_core_traits::debugger::DisasmLine;
    let display = DisplayCache {
        disasm_lines: vec![
            DisasmLine {
                addr: 0x0800_27D2,
                bytes: [0xF0, 0xBC, 0, 0],
                len: 2,
                text: "POP {r4, r5, r6, r7}".to_string(),
                is_pc: true,
                target: None,
            },
            DisasmLine {
                addr: 0x0800_27D2E,
                bytes: [0x02, 0xBC, 0, 0],
                len: 2,
                text: "POP {r1}".to_string(),
                is_pc: false,
                target: Some(0x0800_27CA8),
            },
        ],
        regs: vec![("r0", 1), ("r1", 2)],
        reg_names: vec!["r0", "r1"],
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
        ..DisplayCache::default()
    };
    let bridge = Arc::new(DebugBridge::new(
        display,
        vec![(SpaceId(0), "Memory".to_string(), 0)],
        0,
        AppLanguage::English,
    ));
    bridge.write().selected = Some((0x100, Some(0xAB)));
    bridge.write().edit_input = "42".to_string();
    bridge.write().reg_input = "7".to_string();
    bridge.write().freeze_input = "FF".to_string();
    bridge
}

#[test]
fn update_drives_toolbar_and_nav() {
    let bridge = full_bridge();
    for message in [
        DebugMessage::RefreshPressed,
        DebugMessage::TogglePausePressed,
        DebugMessage::StepFramePressed,
        DebugMessage::StepInstrPressed,
        DebugMessage::PpuOpenPressed,
        DebugMessage::MemPage(1),
        DebugMessage::MemPage(-1),
        DebugMessage::FollowPcToggle,
        DebugMessage::RowSelected(0x110),
    ] {
        drive(&bridge, message);
    }
    assert!(bridge.open_ppu.load(Ordering::Acquire));
    assert!(bridge.outbox_len() != 0);
    // Space stepper with one space cycles and clears selection.
    drive(&bridge, DebugMessage::SpaceNext);
    assert!(bridge.write().selected.is_none());
    drive(&bridge, DebugMessage::SpacePrev);
    // Memory Go resolves valid hex, rejects the rest.
    drive(&bridge, DebugMessage::MemInputChanged("100".to_string()));
    drive(&bridge, DebugMessage::MemGo);
    assert_eq!(bridge.write().nav.mem_addr(), 0x100);
    drive(&bridge, DebugMessage::MemInputChanged("zz".to_string()));
    drive(&bridge, DebugMessage::MemGo);
    assert_eq!(
        bridge.write().display.status,
        StatusMessage::Local("parse failed: zz".to_string())
    );
}

#[test]
fn update_drives_disassembly_pin_and_back() {
    let bridge = full_bridge();
    // Line click pins the anchor and leaves follow mode.
    drive(&bridge, DebugMessage::DisLineClicked(0x300));
    assert!(!bridge.write().nav.follow_pc());
    drive(&bridge, DebugMessage::FollowTarget(0x308));
    // Back pops twice, then reports the empty stack.
    drive(&bridge, DebugMessage::DisBack);
    drive(&bridge, DebugMessage::DisBack);
    drive(&bridge, DebugMessage::DisBack);
    assert_eq!(
        bridge.write().display.status,
        StatusMessage::Local("back stack is empty".to_string())
    );
    // Go resolves valid hex, rejects the rest.
    drive(&bridge, DebugMessage::DisInputChanged("200".to_string()));
    drive(&bridge, DebugMessage::DisGo);
    assert!(!bridge.write().nav.follow_pc());
    drive(&bridge, DebugMessage::DisInputChanged("zz".to_string()));
    drive(&bridge, DebugMessage::DisGo);
    assert_eq!(
        bridge.write().display.status,
        StatusMessage::Local("parse failed: zz".to_string())
    );
    drive(&bridge, DebugMessage::FollowPcToggle);
    assert!(bridge.write().nav.follow_pc());
}

#[test]
fn update_drives_bookmarks_watch_and_freeze() {
    let bridge = full_bridge();
    drive(
        &bridge,
        DebugMessage::BookmarkInputChanged("start".to_string()),
    );
    drive(&bridge, DebugMessage::BookmarkAdd);
    assert_eq!(bridge.write().bookmarks.len(), 1);
    // PC row anchors the bookmark while following.
    assert_eq!(bridge.write().bookmarks[0].0, 0x0800_27D2);
    drive(&bridge, DebugMessage::BookmarkJump(0x0800_27D2));
    drive(&bridge, DebugMessage::BookmarkDelete(0x0800_27D2));
    assert!(bridge.write().bookmarks.is_empty());
    // A full list refuses loudly.
    bridge
        .write()
        .bookmarks
        .extend((0..8).map(|i| (0x100 + i, format!("BM{i}"))));
    drive(&bridge, DebugMessage::BookmarkAdd);
    assert_eq!(
        bridge.write().display.status,
        StatusMessage::Local("bookmark list full".to_string())
    );
    // Watch follows the selection, then clears.
    drive(&bridge, DebugMessage::WatchSet);
    assert_eq!(bridge.write().watch, Some(0x100));
    drive(&bridge, DebugMessage::WatchClear);
    assert!(bridge.write().watch.is_none());
    bridge.write().selected = None;
    drive(&bridge, DebugMessage::WatchSet);
    assert_eq!(
        bridge.write().display.status,
        StatusMessage::Local("watch needs a selection".to_string())
    );
    // Freeze reads the top input, the edit input, then clears.
    bridge.write().selected = Some((0x100, Some(0xAB)));
    drive(&bridge, DebugMessage::FreezeInputChanged("00".to_string()));
    drive(&bridge, DebugMessage::FreezeSet);
    assert_eq!(bridge.write().freeze, Some((0x100, 0x00)));
    drive(&bridge, DebugMessage::EditFreezeSet);
    assert_eq!(bridge.write().freeze, Some((0x100, 0x42)));
    drive(&bridge, DebugMessage::FreezeClear);
    assert!(bridge.write().freeze.is_none());
    assert_eq!(
        bridge.write().display.status,
        StatusMessage::Local("freeze cleared".to_string())
    );
    // Rejections stay loud.
    bridge.write().selected = None;
    drive(&bridge, DebugMessage::FreezeSet);
    assert_eq!(
        bridge.write().display.status,
        StatusMessage::Local("freeze needs a selected row and hex byte".to_string())
    );
}

#[test]
fn update_drives_write_and_register_flow() {
    let bridge = full_bridge();
    // EditWrite stages SelectRow plus EditStage in one batch.
    drive(&bridge, DebugMessage::EditInputChanged("42".to_string()));
    drive(&bridge, DebugMessage::EditWrite);
    assert_eq!(bridge.outbox_len(), 2);
    // Confirm consumes the transaction and clears the input.
    drive(&bridge, DebugMessage::ConfirmWrite);
    assert!(bridge.write().edit_input.is_empty());
    // Cancel clears the optimistic text.
    drive(&bridge, DebugMessage::CancelWrite);
    assert!(bridge.write().display.pending.is_none());
    // Register pick then Set pushes one register write.
    drive(&bridge, DebugMessage::RegSelected("r0"));
    assert_eq!(bridge.write().reg_target, Some("r0"));
    drive(&bridge, DebugMessage::RegInputChanged("7".to_string()));
    drive(&bridge, DebugMessage::RegSet);
    assert_eq!(bridge.outbox_len(), 5);
    // Rejections stay loud.
    bridge.write().reg_target = None;
    drive(&bridge, DebugMessage::RegSet);
    assert_eq!(
        bridge.write().display.status,
        StatusMessage::Local("register write needs a register".to_string())
    );
    bridge.write().selected = None;
    drive(&bridge, DebugMessage::EditWrite);
    assert_eq!(
        bridge.write().display.status,
        StatusMessage::Local("write needs a selection".to_string())
    );
    drive(&bridge, DebugMessage::RegInputChanged("zz".to_string()));
    bridge.write().reg_target = Some("r0");
    drive(&bridge, DebugMessage::RegSet);
    assert_eq!(
        bridge.write().display.status,
        StatusMessage::Local("parse failed: zz".to_string())
    );
}

#[test]
fn view_builders_construct_every_section() {
    let bridge = full_bridge();
    let language = AppLanguage::English;
    let display = bridge.write().display.clone_without_images();
    let selected = bridge.write().selected;
    let _ = view_toolbar(true, language);
    let _ = view_toolbar(false, language);
    let _ = view_space_row("Memory", language);
    let _ = view_mem_nav("0x00000100", language);
    // Mid-row byte selection still lights its row.
    let _ = view_dump_col(&display.dump_rows, &display.diff, Some((0x102, Some(9))));
    let _ = view_dump_col(&display.dump_rows, &[], None);
    let _ = view_regs_section(&display.regs, Some("r0"), "7", language);
    let _ = view_regs_section(&display.regs, None, "", language);
    let _ = view_regs_section(&[], None, "", language);
    let _ = view_edit_row(selected, "42", language);
    let _ = view_edit_row(None, "", language);
    let _ = view_confirm_row(
        Some(vm::PendingWrite {
            addr: 0x100,
            old: Some(0xAB),
            value: 0x42,
        }),
        language,
    );
    let _ = view_confirm_row(None, language);
    let _ = view_dis_nav(true, 0, 0x0800_27D2, "", language);
    let _ = view_dis_nav(false, 2, 0x300, "0x00000300", language);
    let _ = view_dis_col(&[], None, language);
    let _ = view_dis_col(&display.disasm_lines, None, language);
    let _ = view_dis_col(&display.disasm_lines, Some(0x0800_27D2E), language);
    let _ = view_watch_row(Some(0x100), Some(0xAB), language);
    let _ = view_freeze_row(Some((0x100, 0xFF)), "FF", language);
    let _ = view_bookmark_section("tag", &[(0x200, "BM1".to_string())], language);
    let _ = view_watch_row(None, None, language);
    let _ = view_freeze_row(None, "", language);
    let _ = view_bookmark_section("", &[], language);
    // Disassembly lines exercise the target follow button.
    assert_eq!(display.disasm_lines.len(), 2);
}

#[test]
fn view_assembles_follow_and_fixed_modes() {
    let bridge = full_bridge();
    let program = DebugProgram {
        bridge: Arc::clone(&bridge),
    };
    let state = DebugState {
        bridge: Arc::clone(&bridge),
    };
    // Follow-PC mode anchors on the first visible line.
    let _follow = program.view(&state, iced::window::Id::unique());
    drop(_follow);
    // Fixed mode pins the anchor and shows the Back depth.
    drive(&bridge, DebugMessage::DisLineClicked(0x300));
    let _fixed = program.view(&state, iced::window::Id::unique());
    drop(_fixed);
}

#[test]
fn disasm_marker_never_double_marks_pc() {
    use nerust_core_traits::debugger::DisasmLine;

    let pc_line = DisasmLine {
        addr: 0xC293,
        bytes: [0x60, 0, 0, 0],
        len: 1,
        text: "RTS".to_string(),
        is_pc: true,
        target: None,
    };
    // The composed row carries exactly one `>`: the format owns
    // it, the view prefix contributes blanks.
    let composed = format!(
        "{}{}",
        disasm_marker(pc_line.is_pc, true),
        vm::format_disasm_line(&pc_line)
    );
    assert!(composed.starts_with("  >0x"), "got {composed:?}");
    // Pinned non-PC rows keep the `@` in the same column.
    assert_eq!(disasm_marker(false, true), "@ ");
    assert_eq!(disasm_marker(false, false), "  ");
    assert_eq!(disasm_marker(true, false), "  ");
}

#[test]
fn shortcut_and_cursor_keys_map_physical_codes() {
    use iced::keyboard::key::{Code, Physical};
    assert!(matches!(
        shortcut_request(&Physical::Code(Code::F6)),
        Some(DebugAction::StepFrame)
    ));
    assert!(matches!(
        shortcut_request(&Physical::Code(Code::F7)),
        Some(DebugAction::StepInstr)
    ));
    assert!(matches!(
        shortcut_request(&Physical::Code(Code::F9)),
        Some(DebugAction::TogglePause)
    ));
    assert!(shortcut_request(&Physical::Code(Code::KeyA)).is_none());
    assert!(matches!(
        cursor_page(&Physical::Code(Code::ArrowUp)),
        Some(DebugMessage::MemPage(-1))
    ));
    assert!(matches!(
        cursor_page(&Physical::Code(Code::ArrowDown)),
        Some(DebugMessage::MemPage(1))
    ));
    assert!(cursor_page(&Physical::Code(Code::ArrowLeft)).is_none());
}

/// Headless UI-shell lifecycle test (build/pump/rebuild/relayout/draw).
/// Software rasterization only: no display, no emulator contact.
#[test]
fn uishell_headless_build_pump_relayout_draw() {
    use std::sync::atomic::AtomicBool;

    use crate::aux::shell::UiShell;

    let bridge = Arc::new(DebugBridge::new(
        DisplayCache::default(),
        Vec::new(),
        0,
        AppLanguage::English,
    ));
    let (instance, _task) = program::Instance::new(DebugProgram {
        bridge: Arc::clone(&bridge),
    });
    let mut backend = Renderer::new(crate::convert::default_font(), iced::Pixels(16.0));
    let bounds = iced::Size::new(1100.0, 800.0);
    let mut shell = UiShell::new(instance, iced::window::Id::unique(), bounds);
    let mut clipboard = iced_winit::Clipboard::unconnected();
    let mut messages = Vec::new();
    let _ = shell.pump(
        iced::Event::Window(iced::window::Event::RedrawRequested(
            std::time::Instant::now(),
        )),
        iced::mouse::Cursor::default(),
        &mut clipboard,
        &mut backend,
        &mut messages,
    );
    assert!(messages.is_empty());
    shell.push_message(DebugMessage::RefreshPressed);
    let invalidated = AtomicBool::new(true);
    shell.process_messages(
        vec![DebugMessage::FollowPcToggle],
        &mut backend,
        &invalidated,
    );
    assert!(!invalidated.load(Ordering::Acquire));
    shell.relayout(iced::Size::new(900.0, 650.0));
    // Post-relayout pump rebuilds the transient interface from the
    // retained cache and folds back without orphans.
    messages.clear();
    let _ = shell.pump(
        iced::Event::Window(iced::window::Event::RedrawRequested(
            std::time::Instant::now(),
        )),
        iced::mouse::Cursor::default(),
        &mut clipboard,
        &mut backend,
        &mut messages,
    );
    assert!(messages.is_empty());
}

#[test]
fn capture_view_marks_coreless_as_paused_no_core() {
    use nerust_gui_viewmodel::debugger::StatusMessage;
    let bridge = Arc::new(DebugBridge::new(
        DisplayCache::default(),
        Vec::new(),
        0,
        AppLanguage::English,
    ));
    let snap = views::capture_view(&bridge);
    assert!(snap.paused);
    assert!(matches!(snap.status, StatusMessage::NoCore));
}

#[test]
fn capture_view_anchors_to_first_line_in_follow_mode() {
    // full_bridge starts in follow-PC mode (NavState default).
    let bridge = full_bridge();
    let snap = views::capture_view(&bridge);
    assert_eq!(snap.anchor, 0x0800_27D2);
}

#[test]
fn capture_view_anchors_to_pin_once_navigated() {
    let bridge = full_bridge();
    bridge.write().nav.navigate(0x0800_27D2, 0x0300);
    let snap = views::capture_view(&bridge);
    assert_eq!(snap.anchor, 0x0300);
    // Snapshot carries the window contents through untouched.
    assert_eq!(snap.dump_rows.len(), 2);
    assert_eq!(snap.diff, vec![0x100]);
    assert_eq!(snap.regs.len(), 2);
}

#[test]
fn program_boot_and_chrome_are_headless() {
    let bridge = full_bridge();
    let program = DebugProgram {
        bridge: Arc::clone(&bridge),
    };
    assert_eq!(DebugProgram::name(), "nerust_debug");
    assert!(program.window().is_none());
    let _ = program.settings();
    let (state, _task) = program.boot();
    let theme = program.theme(&state, iced::window::Id::unique());
    assert!(matches!(theme, Some(iced::Theme::Light)));
    let _ = program.view(&state, iced::window::Id::unique());
    drop(state);
}
