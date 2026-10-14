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
    *lock_recovered(&bridge.selected) = Some((0x0200_0010, Some(0x00)));
    *lock_recovered(&bridge.freeze_input) = "FF".to_string();
    *lock_recovered(&bridge.edit_input) = "AA".to_string();
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
    // Typing must stay on the UI thread: input-echo messages
    // re-render locally and never queue emu-thread round-trips
    // (a drain would block typing on the thread reply).
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
        lock_recovered(&bridge.outbox).is_empty(),
        "typing never blocks on the emu thread"
    );
    assert!(bridge.view_invalidated.load(Ordering::Acquire));
}

#[test]
fn edit_freeze_set_reads_edit_input() {
    let bridge = freeze_bridge();
    drive(&bridge, DebugMessage::EditFreezeSet);
    assert_eq!(*lock_recovered(&bridge.freeze), Some((0x0200_0010, 0xAA)));
    assert_eq!(
        lock_recovered(&bridge.display).status,
        StatusMessage::Local("frozen 0x02000010=AA".to_string())
    );
}

#[test]
fn freeze_set_reads_top_input() {
    let bridge = freeze_bridge();
    drive(&bridge, DebugMessage::FreezeSet);
    assert_eq!(*lock_recovered(&bridge.freeze), Some((0x0200_0010, 0xFF)));
    assert_eq!(
        lock_recovered(&bridge.display).status,
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
    *lock_recovered(&bridge.selected) = Some((0x100, Some(0xAB)));
    *lock_recovered(&bridge.edit_input) = "42".to_string();
    *lock_recovered(&bridge.reg_input) = "7".to_string();
    *lock_recovered(&bridge.freeze_input) = "FF".to_string();
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
    assert!(!lock_recovered(&bridge.outbox).is_empty());
    // Space stepper with one space cycles and clears selection.
    drive(&bridge, DebugMessage::SpaceNext);
    assert!(lock_recovered(&bridge.selected).is_none());
    drive(&bridge, DebugMessage::SpacePrev);
    // Memory Go resolves valid hex, rejects the rest.
    drive(&bridge, DebugMessage::MemInputChanged("100".to_string()));
    drive(&bridge, DebugMessage::MemGo);
    assert_eq!(lock_recovered(&bridge.nav).mem_addr(), 0x100);
    drive(&bridge, DebugMessage::MemInputChanged("zz".to_string()));
    drive(&bridge, DebugMessage::MemGo);
    assert_eq!(
        lock_recovered(&bridge.display).status,
        StatusMessage::Local("parse failed: zz".to_string())
    );
}

#[test]
fn update_drives_disassembly_pin_and_back() {
    let bridge = full_bridge();
    // Line click pins the anchor and leaves follow mode.
    drive(&bridge, DebugMessage::DisLineClicked(0x300));
    assert!(!lock_recovered(&bridge.nav).follow_pc());
    drive(&bridge, DebugMessage::FollowTarget(0x308));
    // Back pops twice, then reports the empty stack.
    drive(&bridge, DebugMessage::DisBack);
    drive(&bridge, DebugMessage::DisBack);
    drive(&bridge, DebugMessage::DisBack);
    assert_eq!(
        lock_recovered(&bridge.display).status,
        StatusMessage::Local("back stack is empty".to_string())
    );
    // Go resolves valid hex, rejects the rest.
    drive(&bridge, DebugMessage::DisInputChanged("200".to_string()));
    drive(&bridge, DebugMessage::DisGo);
    assert!(!lock_recovered(&bridge.nav).follow_pc());
    drive(&bridge, DebugMessage::DisInputChanged("zz".to_string()));
    drive(&bridge, DebugMessage::DisGo);
    assert_eq!(
        lock_recovered(&bridge.display).status,
        StatusMessage::Local("parse failed: zz".to_string())
    );
    drive(&bridge, DebugMessage::FollowPcToggle);
    assert!(lock_recovered(&bridge.nav).follow_pc());
}

#[test]
fn update_drives_bookmarks_watch_and_freeze() {
    let bridge = full_bridge();
    drive(
        &bridge,
        DebugMessage::BookmarkInputChanged("start".to_string()),
    );
    drive(&bridge, DebugMessage::BookmarkAdd);
    assert_eq!(lock_recovered(&bridge.bookmarks).len(), 1);
    // PC row anchors the bookmark while following.
    assert_eq!(lock_recovered(&bridge.bookmarks)[0].0, 0x0800_27D2);
    drive(&bridge, DebugMessage::BookmarkJump(0x0800_27D2));
    drive(&bridge, DebugMessage::BookmarkDelete(0x0800_27D2));
    assert!(lock_recovered(&bridge.bookmarks).is_empty());
    // A full list refuses loudly.
    lock_recovered(&bridge.bookmarks).extend((0..8).map(|i| (0x100 + i, format!("BM{i}"))));
    drive(&bridge, DebugMessage::BookmarkAdd);
    assert_eq!(
        lock_recovered(&bridge.display).status,
        StatusMessage::Local("bookmark list full".to_string())
    );
    // Watch follows the selection, then clears.
    drive(&bridge, DebugMessage::WatchSet);
    assert_eq!(*lock_recovered(&bridge.watch), Some(0x100));
    drive(&bridge, DebugMessage::WatchClear);
    assert!(lock_recovered(&bridge.watch).is_none());
    *lock_recovered(&bridge.selected) = None;
    drive(&bridge, DebugMessage::WatchSet);
    assert_eq!(
        lock_recovered(&bridge.display).status,
        StatusMessage::Local("watch needs a selection".to_string())
    );
    // Freeze reads the top input, the edit input, then clears.
    *lock_recovered(&bridge.selected) = Some((0x100, Some(0xAB)));
    drive(&bridge, DebugMessage::FreezeInputChanged("00".to_string()));
    drive(&bridge, DebugMessage::FreezeSet);
    assert_eq!(*lock_recovered(&bridge.freeze), Some((0x100, 0x00)));
    drive(&bridge, DebugMessage::EditFreezeSet);
    assert_eq!(*lock_recovered(&bridge.freeze), Some((0x100, 0x42)));
    drive(&bridge, DebugMessage::FreezeClear);
    assert!(lock_recovered(&bridge.freeze).is_none());
    assert_eq!(
        lock_recovered(&bridge.display).status,
        StatusMessage::Local("freeze cleared".to_string())
    );
    // Rejections stay loud.
    *lock_recovered(&bridge.selected) = None;
    drive(&bridge, DebugMessage::FreezeSet);
    assert_eq!(
        lock_recovered(&bridge.display).status,
        StatusMessage::Local("freeze needs a selected row and hex byte".to_string())
    );
}

#[test]
fn update_drives_write_and_register_flow() {
    let bridge = full_bridge();
    // EditWrite stages SelectRow plus EditStage in one batch.
    drive(&bridge, DebugMessage::EditInputChanged("42".to_string()));
    drive(&bridge, DebugMessage::EditWrite);
    assert_eq!(lock_recovered(&bridge.outbox).len(), 2);
    // Confirm consumes the transaction and clears the input.
    drive(&bridge, DebugMessage::ConfirmWrite);
    assert!(lock_recovered(&bridge.edit_input).is_empty());
    // Cancel clears the optimistic text.
    drive(&bridge, DebugMessage::CancelWrite);
    assert!(lock_recovered(&bridge.display).pending.is_none());
    // Register pick then Set pushes one register write.
    drive(&bridge, DebugMessage::RegSelected("r0"));
    assert_eq!(*lock_recovered(&bridge.reg_target), Some("r0"));
    drive(&bridge, DebugMessage::RegInputChanged("7".to_string()));
    drive(&bridge, DebugMessage::RegSet);
    assert_eq!(lock_recovered(&bridge.outbox).len(), 5);
    // Rejections stay loud.
    *lock_recovered(&bridge.reg_target) = None;
    drive(&bridge, DebugMessage::RegSet);
    assert_eq!(
        lock_recovered(&bridge.display).status,
        StatusMessage::Local("register write needs a register".to_string())
    );
    *lock_recovered(&bridge.selected) = None;
    drive(&bridge, DebugMessage::EditWrite);
    assert_eq!(
        lock_recovered(&bridge.display).status,
        StatusMessage::Local("write needs a selection".to_string())
    );
    drive(&bridge, DebugMessage::RegInputChanged("zz".to_string()));
    *lock_recovered(&bridge.reg_target) = Some("r0");
    drive(&bridge, DebugMessage::RegSet);
    assert_eq!(
        lock_recovered(&bridge.display).status,
        StatusMessage::Local("parse failed: zz".to_string())
    );
}

#[test]
fn view_builders_construct_every_section() {
    let bridge = full_bridge();
    let language = AppLanguage::English;
    let display = lock_recovered(&bridge.display).clone_without_images();
    let selected = *lock_recovered(&bridge.selected);
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

/// Headless UI-shell lifecycle test (build/pump/rebuild/relayout/
/// draw). Software rasterization only, so it runs without a display
/// and never contacts the emulator. Lives here to keep `shell`
/// free of aux-window imports.
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
    let mut shell = UiShell::new(instance, iced::window::Id::unique(), bounds, &mut backend);
    let mut clipboard = iced_winit::Clipboard::unconnected();
    let (messages, _) = shell.pump(
        iced::Event::Window(iced::window::Event::RedrawRequested(
            std::time::Instant::now(),
        )),
        iced::mouse::Cursor::default(),
        &mut clipboard,
        &mut backend,
    );
    assert!(messages.is_empty());
    shell.push_message(DebugMessage::RefreshPressed);
    let invalidated = AtomicBool::new(true);
    shell.process_messages(
        vec![DebugMessage::FollowPcToggle],
        iced::window::Id::unique(),
        bounds,
        &mut backend,
        &invalidated,
    );
    assert!(!invalidated.load(Ordering::Acquire));
    shell.relayout(iced::Size::new(900.0, 650.0), &mut backend);
    let theme = iced::Theme::Light;
    let style = <iced::Theme as iced::theme::Base>::base(&theme);
    let Some(ui) = shell.ui_mut() else {
        panic!("UiShell unavailable after relayout in test");
    };
    ui.draw(
        &mut backend,
        &theme,
        &iced::advanced::renderer::Style {
            text_color: style.text_color,
        },
        iced::mouse::Cursor::default(),
    );
}
