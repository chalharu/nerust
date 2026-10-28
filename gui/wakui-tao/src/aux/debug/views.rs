//! Debugger view builders plus the per-rebuild snapshot they read. Widgets see owned locals only;
//! `capture_view` is the single place that locks the bridge.

use iced_tiny_skia::Renderer;
use wakui::debugger::{
    DebugSnapshot, DebuggerChrome, DebuggerRows, DisasmRow, DumpRow, RegRow, pin_marker,
};

use super::DebugMessage;

/// Owned per-rebuild snapshot: widgets read locals only.
/// Built from the app snapshot pushed after each debug turn, never by
/// locking a bridge; widgets stay decoupled from view-definitions.
#[derive(Debug, Clone, Default)]
pub(crate) struct ViewSnapshot {
    pub(super) chrome: DebuggerChrome,
    pub(super) mem_input: String,
    pub(super) dis_input: String,
    pub(super) follow_pc: bool,
    pub(super) back_len: usize,
    pub(super) anchor: u32,
    pub(super) bookmarks: Vec<(u32, String)>,
    pub(super) bm_input: String,
    pub(super) freeze_input: String,
    pub(super) selected: Option<(u32, Option<u8>)>,
    pub(super) edit_input: String,
    pub(super) pending_text: Option<String>,
    pub(super) select_line: String,
    pub(super) watch_line: String,
    pub(super) freeze_line: String,
    pub(super) selected_name: Option<String>,
    pub(super) regs: Vec<RegRow>,
    pub(super) dump_rows: Vec<DumpRow>,
    pub(super) diff: Vec<u32>,
    pub(super) disasm_lines: Vec<DisasmRow>,
    pub(super) status_text: String,
    pub(super) paused: bool,
    pub(super) reg_target: Option<&'static str>,
    pub(super) reg_input: String,
}

/// One coherent pass over the pushed app snapshot: widgets read locals only.
/// The disassembly anchor follows the same rule as before: track PC while
/// following, otherwise hold the snapshot's disassembly address.
pub(crate) fn snapshot_view(snapshot: &DebugSnapshot) -> ViewSnapshot {
    let rows: &DebuggerRows = &snapshot.rows;
    let inputs = &snapshot.inputs;
    // Same anchor rule as before, computed from the snapshot instead
    // of re-locking navigation and display for a second time.
    let anchor = if inputs.follow_pc {
        rows.disasm.first().map(|line| line.addr).unwrap_or(0)
    } else {
        inputs.dis_addr.unwrap_or(0)
    };
    ViewSnapshot {
        chrome: snapshot.chrome.clone(),
        mem_input: inputs.mem_input.clone(),
        dis_input: inputs.dis_input.clone(),
        follow_pc: inputs.follow_pc,
        back_len: inputs.back_len,
        anchor,
        bookmarks: inputs.bookmarks.clone(),
        bm_input: inputs.bm_input.clone(),
        freeze_input: inputs.freeze_input.clone(),
        selected: snapshot.selected,
        edit_input: inputs.edit_input.clone(),
        pending_text: rows.pending_text.clone(),
        select_line: rows.selected_line.clone(),
        watch_line: rows.watch_text.clone(),
        freeze_line: rows.freeze_text.clone(),
        selected_name: Some(snapshot.space_name.clone()).filter(|name| !name.is_empty()),
        regs: rows.regs.clone(),
        dump_rows: rows.dump.clone(),
        diff: rows.changed_addrs.clone(),
        disasm_lines: rows.disasm.clone(),
        status_text: rows.status_text.clone(),
        paused: rows.paused,
        reg_target: snapshot.reg_target,
        reg_input: inputs.reg_input.clone(),
    }
}

/// One pane builders: each renders a single debugger section from plain locals (`view` stays a
/// capture-and-assemble pass). The toolbar carries a single Pause/Resume toggle following the drained pause.
pub(super) fn view_toolbar(
    paused: bool,
    chrome: &DebuggerChrome,
) -> iced::Element<'static, DebugMessage, iced::Theme, Renderer> {
    use iced::widget::{button, row, text};
    row![
        button(text(chrome.pause_label(paused).to_owned()))
            .style(button::secondary)
            .on_press(DebugMessage::TogglePausePressed),
        button(text(chrome.step_frame.clone()))
            .style(button::secondary)
            .on_press(DebugMessage::StepFramePressed),
        button(text(chrome.step_instr.clone()))
            .style(button::secondary)
            .on_press(DebugMessage::StepInstrPressed),
        button(text(chrome.ppu_viewer.clone()))
            .style(button::secondary)
            .on_press(DebugMessage::PpuOpenPressed),
    ]
    .spacing(12)
    .align_y(iced::Alignment::Center)
    .into()
}

