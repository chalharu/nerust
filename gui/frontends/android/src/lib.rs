// Root stays ungated so pure modules compile and test on host.
// Android-bound items below stay target-gated.
#[cfg(any(target_os = "android", test))]
mod bridge_events;
#[cfg(any(target_os = "android", test))]
mod density;

#[cfg(any(target_os = "android", test))]
mod error_dialog_controller;

#[cfg(any(target_os = "android", test))]
mod fetch_seq;

#[cfg(any(target_os = "android", test))]
mod import_metadata;

#[cfg(any(target_os = "android", test))]
mod lifecycle_decision;

#[cfg(any(target_os = "android", test))]
mod saf_decision;

#[cfg(any(target_os = "android", test))]
mod settings_sections;

#[cfg(any(target_os = "android", test))]
// Initializer is already const; the lint misfires on the macro span.
#[allow(clippy::missing_const_for_thread_local)]
mod reentrancy;

#[cfg(target_os = "android")]
mod android;

#[cfg(any(target_os = "android", test))]
use std::sync::Arc;

#[cfg(target_os = "android")]
use std::{ffi::c_void, rc::Rc};

#[cfg(target_os = "android")]
use jni::{
    JavaVM,
    sys::{JNI_VERSION_1_6, jint},
};
#[cfg(target_os = "android")]
use nerust_core_traits::audio::AudioBackendRegistry;
#[cfg(any(target_os = "android", test))]
use nerust_core_traits::factory::CoreFactory;
#[cfg(target_os = "android")]
use nerust_crash_guard::GuardOutcome;
#[cfg(any(target_os = "android", test))]
use nerust_gba_factory::GbaFactory;
#[cfg(any(target_os = "android", test))]
use nerust_gbc_factory::GbcFactory;
#[cfg(any(target_os = "android", test))]
use nerust_gui_shell::registry::SystemRegistry;
#[cfg(any(target_os = "android", test))]
use nerust_nes_factory::NesFactory;
#[cfg(target_os = "android")]
use nerust_render_traits::renderer::PipelineFactory;
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

    // The leaf hook only; android_main keeps its own catch_unwind
    // so a panicking frontend logs and returns instead of exiting.
    static CRASH_HOOK: std::sync::OnceLock<nerust_crash_guard::HookGuard> =
        std::sync::OnceLock::new();
    let _ = CRASH_HOOK.set(nerust_crash_guard::install_default());
}

#[cfg(any(target_os = "android", test))]
fn default_factories() -> Vec<Arc<dyn CoreFactory>> {
    vec![
        Arc::new(NesFactory),
        Arc::new(GbcFactory),
        Arc::new(GbaFactory),
    ]
}

#[cfg(any(target_os = "android", test))]
fn create_system_registry() -> Arc<SystemRegistry> {
    create_system_registry_with(default_factories())
}

/// Inject the factory set from the composition root (`android_main`).
/// Adding a system means extending `default_factories`, not touching call sites.
#[cfg(any(target_os = "android", test))]
fn create_system_registry_with(factories: Vec<Arc<dyn CoreFactory>>) -> Arc<SystemRegistry> {
    Arc::new(SystemRegistry::new(factories))
}

#[cfg(target_os = "android")]
fn create_audio_registry() -> Arc<AudioBackendRegistry> {
    let mut reg = AudioBackendRegistry::new();
    reg.register(0, Box::new(nerust_sound_cpal::CpalFactory));
    Arc::new(reg)
}

#[cfg(target_os = "android")]
fn create_gpu_factory() -> Rc<dyn PipelineFactory> {
    Rc::new(nerust_render_wgpu::WgpuFactory)
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

    match nerust_crash_guard::run_guarded_with(
        || {
            let system_registry = create_system_registry();
            let audio_registry = create_audio_registry();
            let gpu_factory = create_gpu_factory();
            android::run(app, system_registry, audio_registry, gpu_factory)
        },
        nerust_crash_guard::default_sink,
        nerust_crash_guard::capture_backtrace,
    ) {
        GuardOutcome::Completed(Ok(())) => {
            log::info!("android_main: exited cleanly");
            std::process::exit(0);
        }
        GuardOutcome::Completed(Err(error)) => {
            // Init failures are already toasted and the activity finished inside `android::run`.
            // This log is the record for post-loop failures.
            log::error!("android_main: frontend failed: {error:#}");
        }
        // Crash path: logged by the guard; return to the JVM instead
        // of exiting, as before.
        GuardOutcome::Crashed(_) => {}
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
/// Called by the JVM when the native library is loaded.
/// # Safety: `vm` is a valid `JavaVM` pointer from the runtime.
pub unsafe extern "system" fn JNI_OnLoad(
    vm: *mut jni::sys::JavaVM,
    _reserved: *mut c_void,
) -> jint {
    init_android_logging();
    log::info!("JNI_OnLoad: registering MainActivity natives");
    let vm = unsafe { JavaVM::from_raw(vm) };
    // Register native method bindings; succeeds when loaded via `System.loadLibrary("main")`.
    // The app classloader must be on the call stack at that point.
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

    #[test]
    fn registry_injection_builds_reduced_registry() {
        let registry = create_system_registry_with(vec![Arc::new(NesFactory)]);
        let ids: Vec<_> = registry
            .all()
            .iter()
            .map(|factory| factory.system_id().to_string())
            .collect();

        assert_eq!(ids, ["nes"]);
    }
}
