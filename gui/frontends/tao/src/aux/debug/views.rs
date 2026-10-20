//! Debugger view builders plus the per-rebuild snapshot they read. Widgets see owned locals only;
//! `capture_view` is the single place that locks the bridge.

use iced_tiny_skia::Renderer;
use nerust_core_traits::debugger::{DisasmLine, HexRow};
use nerust_gui_viewmodel::debugger::{self as vm, PendingWrite, StatusMessage};
use nerust_settings_core::i18n::{self as ui_msg, UiText, text as ui_text};
use nerust_settings_core::model::language::AppLanguage;

use super::DebugMessage;
use crate::aux::bridge::DebugBridge;

/// Two-char marker prefix for a disassembly row, kept out of the view so the width contract stays
/// unit-tested. The PC `>` itself lives in [`vm::format_disasm_line`]; this only adds the pinned `@` (or blanks), so composing both never double-marks.
pub(super) fn disasm_marker(is_pc: bool, is_pinned: bool) -> &'static str {
    if !is_pc && is_pinned { "@ " } else { "  " }
}

/// Owned per-rebuild snapshot: widgets read locals, never the live bridge. Locks are sequential, so
/// it is one consistent read in practice, strictly more coherent than per-widget locking.
pub(super) struct ViewSnapshot {
    pub(super) language: AppLanguage,
    pub(super) mem_input: String,
    pub(super) dis_input: String,
    pub(super) follow_pc: bool,
    pub(super) back_len: usize,
    pub(super) anchor: u32,
    pub(super) bookmarks: Vec<(u32, String)>,
    pub(super) bm_input: String,
    pub(super) watch: Option<u32>,
    pub(super) watch_value: Option<u8>,
    pub(super) freeze: Option<(u32, u8)>,
    pub(super) freeze_input: String,
    pub(super) selected: Option<(u32, Option<u8>)>,
    pub(super) edit_input: String,
    pub(super) pending: Option<PendingWrite>,
    pub(super) selected_name: Option<String>,
    pub(super) regs: Vec<(&'static str, u64)>,
    pub(super) dump_rows: Vec<HexRow>,
    pub(super) diff: Vec<u32>,
    pub(super) disasm_lines: Vec<DisasmLine>,
    pub(super) status: StatusMessage,
    pub(super) paused: bool,
    pub(super) reg_target: Option<&'static str>,
    pub(super) reg_input: String,
}

/// Lock the bridge once and clone an owned snapshot out of it.
/// No widget code runs here; see `anchor()` for the anchor rule.
pub(super) fn capture_view(bridge: &DebugBridge) -> ViewSnapshot {
    let language = bridge.language();
    let nav = bridge.read().nav.clone();
    let display = bridge.read().display.clone_without_images();
    let bookmarks = bridge.read().bookmarks.clone();
    let bm_input = bridge.read().bm_input.clone();
    let watch = bridge.read().watch;
    let freeze = bridge.read().freeze;
    let freeze_input = bridge.read().freeze_input.clone();
    let selected = bridge.read().selected;
    let edit_input = bridge.read().edit_input.clone();
    let pending = display.pending;
    let selected_name = bridge.selected_name();
    let regs = display.regs;
    let dump_rows = display.dump_rows;
    let diff = display.diff;
    let disasm_lines = display.disasm_lines;
    let status = display.status;
    let mem_input = nav.mem_input().to_string();
    let dis_input = nav.dis_input().to_string();
    let follow_pc = nav.follow_pc();
    let back_len = nav.back_len();
    let watch_value = display.watch_value;
    let paused = display.paused;
    // Coreless windows never drain, so the default Empty status would sit blank forever and the
    // toggle would offer Pause against nothing. Read coreless as paused with an explicit "no core" status — the same agreement as the gtk twin.
    let coreless = bridge.spaces().is_empty();
    let paused = paused || coreless;
    let status = if coreless && matches!(status, vm::StatusMessage::Empty) {
        vm::StatusMessage::NoCore
    } else {
        status
    };
    // Same rule as `anchor()`, computed from the snapshot above
    // instead of re-locking nav and display for a second time.
    let anchor = if follow_pc {
        disasm_lines.first().map(|line| line.addr).unwrap_or(0)
    } else {
        nav.dis_addr().unwrap_or(0)
    };
    let reg_target = bridge.read().reg_target;
    let reg_input = bridge.read().reg_input.clone();
    ViewSnapshot {
        language,
        mem_input,
        dis_input,
        follow_pc,
        back_len,
        anchor,
        bookmarks,
        bm_input,
        watch,
        watch_value,
        freeze,
        freeze_input,
        selected,
        edit_input,
        pending,
        selected_name,
        regs,
        dump_rows,
        diff,
        disasm_lines,
        status,
        paused,
        reg_target,
        reg_input,
    }
}

/// One pane builders: each renders a single debugger section from plain locals (`view` stays a
/// capture-and-assemble pass). The toolbar carries a single Pause/Resume toggle following the drained pause.
pub(super) fn view_toolbar(
    paused: bool,
    language: AppLanguage,
) -> iced::Element<'static, DebugMessage, iced::Theme, Renderer> {
    use iced::widget::{button, row, text};
    row![
        button(text(if paused {
            ui_text(language, UiText::Resume)
        } else {
            ui_text(language, UiText::Pause)
        }))
        .style(button::secondary)
        .on_press(DebugMessage::TogglePausePressed),
        button(text(ui_text(language, UiText::StepFrame)))
            .style(button::secondary)
            .on_press(DebugMessage::StepFramePressed),
        button(text(ui_text(language, UiText::StepInstr)))
            .style(button::secondary)
            .on_press(DebugMessage::StepInstrPressed),
        button(text(ui_text(language, UiText::PpuViewer)))
            .style(button::secondary)
            .on_press(DebugMessage::PpuOpenPressed),
    ]
    .spacing(12)
    .align_y(iced::Alignment::Center)
    .into()
}

