//! Composition-root boot bundle. The composition root builds the session,
//! factory, and loader into an opaque [`Boot`]; native frontends receive
//! only this struct and never name shell, runtime, or render types.

use std::path::PathBuf;

use nerust_gui_runtime::settings::{
    BackendPresentationCapabilities, HostBackendCapabilities, HostWindowCapabilities, SettingsPaths,
};
use nerust_gui_shell::{context::FrontendContext, session::SessionHandle};

use crate::application::ApplicationHost;

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
) -> Result<nerust_frontend_host::bootstrap::Boot, String> {
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
    host.set_rom_loader(rom_loader);
    Ok(nerust_frontend_host::bootstrap::Boot {
        app: Box::new(host),
    })
}
