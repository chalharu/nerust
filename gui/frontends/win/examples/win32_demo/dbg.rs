//! Debugger window: disassembly list plus Pause/Resume/Step.
//!
//! The list shows drained `DebuggerModel` display rows; stepping goes
//! through `SessionHandle::debug_drain_begin`, the same shell drain the
//! gtk twin uses. No view-model here: rows render straight from the
//! display cache.

use nerust_frontend_host::debugger::DebuggerModel;
use nerust_gui_shell::{
    debug::{DebugAction, DebugDrainInput, DrainFlight, DrainPoll},
    session::commands::SessionCommand,
};
use windows::{
    Win32::{
        Foundation::*, Graphics::Gdi::*, System::LibraryLoader::*, UI::WindowsAndMessaging::*,
    },
    core::*,
};

use super::app::{close_dbg, wide, with_dbg};

pub const IDC_LIST: u32 = 101;
pub const IDC_PAUSE: u32 = 102;
pub const IDC_RESUME: u32 = 103;
pub const IDC_STEP: u32 = 104;
pub const IDC_STATUS: u32 = 105;
pub const TIMER_DBG: usize = 11;

pub struct DbgWin {
    pub hwnd: HWND,
    list: HWND,
    status: HWND,
    model: DebuggerModel,
    flight: Option<DrainFlight>,
    prev_rows: Vec<nerust_core_traits::debugger::HexRow>,
}

impl DbgWin {
    pub fn open(host: &mut nerust_win::WinHost) -> Option<Self> {
        unsafe {
            let instance = HINSTANCE(GetModuleHandleW(None).unwrap().0);
            let class = w!("NerustDbg");
            let wcex = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(dbg_proc),
                hInstance: instance,
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap(),
                hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
                lpszClassName: class,
                ..Default::default()
            };
            // Repeated opens reuse the class; ignore already-registered.
            let _ = RegisterClassExW(&wcex);
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class,
                w!("Debugger"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                100,
                100,
                640,
                480,
                HWND(0),
                HMENU(0),
                instance,
                None,
            );
            if hwnd == HWND::default() {
                return None;
            }
            let list = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("LISTBOX"),
                None,
                WS_CHILD | WS_VISIBLE | WS_VSCROLL | WINDOW_STYLE(LBS_NOTIFY as u32),
                8,
                8,
                616,
                380,
                hwnd,
                HMENU(IDC_LIST as isize),
                instance,
                None,
            );
            let status = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                None,
                WS_CHILD | WS_VISIBLE,
                8,
                424,
                616,
                20,
                hwnd,
                HMENU(IDC_STATUS as isize),
                instance,
                None,
            );
            let btn = |id: u32, x: i32, label: PCWSTR| {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("BUTTON"),
                    label,
                    WS_CHILD | WS_VISIBLE | WINDOW_STYLE(BS_PUSHBUTTON as u32),
                    x,
                    396,
                    90,
                    24,
                    hwnd,
                    HMENU(id as isize),
                    instance,
                    None,
                )
            };
            let _ = btn(IDC_PAUSE, 8, w!("Pause"));
            let _ = btn(IDC_RESUME, 104, w!("Resume"));
            let _ = btn(IDC_STEP, 200, w!("Step"));
            SetTimer(hwnd, TIMER_DBG, 500, None);
            let model = DebuggerModel::from_session(host.session());
            let win = Self {
                hwnd,
                list,
                status,
                model,
                flight: None,
                prev_rows: Vec::new(),
            };
            win.refresh_list();
            Some(win)
        }
    }

    pub fn on_timer(&mut self, host: &mut nerust_win::WinHost) {
        if let Some(flight) = self.flight.as_mut() {
            match flight.poll() {
                DrainPoll::Pending => {
                    eprintln!("dbg: drain pending");
                }
                DrainPoll::Done(output) => {
                    eprintln!(
                        "dbg: drain done lines={} paused={}",
                        output.display.disasm_lines.len(),
                        output.display.paused
                    );
                    self.model.apply_drain(output.display, output.reselected);
                    self.prev_rows = self.model.display.dump_rows.clone();
                    self.refresh_list();
                    self.flight = None;
                }
            }
            return;
        }
        let space = host
            .session()
            .debug_spaces()
            .into_iter()
            .next()
            .map(|info| info.id);
        let input = DebugDrainInput {
            space,
            mem_addr: 0,
            dis_addr: None,
            watch: None,
            freeze: None,
            reg_target: None,
            prev_dump_rows: &self.prev_rows,
            need_images: false,
        };
        eprintln!("dbg: start refresh drain");
        self.flight = Some(
            host.session()
                .debug_drain_begin(&[DebugAction::Refresh], &input),
        );
    }

    pub fn on_button(&mut self, host: &mut nerust_win::WinHost, id: u32) {
        match id {
            IDC_PAUSE => {
                let _ = host.dispatch_cmd(SessionCommand::Pause);
            }
            IDC_RESUME => {
                let _ = host.dispatch_cmd(SessionCommand::Resume);
            }
            IDC_STEP => {
                let _ = host.dispatch_cmd(SessionCommand::Pause);
                let space = host
                    .session()
                    .debug_spaces()
                    .into_iter()
                    .next()
                    .map(|info| info.id);
                let input = DebugDrainInput {
                    space,
                    mem_addr: 0,
                    dis_addr: None,
                    watch: None,
                    freeze: None,
                    reg_target: None,
                    prev_dump_rows: &self.prev_rows,
                    need_images: false,
                };
                self.flight = Some(
                    host.session()
                        .debug_drain_begin(&[DebugAction::StepInstr], &input),
                );
            }
            _ => {}
        }
    }

    fn refresh_list(&self) {
        unsafe {
            SendMessageW(self.list, LB_RESETCONTENT, WPARAM(0), LPARAM(0));
            for line in self.model.display.disasm_lines.iter().take(256) {
                let bytes: String = line.bytes[..line.len as usize]
                    .iter()
                    .map(|b| format!("{b:02X}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                let mark = if line.is_pc { ">" } else { " " };
                let text = format!("{mark}{:08X}: {:<12} {}", line.addr, bytes, line.text);
                let mut text16: Vec<u16> = text.encode_utf16().chain([0]).collect();
                text16.pop();
                SendMessageW(
                    self.list,
                    LB_ADDSTRING,
                    WPARAM(0),
                    LPARAM(text16.as_ptr() as isize),
                );
            }
            let state = if self.model.display.paused {
                "PAUSED"
            } else {
                "RUNNING"
            };
            let n = self.model.display.disasm_lines.len();
            let status = format!("{state} | {n} disasm lines");
            let _ = SetWindowTextW(self.status, PCWSTR(wide(&status).as_ptr()));
        }
    }
}

pub unsafe extern "system" fn dbg_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TIMER => {
            with_dbg(|dbg, host| {
                if hwnd == dbg.hwnd {
                    dbg.on_timer(host);
                }
            });
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as u32;
            with_dbg(|dbg, host| {
                if hwnd == dbg.hwnd {
                    dbg.on_button(host, id);
                }
            });
            LRESULT(0)
        }
        WM_DESTROY => {
            close_dbg(hwnd);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