pub(super) fn view_space_row(
    selected_name: &str,
    language: AppLanguage,
) -> iced::Element<'static, DebugMessage, iced::Theme, Renderer> {
    use iced::Length;
    use iced::widget::{button, row, text};
    row![
        text(ui_text(language, UiText::Space)).width(Length::Fixed(150.0)),
        button(text(ui_text(language, UiText::Prev)))
            .style(button::secondary)
            .on_press(DebugMessage::SpacePrev),
        text(selected_name.to_owned()).size(14).width(Length::Fill),
        button(text(ui_text(language, UiText::Next)))
            .style(button::secondary)
            .on_press(DebugMessage::SpaceNext),
    ]
    .spacing(12)
    .align_y(iced::Alignment::Center)
    .into()
}

pub(super) fn view_mem_nav(
    mem_input: &str,
    language: AppLanguage,
) -> iced::Element<'static, DebugMessage, iced::Theme, Renderer> {
    use iced::Length;
    use iced::widget::{button, row, text, text_input};
    row![
        text(ui_text(language, UiText::Page)).width(Length::Fixed(150.0)),
        button(text(ui_text(language, UiText::Prev)))
            .style(button::secondary)
            .on_press(DebugMessage::MemPage(-1)),
        button(text(ui_text(language, UiText::Next)))
            .style(button::secondary)
            .on_press(DebugMessage::MemPage(1)),
        text_input(ui_text(language, UiText::AddrHex), mem_input)
            .on_input(DebugMessage::MemInputChanged)
            .on_submit(DebugMessage::MemGo)
            // Wide enough for the longest placeholder in either
            // language (JA "アドレス16進" clips at 120).
            .width(150),
        button(text(ui_text(language, UiText::Go)))
            .style(button::secondary)
            .on_press(DebugMessage::MemGo),
    ]
    .spacing(12)
    .align_y(iced::Alignment::Center)
    .into()
}