pub(super) fn view_space_row(
    selected_name: &str,
    chrome: &DebuggerChrome,
) -> iced::Element<'static, DebugMessage, iced::Theme, Renderer> {
    use iced::Length;
    use iced::widget::{button, row, text};
    row![
        text(chrome.space.clone()).width(Length::Fixed(150.0)),
        button(text(chrome.prev.clone()))
            .style(button::secondary)
            .on_press(DebugMessage::SpacePrev),
        text(selected_name.to_owned()).size(14).width(Length::Fill),
        button(text(chrome.next.clone()))
            .style(button::secondary)
            .on_press(DebugMessage::SpaceNext),
    ]
    .spacing(12)
    .align_y(iced::Alignment::Center)
    .into()
}

pub(super) fn view_mem_nav(
    mem_input: &str,
    chrome: &DebuggerChrome,
) -> iced::Element<'static, DebugMessage, iced::Theme, Renderer> {
    use iced::Length;
    use iced::widget::{button, row, text, text_input};
    row![
        text(chrome.page.clone()).width(Length::Fixed(150.0)),
        button(text(chrome.prev.clone()))
            .style(button::secondary)
            .on_press(DebugMessage::MemPage(-1)),
        button(text(chrome.next.clone()))
            .style(button::secondary)
            .on_press(DebugMessage::MemPage(1)),
        text_input(&chrome.addr_hex, mem_input)
            .on_input(DebugMessage::MemInputChanged)
            .on_submit(DebugMessage::MemGo)
            // Wide enough for the longest placeholder in either
            // language (JA "アドレス16進" clips at 120).
            .width(150),
        button(text(chrome.go.clone()))
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
    dump_rows: &[DumpRow],
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
        let row_end = hex_row.addr + hex_row.bytes.len() as u32;
        let row_hit = selected_addr.is_some_and(|a| a >= hex_row.addr && a < row_end);
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
        for (i, byte) in hex_row.bytes.iter().enumerate() {
            let addr = hex_row.addr + i as u32;
            let byte = *byte;
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
    regs: &[RegRow],
    reg_target: Option<&'static str>,
    reg_input: &str,
    chrome: &DebuggerChrome,
) -> iced::Element<'static, DebugMessage, iced::Theme, Renderer> {
    use iced::Length;
    use iced::widget::{button, column, row, text, text_input};
    let mid = regs.len().div_ceil(2).min(regs.len());
    let mut halves_row = row![].spacing(16).width(Length::Fill);
    for half in [&regs[..mid], &regs[mid..]] {
        let mut col = column![].spacing(2);
        for row in half.iter() {
            let row_button = button(text(row.text.clone()).size(14).font(iced::Font::MONOSPACE))
                .padding([2, 8])
                .width(Length::Fill);
            col = col.push(
                row_button
                    .style(if Some(row.name) == reg_target {
                        button::secondary
                    } else {
                        button::text
                    })
                    .on_press(DebugMessage::RegSelected(row.name)),
            );
        }
        halves_row = halves_row.push(col);
    }
    let target_text = match reg_target {
        Some(name) => chrome.set_target_label(Some(name)),
        None => chrome.set_target_label(None),
    };
    let edit = row![
        text(target_text).size(14).width(Length::Fixed(140.0)),
        text_input(&chrome.hex, reg_input)
            .on_input(DebugMessage::RegInputChanged)
            .on_submit(DebugMessage::RegSet)
            .width(100),
        button(text(chrome.set.clone()))
            .style(button::secondary)
            .on_press(DebugMessage::RegSet),
    ]
    .spacing(12)
    .align_y(iced::Alignment::Center);
    column![halves_row, edit].spacing(4).into()
}

pub(super) fn view_edit_row(
    select_line: &str,
    selected: Option<(u32, Option<u8>)>,
    edit_input: &str,
    chrome: &DebuggerChrome,
) -> iced::Element<'static, DebugMessage, iced::Theme, Renderer> {
    use iced::Length;
    use iced::widget::{button, row, text, text_input};
    // The target rides on the label and the button: writes always
    // land on the selected row, never on the page anchor.
    // Both lines arrive pre-formatted in the snapshot rows.
    let select_text = select_line.to_owned();
    let write_label = chrome.write_target_label(selected);
    row![
        text(select_text).size(14).width(Length::Fixed(240.0)),
        text_input(&chrome.hex_byte, edit_input)
            .on_input(DebugMessage::EditInputChanged)
            .width(100),
        button(text(write_label))
            .style(button::secondary)
            .on_press(DebugMessage::EditWrite),
        button(text(chrome.watch.clone()))
            .style(button::secondary)
            .on_press(DebugMessage::WatchSet),
        button(text(chrome.freeze.clone()))
            .style(button::secondary)
            .on_press(DebugMessage::EditFreezeSet),
    ]
    .spacing(12)
    .align_y(iced::Alignment::Center)
    .into()
}

