//! Headless session pieces: failing GPU factory, canned ROM loader,
//! and a `FrontendContext` builder. Production never touches this module.

use std::{
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use nerust_core_traits::audio::AudioBackendRegistry;
use nerust_frontend_viewdefs::viewport::testkit::NullFactory;
use nerust_gui_shell::{context::FrontendContext, load::RomLoader, registry::SystemRegistry};

static TEST_DIR_SERIAL: AtomicU64 = AtomicU64::new(0);

fn unique_test_dir() -> PathBuf {
    let id = TEST_DIR_SERIAL.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("nerust-tao-test-{}-{id}", std::process::id()))
}

/// `RomLoader` with a canned result, recording every attempted path.
/// Clone shares innards, so the context copy and test handle observe each other.
#[derive(Debug, Clone)]
pub(crate) struct FakeRomLoader {
    inner: Arc<FakeRomLoaderInner>,
}

#[derive(Debug)]
struct FakeRomLoaderInner {
    result: Mutex<Result<(), String>>,
    attempted: Mutex<Vec<PathBuf>>,
    /// When set, `load_rom` installs a canned NES ROM instead of no-op success.
    nes_image: Mutex<bool>,
}

impl Default for FakeRomLoader {
    fn default() -> Self {
        Self {
            inner: Arc::new(FakeRomLoaderInner {
                result: Mutex::new(Ok(())),
                attempted: Mutex::new(Vec::new()),
                nes_image: Mutex::new(false),
            }),
        }
    }
}

impl FakeRomLoader {
    pub(crate) fn succeeding() -> Self {
        Self::default()
    }

    /// Loader that installs a canned NES ROM: headless tests that need a
    /// loaded core without touching the filesystem.
    pub(crate) fn loading_nes() -> Self {
        let loader = Self::default();
        *crate::aux::bridge::lock_recovered(&loader.inner.nes_image) = true;
        loader
    }

    pub(crate) fn failing(_detail: &str) -> Self {
        let loader = Self::default();
        // Typed failure path: the detail only arms the failure, the text comes from the source chain.
        *crate::aux::bridge::lock_recovered(&loader.inner.result) =
            Err("typed failure".to_string());
        loader
    }

    pub(crate) fn attempted(&self) -> Vec<PathBuf> {
        crate::aux::bridge::lock_recovered(&self.inner.attempted).clone()
    }
}

impl RomLoader for FakeRomLoader {
    fn load_rom(
        &mut self,
        path: &Path,
        target: &mut dyn nerust_gui_shell::load::RomLoadTarget,
    ) -> Result<(), nerust_gui_shell::load::RomLoaderError> {
        crate::aux::bridge::lock_recovered(&self.inner.attempted).push(path.to_path_buf());
        if *crate::aux::bridge::lock_recovered(&self.inner.nes_image) {
            return load_canned_nes(target);
        }
        crate::aux::bridge::lock_recovered(&self.inner.result)
            .clone()
            .map_err(|_| {
                nerust_gui_shell::load::RomLoaderError::from_session(
                    nerust_gui_shell::session::SessionError::NoCore,
                )
            })
    }
}

/// Canned NES ROM install for headless loaded-core tests.
fn load_canned_nes(
    target: &mut dyn nerust_gui_shell::load::RomLoadTarget,
) -> Result<(), nerust_gui_shell::load::RomLoaderError> {
    use nerust_system_traits::factory::settings::FactorySettingsView;
    let factory = nerust_nes_factory::NesFactory::boxed();
    let view = FactorySettingsView {
        language: Default::default(),
        system_config: factory
            .as_system_defaults()
            .unwrap()
            .default_system_settings(),
    };
    let resolved = factory
        .resolve_load_request(&view, factory.default_load_options())
        .unwrap();
    target
        .set_active_system(factory.system_id().as_ref())
        .unwrap();
    let mut rom = vec![0u8; 16 + 16384 + 8192];
    rom[..6].copy_from_slice(&[b'N', b'E', b'S', 0x1a, 1, 1]);
    rom[16..19].copy_from_slice(&[0x4c, 0x00, 0x80]);
    for vector in [0x3ffa, 0x3ffc, 0x3ffe] {
        rom[16 + vector..18 + vector].copy_from_slice(&[0x00, 0x80]);
    }
    target.load_resolved(
        nerust_system_traits::factory::load::MediaObject::new(None, rom),
        resolved,
    )
}

/// Headless `FrontendContext`: temp settings paths, empty registries.
pub(crate) struct TestContext {
    pub(crate) ctx: FrontendContext,
    _dir: PathBuf,
}

impl TestContext {
    pub(crate) fn with_loader(loader: FakeRomLoader) -> Self {
        let dir = unique_test_dir();
        let ctx = FrontendContext {
            gpu_factory: Rc::new(NullFactory {
                fail_create: true,
                ..Default::default()
            }),
            registry: Arc::new(SystemRegistry::new(Vec::new())),
            rom_loader: Box::new(loader.clone()),
            audio_registry: Arc::new(AudioBackendRegistry::new()),
        };
        Self { ctx, _dir: dir }
    }

    pub(crate) fn settings_root(&self) -> PathBuf {
        self._dir.clone()
    }
}