/// Dump rows are text-style buttons; selection inverts, changed rows carry a `*` mark. The list
/// fills its share in a scrollable; shrunken windows scroll natively.
pub(super) fn view_dump_col(
    dump_rows: &[HexRow],
    diff: &[u32],
    selected: Option<(u32, Option<u8>)>,
) -> iced::Element<'static, DebugMessage, iced::Theme, Renderer> {
    use iced::Length;
    use iced::widget::{button, column, row, scrollable, text};
    // Byte-precise selection: every byte is its own button, so the write target reads exactly (the
    // address head keeps the old row-base pick). The drain, transaction, and commit path were already byte-exact; only the click granularity was row-wide.
    let mut dump_col = column![].spacing(2);
    for hex_row in dump_rows {
        let selected_addr = selected.map(|(a, _)| a);
        // Containing-row match: a mid-row pick still lights its row.
        let row_hit = selected_addr
            .is_some_and(|a| a >= hex_row.addr && a < hex_row.addr + u32::from(hex_row.valid));
        let changed = diff.contains(&hex_row.addr);
        let marker = match (row_hit, changed) {
            (true, true) => ">*",
            (true, false) => "> ",
            (false, true) => " *",
            (false, false) => "  ",
        };
        let mut line_row = row![
            button(
                text(format!("{marker}0x{:08X}:", hex_row.addr))
                    .size(14)
                    .font(iced::Font::MONOSPACE),
            )
            .padding([2, 4])
            .style(if row_hit {
                button::secondary
            } else {
                button::text
            })
            .on_press(DebugMessage::RowSelected(hex_row.addr)),
        ]
        .spacing(2);
        for i in 0..hex_row.valid {
            let addr = hex_row.addr + u32::from(i);
            let byte = hex_row.bytes[usize::from(i)];
            let is_selected = selected_addr == Some(addr);
            line_row = line_row.push(
                button(
                    text(format!("{byte:02X}"))
                        .size(14)
                        .font(iced::Font::MONOSPACE),
                )
                .padding([2, 4])
                .style(if is_selected {
                    button::secondary
                } else {
                    button::text
                })
                .on_press(DebugMessage::RowSelected(addr)),
            );
        }
        dump_col = dump_col.push(line_row.width(Length::Fill));
    }
    scrollable(dump_col).height(Length::Fill).into()
}

/// Registers flow into two clickable columns so tall lists cost half the height. Row click picks
/// the write target; the Set row writes the parsed hex value through the drain.
pub(super) fn view_regs_section(
    regs: &[(&'static str, u64)],
    reg_target: Option<&'static str>,
    reg_input: &str,
    language: AppLanguage,
) -> iced::Element<'static, DebugMessage, iced::Theme, Renderer> {
    use iced::Length;
    use iced::widget::{button, column, row, text, text_input};
    let mid = regs.len().div_ceil(2).min(regs.len());
    let mut halves_row = row![].spacing(16).width(Length::Fill);
    for half in [&regs[..mid], &regs[mid..]] {
        let mut col = column![].spacing(2);
        for (name, value) in half.iter() {
            let row_button = button(
                text(vm::format_register(name, *value))
                    .size(14)
                    .font(iced::Font::MONOSPACE),
            )
            .padding([2, 8])
            .width(Length::Fill);
            col = col.push(
                row_button
                    .style(if Some(*name) == reg_target {
                        button::secondary
                    } else {
                        button::text
                    })
                    .on_press(DebugMessage::RegSelected(name)),
            );
        }
        halves_row = halves_row.push(col);
    }
    let target_text = match reg_target {
        Some(name) => format!("{} {name}", ui_text(language, UiText::Set)),
        None => ui_text(language, UiText::PickRegister).to_string(),
    };
    let edit = row![
        text(target_text).size(14).width(Length::Fixed(140.0)),
        text_input(ui_text(language, UiText::Hex), reg_input)
            .on_input(DebugMessage::RegInputChanged)
            .on_submit(DebugMessage::RegSet)
            .width(100),
        button(text(ui_text(language, UiText::Set)))
            .style(button::secondary)
            .on_press(DebugMessage::RegSet),
    ]
    .spacing(12)
    .align_y(iced::Alignment::Center);
    column![halves_row, edit].spacing(4).into()
}

pub(super) fn view_edit_row(
    selected: Option<(u32, Option<u8>)>,
    edit_input: &str,
    language: AppLanguage,
) -> iced::Element<'static, DebugMessage, iced::Theme, Renderer> {
    use iced::Length;
    use iced::widget::{button, row, text, text_input};
    // The target rides on the label and the button: writes always
    // land on the selected row, never on the page anchor.
    let select_text = match selected {
        Some((addr, old)) => ui_msg::selected_row(language, addr, old),
        None => ui_text(language, UiText::SelectDumpRow).to_string(),
    };
    // The target rides on the button: writes always land on the
    // selected row, never on the page anchor.
    let write_label = match selected {
        Some((addr, _)) => format!("{} 0x{addr:08X}", ui_text(language, UiText::Write)),
        None => ui_text(language, UiText::Write).to_string(),
    };
    row![
        text(select_text).size(14).width(Length::Fixed(240.0)),
        text_input(ui_text(language, UiText::HexByte), edit_input)
            .on_input(DebugMessage::EditInputChanged)
            .width(100),
        button(text(write_label))
            .style(button::secondary)
            .on_press(DebugMessage::EditWrite),
        button(text(ui_text(language, UiText::Watch)))
            .style(button::secondary)
            .on_press(DebugMessage::WatchSet),
        button(text(ui_text(language, UiText::Freeze)))
            .style(button::secondary)
            .on_press(DebugMessage::EditFreezeSet),
    ]
    .spacing(12)
    .align_y(iced::Alignment::Center)
    .into()
}

