//! Test-only XTest driver shared with the Tao regression (no GTK dependency).
//! Runtime loading keeps libXtst optional; never use this on a user's display.
use std::ffi::{CStr, c_char, c_int, c_uint, c_ulong, c_void};

unsafe extern "C" {
    fn dlopen(name: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
}

struct Library(*mut c_void);
impl Library {
    fn open(name: &CStr) -> Result<Self, String> {
        // SAFETY: NUL-terminated library name; RTLD_NOW, owned until Drop.
        let handle = unsafe { dlopen(name.as_ptr(), 2) };
        if handle.is_null() {
            Err(format!("optional XTest library unavailable: {name:?}"))
        } else {
            Ok(Self(handle))
        }
    }
    fn symbol(&self, name: &CStr) -> *mut c_void {
        // SAFETY: library remains loaded and the symbol is NUL-terminated.
        let symbol = unsafe { dlsym(self.0, name.as_ptr()) };
        assert!(!symbol.is_null(), "missing X11 test symbol: {name:?}");
        symbol
    }
}
impl Drop for Library {
    fn drop(&mut self) {
        unsafe { dlclose(self.0) };
    }
}

type Display = *mut c_void;
type Key = unsafe extern "C" fn(Display, c_uint, c_int, c_ulong) -> c_int;
type Sync = unsafe extern "C" fn(Display, c_int) -> c_int;
type Keycode = unsafe extern "C" fn(Display, c_ulong) -> u8;
type Focus = unsafe extern "C" fn(Display, c_ulong, c_int, c_ulong) -> c_int;
type Close = unsafe extern "C" fn(Display) -> c_int;
type Open = unsafe extern "C" fn(*const c_char) -> Display;
type Query = unsafe extern "C" fn(Display, *mut c_int, *mut c_int, *mut c_int, *mut c_int) -> c_int;

pub(crate) struct X11 {
    display: Display,
    key: Key,
    sync: Sync,
    keycode: Keycode,
    focus: Focus,
    close: Close,
    held: Vec<u8>,
    _xtst: Library,
    _xlib: Library,
}
impl X11 {
    pub(crate) fn optional() -> Option<Self> {
        match Self::open() {
            Ok(driver) => Some(driver),
            Err(error) => {
                assert!(
                    std::env::var_os("PANECHORD_REQUIRE_XTEST").is_none(),
                    "{error}"
                );
                eprintln!("SKIP physical X11 regression: {error}");
                None
            }
        }
    }
    fn open() -> Result<Self, String> {
        let xlib = Library::open(c"libX11.so.6")?;
        let xtst = Library::open(c"libXtst.so.6")?;
        // SAFETY: the named Xlib/XTest symbols have these C ABI signatures.
        unsafe {
            let open = std::mem::transmute::<*mut c_void, Open>(xlib.symbol(c"XOpenDisplay"));
            let display = open(std::ptr::null());
            if display.is_null() {
                return Err("XOpenDisplay failed (requires isolated Xvfb)".into());
            }
            let driver = Self {
                display,
                key: std::mem::transmute::<*mut c_void, Key>(xtst.symbol(c"XTestFakeKeyEvent")),
                sync: std::mem::transmute::<*mut c_void, Sync>(xlib.symbol(c"XSync")),
                keycode: std::mem::transmute::<*mut c_void, Keycode>(
                    xlib.symbol(c"XKeysymToKeycode"),
                ),
                focus: std::mem::transmute::<*mut c_void, Focus>(xlib.symbol(c"XSetInputFocus")),
                close: std::mem::transmute::<*mut c_void, Close>(xlib.symbol(c"XCloseDisplay")),
                held: Vec::new(),
                _xtst: xtst,
                _xlib: xlib,
            };
            let query = std::mem::transmute::<*mut c_void, Query>(
                driver._xtst.symbol(c"XTestQueryExtension"),
            );
            let (mut event, mut error, mut major, mut minor) = (0, 0, 0, 0);
            if query(display, &mut event, &mut error, &mut major, &mut minor) == 0 {
                return Err("X server lacks optional XTEST extension".into());
            }
            Ok(driver)
        }
    }
    pub(crate) fn focus(&self, window: c_ulong) {
        // SAFETY: caller supplies a mapped window on this isolated X server.
        unsafe {
            (self.focus)(self.display, window, 2, 0);
            (self.sync)(self.display, 0);
        }
    }
    pub(crate) fn press(&mut self, keysym: c_ulong) {
        unsafe {
            let code = (self.keycode)(self.display, keysym);
            assert_ne!(code, 0, "keysym {keysym:x} is not mapped");
            assert_ne!((self.key)(self.display, code.into(), 1, 0), 0);
            self.held.push(code);
            (self.sync)(self.display, 0);
        }
    }
    pub(crate) fn release(&mut self) {
        unsafe {
            for code in self.held.drain(..).rev() {
                assert_ne!((self.key)(self.display, code.into(), 0, 0), 0);
            }
            (self.sync)(self.display, 0);
        }
    }
    pub(crate) fn tap(&mut self, keysym: c_ulong) {
        self.chord(&[], keysym);
    }
    pub(crate) fn chord(&mut self, modifiers: &[c_ulong], keysym: c_ulong) {
        assert!(self.held.is_empty());
        for &modifier in modifiers {
            self.press(modifier);
        }
        self.press(keysym);
        self.release();
    }
    #[allow(dead_code)] // GTK4 tests compose; Tao shares only the physical key driver.
    pub(crate) fn compose(&mut self, text: &str) {
        // GTK IMContextSimple's Unicode entry, not an external Japanese IME.
        for ch in text.chars() {
            self.chord(&[0xffe3, 0xffe1], b'u'.into());
            for digit in format!("{:x}", u32::from(ch)).bytes() {
                self.tap(digit.into());
            }
            self.tap(0xff0d); // Return commits one code point.
        }
    }
}
impl Drop for X11 {
    fn drop(&mut self) {
        // Release held modifiers even when an assertion unwinds.
        self.release();
        unsafe { (self.close)(self.display) };
    }
}
