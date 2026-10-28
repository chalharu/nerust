use super::views::*;
use super::*;
use nerust_frontend_host::debugger::{
    DebugInputs, DebugSnapshot, DebuggerRows, DisasmRow, DumpRow, RegRow, pin_marker,
};

fn test_snapshot() -> DebugSnapshot {
    let rows = DebuggerRows {
        disasm: vec![
            DisasmRow {
                addr: 0x0800_27D2,
                is_pc: true,
                text: "POP {r4, r5, r6, r7}".to_string(),
                target: None,
            },
            DisasmRow {
                addr: 0x0800_27D2E,
                is_pc: false,
                text: "POP {r1}".to_string(),
                target: Some(0x0800_27CA8),
            },
        ],
        dump: vec![
            DumpRow {
                addr: 0x100,
                bytes: vec![0xAB; 16],
            },
            DumpRow {
                addr: 0x110,
                bytes: vec![1, 2, 3],
            },
        ],
        changed_addrs: vec![0x100],
        regs: vec![
            RegRow {
                name: "r0",
                text: "r0 00000001".to_string(),
            },
            RegRow {
                name: "r1",
                text: "r1 00000002".to_string(),
            },
        ],
        status_text: "Running".to_string(),
        paused: false,
        pending_text: None,
        watch_text: "watch 0x00000100".to_string(),
        freeze_text: "no freeze".to_string(),
        selected_line: "row 0x00000100".to_string(),
    };
    DebugSnapshot {
        chrome: chrome(),
        rows,
        inputs: DebugInputs {
            mem_input: "0x100".to_string(),
            dis_input: "0x200".to_string(),
            follow_pc: true,
            back_len: 0,
            dis_addr: Some(0x0800_27D2),
            mem_addr: Some(0x100),
            bm_input: String::new(),
            bookmarks: vec![(0x200, "BM1".to_string())],
            freeze_input: "FF".to_string(),
            edit_input: "42".to_string(),
            reg_input: "7".to_string(),
            watch: Some(0x100),
            watch_value: Some(0xAB),
            freeze: Some((0x100, 0xFF)),
        },
        selected: Some((0x100, Some(0xAB))),
        reg_target: Some("r0"),
        space_name: "Memory".to_string(),
        has_spaces: true,
    }
}

fn chrome() -> nerust_frontend_host::debugger::DebuggerChrome {
    nerust_frontend_host::debugger::DebuggerChrome {
        debugger: "Debugger".to_string(),
        pause: "Pause".to_string(),
        resume: "Resume".to_string(),
        step_frame: "Step Frame".to_string(),
        step_instr: "Step Instr".to_string(),
        ppu_viewer: "PPU".to_string(),
        refresh: "Refresh".to_string(),
        space: "Space".to_string(),
        prev: "Prev".to_string(),
        next: "Next".to_string(),
        page: "Page".to_string(),
        addr_hex: "Addr".to_string(),
        go: "Go".to_string(),
        back: "Back".to_string(),
        disassembly: "Disassembly".to_string(),
        follow_pc: "Follow".to_string(),
        follow_pc_tag: "(follow)".to_string(),
        fixed_tag: "(fixed)".to_string(),
        no_disassembly: "No disassembly".to_string(),
        disassembly_hint: "hint".to_string(),
        registers: "Registers".to_string(),
        pick_register: "Pick".to_string(),
        hex: "Hex".to_string(),
        set: "Set".to_string(),
        select_dump_row: "Select".to_string(),
        write: "Write".to_string(),
        hex_byte: "Byte".to_string(),
        watch: "Watch".to_string(),
        freeze: "Freeze".to_string(),
        confirm: "Confirm".to_string(),
        cancel: "Cancel".to_string(),
        watch_none: "none".to_string(),
        freeze_none: "none".to_string(),
        clear: "Clear".to_string(),
        bookmarks: "Bookmarks".to_string(),
        label: "Label".to_string(),
        add: "Add".to_string(),
        memory: "Memory".to_string(),
    }
}

fn drive(queue: &DebugQueue, view: &SharedView, message: DebugMessage) {
    let program = DebugProgram {
        view: Rc::clone(view),
        queue: Rc::clone(queue),
    };
    let mut state = DebugState {
        view: Rc::clone(view),
        queue: Rc::clone(queue),
    };
    let _ = program.update(&mut state, message);
}

