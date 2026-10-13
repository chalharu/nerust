use super::*;
use nerust_core_traits::debugger::{DisasmLine, HexRow};

fn cached(rows: bool) -> DisplayCache {
    DisplayCache {
        dump_rows: if rows {
            vec![HexRow {
                addr: 0x10,
                valid: 1,
                bytes: [0xAB, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            }]
        } else {
            Vec::new()
        },
        regs: if rows { vec![("a", 1)] } else { Vec::new() },
        status: StatusMessage::Paused,
        paused: true,
        ..DisplayCache::default()
    }
}

#[test]
fn restore_keeps_paused_sections_over_running_drain() {
    let old = cached(true);
    let mut new = DisplayCache {
        paused: false,
        status: StatusMessage::Running,
        ..DisplayCache::default()
    };
    new.restore_paused_sections(&old);
    assert_eq!(new.dump_rows.len(), 1);
    assert_eq!(new.regs, vec![("a", 1)]);
    assert_eq!(new.status, StatusMessage::Running, "fresh chrome stays");
    // Paused drains pass through untouched.
    let mut paused_new = DisplayCache {
        paused: true,
        ..DisplayCache::default()
    };
    paused_new.restore_paused_sections(&old);
    assert!(paused_new.dump_rows.is_empty());
}

#[test]
fn ppu_hover_hint_is_canonical() {
    // Single wording shared by the tao and GTK viewers.
    assert_eq!(PPU_HOVER_HINT, "hover a tile");
}

#[test]
fn scale2x_doubles_each_pixel() {
    // 2x1 red/green becomes 4x2 blocks.
    let rgba = vec![255, 0, 0, 0xFF, 0, 255, 0, 0xFF];
    let out = scale2x_nearest(&rgba, 2, 1);
    assert_eq!(out.len(), 4 * 2 * 4);
    // Top row is red red green green.
    assert_eq!(&out[0..4], &[255, 0, 0, 0xFF]);
    assert_eq!(&out[4..8], &[255, 0, 0, 0xFF]);
    assert_eq!(&out[8..12], &[0, 255, 0, 0xFF]);
    assert_eq!(&out[12..16], &[0, 255, 0, 0xFF]);
    // Bottom row repeats the top row.
    assert_eq!(&out[16..20], &[255, 0, 0, 0xFF]);
    assert_eq!(&out[24..28], &[0, 255, 0, 0xFF]);
}

#[test]
fn navigate_pins_and_pushes_anchor() {
    let mut nav = NavState::new(4, 0);
    assert!(nav.follow_pc());
    assert_eq!(nav.dis_addr(), None);
    nav.navigate(0xC03A, 0xC034);
    assert!(!nav.follow_pc());
    assert_eq!(nav.dis_addr(), Some(0xC034));
    assert_eq!(nav.dis_input(), "0x0000C034");
    assert_eq!(nav.back_len(), 1);
}

#[test]
fn navigate_caps_and_dedups() {
    let mut nav = NavState::new(1, 0);
    for i in 0..(DISASM_BACK_CAP as u32 + 5) {
        nav.navigate(0xC000 + i, 0xC100 + i);
    }
    assert_eq!(nav.back_len(), DISASM_BACK_CAP);
    assert_eq!(nav.dis_addr(), Some(0xC100 + DISASM_BACK_CAP as u32 + 4));
    // Consecutive duplicates never stack: same anchor twice.
    let mut dup = NavState::new(1, 0);
    dup.navigate(0xC03A, 0xC034);
    dup.navigate(0xC03A, 0xC034);
    assert_eq!(dup.back_len(), 1);
}

#[test]
fn go_back_pops_to_pin() {
    let mut nav = NavState::new(1, 0);
    assert_eq!(nav.go_back(), None);
    nav.navigate(0xC03A, 0xC034);
    nav.navigate(0xC034, 0xC000);
    assert_eq!(nav.go_back(), Some(0xC034));
    assert_eq!(nav.dis_addr(), Some(0xC034));
    assert_eq!(nav.go_back(), Some(0xC03A));
    assert_eq!(nav.go_back(), None);
    assert_eq!(nav.back_len(), 0);
}

#[test]
fn space_switch_resets_to_start() {
    let mut nav = NavState::new(4, 0);
    nav.go_mem(0x0100);
    nav.set_space(1, 0x2000);
    assert_eq!(nav.space_idx(), 1);
    assert_eq!(nav.mem_addr(), 0x2000);
    assert_eq!(nav.mem_input(), "0x00002000");
    // Out-of-range index keeps the current space.
    nav.set_space(9, 0);
    assert_eq!(nav.space_idx(), 1);
}

#[test]
fn cycle_space_wraps_around() {
    let mut nav = NavState::new(4, 0);
    assert_eq!(nav.cycle_space(1), Some(1));
    assert_eq!(nav.cycle_space(1), Some(2));
    assert_eq!(nav.cycle_space(-1), Some(1));
    assert_eq!(nav.cycle_space(-2), Some(3));
    assert_eq!(nav.cycle_space(1), Some(0));
    let mut empty = NavState::new(0, 0);
    assert_eq!(empty.cycle_space(1), None);
}

#[test]
fn page_mem_moves_by_delta() {
    let mut nav = NavState::new(1, 0x0100);
    nav.page_mem(0x80);
    assert_eq!(nav.mem_addr(), 0x0180);
    nav.page_mem(-0x100);
    assert_eq!(nav.mem_addr(), 0x0080);
    nav.page_mem(-0x1000);
    assert_eq!(nav.mem_addr(), 0);
}

#[test]
fn follow_reenables_pc_mode() {
    let mut nav = NavState::new(1, 0);
    nav.navigate(0xC03A, 0xC034);
    nav.follow();
    assert!(nav.follow_pc());
    assert_eq!(nav.dis_addr(), None);
}

#[test]
fn diff_rows_marks_changed_only() {
    use nerust_core_traits::debugger::HexRow;
    let row = |addr: u32, bytes: &[u8]| HexRow {
        addr,
        valid: bytes.len() as u8,
        bytes: {
            let mut all = [0u8; 16];
            all[..bytes.len()].copy_from_slice(bytes);
            all
        },
    };
    let prev = vec![row(0, &[0x00]), row(16, &[0x00])];
    let next = vec![row(0, &[0x42]), row(16, &[0x00]), row(32, &[0x00])];
    // Changed row marks; unchanged and added rows do not.
    assert_eq!(diff_rows(&prev, &next), vec![0]);
    assert!(diff_rows(&prev, &prev).is_empty());
}

#[test]
fn pending_write_renders_in_both_languages() {
    use nerust_settings_core::model::language::AppLanguage;
    let staged = PendingWrite {
        addr: 0,
        old: Some(0xAB),
        value: 0x42,
    };
    assert_eq!(
        staged.render(AppLanguage::English),
        "write 42 to 0x00000000 (was AB)?"
    );
    let unknown = PendingWrite {
        addr: 0x80,
        old: None,
        value: 0xCC,
    };
    assert_eq!(
        unknown.render(AppLanguage::English),
        "write CC to 0x00000080 (was ??)?"
    );
    assert_eq!(
        unknown.render(AppLanguage::Japanese),
        "0x00000080 に CC を書込 (元 ??)?"
    );
}

#[test]
fn status_message_renders_every_variant() {
    use nerust_settings_core::model::language::AppLanguage;
    let en = AppLanguage::English;
    assert_eq!(StatusMessage::Empty.render(en), "");
    assert_eq!(StatusMessage::Paused.render(en), "paused");
    assert_eq!(StatusMessage::Running.render(en), "running");
    assert_eq!(StatusMessage::Resume.render(en), "Resume");
    assert_eq!(StatusMessage::NothingToWrite.render(en), "nothing to write");
    assert_eq!(StatusMessage::NoCore.render(en), "no core");
    assert_eq!(StatusMessage::Stepped(7).render(en), "stepped 7");
    assert_eq!(
        StatusMessage::WroteMemory {
            addr: 0x10,
            value: 0x42
        }
        .render(en),
        "wrote 42 to 0x00000010 (width 1)"
    );
    assert_eq!(
        StatusMessage::RegisterWrote {
            name: "r7",
            value: 1
        }
        .render(en),
        "wrote 0x0001 to register r7"
    );
    assert_eq!(
        StatusMessage::WriteRefused("X".to_string()).render(en),
        "write refused: X"
    );
    assert_eq!(
        StatusMessage::Local("custom".to_string()).render(en),
        "custom"
    );
    // One Japanese spot-check pins the language threading.
    assert_eq!(
        StatusMessage::Paused.render(AppLanguage::Japanese),
        "一時停止中"
    );
}

#[test]
fn parse_hex_addr_accepts_plain_0x_and_dollar() {
    assert_eq!(parse_hex_addr("0010"), Some(0x10));
    assert_eq!(parse_hex_addr("0x0010"), Some(0x10));
    assert_eq!(parse_hex_addr("$0010"), Some(0x10));
    assert_eq!(parse_hex_addr("  $C03A  "), Some(0xC03A));
    assert_eq!(parse_hex_addr("FFFFFFFF"), Some(0xFFFF_FFFF));
    assert_eq!(parse_hex_addr(""), None);
    assert_eq!(parse_hex_addr("zz"), None);
    assert_eq!(parse_hex_addr("0x"), None);
    assert_eq!(parse_hex_addr("$"), None);
    assert_eq!(parse_hex_addr("1FFFFFFFF"), None);
    assert_eq!(parse_hex_addr("-1"), None);
}

#[test]
fn display_cache_covers_all_sections() {
    use nerust_core_traits::debugger::HexRow;
    // Snapshot shape: every section the drain refreshes has a slot.
    let cache = DisplayCache {
        regs: vec![("a", 0)],
        dump_rows: vec![HexRow {
            addr: 0,
            valid: 1,
            bytes: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        }],
        diff: vec![0],
        disasm_lines: vec![DisasmLine {
            addr: 0xC000,
            bytes: [0xEA, 0, 0, 0],
            len: 1,
            text: "NOP".to_string(),
            is_pc: true,
            target: None,
        }],
        panels: vec![],
        images: vec![("Pattern left".to_string(), 256, 256, vec![0xFF; 4])],
        images_gen: 7,
        watch_value: Some(0x42),
        paused: true,
        reg_names: vec!["a"],
        status: StatusMessage::Paused,
        pending: Some(PendingWrite {
            addr: 0,
            old: Some(0x00),
            value: 0x42,
        }),
    };
    assert_eq!(cache.dump_rows.len(), 1);
    assert_eq!(cache.disasm_lines[0].addr, 0xC000);
    assert_eq!(cache.images.len(), 1);
    // The imageless clone keeps the generation: viewers compare
    // generations, never pixels.
    let light = cache.clone_without_images();
    assert!(light.images.is_empty());
    assert_eq!(light.images_gen, 7);
}

#[test]
fn format_register_orders_value() {
    assert_eq!(format_register("b", 2), "b: 0x0002");
    assert_eq!(format_register("a", 1), "a: 0x0001");
}

#[test]
fn space_key_resolves_display_keys_without_fabrication() {
    use nerust_core_traits::debugger::{SpaceAccess, SpaceId, SpaceInfo, SpaceTable};
    static ENTRIES: [SpaceInfo; 1] = [SpaceInfo {
        id: SpaceId(0),
        key: "wram",
        name: "WRAM",
        address_bits: 13,
        range: 0x0000..=0x1FFF,
        access: SpaceAccess::ReadWrite,
    }];
    let table = SpaceTable::build(&ENTRIES);
    assert_eq!(
        space_key(
            &table,
            &nerust_core_traits::debugger::DebuggerError::UnknownSpace(SpaceId(9))
        ),
        None,
        "unknown spaces never fabricate a key"
    );
    assert_eq!(
        space_key(
            &table,
            &nerust_core_traits::debugger::DebuggerError::ReadOnlySpace(SpaceId(0))
        ),
        Some("wram")
    );
    assert_eq!(
        space_key(
            &table,
            &nerust_core_traits::debugger::DebuggerError::UnmappedAddress {
                space: SpaceId(0),
                addr: 0x2000
            }
        ),
        Some("wram")
    );
    assert_eq!(
        space_key(
            &table,
            &nerust_core_traits::debugger::DebuggerError::BadWidth(0)
        ),
        None
    );
}

#[test]
fn format_panels_covers_all_cell_kinds() {
    use nerust_core_traits::debugger::ColumnKind;
    use nerust_core_traits::debugger::{CellValue, DebugPanel, PanelColumn, PanelRow};
    let panels = [DebugPanel {
        id: "p",
        label_id: "P",
        columns: &[
            PanelColumn {
                id: "t",
                label_id: "T",
                kind: ColumnKind::Text,
            },
            PanelColumn {
                id: "h",
                label_id: "H",
                kind: ColumnKind::Hex { digits: 2 },
            },
            PanelColumn {
                id: "d",
                label_id: "D",
                kind: ColumnKind::Dec,
            },
            PanelColumn {
                id: "b",
                label_id: "B",
                kind: ColumnKind::Bool,
            },
        ],
        rows: vec![PanelRow {
            key: "row",
            cells: vec![
                CellValue::Text("s".to_string()),
                CellValue::U64(0xAB),
                CellValue::U64(241),
                CellValue::Bool(true),
            ],
        }],
    }];
    assert_eq!(format_panels(&panels), "== P ==\nrow: s AB 241 yes\n");
    assert_eq!(format_panels(&[]), "(no panels)");
    // Kind/cell mismatches degrade instead of panicking.
    assert_eq!(format_cell(&CellValue::U64(7), ColumnKind::Text), "7");
    assert_eq!(
        format_cell(&CellValue::Bool(false), ColumnKind::Hex { digits: 2 }),
        "false"
    );
}

#[test]
fn rgba_image_blits_through_palette() {
    use nerust_core_traits::debugger::{DebugImage, ImageFormat};
    let image = DebugImage {
        id: "left",
        label_id: "Pattern left",
        width: 2,
        height: 1,
        format: ImageFormat::Indexed2bpp,
        palette: vec![[0, 0, 0], [255, 0, 0], [0, 255, 0], [0, 0, 255]],
        pixels: vec![0, 3],
    };
    assert_eq!(
        rgba_image(&image),
        (
            "Pattern left".to_string(),
            2,
            1,
            vec![0, 0, 0, 0xFF, 0, 0, 255, 0xFF]
        )
    );
}

#[test]
fn rgba_image_blits_4bpp_through_palette() {
    use nerust_core_traits::debugger::{DebugImage, ImageFormat};
    let mut palette = vec![[0, 0, 0]; 16];
    palette[15] = [1, 2, 3];
    let image = DebugImage {
        id: "tiles",
        label_id: "Tiles",
        width: 2,
        height: 1,
        format: ImageFormat::Indexed4bpp,
        palette,
        pixels: vec![0, 15],
    };
    assert_eq!(
        rgba_image(&image),
        (
            "Tiles".to_string(),
            2,
            1,
            vec![0, 0, 0, 0xFF, 1, 2, 3, 0xFF]
        )
    );
}

#[test]
fn format_disasm_line_aligns_bytes() {
    let row = DisasmLine {
        addr: 0xC03A,
        bytes: [0x88, 0, 0, 0],
        len: 1,
        text: "DEY".to_string(),
        is_pc: true,
        target: None,
    };
    assert_eq!(format_disasm_line(&row), ">0x0000C03A: 88          DEY");
    let row = DisasmLine {
        addr: 0xC03B,
        bytes: [0xD0, 0xF7, 0, 0],
        len: 2,
        text: "BNE $C034".to_string(),
        is_pc: false,
        target: Some(0xC034),
    };
    assert_eq!(
        format_disasm_line(&row),
        " 0x0000C03B: D0 F7       BNE $C034"
    );
}

#[test]
fn write_transaction_flows_to_commit() {
    use nerust_core_traits::debugger::SpaceId;
    let mut txn = WriteTransaction::Empty;
    assert_eq!(txn.pending(), None);
    assert_eq!(txn.take_commit(), None);
    // Staging without preparation is a no-op.
    txn.stage(0x42);
    assert_eq!(txn.take_commit(), None);
    // Prepare, stage, commit consumes exactly once.
    txn.prepare(SpaceId(0), 0x10, Some(0x00));
    assert_eq!(txn.pending(), None);
    txn.stage(0x42);
    assert_eq!(
        txn.pending(),
        Some(PendingWrite {
            addr: 0x10,
            old: Some(0x00),
            value: 0x42,
        })
    );
    assert_eq!(txn.take_commit(), Some((SpaceId(0), 0x10, 0x42)));
    assert_eq!(txn, WriteTransaction::Empty);
    assert_eq!(txn.take_commit(), None);
}

#[test]
fn write_transaction_prepare_overwrites() {
    use nerust_core_traits::debugger::SpaceId;
    let mut txn = WriteTransaction::Empty;
    txn.prepare(SpaceId(0), 0x10, Some(0x00));
    txn.stage(0x42);
    // Fresh selection drops the staged value.
    txn.prepare(SpaceId(0), 0x20, Some(0xFF));
    assert_eq!(txn.pending(), None);
    assert_eq!(txn.take_commit(), None);
}

#[test]
fn write_transaction_traffic_kills_uncommitted() {
    use nerust_core_traits::debugger::SpaceId;
    let mut txn = WriteTransaction::Empty;
    // Empty traffic is a no-op.
    txn.on_state_changing_traffic();
    assert_eq!(txn, WriteTransaction::Empty);
    // Prepared and staged values die on state-changing traffic.
    txn.prepare(SpaceId(0), 0x10, Some(0x00));
    txn.on_state_changing_traffic();
    assert_eq!(txn, WriteTransaction::Empty);
    txn.prepare(SpaceId(0), 0x10, Some(0x00));
    txn.stage(0x42);
    txn.on_state_changing_traffic();
    assert_eq!(txn, WriteTransaction::Empty);
    assert_eq!(txn.pending(), None);
    // Explicit cancel clears too.
    txn.prepare(SpaceId(0), 0x10, Some(0x00));
    txn.clear();
    assert_eq!(txn, WriteTransaction::Empty);
}