pub(super) fn view_confirm_row(
    pending_text: Option<String>,
    chrome: &DebuggerChrome,
) -> Option<iced::Element<'static, DebugMessage, iced::Theme, Renderer>> {
    use iced::Length;
    use iced::widget::{button, row, text};
    pending_text.map(|pending| {
        row![
            text(pending).size(14).width(Length::Fill),
            button(text(chrome.confirm.clone()))
                .style(button::primary)
                .on_press(DebugMessage::ConfirmWrite),
            button(text(chrome.cancel.clone()))
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
    chrome: &DebuggerChrome,
) -> iced::Element<'static, DebugMessage, iced::Theme, Renderer> {
    use iced::widget::{button, row, text, text_input};
    let follow_button =
        button(text(chrome.follow_pc.clone())).on_press(DebugMessage::FollowPcToggle);
    let back_button = button(text(chrome.back_label(back_len))).on_press(DebugMessage::DisBack);
    // The title names the anchor: clicks pin it visibly instead of
    // silently moving the window.
    let title = chrome.disassembly_title(follow_pc, anchor);
    row![
        text(title),
        if follow_pc {
            follow_button.style(button::secondary)
        } else {
            follow_button.style(button::text)
        },
        text_input(&chrome.addr_hex, dis_input)
            .on_input(DebugMessage::DisInputChanged)
            .on_submit(DebugMessage::DisGo)
            // Same JA placeholder width as the memory input above.
            .width(150),
        button(text(chrome.go.clone()))
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
    disasm_lines: &[DisasmRow],
    pinned: Option<u32>,
    chrome: &DebuggerChrome,
) -> iced::Element<'static, DebugMessage, iced::Theme, Renderer> {
    use iced::Length;
    use iced::widget::{button, column, row, scrollable, text};
    let mut dis_col = column![].spacing(2);
    if disasm_lines.is_empty() {
        dis_col = dis_col.push(text(chrome.no_disassembly.clone()).size(14));
        dis_col = dis_col.push(text(chrome.disassembly_hint.clone()).size(14));
    }
    for line in disasm_lines {
        // Marker column is two chars wide for every row. The PC `>` already ships
        // inside the pre-formatted line text; the pin column only marks the anchor.
        let is_pinned = pinned == Some(line.addr);
        let marker = pin_marker(line.is_pc, is_pinned);
        let line_button = button(
            text(format!("{marker}{}", line.text))
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
    watch_text: &str,
    chrome: &DebuggerChrome,
) -> iced::Element<'static, DebugMessage, iced::Theme, Renderer> {
    use iced::widget::{button, row, text};
    row![
        button(text(chrome.watch.clone()))
            .style(button::secondary)
            .on_press(DebugMessage::WatchSet),
        text(watch_text.to_owned())
            .size(14)
            .width(iced::Length::Fill),
        button(text(chrome.clear.clone()))
            .style(button::text)
            .on_press(DebugMessage::WatchClear),
    ]
    .spacing(12)
    .align_y(iced::Alignment::Center)
    .into()
}

pub(super) fn view_freeze_row(
    freeze_text: &str,
    freeze_input: &str,
    chrome: &DebuggerChrome,
) -> iced::Element<'static, DebugMessage, iced::Theme, Renderer> {
    use iced::widget::{button, row, text, text_input};
    row![
        text_input(&chrome.hex_byte, freeze_input)
            .on_input(DebugMessage::FreezeInputChanged)
            .width(100),
        button(text(chrome.freeze.clone()))
            .style(button::secondary)
            .on_press(DebugMessage::FreezeSet),
        text(freeze_text.to_owned())
            .size(14)
            .width(iced::Length::Fill),
        button(text(chrome.clear.clone()))
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
    chrome: &DebuggerChrome,
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
        text(chrome.bookmarks.clone()).size(16),
        row![
            text_input(&chrome.label, bm_input)
                .on_input(DebugMessage::BookmarkInputChanged)
                .on_submit(DebugMessage::BookmarkAdd)
                .width(iced::Length::Fill),
            button(text(chrome.add.clone()))
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
