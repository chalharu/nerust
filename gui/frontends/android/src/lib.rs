#![cfg(target_os = "android")]

mod import_metadata;

mod android;

use std::{
    any::Any,
    backtrace::Backtrace,
    ffi::c_void,
    panic::{self, AssertUnwindSafe},
    rc::Rc,
    sync::{Arc, Once},
};

use jni::{
    JavaVM,
    sys::{JNI_VERSION_1_6, jint},
};
use nerust_core_traits::audio::AudioBackendRegistry;
use nerust_gbc_factory::GbcFactory;
use nerust_gui_shell::registry::SystemRegistry;
use nerust_nes_factory::NesFactory;
use nerust_render_traits::renderer::GpuFactory;
use winit::platform::android::activity::AndroidApp;

const ANDROID_LOG_TAG: &str = "Nerust";

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

fn create_system_registry() -> Arc<SystemRegistry> {
    Arc::new(SystemRegistry::new(vec![
        Arc::new(NesFactory),
        Arc::new(GbcFactory),
    ]))
}

fn create_audio_registry() -> Arc<AudioBackendRegistry> {
    let mut reg = AudioBackendRegistry::new();
    reg.register(0, Box::new(nerust_sound_cpal::CpalFactory));
    Arc::new(reg)
}

fn create_gpu_factory() -> Rc<dyn GpuFactory> {
    Rc::new(nerust_render_wgpu::WgpuFactory)
}

fn panic_payload_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_owned()
    }
}

#[unsafe(no_mangle)]
pub fn android_main(app: AndroidApp) {
    init_android_logging();

    let internal_data_path = app
        .internal_data_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<unavailable>".to_owned());
    log::info!("android_main: starting (internal_data_path={internal_data_path})");

    match panic::catch_unwind(AssertUnwindSafe(|| {
        let system_registry = create_system_registry();
        let audio_registry = create_audio_registry();
        let gpu_factory = create_gpu_factory();
        android::run(app, system_registry, audio_registry, gpu_factory)
    })) {
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

#[cfg(test)]
mod tests {
    use nerust_core_traits::factory::load::MediaObject;

    use super::*;

    fn minimal_gbc_rom() -> Vec<u8> {
        let mut rom = vec![0; 0x8000];
        rom[0x0104..0x0134].copy_from_slice(&[
            0xCE, 0xED, 0x66, 0x66, 0xCC, 0x0D, 0x00, 0x0B, 0x03, 0x73, 0x00, 0x83, 0x00, 0x0C,
            0x00, 0x0D, 0x00, 0x08, 0x11, 0x1F, 0x88, 0x89, 0x00, 0x0E, 0xDC, 0xCC, 0x6E, 0xE6,
            0xDD, 0xDD, 0xD9, 0x99, 0xBB, 0xBB, 0x67, 0x63, 0x6E, 0x0E, 0xEC, 0xCC, 0xDD, 0xDC,
            0x99, 0x9F, 0xBB, 0xB9, 0x33, 0x3E,
        ]);
        let mut checksum = 0u8;
        for byte in &rom[0x0134..=0x014C] {
            checksum = checksum.wrapping_sub(*byte).wrapping_sub(1);
        }
        rom[0x014D] = checksum;
        rom
    }

    #[test]
    fn registry_contains_nes_and_gbc_factories() {
        let registry = create_system_registry();
        let ids: Vec<_> = registry
            .all()
            .iter()
            .map(|factory| factory.system_id().to_string())
            .collect();

        assert_eq!(ids, ["nes", "gbc"]);
    }

    #[test]
    fn registry_dispatches_nes_and_gbc_media() {
        let registry = create_system_registry();
        let nes = MediaObject::new(None, b"NES\x1a".to_vec());
        let gbc = MediaObject::new(None, minimal_gbc_rom());

        assert_eq!(
            registry
                .detect(&nes)
                .unwrap()
                .expect("NES factory")
                .system_id()
                .to_string(),
            "nes"
        );
        assert_eq!(
            registry
                .detect(&gbc)
                .unwrap()
                .expect("GBC factory")
                .system_id()
                .to_string(),
            "gbc"
        );
    }
}
