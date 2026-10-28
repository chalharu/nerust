//! Composition-root boot bundle. Native frontends receive an opaque
//! [`Boot`] instead of naming shell, runtime, or render types: the
//! composition root (`nerust` binary) builds it from its
//! [`FrontendContext`](nerust_gui_shell::context::FrontendContext) and
//! hands the built bundle to `tao`/`gtk` entry points. Frontend prod code
//! therefore names only host and `wakui` types.

use std::path::{Path, PathBuf};

use nerust_gui_runtime::settings::{
    BackendPresentationCapabilities, HostBackendCapabilities, HostWindowCapabilities, SettingsPaths,
};
use nerust_gui_shell::{context::FrontendContext, session::SessionHandle};

use crate::application::ApplicationHost;

/// Opaque ROM-loader slot. Frontends store this and load through
/// [`RomLoaderSlot::load`], never naming the shell loader vocabulary.
pub struct RomLoaderSlot(Box<dyn nerust_gui_shell::load::RomLoader>);

impl RomLoaderSlot {
    /// Load a ROM through the owned host. Returns the failure detail;
    /// presenting it is the frontend's job.
    pub fn load(&mut self, host: &mut ApplicationHost, path: &Path) -> Result<(), String> {
        host.load_path(&mut *self.0, path)
            .map_err(|error| error.to_string())
    }
}

/// Built session bundle handed from the composition root to a frontend.
/// All shell/runtime/render types are consumed inside
/// [`boot`]; frontend signatures name only this struct.
pub struct Boot {
    pub host: ApplicationHost,
    pub loader: RomLoaderSlot,
}

/// Backend capabilities for the production frontends. Window chrome is
/// identical on both; only live-tick presentation differs (`Some` enables
/// the vsync-gated path the tao runtime drives, `None` leaves the gtk
/// main loop in charge).
pub fn backend_capabilities(
    remembers_window_size: bool,
    supports_vsync: Option<bool>,
) -> HostBackendCapabilities {
    HostBackendCapabilities {
        window: HostWindowCapabilities {
            remembers_window_size,
            supports_fullscreen_default: true,
            supports_scaling: true,
        },
        presentation: supports_vsync
            .map(|supports_vsync| BackendPresentationCapabilities { supports_vsync }),
    }
}

/// Build the boot bundle from the composition root's context. `settings_root`
/// overrides the platform settings directory (`None` keeps the default);
/// test harnesses pass a temp dir.
pub fn boot(
    ctx: FrontendContext,
    capabilities: HostBackendCapabilities,
    settings_root: Option<PathBuf>,
) -> Result<Boot, String> {
    let FrontendContext {
        gpu_factory,
        registry,
        rom_loader,
        audio_registry,
    } = ctx;
    let session = match settings_root {
        Some(root) => SessionHandle::new_with_settings_paths(
            capabilities,
            registry,
            audio_registry,
            SettingsPaths::from_root(root),
        ),
        None => SessionHandle::new(capabilities, registry, audio_registry),
    }
    .map_err(|error| {
        log::error!("failed to create core: {error}");
        error.to_string()
    })?;
    let mut host = ApplicationHost::new(session);
    // The factory enters the view-definition side once; frontend
    // signatures never name the pipeline type.
    host.set_gpu_factory(gpu_factory);
    Ok(Boot {
        host,
        loader: RomLoaderSlot(rom_loader),
    })
}
