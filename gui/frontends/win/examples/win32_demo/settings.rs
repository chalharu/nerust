//! Settings dialog: general page bound to the shared `SettingsController`.
//!
//! A modal-shaped child window (parent disabled while open) with a
//! language combo, a sample-rate combo, a fullscreen checkbox, and
//! OK/Cancel. OK stages through the controller mailbox exactly like the
//! gtk twin; the controls are plain Win32, the protocol is shared.

use nerust_frontend_host::settings::{SettingsController, SettingsEvent};
use nerust_gui_viewmodel::settings::dto::ChoiceView;
use nerust_settings_core::model::language::AppLanguage;
use windows::{
    Win32::{
        Foundation::*, Graphics::Gdi::*, System::LibraryLoader::*, UI::Controls::*,
        UI::Input::KeyboardAndMouse::EnableWindow, UI::WindowsAndMessaging::*,
    },
    core::*,
};

use super::app::{close_settings, wide, with_settings};

const IDC_FULLSCREEN: u32 = 203;
const IDC_ERROR: u32 = 204;
const OK_ID: u32 = 1;
const CANCEL_ID: u32 = 2;

pub struct SettingsDlg {
    pub hwnd: HWND,
    pub(crate) parent: HWND,
    controller: SettingsController,
    lang_combo: HWND,
    rate_combo: HWND,
    fs_check: HWND,
    err_label: HWND,
    lang_choices: Vec<ChoiceView<AppLanguage>>,
    rate_choices: Vec<ChoiceView<u32>>,
}

impl SettingsDlg {
    pub fn open(parent: HWND, host: &mut nerust_win::WinHost) -> Option<Self> {
        unsafe {
            let instance = HINSTANCE(GetModuleHandleW(None).unwrap().0);
            let class = w!("NerustSettings");
            let wcex = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(settings_proc),
                hInstance: instance,
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap(),
                hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
                lpszClassName: class,
                ..Default::default()
            };
            let _ = RegisterClassExW(&wcex);
            let controller = host.open_settings();
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class,
                w!("Settings"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                200,
                200,
                420,
                260,
                HWND(0),
                HMENU(0),
                instance,
                None,
            );
            if hwnd == HWND::default() {
                let _ = host.close_settings(&controller);
                return None;
            }
            let label = |y: i32, text: PCWSTR| {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("STATIC"),
                    text,
                    WS_CHILD | WS_VISIBLE,
                    12,
                    y,
                    120,
                    22,
                    hwnd,
                    HMENU(0),
                    instance,
                    None,
                )
            };
            let _ = label(14, w!("Language:"));
            let _ = label(48, w!("Sample rate:"));
            let lang_combo = combo(hwnd, instance, 140, 12);
            let rate_combo = combo(hwnd, instance, 140, 46);
            let fs_check = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("BUTTON"),
                w!("Fullscreen"),
                WS_CHILD | WS_VISIBLE | WINDOW_STYLE(BS_AUTOCHECKBOX as u32),
                12,
                82,
                200,
                22,
                hwnd,
                HMENU(IDC_FULLSCREEN as isize),
                instance,
                None,
            );
            let err_label = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                None,
                WS_CHILD | WS_VISIBLE,
                12,
                112,
                380,
                44,
                hwnd,
                HMENU(IDC_ERROR as isize),
                instance,
                None,
            );
            let button = |id: u32, x: i32, text: PCWSTR| {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("BUTTON"),
                    text,
                    WS_CHILD | WS_VISIBLE | WINDOW_STYLE(BS_PUSHBUTTON as u32),
                    x,
                    170,
                    90,
                    26,
                    hwnd,
                    HMENU(id as isize),
                    instance,
                    None,
                )
            };
            let _ = button(1, 200, w!("OK"));
            let _ = button(2, 300, w!("Cancel"));

            let gv = controller.vm.general.view.get();
            let av = controller.vm.audio.view.get();
            let vv = controller.vm.video.view.get();
            for choice in &gv.language_choices {
                combo_add(lang_combo, &choice.label);
            }
            combo_select(lang_combo, position(&gv.language_choices, &gv.language));
            for choice in &av.sample_rate_choices {
                combo_add(rate_combo, &choice.label);
            }
            combo_select(
                rate_combo,
                position(&av.sample_rate_choices, &av.sample_rate),
            );
            if vv.fullscreen_default {
                SendMessageW(
                    fs_check,
                    BM_SETCHECK,
                    WPARAM(BST_CHECKED.0 as usize),
                    LPARAM(0),
                );
            }
            let _ = EnableWindow(parent, false);
            Some(Self {
                hwnd,
                parent,
                controller,
                lang_combo,
                rate_combo,
                fs_check,
                err_label,
                lang_choices: gv.language_choices,
                rate_choices: av.sample_rate_choices,
            })
        }
    }

    fn on_ok(&mut self, host: &mut nerust_win::WinHost) -> bool {
        eprintln!("settings: on_ok begin");
        if let Some(choice) = combo_choice(self.lang_combo, &self.lang_choices) {
            let _ = self.controller.update(SettingsEvent::SetLanguage(choice));
        }
        if let Some(choice) = combo_choice(self.rate_combo, &self.rate_choices) {
            let _ = self.controller.update(SettingsEvent::SetSampleRate(choice));
        }
        let checked = unsafe { SendMessageW(self.fs_check, BM_GETCHECK, WPARAM(0), LPARAM(0)).0 }
            == BST_CHECKED.0 as isize;
        let _ = self
            .controller
            .update(SettingsEvent::ToggleFullscreenDefault(checked));
        let _ = self.controller.update(SettingsEvent::Submit);
        eprintln!(
            "settings: submit done error={:?}",
            self.controller.error_message
        );
        match host.close_settings(&self.controller) {
            Ok(true) => true,
            _ => {
                let banner = self.controller.validation_banner();
                let message = banner.headline.as_deref().unwrap_or("invalid settings");
                unsafe {
                    let _ = SetWindowTextW(self.err_label, PCWSTR(wide(message).as_ptr()));
                }
                false
            }
        }
    }

    fn on_cancel(&mut self, host: &mut nerust_win::WinHost) {
        let _ = host.close_settings(&self.controller);
    }
}

