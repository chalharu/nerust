#![cfg(target_os = "android")]

mod import_metadata;

mod android;

use std::{ffi::c_void, rc::Rc, sync::Arc};

use jni::{
    JavaVM,
    sys::{JNI_VERSION_1_6, jint},
};
use nerust_core_traits::audio::AudioBackendRegistry;
use nerust_crash_guard::GuardOutcome;
use nerust_gba_factory::GbaFactory;
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

    // The leaf hook only; android_main keeps its own catch_unwind
    // so a panicking frontend logs and returns instead of exiting.
    nerust_crash_guard::install();
}

fn create_system_registry() -> Arc<SystemRegistry> {
    Arc::new(SystemRegistry::new(vec![
        Arc::new(NesFactory),
        Arc::new(GbcFactory),
        Arc::new(GbaFactory),
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

#[unsafe(no_mangle)]
pub fn android_main(app: AndroidApp) {
    init_android_logging();

    let internal_data_path = app
        .internal_data_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<unavailable>".to_owned());
    log::info!("android_main: starting (internal_data_path={internal_data_path})");

    match nerust_crash_guard::run_guarded(
        || {
            let system_registry = create_system_registry();
            let audio_registry = create_audio_registry();
            let gpu_factory = create_gpu_factory();
            android::run(app, system_registry, audio_registry, gpu_factory)
        },
        // Logged by the guard already; nothing to show on a dead loop.
        |_| {},
    ) {
        GuardOutcome::Completed(Ok(())) => {
            log::info!("android_main: exited cleanly");
            std::process::exit(0);
        }
        GuardOutcome::Completed(Err(error)) => {
            log::error!("android_main: frontend failed: {error:#}");
        }
        // Crash path: logged by the guard; return to the JVM instead
        // of exiting, as before.
        GuardOutcome::Crashed(_) => {}
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

    fn minimal_gba_rom() -> Vec<u8> {
        let mut rom = vec![0; 0xC0];
        nerust_gba_core::cartridge::header::finalize_test_gba_rom(&mut rom);
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

        assert_eq!(ids, ["nes", "gbc", "gba"]);
    }

    #[test]
    fn registry_dispatches_nes_and_gbc_media() {
        let registry = create_system_registry();
        let nes = MediaObject::new(None, b"NES\x1a".to_vec());
        let gbc = MediaObject::new(None, minimal_gbc_rom());
        let gba = MediaObject::new(None, minimal_gba_rom());

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
        assert_eq!(
            registry
                .detect(&gba)
                .unwrap()
                .expect("GBA factory")
                .system_id()
                .to_string(),
            "gba"
        );
    }
}
