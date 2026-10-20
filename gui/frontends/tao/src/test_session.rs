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

use crate::test_gpu::FakeGpuFactory;
use nerust_core_traits::audio::AudioBackendRegistry;
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
}

impl Default for FakeRomLoader {
    fn default() -> Self {
        Self {
            inner: Arc::new(FakeRomLoaderInner {
                result: Mutex::new(Ok(())),
                attempted: Mutex::new(Vec::new()),
            }),
        }
    }
}

impl FakeRomLoader {
    pub(crate) fn succeeding() -> Self {
        Self::default()
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
        _target: &mut dyn nerust_gui_shell::load::RomLoadTarget,
    ) -> Result<(), nerust_gui_shell::load::RomLoaderError> {
        crate::aux::bridge::lock_recovered(&self.inner.attempted).push(path.to_path_buf());
        crate::aux::bridge::lock_recovered(&self.inner.result)
            .clone()
            .map_err(|_| {
                nerust_gui_shell::load::RomLoaderError::from_session(
                    nerust_gui_shell::session::SessionError::NoCore,
                )
            })
    }
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
            gpu_factory: Rc::new(FakeGpuFactory),
            registry: Arc::new(SystemRegistry::new(Vec::new())),
            rom_loader: Box::new(loader.clone()),
            audio_registry: Arc::new(AudioBackendRegistry::new()),
        };
        Self { ctx, _dir: dir }
    }

    pub(crate) fn settings_paths(&self) -> nerust_gui_runtime::settings::SettingsPaths {
        nerust_gui_runtime::settings::SettingsPaths {
            settings_file: self._dir.join("settings.toml"),
            central_storage_root: self._dir.join("storage"),
        }
    }
}