pub(super) fn view_confirm_row(
    pending: Option<vm::PendingWrite>,
    language: AppLanguage,
) -> Option<iced::Element<'static, DebugMessage, iced::Theme, Renderer>> {
    use iced::Length;
    use iced::widget::{button, row, text};
    pending.map(|pending| {
        row![
            text(pending.render(language)).size(14).width(Length::Fill),
            button(text(ui_text(language, UiText::Confirm)))
                .style(button::primary)
                .on_press(DebugMessage::ConfirmWrite),
            button(text(ui_text(language, UiText::Cancel)))
                .style(button::text)
                .on_press(DebugMessage::CancelWrite),
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center)
        .into()
    })
}
pub(super) fn view_dis_nav(
    follow_pc: bool,
    back_len: usize,
    anchor: u32,
    dis_input: &str,
    language: AppLanguage,
) -> iced::Element<'static, DebugMessage, iced::Theme, Renderer> {
    use iced::widget::{button, row, text, text_input};
    let follow_button =
        button(text(ui_text(language, UiText::FollowPc))).on_press(DebugMessage::FollowPcToggle);
    let back_button = button(text(format!(
        "{} ({back_len})",
        ui_text(language, UiText::Back)
    )))
    .on_press(DebugMessage::DisBack);
    // The title names the anchor: clicks pin it visibly instead of
    // silently moving the window.
    let title = if follow_pc {
        format!(
            "{} {}",
            ui_text(language, UiText::Disassembly),
            ui_text(language, UiText::FollowPcTag)
        )
    } else {
        format!(
            "{} @0x{anchor:08X} {}",
            ui_text(language, UiText::Disassembly),
            ui_text(language, UiText::FixedTag)
        )
    };
    row![
        text(title),
        if follow_pc {
            follow_button.style(button::secondary)
        } else {
            follow_button.style(button::text)
        },
        text_input(ui_text(language, UiText::AddrHex), dis_input)
            .on_input(DebugMessage::DisInputChanged)
            .on_submit(DebugMessage::DisGo)
            // Same JA placeholder width as the memory input above.
            .width(150),
        button(text(ui_text(language, UiText::Go)))
            .style(button::secondary)
            .on_press(DebugMessage::DisGo),
        if back_len > 0 {
            back_button.style(button::secondary)
        } else {
            back_button.style(button::text)
        },
    ]
    .spacing(12)
    .align_y(iced::Alignment::Center)
    .into()
}