fn harness() -> (SharedView, DebugQueue) {
    (
        Rc::new(RefCell::new(ViewSnapshot::default())),
        Rc::new(RefCell::new(Vec::new())),
    )
}

use std::cell::RefCell;
use std::rc::Rc;

#[test]
fn update_queues_gestures_for_the_host_drain() {
    // iced update never touches the app: every gesture queues for the drain.
    let (view, queue) = harness();
    for message in [
        DebugMessage::RefreshPressed,
        DebugMessage::MemInputChanged("0x100".to_string()),
        DebugMessage::RowSelected(0x110),
        DebugMessage::WatchClear,
    ] {
        drive(&queue, &view, message);
    }
    assert_eq!(queue.borrow().len(), 4);
    assert!(
        matches!(queue.borrow()[2], DebugMessage::RowSelected(0x110)),
        "order is preserved for the drain"
    );
}

#[test]
fn push_snapshot_rebuilds_locals_from_the_app() {
    let snapshot = test_snapshot();
    let mapped = snapshot_view(&snapshot);
    // Follow-PC mode anchors on the first visible line.
    assert_eq!(mapped.anchor, 0x0800_27D2);
    assert!(mapped.follow_pc);
    assert_eq!(mapped.dump_rows.len(), 2);
    assert_eq!(mapped.diff, vec![0x100]);
    assert_eq!(mapped.regs.len(), 2);
    assert_eq!(mapped.bookmarks, vec![(0x200, "BM1".to_string())]);
    assert_eq!(mapped.select_line, "row 0x00000100");
    assert_eq!(mapped.watch_line, "watch 0x00000100");
}

#[test]
fn snapshot_view_anchors_to_pin_once_navigated() {
    let mut snapshot = test_snapshot();
    snapshot.inputs.follow_pc = false;
    snapshot.inputs.dis_addr = Some(0x0300);
    let mapped = snapshot_view(&snapshot);
    assert!(!mapped.follow_pc);
    assert_eq!(mapped.anchor, 0x0300);
}

#[test]
fn snapshot_view_marks_coreless_as_paused_no_core() {
    let mut snapshot = test_snapshot();
    snapshot.rows.paused = true;
    snapshot.rows.status_text = "No core".to_string();
    snapshot.has_spaces = false;
    snapshot.space_name = String::new();
    let mapped = snapshot_view(&snapshot);
    assert!(mapped.paused);
    assert!(!mapped.status_text.is_empty());
    assert!(mapped.selected_name.is_none());
}

#[test]
fn view_builders_construct_every_section() {
    let snapshot = test_snapshot();
    let mapped = snapshot_view(&snapshot);
    let chrome = &mapped.chrome;
    let _ = view_toolbar(true, chrome);
    let _ = view_toolbar(false, chrome);
    let _ = view_space_row("Memory", chrome);
    let _ = view_mem_nav("0x00000100", chrome);
    // Mid-row byte selection still lights its row.
    let _ = view_dump_col(&mapped.dump_rows, &mapped.diff, Some((0x102, Some(9))));
    let _ = view_dump_col(&mapped.dump_rows, &[], None);
    let _ = view_regs_section(&mapped.regs, Some("r0"), "7", chrome);
    let _ = view_regs_section(&mapped.regs, None, "", chrome);
    let _ = view_regs_section(&[], None, "", chrome);
    let _ = view_edit_row(&mapped.select_line, mapped.selected, "42", chrome);
    let _ = view_edit_row("", None, "", chrome);
    let _ = view_confirm_row(mapped.pending_text.clone(), chrome);
    let _ = view_confirm_row(None, chrome);
    let _ = view_dis_nav(true, 0, 0x0800_27D2, "", chrome);
    let _ = view_dis_nav(false, 2, 0x300, "0x00000300", chrome);
    let _ = view_dis_col(&[], None, chrome);
    let _ = view_dis_col(&mapped.disasm_lines, None, chrome);
    let _ = view_dis_col(&mapped.disasm_lines, Some(0x0800_27D2E), chrome);
    let _ = view_watch_row(&mapped.watch_line, chrome);
    let _ = view_freeze_row(&mapped.freeze_line, "FF", chrome);
    let _ = view_bookmark_section("tag", &[(0x200, "BM1".to_string())], chrome);
    let _ = view_watch_row("", chrome);
    let _ = view_freeze_row("", "", chrome);
    let _ = view_bookmark_section("", &[], chrome);
    // Disassembly lines exercise the target follow button.
    assert_eq!(mapped.disasm_lines.len(), 2);
}

