//! PPU viewer window: live pattern-table image via the shared live flight.
//!
//! The 500ms timer starts `debug_live_images_begin` and polls it to
//! completion, then blits the first image. Generation tracking skips
//! repaints, mirroring the gtk twin's follow tick.

use nerust_gui_shell::debug::LiveImagesPoll;
use windows::{
    Win32::{
        Foundation::*, Graphics::Gdi::*, System::LibraryLoader::*, UI::WindowsAndMessaging::*,
    },
    core::*,
};

use super::app::{close_ppu, rgba_to_bgrx, wide, with_ppu};
use super::fluent;

pub const TIMER_PPU: usize = 12;

pub struct PpuWin {
    pub hwnd: HWND,
    flight: Option<nerust_gui_shell::debug::LiveImagesFlight>,
    generation: u64,
    picture: Option<PpuPicture>,
}

/// Render-ready PPU picture: RGBA converted once at poll time so paint
/// only stretches.
struct PpuPicture {
    width: u32,
    height: u32,
    bgrx: Vec<u8>,
}

impl PpuWin {
    pub fn open() -> Option<Self> {
        unsafe {
            let instance = HINSTANCE(GetModuleHandleW(None).unwrap().0);
            let class = w!("NerustPpu");
            let wcex = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(ppu_proc),
                hInstance: instance,
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap(),
                hbrBackground: fluent::dark_brush(),
                lpszClassName: class,
                ..Default::default()
            };
            let _ = RegisterClassExW(&wcex);
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class,
                w!("PPU viewer"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                760,
                100,
                300,
                320,
                HWND(0),
                HMENU(0),
                instance,
                None,
            );
            if hwnd == HWND::default() {
                return None;
            }
            fluent::apply_chrome(hwnd);
            SetTimer(hwnd, TIMER_PPU, 500, None);
            Some(Self {
                hwnd,
                flight: None,
                generation: 0,
                picture: None,
            })
        }
    }

    pub fn on_timer(&mut self, host: &mut nerust_win::WinHost) {
        if let Some(flight) = self.flight.as_mut() {
            match flight.poll() {
                LiveImagesPoll::Pending => {}
                LiveImagesPoll::Done(batch) => {
                    if let Some(batch) = batch
                        && batch.generation != self.generation
                        && let Some(image) = batch.images.into_iter().next()
                    {
                        self.generation = batch.generation;
                        unsafe {
                            let title = format!("PPU viewer — {}", image.label);
                            let _ = SetWindowTextW(self.hwnd, PCWSTR(wide(&title).as_ptr()));
                        }
                        self.picture = Some(PpuPicture {
                            width: image.width,
                            height: image.height,
                            bgrx: rgba_to_bgrx(image.width, image.height, &image.rgba),
                        });
                        unsafe {
                            let _ = InvalidateRect(self.hwnd, None, FALSE);
                        }
                    }
                    self.flight = None;
                }
            }
            return;
        }
        self.flight = Some(host.session().debug_live_images_begin(true));
    }

    pub fn on_paint(&mut self) {
        unsafe {
            let ps = &mut PAINTSTRUCT::default();
            let hdc = BeginPaint(self.hwnd, ps);
            if let Some(picture) = self.picture.as_ref() {
                let (w, h) = (picture.width, picture.height);
                let scale = if w < 200 { 2 } else { 1 };
                let bgrx = &picture.bgrx;
                let mut bmi = BITMAPINFO::default();
                bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
                bmi.bmiHeader.biWidth = w as i32;
                bmi.bmiHeader.biHeight = -(h as i32);
                bmi.bmiHeader.biPlanes = 1;
                bmi.bmiHeader.biBitCount = 32;
                bmi.bmiHeader.biCompression = BI_RGB.0;
                StretchDIBits(
                    hdc,
                    0,
                    0,
                    (w * scale) as i32,
                    (h * scale) as i32,
                    0,
                    0,
                    w as i32,
                    h as i32,
                    Some(bgrx.as_ptr() as *const _),
                    &bmi,
                    DIB_RGB_COLORS,
                    SRCCOPY,
                );
            }
            let _ = EndPaint(self.hwnd, ps);
        }
    }
}

pub unsafe extern "system" fn ppu_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TIMER => {
            with_ppu(|ppu, host| {
                if hwnd == ppu.hwnd {
                    ppu.on_timer(host);
                }
            });
            LRESULT(0)
        }
        WM_PAINT => {
            with_ppu(|ppu, _| {
                if hwnd == ppu.hwnd {
                    ppu.on_paint();
                }
            });
            LRESULT(0)
        }
        WM_DESTROY => {
            close_ppu(hwnd);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
