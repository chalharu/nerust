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
pub unsafe extern "system" fn JNI_OnLoad(
    vm: *mut jni::sys::JavaVM,
    _reserved: *mut c_void,
) -> jint {
    let vm = unsafe { JavaVM::from_raw(vm) };
    match vm.attach_current_thread(|env| android::register_main_activity_natives(env)) {
        Ok(()) => JNI_VERSION_1_6,
        Err(error) => {
            eprintln!("failed to register Android JNI callbacks: {error:?}");
            0
        }
    }
}

#[cfg(not(target_os = "android"))]
pub fn android_entrypoint_stub() {}
