use core::fmt::{self, Write as _};

#[cfg(unix)]
use std::{
    ffi::c_void,
    mem, ptr,
    sync::atomic::{AtomicBool, Ordering},
};

#[cfg(unix)]
static SIGNAL_HANDLER_ACTIVE: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
const BACKTRACE_DEPTH: usize = 64;

#[cfg(unix)]
pub(crate) fn install() {
    unsafe {
        install_signal(libc::SIGBUS);
    }
}

#[cfg(not(unix))]
pub(crate) fn install() {}

#[cfg(unix)]
unsafe fn install_signal(signal: libc::c_int) {
    let mut action = unsafe { mem::zeroed::<libc::sigaction>() };
    action.sa_sigaction = signal_handler as *const () as libc::sighandler_t;
    action.sa_flags = libc::SA_SIGINFO | libc::SA_RESETHAND;
    unsafe {
        libc::sigemptyset(&mut action.sa_mask);
    }
    if unsafe { libc::sigaction(signal, &action, ptr::null_mut()) } != 0 {
        log::warn!(
            "failed to install {} crash handler: {}",
            signal_name(signal),
            std::io::Error::last_os_error()
        );
    }
}

#[cfg(unix)]
unsafe extern "C" fn signal_handler(
    signal: libc::c_int,
    info: *mut libc::siginfo_t,
    context: *mut c_void,
) {
    if SIGNAL_HANDLER_ACTIVE.swap(true, Ordering::SeqCst) {
        unsafe {
            libc::_exit(128 + signal);
        }
    }

    let mut stderr = StackBuffer::new();
    write_stderr(b"\n");
    let _ = writeln!(stderr, "============== nerust fatal signal ==============");
    let _ = writeln!(stderr, "signal: {} ({signal})", signal_name(signal));
    let _ = writeln!(stderr, "process id: {}", unsafe { libc::getpid() });
    let _ = writeln!(
        stderr,
        "binary: {} {}",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION")
    );
    let _ = writeln!(
        stderr,
        "target: {}/{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    if !info.is_null() {
        let signal_info = unsafe { &*info };
        let _ = writeln!(
            stderr,
            "signal code: {} ({})",
            signal_code_name(signal, signal_info.si_code),
            signal_info.si_code
        );
        let _ = writeln!(stderr, "signal errno: {}", signal_info.si_errno);
        let _ = writeln!(stderr, "fault address: {:#018x}", unsafe {
            signal_info.si_addr() as usize
        });
    }
    if let Some(instruction_pointer) = instruction_pointer(context) {
        let _ = writeln!(stderr, "instruction pointer: {instruction_pointer:#018x}");
    }
    let _ = writeln!(stderr, "native backtrace:");
    write_stderr(stderr.as_bytes());
    dump_native_backtrace();
    write_stderr(b"=================================================\n");

    unsafe {
        libc::_exit(128 + signal);
    }
}

#[cfg(unix)]
fn signal_name(signal: libc::c_int) -> &'static str {
    match signal {
        libc::SIGBUS => "SIGBUS",
        _ => "UNKNOWN",
    }
}

#[cfg(unix)]
fn signal_code_name(signal: libc::c_int, code: libc::c_int) -> &'static str {
    match signal {
        libc::SIGBUS => bus_code_name(code),
        _ => "UNKNOWN",
    }
}

#[cfg(unix)]
fn bus_code_name(code: libc::c_int) -> &'static str {
    match code {
        libc::BUS_ADRALN => "BUS_ADRALN",
        libc::BUS_ADRERR => "BUS_ADRERR",
        libc::BUS_OBJERR => "BUS_OBJERR",
        #[cfg(any(target_os = "linux", target_os = "android"))]
        libc::BUS_MCEERR_AR => "BUS_MCEERR_AR",
        #[cfg(any(target_os = "linux", target_os = "android"))]
        libc::BUS_MCEERR_AO => "BUS_MCEERR_AO",
        _ => "UNKNOWN",
    }
}

#[cfg(all(unix, target_os = "linux", target_arch = "x86_64"))]
fn instruction_pointer(context: *mut c_void) -> Option<usize> {
    if context.is_null() {
        return None;
    }

    unsafe {
        let context = &*(context as *const libc::ucontext_t);
        Some(context.uc_mcontext.gregs[libc::REG_RIP as usize] as usize)
    }
}

#[cfg(all(unix, target_os = "linux", target_arch = "aarch64"))]
fn instruction_pointer(context: *mut c_void) -> Option<usize> {
    if context.is_null() {
        return None;
    }

    unsafe {
        let context = &*(context as *const libc::ucontext_t);
        Some(context.uc_mcontext.pc as usize)
    }
}

#[cfg(all(
    unix,
    not(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64")
    ))
))]
fn instruction_pointer(_: *mut c_void) -> Option<usize> {
    None
}

#[cfg(unix)]
fn dump_native_backtrace() {
    let mut frames = [ptr::null_mut(); BACKTRACE_DEPTH];
    let frame_count = unsafe { backtrace(frames.as_mut_ptr(), BACKTRACE_DEPTH as libc::c_int) };
    if frame_count <= 0 {
        write_stderr(b"  <native backtrace unavailable>\n");
        return;
    }

    unsafe {
        backtrace_symbols_fd(frames.as_ptr(), frame_count, libc::STDERR_FILENO);
    }
}

#[cfg(unix)]
fn write_stderr(bytes: &[u8]) {
    let mut remaining = bytes;
    while !remaining.is_empty() {
        let written = unsafe {
            libc::write(
                libc::STDERR_FILENO,
                remaining.as_ptr() as *const libc::c_void,
                remaining.len(),
            )
        };
        if written <= 0 {
            break;
        }
        remaining = &remaining[written as usize..];
    }
}

#[cfg(unix)]
struct StackBuffer {
    bytes: [u8; 1024],
    len: usize,
}

#[cfg(unix)]
impl StackBuffer {
    const fn new() -> Self {
        Self {
            bytes: [0; 1024],
            len: 0,
        }
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

#[cfg(unix)]
impl fmt::Write for StackBuffer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let bytes = s.as_bytes();
        let available = self.bytes.len().saturating_sub(self.len);
        let written = available.min(bytes.len());
        self.bytes[self.len..self.len + written].copy_from_slice(&bytes[..written]);
        self.len += written;

        if written != bytes.len() {
            return Err(fmt::Error);
        }

        Ok(())
    }
}

#[cfg(unix)]
unsafe extern "C" {
    fn backtrace(buffer: *mut *mut c_void, size: libc::c_int) -> libc::c_int;
    fn backtrace_symbols_fd(buffer: *const *mut c_void, size: libc::c_int, fd: libc::c_int);
}

#[cfg(all(test, unix))]
mod tests {
    use super::{bus_code_name, signal_name};

    #[test]
    fn names_sigbus() {
        assert_eq!(signal_name(libc::SIGBUS), "SIGBUS");
    }

    #[test]
    fn names_bus_code() {
        assert_eq!(bus_code_name(libc::BUS_ADRERR), "BUS_ADRERR");
    }
}
