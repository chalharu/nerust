#[cfg(target_os = "android")]
mod android;

#[cfg(target_os = "android")]
use std::ffi::c_void;

#[cfg(target_os = "android")]
use jni::JavaVM;

#[cfg(target_os = "android")]
use jni::sys::{JNI_VERSION_1_6, jint};

#[cfg(target_os = "android")]
use winit::platform::android::activity::AndroidApp;

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub fn android_main(app: AndroidApp) {
    let _ = simple_logger::SimpleLogger::new()
        .with_level(log::LevelFilter::Warn)
        .env()
        .init();
    if let Err(error) = android::run(app) {
        log::error!("Android frontend failed: {error}");
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
/// # Safety
///
/// Called by the JVM when the native library is loaded.  `vm` must be a valid
/// pointer to the `JavaVM` instance provided by the runtime.
pub unsafe extern "system" fn JNI_OnLoad(
    vm: *mut jni::sys::JavaVM,
    _reserved: *mut c_void,
) -> jint {
    let vm = unsafe { JavaVM::from_raw(vm) };
    // Register native method bindings.  This succeeds when the library is loaded
    // via `System.loadLibrary("main")` in the companion-object init because the
    // app classloader is on the call stack at that point.
    if let Err(error) = vm.attach_current_thread(android::register_main_activity_natives) {
        eprintln!("JNI_OnLoad: native registration failed: {error:?}");
    }
    JNI_VERSION_1_6
}

#[cfg(not(target_os = "android"))]
pub fn android_entrypoint_stub() {}
