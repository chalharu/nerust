#[cfg(target_os = "android")]
mod android;

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

#[cfg(not(target_os = "android"))]
pub fn android_entrypoint_stub() {}