#[test]
fn view_assembles_follow_and_fixed_modes() {
    let (view, queue) = harness();
    *view.borrow_mut() = snapshot_view(&test_snapshot());
    let program = DebugProgram {
        view: Rc::clone(&view),
        queue: Rc::clone(&queue),
    };
    let state = DebugState {
        view: Rc::clone(&view),
        queue: Rc::clone(&queue),
    };
    // Follow-PC mode anchors on the first visible line.
    let _follow = program.view(&state, iced::window::Id::unique());
    drop(_follow);
    // Fixed mode pins the anchor and shows the Back depth.
    let mut fixed = test_snapshot();
    fixed.inputs.follow_pc = false;
    fixed.inputs.dis_addr = Some(0x300);
    fixed.inputs.back_len = 2;
    *view.borrow_mut() = snapshot_view(&fixed);
    let _fixed = program.view(&state, iced::window::Id::unique());
    drop(_fixed);
}

#[test]
fn disasm_marker_never_double_marks_pc() {
    // The PC row carries exactly one `>`: the pin column contributes blanks.
    assert_eq!(pin_marker(true, true), "  ");
    // Pinned non-PC rows keep the `@` in the same column.
    assert_eq!(pin_marker(false, true), "@ ");
    assert_eq!(pin_marker(false, false), "  ");
    assert_eq!(pin_marker(true, false), "  ");
}

#[test]
fn shortcut_and_cursor_keys_map_physical_codes() {
    use iced::keyboard::key::{Code, Physical};
    assert!(matches!(
        shortcut_request(&Physical::Code(Code::F6)),
        Some(DebugMessage::StepFramePressed)
    ));
    assert!(matches!(
        shortcut_request(&Physical::Code(Code::F7)),
        Some(DebugMessage::StepInstrPressed)
    ));
    assert!(matches!(
        shortcut_request(&Physical::Code(Code::F9)),
        Some(DebugMessage::TogglePausePressed)
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

#[test]
fn program_boot_shares_view_and_queue() {
    let (view, queue) = harness();
    let program = DebugProgram {
        view: Rc::clone(&view),
        queue: Rc::clone(&queue),
    };
    assert_eq!(DebugProgram::name(), "nerust_debug");
    assert!(program.window().is_none());
    let _ = program.settings();
    let (state, _task) = program.boot();
    let theme = program.theme(&state, iced::window::Id::unique());
    assert!(matches!(theme, Some(iced::Theme::Light)));
    // Update queues instead of touching the app.
    let _ = program.update(
        &mut DebugState {
            view: Rc::clone(&view),
            queue: Rc::clone(&queue),
        },
        DebugMessage::MemInputChanged("1234".into()),
    );
    assert_eq!(queue.borrow().len(), 1);
    // Pushed snapshots render without a bridge.
    *view.borrow_mut() = snapshot_view(&test_snapshot());
    let _ = program.view(&state, iced::window::Id::unique());
    drop(state);
}

#[test]
fn outer_debug_window_uses_common_title_and_existing_grouped_pipeline() {
    use crate::aux::window::WindowProgram;
    use nerust_frontend_host::{focus::AuxKind, window::Window};

    let (view, queue) = harness();
    *view.borrow_mut() = snapshot_view(&test_snapshot());
    let mut window = Window::auxiliary(AuxKind::Debug);
    window.title = "owned debugger title".into();
    let program = WindowProgram {
        window,
        program: DebugProgram {
            view: Rc::clone(&view),
            queue: Rc::clone(&queue),
        },
    };
    let (mut state, _) = program.boot();
    let id = iced::window::Id::unique();
    assert_eq!(program.title(&state, id), "owned debugger title");
    drop(program.update(&mut state, DebugMessage::MemInputChanged("1234".into())));
    assert_eq!(queue.borrow().len(), 1);
    let element = program.view(&state, id);
    assert!(
        !iced::advanced::widget::Tree::new(element.as_widget())
            .children
            .is_empty()
    );
}
