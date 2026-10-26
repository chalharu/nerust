//! WinUI bootstrap + application start.
//!
//! Unpackaged WinUI 3 needs `MddBootstrapInitialize` (1.4 line, forward
//! compatible with newer 1.x runtimes) before any XAML call. The
//! bootstrap DLL ships beside the exe for self-contained deployment or
//! resolves from the installed framework. Raw kernel32 imports keep this
//! module free of the `windows` crate so its types never mix with the
//! checked-in XAML bindings.

use windows_core::Result;

use crate::bindings::Microsoft::UI::Xaml::{Application, ApplicationInitializationCallback};

use crate::ui::app::App;

/// Framework version the bindings were generated against (AppSDK 1.4).
const BOOTSTRAP_MAJOR_MINOR: u32 = 0x0001_0004;

#[link(name = "kernel32")]
#[cfg(windows)]
unsafe extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut std::ffi::c_void;
    fn GetProcAddress(module: *mut std::ffi::c_void, name: *const u8) -> *mut std::ffi::c_void;
}

/// Load the AppSDK bootstrapper and initialize the framework.
#[cfg(windows)]
fn bootstrap() -> Result<()> {
    let name: Vec<u16> = "Microsoft.WindowsAppRuntime.Bootstrap.dll"
        .encode_utf16()
        .chain([0])
        .collect();
    let module = unsafe { LoadLibraryW(name.as_ptr()) };
    if module.is_null() {
        return Err(windows_core::Error::new(
            windows_core::HRESULT(-2147467259),
            "win32",
        ));
    }
    let proc = unsafe { GetProcAddress(module, c"MddBootstrapInitialize".as_ptr().cast()) };
    if proc.is_null() {
        return Err(windows_core::Error::new(
            windows_core::HRESULT(-2147467259),
            "win32",
        ));
    }
    type Init = unsafe extern "system" fn(u32, *const u16, u64) -> i32;
    let init: Init = unsafe { std::mem::transmute(proc) };
    // versionTag NULL (stable), minVersion 0 (any 1.4+ framework).
    let hr = unsafe { init(BOOTSTRAP_MAJOR_MINOR, std::ptr::null(), 0) };
    windows_core::HRESULT(hr).ok()
}

pub fn run() -> Result<()> {
    bootstrap()?;
    Application::Start(&ApplicationInitializationCallback::new(|_| {
        App::run_app()?;
        Ok(())
    }))
}
