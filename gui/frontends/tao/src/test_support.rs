//! Headless test doubles: fake GPU factory/ROM loader, recording
//! menu sink, and a `FrontendContext` builder on temp settings paths.
//! Production code never touches this module.

use std::{
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use nerust_core_traits::audio::AudioBackendRegistry;
use nerust_gui_shell::{context::FrontendContext, load::RomLoader, registry::SystemRegistry};
use nerust_render_traits::renderer::{GpuRenderer, RendererConfig, RendererError};

use crate::menu::{MenuSink, MenuSync};

static TEST_DIR_SERIAL: AtomicU64 = AtomicU64::new(0);

fn unique_test_dir() -> PathBuf {
    let id = TEST_DIR_SERIAL.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("nerust-tao-test-{}-{id}", std::process::id()))
}

/// `GpuFactory` that never succeeds: host tests run windowless, so
/// renderer creation must fail closed, never touch the GPU.
#[derive(Debug, Default)]
pub(crate) struct FakeGpuFactory;

impl nerust_render_traits::renderer::GpuFactory for FakeGpuFactory {
    fn create_renderer(
        &self,
        _config: &RendererConfig,
        _display_handle: raw_window_handle::RawDisplayHandle,
    ) -> Result<Box<dyn GpuRenderer>, RendererError> {
        Err(RendererError::new(
            "headless",
            Box::new(std::io::Error::other("no GPU in tests")),
        ))
    }
}

/// `RomLoader` with a canned result, recording every attempted path.
/// Clone shares the same innards, so the copy inside `FrontendContext`
/// and the test's handle observe each other.
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

    pub(crate) fn failing(detail: &str) -> Self {
        let loader = Self::default();
        *crate::aux::bridge::lock_recovered(&loader.inner.result) = Err(detail.to_string());
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
            .map_err(nerust_gui_shell::load::RomLoaderError::Load)
    }
}

/// `DialogPorts` with a canned pick, recording shown errors. Clone
/// shares state like the loader above.
#[derive(Debug, Clone, Default)]
pub(crate) struct FakeDialogs {
    inner: Arc<FakeDialogsInner>,
}

#[derive(Debug, Default)]
struct FakeDialogsInner {
    pick: Mutex<Option<PathBuf>>,
    errors: Mutex<Vec<String>>,
}

impl FakeDialogs {
    pub(crate) fn with_pick(pick: Option<PathBuf>) -> Self {
        let dialogs = Self::default();
        *crate::aux::bridge::lock_recovered(&dialogs.inner.pick) = pick;
        dialogs
    }

    pub(crate) fn errors(&self) -> Vec<String> {
        crate::aux::bridge::lock_recovered(&self.inner.errors).clone()
    }
}

impl crate::runtime::host::DialogPorts for FakeDialogs {
    fn pick_rom_file(
        &self,
        _language: nerust_settings_core::model::language::AppLanguage,
        _filters: Vec<(String, Vec<String>)>,
    ) -> Option<PathBuf> {
        crate::aux::bridge::lock_recovered(&self.inner.pick).clone()
    }

    fn show_load_error(
        &self,
        _language: nerust_settings_core::model::language::AppLanguage,
        detail: &str,
    ) {
        crate::aux::bridge::lock_recovered(&self.inner.errors).push(detail.to_string());
    }
}

/// `MenuSink` that records sync snapshots for assertions.
#[derive(Debug, Default)]
pub(crate) struct MenuRecorder {
    updates: Mutex<Vec<MenuSyncSnapshot>>,
}

/// Shared handle to a recorder: the owned copy lives behind the
/// `Box<dyn MenuSink>`, tests read through the clone.
#[derive(Debug, Clone, Default)]
pub(crate) struct MenuRecorderHandle {
    inner: Arc<MenuRecorder>,
}

impl MenuRecorderHandle {
    pub(crate) fn updates(&self) -> Vec<MenuSyncSnapshot> {
        crate::aux::bridge::lock_recovered(&self.inner.updates).clone()
    }

    pub(crate) fn into_box(self) -> Box<dyn MenuSink> {
        Box::new(self)
    }
}

impl MenuSink for MenuRecorderHandle {
    fn update(&mut self, sync: MenuSync<'_>) {
        crate::aux::bridge::lock_recovered(&self.inner.updates).push(MenuSyncSnapshot {
            loaded: sync.loaded,
            paused: sync.paused,
            settings_open: sync.settings_open,
        });
    }

    fn init_for_window(&self, _window: &tao::window::Window) {}

    fn clear_event_handler(&self) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MenuSyncSnapshot {
    pub(crate) loaded: bool,
    pub(crate) paused: bool,
    pub(crate) settings_open: bool,
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

/// Headless `HostState`: temp settings paths, recording menu.
pub(crate) struct TestHost {
    pub(crate) host: crate::runtime::host::HostState,
    pub(crate) menu: MenuRecorderHandle,
    pub(crate) loader: FakeRomLoader,
    pub(crate) dialogs: FakeDialogs,
}

impl TestHost {
    pub(crate) fn new() -> Self {
        Self::with_pick(None)
    }

    pub(crate) fn with_pick(pick: Option<PathBuf>) -> Self {
        Self::with_loader_and_pick(FakeRomLoader::succeeding(), pick)
    }

    pub(crate) fn with_loader_and_pick(loader: FakeRomLoader, pick: Option<PathBuf>) -> Self {
        let support = TestContext::with_loader(loader.clone());
        let paths = support.settings_paths();
        let menu = MenuRecorderHandle::default();
        let dialogs = FakeDialogs::with_pick(pick);
        let host = crate::runtime::host::HostState::new_with_settings_paths(
            support.ctx,
            menu.clone().into_box(),
            Box::new(dialogs.clone()),
            Some(paths),
        )
        .expect("headless host builds on temp settings paths");
        Self {
            host,
            menu,
            loader,
            dialogs,
        }
    }
}
