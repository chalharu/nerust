//! SPIKE-ONLY image thread-path test. Deleted with the spike branch.
//!
//! Drives a real NES core through `spike_images`: pause-gated,
//! two 128x128 indexed images with 4-entry palettes. Content
//! validation lives in nes-core unit tests and the Xvfb eyeball.

use std::sync::Arc;

use nerust_core_traits::{
    audio::AudioBackendRegistry, debugger::InspectError, factory::load::MediaObject,
};
use nerust_gui_runtime::settings::{HostBackendCapabilities, HostWindowCapabilities};

use super::SessionHandle;
use super::test_util::test_view;
use crate::{registry::SystemRegistry, session::RomLoadTarget, test_helpers::test_rom};

fn nes_session() -> SessionHandle {
    use nerust_core_traits::factory::CoreFactory;
    let registry = Arc::new(SystemRegistry::new(vec![Arc::new(
        nerust_nes_factory::NesFactory,
    )]));
    let audio_registry = Arc::new(AudioBackendRegistry::new());
    let capabilities = HostBackendCapabilities {
        window: HostWindowCapabilities {
            remembers_window_size: false,
            supports_fullscreen_default: false,
            supports_scaling: false,
        },
        presentation: None,
    };
    let mut session = SessionHandle::new_ephemeral(capabilities, registry, audio_registry);
    session
        .set_active_system(nerust_nes_factory::NesFactory.system_id().as_ref())
        .expect("spike: NES activation must work");
    let options = session
        .factory()
        .expect("spike: NES factory must resolve")
        .default_load_options();
    let resolved = session
        .factory()
        .expect("spike: NES factory must resolve")
        .resolve_load_request(&test_view(&session), options)
        .expect("spike: synthetic NROM must resolve");
    session
        .load_resolved(MediaObject::new(None, test_rom()), resolved)
        .expect("spike: synthetic NROM must load");
    session
}

#[test]
fn spike_images_require_pause() {
    let session = nes_session();
    // Freshly loaded sessions free-run: images must refuse.
    assert!(matches!(
        session.spike_images(),
        Ok(Err(InspectError::NotPaused)) | Err(_)
    ));
}

#[test]
fn spike_images_return_two_indexed_views() {
    use nerust_core_traits::debugger::ImageFormat;
    let session = nes_session();
    session.spike_pause().expect("spike: pause must work");
    let images = session
        .spike_images()
        .expect("spike: images transport must work")
        .expect("spike: paused images must succeed");
    assert_eq!(images.len(), 2);
    for image in &images {
        assert_eq!((image.width, image.height), (128, 128));
        assert_eq!(image.format, ImageFormat::Indexed { bits_per_pixel: 2 });
        assert_eq!(image.pixels.len(), 128 * 128);
        assert_eq!(image.palette.len(), 4);
        assert!(image.pixels.iter().all(|&p| p < 4));
    }
}
