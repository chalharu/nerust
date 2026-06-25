mod import_metadata;

#[cfg(target_os = "android")]
mod android;

#[cfg(target_os = "android")]
use std::{
    any::Any,
    backtrace::Backtrace,
    ffi::c_void,
    panic::{self, AssertUnwindSafe},
    sync::Once,
};

#[cfg(target_os = "android")]
use jni::JavaVM;
#[cfg(target_os = "android")]
use jni::sys::{JNI_VERSION_1_6, jint};
#[cfg(target_os = "android")]
use winit::platform::android::activity::AndroidApp;

#[cfg(target_os = "android")]
const ANDROID_LOG_TAG: &str = "Nerust";

#[cfg(target_os = "android")]
fn init_android_logging() {
    android_logger::init_once(
        android_logger::Config::default()
            .with_tag(ANDROID_LOG_TAG)
            .with_max_level(log::LevelFilter::Info),
    );

    static PANIC_HOOK: Once = Once::new();
    PANIC_HOOK.call_once(|| {
        let previous_hook = panic::take_hook();
        panic::set_hook(Box::new(move |panic_info| {
            let thread = std::thread::current();
            let thread_name = thread.name().unwrap_or("unnamed");
            let backtrace = Backtrace::force_capture();
            log::error!("panic on thread '{thread_name}': {panic_info}\nbacktrace:\n{backtrace}");
            previous_hook(panic_info);
        }));
    });
}

#[cfg(target_os = "android")]
fn panic_payload_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_owned()
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub fn android_main(app: AndroidApp) {
    init_android_logging();

    let internal_data_path = app
        .internal_data_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<unavailable>".to_owned());
    log::info!("android_main: starting (internal_data_path={internal_data_path})");

    match panic::catch_unwind(AssertUnwindSafe(|| android::run(app))) {
        Ok(Ok(())) => {
            log::info!("android_main: exited cleanly");
        }
        Ok(Err(error)) => {
            log::error!("android_main: frontend failed: {error:#}");
        }
        Err(payload) => {
            log::error!(
                "android_main: frontend panicked: {}",
                panic_payload_message(payload.as_ref())
            );
        }
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
    init_android_logging();
    log::info!("JNI_OnLoad: registering MainActivity natives");
    let vm = unsafe { JavaVM::from_raw(vm) };
    // Register native method bindings.  This succeeds when the library is loaded
    // via `System.loadLibrary("main")` in the companion-object init because the
    // app classloader is on the call stack at that point.
    if let Err(error) = vm.attach_current_thread(android::register_main_activity_natives) {
        log::error!("JNI_OnLoad: native registration failed: {error:?}");
    } else {
        log::info!("JNI_OnLoad: native registration complete");
    }
    JNI_VERSION_1_6
}

#[cfg(not(target_os = "android"))]
pub fn android_entrypoint_stub() {}