fn combo(hwnd: HWND, instance: HINSTANCE, x: i32, y: i32) -> HWND {
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("COMBOBOX"),
            None,
            WS_CHILD | WS_VISIBLE | WS_VSCROLL | WINDOW_STYLE(CBS_DROPDOWNLIST as u32),
            x,
            y,
            240,
            200,
            hwnd,
            HMENU(0),
            instance,
            None,
        )
    }
}

fn combo_add(combo: HWND, label: &str) {
    unsafe {
        SendMessageW(
            combo,
            CB_ADDSTRING,
            WPARAM(0),
            LPARAM(wide(label).as_ptr() as isize),
        );
    }
}

fn combo_select(combo: HWND, index: Option<usize>) {
    if let Some(i) = index {
        unsafe {
            SendMessageW(combo, CB_SETCURSEL, WPARAM(i), LPARAM(0));
        }
    }
}

fn position<T: Clone + PartialEq + Eq>(choices: &[ChoiceView<T>], value: &T) -> Option<usize> {
    choices.iter().position(|choice| &choice.value == value)
}

fn combo_choice<T: Clone + PartialEq + Eq>(
    combo: HWND,
    choices: &[ChoiceView<T>],
) -> Option<ChoiceView<T>> {
    let selected = unsafe { SendMessageW(combo, CB_GETCURSEL, WPARAM(0), LPARAM(0)).0 };
    (selected >= 0)
        .then(|| choices.get(selected as usize).cloned())
        .flatten()
}

/// Verification hook: drive the real OK path headlessly (control reads,
/// controller events, mailbox submit, host apply). Used by --settings-apply.
/// DestroyWindow runs outside the APP borrow: it pumps WM_DESTROY
/// synchronously, which must not reenter a held borrow.
pub fn drive_submit() -> bool {
    let (applied, hwnd) = super::app::with_settings(|dlg, host| {
        let applied = dlg.on_ok(host);
        (applied, dlg.hwnd)
    })
    .unwrap_or((false, HWND::default()));
    if applied && hwnd != HWND::default() {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
    applied
}

pub unsafe extern "system" fn settings_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as u32;
            match id {
                OK_ID => {
                    // IDOK: apply on success, otherwise keep editing.
                    let applied = with_settings(|dlg, host| {
                        if hwnd == dlg.hwnd {
                            dlg.on_ok(host)
                        } else {
                            false
                        }
                    })
                    .unwrap_or(false);
                    if applied {
                        unsafe {
                            let _ = DestroyWindow(hwnd);
                        }
                    }
                    LRESULT(0)
                }
                CANCEL_ID => {
                    with_settings(|dlg, host| {
                        if hwnd == dlg.hwnd {
                            dlg.on_cancel(host);
                        }
                    });
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                    LRESULT(0)
                }
                _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
            }
        }
        WM_CLOSE => {
            with_settings(|dlg, host| {
                if hwnd == dlg.hwnd {
                    dlg.on_cancel(host);
                }
            });
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            close_settings(hwnd);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