pub(super) fn view_dis_col(
    disasm_lines: &[nerust_core_traits::debugger::DisasmLine],
    pinned: Option<u32>,
    language: AppLanguage,
) -> iced::Element<'static, DebugMessage, iced::Theme, Renderer> {
    use iced::Length;
    use iced::widget::{button, column, row, scrollable, text};
    let mut dis_col = column![].spacing(2);
    if disasm_lines.is_empty() {
        dis_col = dis_col.push(text(ui_text(language, UiText::NoDisassembly)).size(14));
        dis_col = dis_col.push(text(ui_text(language, UiText::DisassemblyHint)).size(14));
    }
    for line in disasm_lines {
        // Marker column is two chars wide for every row. The PC `>` already comes from
        // `format_disasm_line`, so the PC row contributes blanks here; the pinned anchor row carries `@`. (Prepending another `"> "` double-marked the PC row.)
        let is_pinned = pinned == Some(line.addr);
        let marker = disasm_marker(line.is_pc, is_pinned);
        let line_button = button(
            text(format!("{marker}{}", vm::format_disasm_line(line)))
                .size(14)
                .font(iced::Font::MONOSPACE),
        )
        .padding([2, 8])
        .width(Length::Fill)
        .on_press(DebugMessage::DisLineClicked(line.addr));
        let line_button = if line.is_pc || is_pinned {
            line_button.style(button::secondary)
        } else {
            line_button.style(button::text)
        };
        let mut line_row = row![line_button].spacing(4);
        if let Some(target) = line.target {
            line_row = line_row.push(
                button(text("→").size(14))
                    .padding([2, 8])
                    .width(Length::Fixed(40.0))
                    .style(button::secondary)
                    .on_press(DebugMessage::FollowTarget(target)),
            );
        }
        dis_col = dis_col.push(line_row);
    }
    scrollable(dis_col).height(Length::Fill).into()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn view_watch_row(
    watch: Option<u32>,
    watch_value: Option<u8>,
    language: AppLanguage,
) -> iced::Element<'static, DebugMessage, iced::Theme, Renderer> {
    use iced::widget::{button, row, text};
    let watch_text = match (watch, watch_value) {
        (Some(addr), Some(value)) => ui_msg::watch_value(language, addr, Some(value)),
        (Some(addr), None) => ui_msg::watch_value(language, addr, None),
        (None, _) => ui_text(language, UiText::WatchNone).to_string(),
    };
    row![
        button(text(ui_text(language, UiText::Watch)))
            .style(button::secondary)
            .on_press(DebugMessage::WatchSet),
        text(watch_text).size(14).width(iced::Length::Fill),
        button(text(ui_text(language, UiText::Clear)))
            .style(button::text)
            .on_press(DebugMessage::WatchClear),
    ]
    .spacing(12)
    .align_y(iced::Alignment::Center)
    .into()
}

pub(super) fn view_freeze_row(
    freeze: Option<(u32, u8)>,
    freeze_input: &str,
    language: AppLanguage,
) -> iced::Element<'static, DebugMessage, iced::Theme, Renderer> {
    use iced::widget::{button, row, text, text_input};
    let freeze_text = match freeze {
        Some((addr, value)) => ui_msg::frozen(language, addr, value),
        None => ui_text(language, UiText::FreezeNone).to_string(),
    };
    row![
        text_input(ui_text(language, UiText::HexByte), freeze_input)
            .on_input(DebugMessage::FreezeInputChanged)
            .width(100),
        button(text(ui_text(language, UiText::Freeze)))
            .style(button::secondary)
            .on_press(DebugMessage::FreezeSet),
        text(freeze_text).size(14).width(iced::Length::Fill),
        button(text(ui_text(language, UiText::Clear)))
            .style(button::text)
            .on_press(DebugMessage::FreezeClear),
    ]
    .spacing(12)
    .align_y(iced::Alignment::Center)
    .into()
}

pub(super) fn view_bookmark_section(
    bm_input: &str,
    bookmarks: &[(u32, String)],
    language: AppLanguage,
) -> iced::Element<'static, DebugMessage, iced::Theme, Renderer> {
    use iced::widget::{button, column, row, text, text_input};
    let mut bookmarks_col = column![].spacing(2);
    for (addr, label) in bookmarks {
        bookmarks_col = bookmarks_col.push(
            row![
                button(
                    text(format!("{label} 0x{addr:08X}"))
                        .size(14)
                        .font(iced::Font::MONOSPACE),
                )
                .padding([2, 8])
                .style(button::text)
                .on_press(DebugMessage::BookmarkJump(*addr)),
                button(text("×").size(14))
                    .padding([4, 10])
                    .style(button::text)
                    .on_press(DebugMessage::BookmarkDelete(*addr)),
            ]
            .spacing(4)
            .align_y(iced::Alignment::Center),
        );
    }
    column![
        text(ui_text(language, UiText::Bookmarks)).size(16),
        row![
            text_input(ui_text(language, UiText::Label), bm_input)
                .on_input(DebugMessage::BookmarkInputChanged)
                .on_submit(DebugMessage::BookmarkAdd)
                .width(iced::Length::Fill),
            button(text(ui_text(language, UiText::Add)))
                .style(button::secondary)
                .on_press(DebugMessage::BookmarkAdd),
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center),
        bookmarks_col,
    ]
    .spacing(12)
    .into()
}
