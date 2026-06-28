use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use clap::{Arg, Command};
use log::LevelFilter;
use nerust_factory_nes::NesFactory;
use nerust_gui_runtime::{rom::load_rom_path, settings::HostBackendIdentity};
use nerust_gui_shell::{
    context::FrontendContext,
    factory::CoreFactory,
    load::{MediaObject, RomLoader, RomLoaderError, SystemLoadOptions},
    session::SessionHandle,
    session::commands::SessionCommand,
};
use nerust_run_options::RunOptions;
use nerust_screen_video::GpuFactory;
use simple_logger::SimpleLogger;

#[allow(unreachable_code)]
fn create_factory() -> Box<dyn GpuFactory> {
    #[cfg(feature = "wgpu")]
    return Box::new(nerust_backend_wgpu::WgpuFactory);
    #[cfg(feature = "opengl")]
    return Box::new(nerust_backend_opengl::GlFactory);
    #[cfg(not(any(feature = "wgpu", feature = "opengl")))]
    compile_error!("No backend selected. Enable feature 'wgpu' or 'opengl'.");
}

fn parse_cli_args() -> RunOptions {
    let app = Command::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .author(env!("CARGO_PKG_AUTHORS"))
        .about(env!("CARGO_PKG_DESCRIPTION"))
        .arg(Arg::new("filename").help("Rom file name"))
        .arg(
            Arg::new("mmc3-irq-variant")
                .long("mmc3-irq-variant")
                .value_parser(["sharp", "nec"])
                .help("Override mapper 4 MMC3 IRQ behavior"),
        );

    let matches = app.get_matches();
    RunOptions {
        rom_path: matches.get_one::<String>("filename").map(PathBuf::from),
        mmc3_irq_variant: matches.get_one::<String>("mmc3-irq-variant").cloned(),
    }
}

#[allow(unreachable_code)]
fn create_identity() -> HostBackendIdentity {
    #[cfg(feature = "gtk")]
    return HostBackendIdentity::gtk_opengl();
    #[cfg(feature = "tao")]
    return HostBackendIdentity::tao_wgpu();
    #[cfg(not(any(feature = "gtk", feature = "tao")))]
    compile_error!("No frontend selected. Enable feature 'gtk' or 'tao'.");
}

struct LiveRomLoader {
    factory: Arc<dyn CoreFactory>,
    pending_options: Option<SystemLoadOptions>,
}

impl RomLoader for LiveRomLoader {
    fn load_rom(&self, path: &Path, session: &mut SessionHandle) -> Result<(), RomLoaderError> {
        let loaded = load_rom_path(path).map_err(|e| RomLoaderError(e.to_string()))?;
        let (rom_path, data) = loaded.into_parts();
        let media = MediaObject::new(Some(rom_path), data);
        let options = self
            .pending_options
            .clone()
            .unwrap_or_else(|| session.default_load_options());
        let resolved = self
            .factory
            .resolve_load_request(session.settings_snapshot(), options)
            .map_err(|e| RomLoaderError(format!("resolve: {e}")))?;
        session
            .load_resolved(media, resolved)
            .map_err(|e| RomLoaderError(format!("load: {e}")))?;
        let _ = session.run_command(SessionCommand::Resume);
        Ok(())
    }
}

pub fn run() {
    SimpleLogger::new()
        .with_level(LevelFilter::Warn)
        .env()
        .init()
        .unwrap();

    let options = parse_cli_args();
    let gpu_factory = create_factory();
    let core_factory: Arc<dyn CoreFactory> = Arc::new(NesFactory);

    let pending_options = options.mmc3_irq_variant.as_deref().map(|variant| {
        let options_bytes = match variant {
            "sharp" => nerust_factory_nes::MMC3_OPTION_SHARP.to_vec(),
            "nec" => nerust_factory_nes::MMC3_OPTION_NEC.to_vec(),
            _ => Vec::new(),
        };
        SystemLoadOptions { options_bytes }
    });

    let rom_loader = Box::new(LiveRomLoader {
        factory: Arc::clone(&core_factory),
        pending_options,
    });

    let identity = create_identity();

    let ctx = FrontendContext {
        gpu_factory: Rc::from(gpu_factory),
        core_factory,
        rom_loader,
        host_backend_identity: identity,
    };

    #[cfg(all(feature = "gtk", not(clippy)))]
    nerust_gtk::run(ctx, options);
    #[cfg(all(feature = "tao", not(clippy)))]
    nerust_tao::run(ctx, options);
    #[cfg(not(any(feature = "gtk", feature = "tao", clippy)))]
    compile_error!("No frontend selected. Enable feature 'gtk' or 'tao'.");
    #[cfg(clippy)]
    {
        let _ = ctx;
        let _ = options;
    }
}
